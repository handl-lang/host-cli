use handl_ext_cli::BasicCli;
use handl_kit_canonical::CanonicalKit;
use handl_kit_hello::HelloKit;
use handl_platform::{CapabilityRequest, ConfigHostApplier, DefaultHost, HostApplier, PlatformError};
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let applier = ConfigHostApplier::from_toml("")?;
    applier.apply(&mut host)?;

    let args = std::env::args().collect::<Vec<_>>();
    let (kit_name, command, command_args) = parse_args(&args)?;

    match kit_name {
        "hello" => host.register_plugin(HelloKit::new()),
        "canonical" => host.register_plugin(CanonicalKit::new()),
        _ => return Err(PlatformError::InvalidMetadata(format!("unknown kit: {kit_name}"))),
    };

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

fn parse_args(args: &[String]) -> Result<(&str, &str, Vec<&str>), PlatformError> {
    let mut kit_name = "hello";
    let mut command = "";
    let mut command_args = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--kit" => {
                i += 1;
                kit_name = args
                    .get(i)
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing --kit value".into()))?;
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
