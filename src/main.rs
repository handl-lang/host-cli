use handl_ext_cli::BasicCli;
use handl_ext_kit::BasicKitLoader;
use handl_platform::{
    CapabilityRequest, ConfigHostApplier, DefaultHost, HostApplier, KitLoader, PlatformError,
};
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let applier = ConfigHostApplier::from_toml("")?;
    applier.apply(&mut host)?;

    let args = std::env::args().collect::<Vec<_>>();
    let (kit_name, command, command_args) = parse_args(&args)?;

    let loader = BasicKitLoader::new();
    let kit = loader.load(&kit_name)?;
    host.register_plugin(kit);

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

fn parse_args(args: &[String]) -> Result<(String, &str, Vec<&str>), PlatformError> {
    let mut kit_name = std::env::var("HANDL_KIT").unwrap_or_else(|_| "hello".to_string());
    let mut command = "";
    let mut command_args = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
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
    Ok((kit_name, command, command_args))
}
