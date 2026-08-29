use handl_plugin_applier::DefaultApplierPlugin;
use handl_plugin_ext_cli::BasicCli;
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let args = std::env::args().collect::<Vec<_>>();
    let (applier_id, config_path, kit_name, command, command_args) = parse_args(&args)?;

    if applier_id != "applier.default" {
        return Err(PlatformError::InvalidMetadata(format!("unknown applier: {applier_id}")));
    }

    host.register_plugin(DefaultApplierPlugin::new(config_path).with_fallback_kit(kit_name));
    host.register_plugin(BasicCli::new());
    host.load()?;

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
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --applier value".into()))?
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
