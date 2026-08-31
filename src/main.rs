use handl_kit_canonical::Kit;
use handl_language::LangError;
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use serde_json::{Value, json};

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

    // Dispatch: run/eval use the data-driven Kit directly (ADR 0044).
    // Other commands go through the `cli` capability as before.
    match command {
        "run" => run_file(command_args.first().copied()),
        "eval" => eval_expression(command_args.first().copied()),
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

fn run_file(path: Option<&str>) -> Result<(), PlatformError> {
    let path =
        path.ok_or_else(|| PlatformError::InvalidMetadata("usage: hnl run <file.hnl>".into()))?;

    let source = std::fs::read_to_string(path)
        .map_err(|e| PlatformError::InvalidMetadata(format!("failed to read {path}: {e}")))?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let value = kit.run(&source).map_err(lang_to_platform)?;
    print_value(&value);
    Ok(())
}

fn eval_expression(expr: Option<&str>) -> Result<(), PlatformError> {
    let expr_str =
        expr.ok_or_else(|| PlatformError::InvalidMetadata("usage: hnl eval <expression>".into()))?;

    let kit = Kit::new().map_err(lang_to_platform)?;
    let value = kit.eval(expr_str).map_err(lang_to_platform)?;
    print_value(&value);
    Ok(())
}

/// Convert a [`LangError`] to a [`PlatformError`].
fn lang_to_platform(e: LangError) -> PlatformError {
    PlatformError::Plugin(e.to_string())
}

/// Print a `serde_json::Value` the way the CLI presents eval results: bare
/// strings are printed without quotes, everything else as pretty-printed JSON.
fn print_value(value: &Value) {
    if let Some(s) = value.as_str() {
        println!("{s}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}

fn gen_grammar(_kit_name: &str, output_path: Option<&str>) -> Result<(), PlatformError> {
    let kit = Kit::new().map_err(lang_to_platform)?;
    // Grammar rules are no longer collected from the legacy token factories.
    // The data-driven textmate emitter will provide rule data in a future
    // extension; for now we emit an empty grammar skeleton.
    let rules: Vec<handl_host_tooling::GrammarRule> = Vec::new();
    let grammar =
        handl_host_tooling::generate_textmate(&rules, kit.scope_name(), &kit.file_extensions());

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
