//! `hnl` — a dumb host for `hnl_ext_*` extensions (ADR-0149).
//!
//! The host knows nothing about tools. It discovers shared libraries named
//! `libhnl_ext_<name>.{so,dylib,dll}` (see [`handl_ext::discover`] for the
//! search order), reads each extension's manifest, and dispatches argv to
//! the command the manifest declares. Everything after the command name —
//! flags, paths, multi-word surfaces like `compile js` — is the
//! extension's own parsing problem.
//!
//! The only commands implemented here are host-level meta commands:
//!
//! ```text
//! hnl --help                     list loaded extensions + their commands
//! hnl version                    print the host version
//! hnl ext list                   table of discovered extensions
//! hnl ext doctor                 diagnose the discovery dirs
//! hnl <command> [args...]        dispatch to the extension serving <command>
//! ```

use std::io::Write;
use std::path::PathBuf;

use handl_ext::{Discovery, ExtRequest};

/// Run the CLI, returning the process exit code.
pub fn run(args: &[String]) -> i32 {
    let parsed = match parse_host_args(args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    // Discovery is eager (ADR-0149 §5): read every candidate manifest at
    // startup so `--help`, `ext list`, and dispatch all see the same set.
    // Manifest reads only — no extension tool code runs until dispatch.
    let discovery = handl_ext::discover(&parsed.ext_dirs);
    match parsed.action {
        Action::Help => {
            print!("{}", help_text(&discovery));
            0
        }
        Action::Version => {
            println!("hnl {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Action::ExtList => {
            print!("{}", ext_list_text(&discovery));
            0
        }
        Action::ExtDoctor => {
            print!("{}", ext_doctor_text(&discovery));
            if discovery.failures.is_empty() { 0 } else { 1 }
        }
        Action::Command { name, args } => dispatch(&discovery, &name, &args),
    }
}

// ---------------------------------------------------------------------------
// Host-level argument parsing
// ---------------------------------------------------------------------------

/// What the invocation asks the host to do.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    Version,
    ExtList,
    ExtDoctor,
    Command { name: String, args: Vec<String> },
}

/// Parsed host invocation: `--ext-dir` flags plus the action.
#[derive(Debug, PartialEq, Eq)]
struct ParsedHostArgs {
    ext_dirs: Vec<PathBuf>,
    action: Action,
}

/// Parse argv into host-level flags and an action.
///
/// Only `--ext-dir` and the meta words are consumed by the host; the first
/// non-flag token is the command (or meta command), and everything after it
/// passes through verbatim to the extension. `hnl --kit foo fmt x` style
/// global flags of the old host are gone: options belong to extensions,
/// which receive them via `args`/`env`.
fn parse_host_args(args: &[String]) -> Result<ParsedHostArgs, String> {
    let mut ext_dirs = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--ext-dir" => {
                i += 1;
                let dir = args
                    .get(i)
                    .ok_or_else(|| "missing --ext-dir value".to_string())?;
                ext_dirs.push(PathBuf::from(dir));
            }
            // An extension dir can also be given as `--ext-dir=path`.
            s if s.starts_with("--ext-dir=") => {
                ext_dirs.push(PathBuf::from(&s["--ext-dir=".len()..]));
            }
            "--help" | "-h" | "help" => {
                return Ok(ParsedHostArgs {
                    ext_dirs,
                    action: Action::Help,
                });
            }
            "--version" | "-V" | "version" => {
                return Ok(ParsedHostArgs {
                    ext_dirs,
                    action: Action::Version,
                });
            }
            "ext" => {
                let sub = args.get(i + 1).map(String::as_str).unwrap_or("");
                let action = match sub {
                    "list" => Action::ExtList,
                    "doctor" => Action::ExtDoctor,
                    _ => {
                        return Err(format!(
                            "usage: hnl ext <list|doctor>\nunknown ext subcommand `{sub}`"
                        ));
                    }
                };
                return Ok(ParsedHostArgs { ext_dirs, action });
            }
            "--" => { /* separator: the next token is the command */ }
            s => {
                return Ok(ParsedHostArgs {
                    ext_dirs,
                    action: Action::Command {
                        name: s.to_string(),
                        args: args[i + 1..].to_vec(),
                    },
                });
            }
        }
        i += 1;
    }
    Err("usage: hnl [--ext-dir <dir>] <command> [args...]\ntry `hnl --help`".into())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Route `command`/`args` to the extension that declared `command`.
fn dispatch(discovery: &Discovery, command: &str, args: &[String]) -> i32 {
    let Some(ext) = discovery.find_command(command) else {
        eprintln!("{}", unknown_command_text(discovery, command));
        return 2;
    };
    let request = ExtRequest {
        args: args.to_vec(),
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        env: std::env::vars().collect(),
    };
    match ext.dispatch(command, &request) {
        Ok(response) => {
            write_stream(std::io::stdout(), &response.stdout);
            write_stream(std::io::stderr(), &response.stderr);
            response.code
        }
        Err(e) => {
            eprintln!("hnl: {e}");
            2
        }
    }
}

/// Write `bytes` to `stream`; a closed pipe (e.g. `hnl fmt | head -1`)
/// exits quietly rather than panicking on a failed `print!`.
fn write_stream(mut stream: impl Write, bytes: &str) {
    match stream.write_all(bytes.as_bytes()).and_then(|_| stream.flush()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("hnl: write failed: {e}");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// Meta-command rendering
// ---------------------------------------------------------------------------

/// `hnl --help`: usage + meta commands + the loaded extension command table.
fn help_text(discovery: &Discovery) -> String {
    let mut out = String::from(
        "usage: hnl [--ext-dir <dir>] <command> [args...]\n\
         \n\
         host commands:\n\
         \x20 help                     show this help\n\
         \x20 version                  print the host version\n\
         \x20 ext list                 list discovered extensions\n\
         \x20 ext doctor               diagnose extension discovery\n",
    );
    if discovery.extensions.is_empty() {
        out.push_str("\nextension commands: none loaded\n");
    } else {
        out.push_str("\nextension commands:\n");
        for ext in &discovery.extensions {
            for spec in &ext.manifest().commands {
                out.push_str(&format!(
                    "  {:<14} {:<42} ({} {})\n",
                    spec.name,
                    spec.usage,
                    ext.manifest().name,
                    ext.manifest().version,
                ));
            }
        }
    }
    out.push_str(
        "\ndiscovery: --ext-dir <dir> → $HANDL_EXT_DIR → ~/.handl/ext/\n\
         \x20          (dev builds also scan <exe>/ext/ and <exe>/ for libhnl_ext_*)\n",
    );
    out
}

/// `hnl ext list`: one block per loaded extension.
fn ext_list_text(discovery: &Discovery) -> String {
    let mut out = String::new();
    if discovery.extensions.is_empty() {
        out.push_str("no extensions loaded\n");
    }
    for ext in &discovery.extensions {
        out.push_str(&format!(
            "{} {} — {}\n",
            ext.manifest().name,
            ext.manifest().version,
            ext.path().display(),
        ));
        for spec in &ext.manifest().commands {
            out.push_str(&format!("    {:<14} {}\n", spec.name, spec.usage));
        }
    }
    if !discovery.skipped_duplicates.is_empty() {
        out.push_str("\nskipped duplicates (earlier dir won):\n");
        for (name, path) in &discovery.skipped_duplicates {
            out.push_str(&format!("    {name} — {}\n", path.display()));
        }
    }
    out
}

/// `hnl ext doctor`: every candidate dir, every load failure, every dupe.
fn ext_doctor_text(discovery: &Discovery) -> String {
    let mut out = String::from("discovery dirs:\n");
    for dir in &discovery.dirs {
        out.push_str(&format!(
            "  [{}] {} {}\n",
            dir.source.label(),
            if dir.exists { "ok     " } else { "missing" },
            dir.path.display(),
        ));
    }
    out.push_str(&format!(
        "\nloaded: {} extension(s), {} command(s)\n",
        discovery.extensions.len(),
        discovery
            .extensions
            .iter()
            .map(|e| e.manifest().commands.len())
            .sum::<usize>(),
    ));
    if !discovery.failures.is_empty() {
        out.push_str("\nload failures:\n");
        for f in &discovery.failures {
            out.push_str(&format!("  {}: {}\n", f.path.display(), f.error));
        }
    }
    if !discovery.skipped_duplicates.is_empty() {
        out.push_str("\nskipped duplicates (earlier dir won):\n");
        for (name, path) in &discovery.skipped_duplicates {
            out.push_str(&format!("  {name} — {}\n", path.display()));
        }
    }
    out
}

/// The "unknown command" diagnostic: what was asked, what is loaded, and
/// where the host looked.
fn unknown_command_text(discovery: &Discovery, command: &str) -> String {
    let mut out = format!("hnl: unknown command `{command}`\n");
    if discovery.extensions.is_empty() {
        out.push_str("no extensions loaded; searched:\n");
        for dir in &discovery.dirs {
            out.push_str(&format!(
                "  {} {}\n",
                if dir.exists { "  " } else { "× " },
                dir.path.display(),
            ));
        }
        out.push_str("hint: install extensions to ~/.handl/ext/ or pass --ext-dir <dir>\n");
    } else {
        out.push_str("loaded commands:\n");
        for (cmd, exts) in discovery.command_table() {
            out.push_str(&format!("  {cmd:<14} ({})\n", exts.join(", ")));
        }
        out.push_str("hint: `hnl --help` lists all commands with usage\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn command_passes_remaining_args_verbatim() {
        let p = parse_host_args(&argv(&["hnl", "fmt", "--check", "--ext-dir", "x", "f.hnl"]))
            .expect("parses");
        // `--ext-dir` *after* the command belongs to the extension.
        assert_eq!(
            p.action,
            Action::Command {
                name: "fmt".into(),
                args: vec!["--check".into(), "--ext-dir".into(), "x".into(), "f.hnl".into()],
            }
        );
        assert!(p.ext_dirs.is_empty());
    }

    #[test]
    fn ext_dir_is_repeatable_and_supports_equals_form() {
        let p = parse_host_args(&argv(&[
            "hnl",
            "--ext-dir",
            "a",
            "--ext-dir=b",
            "fmt",
            "f.hnl",
        ]))
        .expect("parses");
        assert_eq!(p.ext_dirs, vec![PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(
            p.action,
            Action::Command {
                name: "fmt".into(),
                args: vec!["f.hnl".into()],
            }
        );
    }

    #[test]
    fn meta_commands_are_host_level() {
        assert_eq!(
            parse_host_args(&argv(&["hnl", "--help"])).unwrap().action,
            Action::Help
        );
        assert_eq!(
            parse_host_args(&argv(&["hnl", "help"])).unwrap().action,
            Action::Help
        );
        assert_eq!(
            parse_host_args(&argv(&["hnl", "version"])).unwrap().action,
            Action::Version
        );
        assert_eq!(
            parse_host_args(&argv(&["hnl", "--version"])).unwrap().action,
            Action::Version
        );
        assert_eq!(
            parse_host_args(&argv(&["hnl", "ext", "list"])).unwrap().action,
            Action::ExtList
        );
        assert_eq!(
            parse_host_args(&argv(&["hnl", "ext", "doctor"])).unwrap().action,
            Action::ExtDoctor
        );
        assert!(parse_host_args(&argv(&["hnl", "ext"])).is_err());
        assert!(parse_host_args(&argv(&["hnl", "ext", "bogus"])).is_err());
    }

    #[test]
    fn no_command_is_a_usage_error() {
        assert!(parse_host_args(&argv(&["hnl"])).is_err());
        assert!(parse_host_args(&argv(&["hnl", "--ext-dir"])).is_err());
    }

    #[test]
    fn unknown_command_names_loaded_commands() {
        let discovery = Discovery::default();
        let text = unknown_command_text(&discovery, "bogus");
        assert!(text.contains("unknown command `bogus`"));
        assert!(text.contains("no extensions loaded"));
    }
}
