use handl_ext_cli::BasicCli;
use handl_lib_hello::HelloLibraryPlugin;
use handl_platform::{CapabilityRequest, ConfigHostApplier, DefaultHost, HostApplier, PlatformError};
use serde_json::json;

fn main() -> Result<(), PlatformError> {
    let mut host = DefaultHost::new();
    let applier = ConfigHostApplier::from_toml("")?;
    applier.apply(&mut host)?;
    host.register_plugin(BasicCli::new());
    host.register_plugin(HelloLibraryPlugin::new());
    host.load()?;

    let args = std::env::args().collect::<Vec<_>>();
    let name = args
        .windows(2)
        .find(|w| w[0] == "eval")
        .and_then(|w| parse_greet_arg(&w[1]))
        .unwrap_or("world".to_string());

    let resp = host.run(&CapabilityRequest {
        capability: "hello.greet".into(),
        payload: json!({ "name": name }),
    })?;

    if let Some(s) = resp.0.as_str() {
        println!("{s}");
    }
    Ok(())
}

fn parse_greet_arg(expr: &str) -> Option<String> {
    let rest = expr.strip_prefix("hello.greet")?;
    let rest = rest.trim();
    let inner = rest.strip_prefix("(")?.strip_suffix(")")?;
    let inner = inner.trim();
    let quoted = inner.strip_prefix('"')?.strip_suffix('"')?;
    Some(quoted.to_string())
}
