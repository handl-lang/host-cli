use handl_kit_canonical::{Kit, KitNode, KitValue};
use handl_platform::{CallableLookup, CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use handl_plugin_vm::{Expr, HandlerClause, Pattern};
use handl_plugin_vm_backend_rust_interpreter::{CanonicalEval, CanonicalVm};
use handl_plugin_vm_common::{DynamicRuntime, ModuleRuntime};
use handl_plugin_vm_values_json::{CanonicalContext, NonFunctionValue};
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let args = std::env::args().collect::<Vec<_>>();
    let (applier_id, config_path, kit_name, command, command_args) = parse_args(&args)?;

    // Handle gen-grammar before loading the plugin host — it only needs the kit.
    if command == "gen-grammar" {
        return gen_grammar(&kit_name, command_args.first().copied());
    }

    // Load the kit via the plugin system. This registers ModuleRuntime,
    // plugin-lib-* (which contribute modules to it), and all language
    // operations (lexer, parser, formatter, etc.).
    let mut host = DefaultHost::new();
    host.register_plugin(BootstrapPlugin::new(config_path, kit_name, applier_id));
    host.register_plugin(BasicCli::new());
    host.load()?;

    // Validate the composition before running any command.
    if let Err(e) = host.resolve_composition() {
        eprintln!("composition error: {e}");
        return Err(PlatformError::Plugin(e.to_string()));
    }

    // Dispatch: run/eval use the VM directly with the kit's ModuleRuntime;
    // other commands go through the capability system.
    match command {
        "run" => run_file(command_args.first().copied(), &host),
        "eval" => eval_expression(command_args.first().copied(), &host),
        _ => {
            let resp = host.run(&CapabilityRequest {
                capability: "cli".into(),
                payload: json!({
                    "command": command,
                    "args": command_args,
                }),
            })?;

            if let Some(s) = resp.0.as_str() {
                println!("{s}");
            }
            Ok(())
        }
    }
}

/// Extracts the ModuleRuntime from the host's plugin registry and wraps it
/// in a DynamicRuntime with standard resource limits.
fn build_runtime(host: &DefaultHost) -> Result<DynamicRuntime, PlatformError> {
    let modules = host
        .plugins()
        .get_plugin::<ModuleRuntime>()
        .cloned()
        .ok_or_else(|| {
            PlatformError::Plugin("ModuleRuntime not found in kit's plugin registry".into())
        })?;
    Ok(DynamicRuntime::new(modules)
        .with_max_depth(256)
        .with_max_calls(100_000))
}

fn new_context(runtime: &DynamicRuntime) -> CanonicalContext {
    let mut ctx = CanonicalContext::new();
    if let Some(max) = runtime.max_depth() {
        ctx.set_max_depth(max);
    }
    if let Some(max) = runtime.max_calls() {
        ctx.set_max_calls(max);
    }
    ctx.set_eval(Box::new(CanonicalEval::new(
        runtime.clone(),
        runtime.max_depth(),
        runtime.max_calls(),
    )));
    ctx
}

fn run_file(path: Option<&str>, host: &DefaultHost) -> Result<(), PlatformError> {
    let path = path.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl run <file.hnl>".into())
    })?;

    let source = std::fs::read_to_string(path).map_err(|e| {
        PlatformError::InvalidMetadata(format!("failed to read {path}: {e}"))
    })?;

    let kit = Kit::new();
    let stmts = kit.parse_module(&source)?;

    let runtime = build_runtime(host)?;
    let mut ctx = new_context(&runtime);
    let vm = CanonicalVm::new(runtime);

    let mut last_value = NonFunctionValue::new(serde_json::Value::Null);

    for stmt in &stmts {
        let expr = kitnode_to_expr(stmt)?;
        last_value = vm.eval(&expr, &mut ctx).map_err(|e| {
            PlatformError::Plugin(format!("evaluation error: {e:?}"))
        })?;
    }

    // Print the last expression's value.
    let inner = last_value.into_inner();
    if let Some(s) = inner.as_str() {
        println!("{s}");
    } else {
        println!("{}", serde_json::to_string_pretty(&inner).unwrap_or_default());
    }

    Ok(())
}

fn eval_expression(expr: Option<&str>, host: &DefaultHost) -> Result<(), PlatformError> {
    let expr_str = expr.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl eval <expression>".into())
    })?;

    let kit = Kit::new();
    let node = kit.parse(expr_str)?;

    let runtime = build_runtime(host)?;
    let mut ctx = new_context(&runtime);
    let vm = CanonicalVm::new(runtime);

    let expr = kitnode_to_expr(&node)?;
    let value = vm.eval(&expr, &mut ctx).map_err(|e| {
        PlatformError::Plugin(format!("evaluation error: {e:?}"))
    })?;

    let inner = value.into_inner();
    if let Some(s) = inner.as_str() {
        println!("{s}");
    } else {
        println!("{}", serde_json::to_string_pretty(&inner).unwrap_or_default());
    }

    Ok(())
}

fn kitnode_to_expr(node: &KitNode) -> Result<Expr<NonFunctionValue>, PlatformError> {
    Ok(match node {
        KitNode::Literal(v) => Expr::Value(kitvalue_to_nonfunction(v)),
        KitNode::Var(v) => Expr::Var(v.name.clone()),
        KitNode::Call(c) => Expr::call(c.name.clone(), c.args.iter().map(kitnode_to_expr).collect::<Result<Vec<_>, _>>()?),
        KitNode::Apply(a) => Expr::Apply {
            target: Box::new(kitnode_to_expr(&a.target)?),
            args: a.args.iter().map(kitnode_to_expr).collect::<Result<Vec<_>, _>>()?,
        },
        KitNode::Index(i) => {
            let target = Box::new(kitnode_to_expr(&i.target)?);
            match &i.index {
                KitNode::Literal(KitValue::String(s)) => {
                    Expr::FieldAccess { target, field: s.clone() }
                }
                _ => Expr::Index { target, index: Box::new(kitnode_to_expr(&i.index)?) },
            }
        },
        KitNode::Pipe(p) => Expr::Pipe(
            Box::new(kitnode_to_expr(&p.left)?),
            Box::new(kitnode_to_expr(&p.right)?),
        ),
        KitNode::Array(a) => Expr::Array(a.elements.iter().map(kitnode_to_expr).collect::<Result<Vec<_>, _>>()?),
        KitNode::Object(o) => Expr::Object(o.entries.iter().map(|(k, v)| Ok((k.clone(), kitnode_to_expr(v)?))).collect::<Result<Vec<_>, _>>()?),
        KitNode::Let(l) => Expr::Assign {
            target: Pattern::Identifier(l.name.clone()),
            value: Box::new(kitnode_to_expr(&l.value)?),
        },
        KitNode::Const(l) => Expr::Assign {
            target: Pattern::Identifier(l.name.clone()),
            value: Box::new(kitnode_to_expr(&l.value)?),
        },
        KitNode::Assign(a) => Expr::Assign {
            target: kitnode_to_pattern(&a.target)?,
            value: Box::new(kitnode_to_expr(&a.value)?),
        },
        KitNode::Block(a) => {
            // Evaluate all statements in sequence, return last.
            // For simplicity, wrap as a sequence of assigns.
            if a.elements.is_empty() {
                Expr::Value(NonFunctionValue::new(serde_json::Value::Null))
            } else {
                let mut result = kitnode_to_expr(&a.elements[0])?;
                for elem in &a.elements[1..] {
                    let _ = kitnode_to_expr(elem)?;
                    result = kitnode_to_expr(elem)?;
                }
                result
            }
        }
        KitNode::Match(m) => Expr::Match {
            subject: Box::new(kitnode_to_expr(&m.subject)?),
            arms: m.arms.iter().map(|arm| {
                match &arm.pattern {
                    KitNode::Literal(_) => {
                        let pattern_expr = kitnode_to_expr(&arm.pattern)
                            .unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Null)));
                        let guard = Expr::call("cmp.eq", vec![
                            Expr::Var("__match_subject".into()),
                            pattern_expr,
                        ]);
                        let orig_guard = arm.guard.as_ref().map(|g| kitnode_to_expr(g)
                            .unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Bool(true)))));
                        let combined_guard = if let Some(og) = orig_guard {
                            Expr::call("bool.and", vec![guard, og])
                        } else {
                            guard
                        };
                        (Pattern::Identifier("__match_subject".into()), Some(combined_guard),
                         kitnode_to_expr(&arm.body).unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Null))))
                    }
                    _ => {
                        let pattern = kitnode_to_pattern(&arm.pattern)
                            .unwrap_or(Pattern::Identifier("_".into()));
                        let guard = arm.guard.as_ref().map(|g| kitnode_to_expr(g)
                            .unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Bool(true)))));
                        let body = kitnode_to_expr(&arm.body)
                            .unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Null)));
                        (pattern, guard, body)
                    }
                }
            }).collect(),
        },
        KitNode::Handle(h) => Expr::Handle {
            body: Box::new(kitnode_to_expr(&h.expr)?),
            handlers: h.handlers.iter().map(|handler| {
                HandlerClause {
                    op_name: handler.op_name.clone(),
                    params: handler.params.clone(),
                    continuation_name: handler.continuation_name.clone(),
                    body: kitnode_to_expr(&handler.body).unwrap_or(Expr::Value(NonFunctionValue::new(serde_json::Value::Null))),
                }
            }).collect(),
        },
        KitNode::Function(f) => Expr::Function {
            type_params: Vec::new(),
            params: f.params.iter().map(|p| (Pattern::Identifier(p.clone()), None)).collect(),
            ret: None,
            body: Box::new(kitnode_to_expr(&f.body)?),
        },
        KitNode::Import(_) | KitNode::Export(_) | KitNode::Module(_) => {
            // Skip declarations at the VM level for now.
            Expr::Value(NonFunctionValue::new(serde_json::Value::Null))
        }
    })
}

fn kitnode_to_pattern(node: &KitNode) -> Result<Pattern, PlatformError> {
    Ok(match node {
        KitNode::Var(v) if v.name == "_" => Pattern::Identifier("_".into()),
        KitNode::Var(v) => Pattern::Identifier(v.name.clone()),
        KitNode::Literal(KitValue::Null) => Pattern::Identifier("_".into()),
        _ => return Err(PlatformError::Plugin(format!(
            "unsupported pattern node: {node:?}"
        ))),
    })
}

fn kitvalue_to_nonfunction(v: &KitValue) -> NonFunctionValue {
    NonFunctionValue::new(match v {
        KitValue::Null => serde_json::Value::Null,
        KitValue::Number(n) => serde_json::json!(n),
        KitValue::String(s) => serde_json::json!(s),
        KitValue::Bool(b) => serde_json::json!(b),
    })
}

fn gen_grammar(_kit_name: &str, output_path: Option<&str>) -> Result<(), PlatformError> {
    let kit = Kit::new();
    let info = handl_host_tooling::GrammarInfoProvider::grammar_info(&kit);
    let grammar = handl_host_tooling::generate_textmate(&info);

    let json = serde_json::to_string_pretty(&grammar)
        .map_err(|e| PlatformError::InvalidMetadata(e.to_string()))?;

    match output_path {
        Some(path) => {
            std::fs::write(path, &json).map_err(|e| {
                PlatformError::InvalidMetadata(format!("failed to write {path}: {e}"))
            })?;
            eprintln!("Generated TextMate grammar: {path}");
        }
        None => {
            println!("{json}");
        }
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(String, String, String, &str, Vec<&str>), PlatformError> {
    let mut applier_id = "applier.default".to_string();
    let mut config_path = ".handl.toml".to_string();
    let mut kit_name = std::env::var("HANDL_KIT").unwrap_or_else(|_| "hello".to_string());
    let mut command = "";
    let mut command_args = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--applier" => {
                i += 1;
                applier_id = args
                    .get(i)
                    .ok_or_else(|| {
                        PlatformError::InvalidMetadata("missing --applier value".into())
                    })?
                    .to_string();
            }
            "--config" => {
                i += 1;
                config_path = args
                    .get(i)
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --config value".into()))?
                    .to_string();
            }
            "--kit" => {
                i += 1;
                kit_name = args
                    .get(i)
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --kit value".into()))?
                    .to_string();
            }
            "--" => {
                // Separator: following args are the command and its args.
            }
            _ if command.is_empty() => {
                command = &args[i];
            }
            _ => {
                command_args.push(args[i].as_str());
            }
        }
        i += 1;
    }
    if command.is_empty() {
        return Err(PlatformError::InvalidMetadata("missing command".into()));
    }
    Ok((applier_id, config_path, kit_name, command, command_args))
}
