use handl_kit_canonical::Kit;
use handl_platform::{
    AstNode, CapabilityRequest, DefaultHost, PlatformError,
};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use serde_json::{json, Value};

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

    // Dispatch: run/eval route through the host's plugin system like
    // every other operation (ADR 0040), dispatching an `Eval`
    // `CapabilityRequest` to the `BasicEvaluator` loaded by the kit.
    // Other commands go through the `cli` capability as before.
    match command {
        "run" => run_file(command_args.first().copied(), &mut host),
        "eval" => eval_expression(command_args.first().copied(), &mut host),
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

fn run_file(path: Option<&str>, host: &mut DefaultHost) -> Result<(), PlatformError> {
    let path = path.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl run <file.hnl>".into())
    })?;

    let source = std::fs::read_to_string(path).map_err(|e| {
        PlatformError::InvalidMetadata(format!("failed to read {path}: {e}"))
    })?;

    let kit = Kit::new();
    let stmts = kit.parse_module_to_ast(&source)?;
    let value = eval_stmts_via_plugins(&stmts, host)?;
    print_value(&value);
    Ok(())
}

fn eval_expression(expr: Option<&str>, host: &mut DefaultHost) -> Result<(), PlatformError> {
    let expr_str = expr.ok_or_else(|| {
        PlatformError::InvalidMetadata("usage: hnl eval <expression>".into())
    })?;

    let kit = Kit::new();
    let ast = kit.parse_to_ast(expr_str)?;
    let value = eval_ast_via_plugins(&ast, host)?;
    print_value(&value);
    Ok(())
}

/// Dispatch an `Eval` `CapabilityRequest` for a single [`AstNode`] through the
/// host's plugin system.
///
/// The `BasicEvaluator` plugin (registered by the kit) handles the `"eval"`
/// capability: it reads `payload["kind"]` as the AST kind and treats the rest
/// of `payload` as the `AstNode.payload`. We merge the kind into the node's
/// payload so the evaluator can route to the matching `Behavior<Eval>` rule.
///
/// Returns the evaluated `serde_json::Value` on success, or an error when no
/// rule matches (e.g. unsupported kind) or a rule fails (e.g. unbound
/// variable, unknown function).
fn eval_ast_via_plugins(ast: &AstNode, host: &mut DefaultHost) -> Result<Value, PlatformError> {
    let mut payload = ast.payload.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("kind".into(), json!(ast.kind));
    } else {
        // Payload isn't an object — wrap it so the evaluator still sees a
        // `kind`. (All real AST payloads are objects; this is defensive.)
        payload = json!({ "kind": ast.kind, "value": payload });
    }

    let resp = host.run(&CapabilityRequest {
        capability: "eval".into(),
        payload,
    })?;
    Ok(resp.0)
}

/// Evaluate a sequence of statement [`AstNode`]s through the plugin system,
/// returning the last statement's value.
///
/// Each statement is dispatched independently (the `BasicEvaluator` builds a
/// fresh `EvalContext` per dispatch), so this only succeeds when every
/// statement is self-contained.
fn eval_stmts_via_plugins(
    stmts: &[AstNode],
    host: &mut DefaultHost,
) -> Result<Value, PlatformError> {
    let mut last = Value::Null;
    for stmt in stmts {
        last = eval_ast_via_plugins(stmt, host)?;
    }
    Ok(last)
}

/// Print a `serde_json::Value` the way the CLI presents eval results: bare
/// strings are printed without quotes, everything else as pretty-printed JSON.
fn print_value(value: &Value) {
    if let Some(s) = value.as_str() {
        println!("{s}");
    } else {
        println!("{}", serde_json::to_string_pretty(value).unwrap_or_default());
    }
}

fn gen_grammar(_kit_name: &str, output_path: Option<&str>) -> Result<(), PlatformError> {
    let kit = Kit::new();
    let rules = kit.grammar_rules();
    let grammar = handl_host_tooling::generate_textmate(
        &rules,
        kit.scope_name(),
        &kit.file_extensions(),
    );

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
