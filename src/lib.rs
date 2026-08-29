use handl_platform::{
    Behavior, CapabilityId, CapabilityMetadata, CapabilityRequest, CapabilityResponse,
    Contribution, Operation, OptionSchema, PlatformError, Plugin, PluginContext, ResolvedOptions,
    RuleAcceptor,
};
use serde_json::json;

/// CLI operation marker.
#[derive(Debug, Clone, Copy)]
pub struct Cli;

impl Operation for Cli {
    type Request = CliRequest;
}

/// Request passed to `Cli` behaviors.
#[derive(Debug, Clone)]
pub struct CliRequest {
    /// CLI command name.
    pub command: String,
    /// CLI command arguments.
    pub args: Vec<String>,
    /// Resolved CLI options.
    pub options: ResolvedOptions,
}

/// Behavior trait for handling a specific CLI command.
pub trait CliBehavior: Behavior<Cli> {}

impl Contribution for Box<dyn CliBehavior> {
    fn id(&self) -> &str {
        self.kind()
    }
}

/// Basic `RuleAcceptor` plugin for the `Cli` operation.
pub struct BasicCli {
    /// Registered CLI behaviors, keyed by command.
    pub rules: Vec<Box<dyn CliBehavior>>,
    /// Option schemas contributed for the CLI.
    pub option_schemas: Vec<OptionSchema>,
    /// Capabilities provided by the CLI.
    pub capabilities: Vec<CapabilityId>,
}

impl BasicCli {
    /// Create a new `BasicCli` with no rules.
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            option_schemas: Vec::new(),
            capabilities: vec!["cli".into()],
        }
    }
}

impl Default for BasicCli {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for BasicCli {
    fn id(&self) -> &str {
        "cli.basic"
    }

    fn name(&self) -> &str {
        "Basic CLI"
    }

    fn capabilities(&self) -> &[CapabilityId] {
        &self.capabilities
    }

    fn metadata(&self, capability: &CapabilityId) -> Option<CapabilityMetadata> {
        self.capabilities.contains(capability).then(|| {
            serde_json::json!({
                "command": "cli",
                "description": "Dispatches CLI commands to capabilities"
            })
        })
    }

    fn run(
        &self,
        request: &CapabilityRequest,
        ctx: &mut PluginContext,
    ) -> Result<CapabilityResponse, PlatformError> {
        if !self.capabilities.contains(&request.capability) {
            return Err(PlatformError::UnknownCapability(request.capability.clone()));
        }
        match &request.payload {
            serde_json::Value::Object(map) => {
                let command = map
                    .get("command")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| PlatformError::InvalidMetadata("missing command".into()))?;
                let args = map
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let command_id = CapabilityId::from(command);
                let plugin = ctx
                    .plugins
                    .resolve(&command_id)
                    .ok_or_else(|| PlatformError::UnknownCapability(command_id.clone()))?;
                let mut payload = json!({ "args": args });
                if command == "eval" {
                    let kind = args
                        .first()
                        .and_then(|a| a.split('(').next())
                        .map(|s| s.trim())
                        .unwrap_or("eval");
                    payload["kind"] = json!(kind);
                }
                let request = CapabilityRequest {
                    capability: command_id,
                    payload,
                };
                plugin.run(&request, ctx)
            }
            _ => Err(PlatformError::InvalidMetadata(
                "expected object payload".into(),
            )),
        }
    }
}

impl RuleAcceptor for BasicCli {
    type Rule = Box<dyn CliBehavior>;

    fn accept(&mut self, rule: Self::Rule) -> Result<(), PlatformError> {
        self.rules.push(rule);
        Ok(())
    }
}
