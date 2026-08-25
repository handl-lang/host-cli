use handl_platform::{CapabilityId, CapabilityRequest, DefaultHost, Host, PlatformError};
use std::collections::HashMap;

fn leak_str(s: &str) -> &'static str {
    &*Box::leak(s.to_string().into_boxed_str())
}

#[derive(Debug)]
pub enum RunError {
    Clap(String),
    Json(String),
    Dispatch(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Clap(msg) => write!(f, "{msg}"),
            RunError::Json(msg) => write!(f, "json error: {msg}"),
            RunError::Dispatch(msg) => write!(f, "dispatch error: {msg}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<clap::Error> for RunError {
    fn from(e: clap::Error) -> Self {
        RunError::Clap(e.to_string())
    }
}

impl From<serde_json::Error> for RunError {
    fn from(e: serde_json::Error) -> Self {
        RunError::Json(e.to_string())
    }
}

impl From<PlatformError> for RunError {
    fn from(e: PlatformError) -> Self {
        RunError::Dispatch(e.to_string())
    }
}

pub fn run(
    args: impl Iterator<Item = String>,
    host: &mut DefaultHost,
) -> Result<(i32, String), RunError> {
    let mut cmd = clap::Command::new("handl")
        .about("Metadata-driven capability orchestrator for handl");

    let capabilities = host.plugins().capabilities();
    let mut by_subcommand: HashMap<String, CapabilityId> = HashMap::new();

    for (cap_id, meta) in capabilities {
        let Some(command) = meta["command"].as_str() else {
            continue;
        };
        let command_str = command.to_string();
        let command_static = leak_str(command);
        let about = meta["description"].as_str().unwrap_or_default();
        let about_static = leak_str(about);

        let mut sub = clap::Command::new(command_static)
            .about(about_static)
            .subcommand_required(false)
            .arg_required_else_help(false)
            .arg(
                clap::Arg::new("args")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .action(clap::ArgAction::Append),
            );

        if let Some(aliases) = meta["aliases"].as_array() {
            for alias in aliases.iter().filter_map(|v| v.as_str()) {
                let alias_static = leak_str(alias);
                sub = sub.visible_alias(alias_static);
            }
        }

        by_subcommand.insert(command_str, cap_id);
        cmd = cmd.subcommand(sub);
    }

    let args_vec: Vec<String> = args.collect();
    let mut full_args = vec!["handl".to_string()];
    full_args.extend(args_vec);
    let matches = cmd.try_get_matches_from(full_args)?;
    let (sub_name, sub_matches) = matches
        .subcommand()
        .ok_or_else(|| RunError::Clap("no subcommand".to_string()))?;

    let capability = by_subcommand
        .get(sub_name)
        .ok_or_else(|| RunError::Dispatch(format!("no capability for command: {sub_name}")))?;

    let arg_values: Vec<String> = sub_matches
        .get_raw("args")
        .map(|vals| vals.map(|v| v.to_string_lossy().into_owned()).collect())
        .unwrap_or_default();

    let request = CapabilityRequest {
        capability: capability.clone(),
        payload: serde_json::json!({"args": arg_values}),
    };

    let response = host.run(&request)?;
    Ok((0, serde_json::to_string_pretty(&response.0)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use handl_ext_eval::EvalPlugin;
    use handl_ext_fmt::FmtPlugin;
    use handl_platform::{CapabilityResponse, DefaultHost, InProcessPlugin};

    #[test]
    fn dispatches_to_in_process_plugin() {
        let mut host = DefaultHost::new();
        host.register_plugin(InProcessPlugin::new(
            "echo",
            &["echo".into()],
            serde_json::json!({
                "command": "echo",
                "description": "Echo command"
            }),
            |req, _ctx| {
                Ok(CapabilityResponse(serde_json::json!({
                    "echoed": req.capability.as_str(),
                    "args": req.payload["args"],
                })))
            },
        ));
        let (code, out) =
            run(vec!["echo", "hello"].into_iter().map(String::from), &mut host).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("echoed"));
        assert!(out.contains("hello"));
    }

    #[test]
    fn dispatches_to_fmt_plugin() {
        let mut host = DefaultHost::new();
        host.register_plugin(FmtPlugin::new());
        let (code, out) = run(vec!["fmt"].into_iter().map(String::from), &mut host).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("formatted"));
        assert!(out.contains("true"));
    }

    #[test]
    fn dispatches_to_eval_plugin() {
        let mut host = DefaultHost::new();
        host.register_plugin(EvalPlugin::new());
        let (code, out) =
            run(vec!["eval", "42"].into_iter().map(String::from), &mut host).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("42.0"));
    }

    #[test]
    fn unknown_subcommand_returns_error() {
        let mut host = DefaultHost::new();
        let err = run(vec!["nope"].into_iter().map(String::from), &mut host).unwrap_err();
        assert!(err.to_string().contains("nope") || err.to_string().contains("unknown"));
    }
}
