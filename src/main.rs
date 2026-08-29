use handl_kit_canonical::{Kit, KitNode, KitValue};
use handl_kit_cli::{CliHost, CliLimits};
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use handl_plugin_vm::{Expr, Pattern};
use handl_plugin_vm_backend_rust_interpreter::CanonicalVm;
use handl_plugin_vm_values_json::NonFunctionValue;
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let args = std::env::args().collect::<Vec<_>>();
    let (applier_id, config_path, kit_name, command, command_args) = parse_args(&args)?;

    // Handle gen-grammar before loading the plugin host — it only needs the kit.
    if command == "gen-grammar" {
        return gen_grammar(&kit_name, command_args.first().copied());
    }

    // Handle run command — uses the VM directly, bypassing the plugin system.
    if command == "run" {
        return run_file(command_args.first().copied());
    }

    host.register_plugin(BootstrapPlugin::new(config_path, kit_name, applier_id));
    host.register_plugin(BasicCli::new());
    host.load()?;

    // Validate the composition before running any command.
    if let Err(e) = host.resolve_composition() {
        eprintln!("composition error: {e}");
        return Err(PlatformError::Plugin(e.to_string()));
    }

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

fn run_file(path: Option<&str>) -> Result<(), PlatformError> {
    let path = path.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl run <file.hnl>".into())
    })?;

    let source = std::fs::read_to_string(path).map_err(|e| {
        PlatformError::InvalidMetadata(format!("failed to read {path}: {e}"))
    })?;

    let kit = Kit::new();
    let stmts = kit.parse_module(&source)?;

    let cli_host = CliHost::new(CliLimits::default());
    let mut ctx = cli_host.new_context();
    let vm = CanonicalVm::new(cli_host);

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

fn kitnode_to_expr(node: &KitNode) -> Result<Expr<NonFunctionValue>, PlatformError> {
    Ok(match node {
        KitNode::Literal(v) => Expr::Value(kitvalue_to_nonfunction(v)),
        KitNode::Var(v) => Expr::Var(v.name.clone()),
        KitNode::Call(c) => Expr::call(c.name.clone(), c.args.iter().map(kitnode_to_expr).collect::<Result<Vec<_>, _>>()?),
        KitNode::Apply(a) => Expr::Apply {
            target: Box::new(kitnode_to_expr(&a.target)?),
            args: a.args.iter().map(kitnode_to_expr).collect::<Result<Vec<_>, _>>()?,
        },
        KitNode::Index(i) => Expr::Index {
            target: Box::new(kitnode_to_expr(&i.target)?),
            index: Box::new(kitnode_to_expr(&i.index)?),
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
            arms: Vec::new(),
        },
        KitNode::Handle(h) => Expr::Handle {
            body: Box::new(kitnode_to_expr(&h.expr)?),
            handlers: Vec::new(),
        },
        KitNode::Import(_) | KitNode::Export(_) | KitNode::Module(_) => {
            // Skip declarations at the VM level for now.
            Expr::Value(NonFunctionValue::new(serde_json::Value::Null))
        }
    })
}

fn kitnode_to_pattern(node: &KitNode) -> Result<Pattern, PlatformError> {
    Ok(match node {
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
