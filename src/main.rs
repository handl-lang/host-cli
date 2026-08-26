use handl_ext_cli::BasicCli;
use handl_platform::{CapabilityRequest, ConfigHostApplier, DefaultHost, HostApplier, PlatformError};

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let applier = ConfigHostApplier::from_toml("")?;
    applier.apply(&mut host)?;
    host.register_plugin(BasicCli::new());
    host.load()?;
    host.run(&CapabilityRequest {
        capability: "cli".into(),
        payload: serde_json::Value::Null,
    })?;
    Ok(())
}
