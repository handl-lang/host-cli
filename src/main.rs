//! `hnl` — the canonical handl CLI (ADR-0090).
//!
//! The `hnl` binary lives in `crates/host-cli` and dispatches directly to the
//! bootstrap tooling crates. It is a thin dispatcher: no business logic lives
//! here — each subcommand calls into the relevant tool crate and renders
//! diagnostics through the shared `plugin-ext-diag` renderer.
//!
//! # Command surface (v0.1)
//!
//! ```text
//! hnl fmt   [--check] [--write] [paths...]    — format files
//! hnl lint  [--fix] [--fail-on-warn] [paths...] — lint files
//! hnl check [paths...]                         — typecheck files
//! hnl test  [--list] [--filter <pat>] [--tag <tag>] [--jobs N|auto]
//!           [--report human|json] [--changed] [--cache] [paths...] — run tests
//! hnl run   <file.handl> [-- <args...>]        — run a file
//! hnl eval  '<expression>'                     — evaluate an expression
//! hnl typecheck '<expression>'                 — typecheck a single expression
//! hnl gen-grammar [output_path]                — generate TextMate grammar
//! ```
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0    | success |
//! | 1    | check/lint/test/fmt-check failure |
//! | 2    | usage or parse error |

use std::io::Read;
use std::path::{Path, PathBuf};

use handl_kit_canonical::Kit;
use handl_language::LangError;
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use handl_plugin_ext_diag::{render_diagnostic, render_diagnostics, Severity};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(run(&args));
}

/// Run the CLI, returning the process exit code.
fn run(args: &[String]) -> i32 {
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    dispatch(parsed)
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

/// Parsed CLI invocation: global options + command + command arguments.
struct ParsedArgs {
    applier_id: String,
    config_path: String,
    kit_name: String,
    command: String,
    command_args: Vec<String>,
    engine: Option<String>,
}

/// Parse the argument vector into global options and a command + args.
///
/// Global flags (`--applier`, `--config`, `--kit`, `--engine`) are consumed
/// wherever they appear before or interleaved with the command. The first
/// non-flag token is the command; everything after it (including subcommand
/// flags like `--check`, `--fix`) is collected verbatim into `command_args`
/// for the command handler to parse.
fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut applier_id = "applier.default".to_string();
    let mut config_path = ".handl.toml".to_string();
    let mut kit_name =
        std::env::var("HANDL_KIT").unwrap_or_else(|_| "hello".to_string());
    let mut command = String::new();
    let mut command_args = Vec::new();
    let mut engine: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        // Once the command is known, everything else belongs to it (including
        // subcommand flags). This lets `hnl fmt --check file.handl` work.
        if !command.is_empty() {
            command_args.push(arg.clone());
            i += 1;
            continue;
        }
        match arg.as_str() {
            "--applier" => {
                i += 1;
                applier_id = args
                    .get(i)
                    .ok_or_else(|| "missing --applier value".to_string())?
                    .clone();
            }
            "--config" => {
                i += 1;
                config_path = args
                    .get(i)
                    .ok_or_else(|| "missing --config value".to_string())?
                    .clone();
            }
            "--kit" => {
                i += 1;
                kit_name = args
                    .get(i)
                    .ok_or_else(|| "missing --kit value".to_string())?
                    .clone();
            }
            "--engine" => {
                i += 1;
                engine = Some(
                    args.get(i)
                        .ok_or_else(|| "missing --engine value".to_string())?
                        .clone(),
                );
            }
            "--" => {
                // Separator: following args are the command and its args.
            }
            _ => {
                command = arg.clone();
            }
        }
        i += 1;
    }
    if command.is_empty() {
        return Err("usage: hnl <command> [args...]\ncommands: fmt, lint, check, test, run, eval, typecheck, gen-grammar".into());
    }
    Ok(ParsedArgs {
        applier_id,
        config_path,
        kit_name,
        command,
        command_args,
        engine,
    })
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn dispatch(p: ParsedArgs) -> i32 {
    let ParsedArgs {
        applier_id,
        config_path,
        kit_name,
        command,
        command_args,
        engine,
    } = p;

    match command.as_str() {
        "fmt" => fmt_command(&command_args),
        "lint" => lint_command(&command_args),
        "check" => check_command(&command_args),
        "test" => test_command(&command_args),
        "run" => run_file(command_args.first().map(String::as_str), engine.as_deref()),
        "eval" => eval_expression(command_args.first().map(String::as_str)),
        "typecheck" => typecheck_expression(command_args.first().map(String::as_str)),
        "gen-grammar" => gen_grammar(&kit_name, command_args.first().map(String::as_str)),
        // Unknown commands go through the `cli` capability dispatch.
        _ => cli_dispatch(
            &applier_id,
            &config_path,
            &kit_name,
            &command,
            &command_args,
        ),
    }
}

// ---------------------------------------------------------------------------
// `hnl fmt [--check] [--write] [paths...]`
// ---------------------------------------------------------------------------

fn fmt_command(args: &[String]) -> i32 {
    let mut check = false;
    let mut write = false;
    let mut paths = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--check" => check = true,
            "--write" => write = true,
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("fmt: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
    }

    if check && write {
        eprintln!("fmt: --check and --write are mutually exclusive");
        return 2;
    }

    let mut had_failure = false;

    if paths.is_empty() {
        // Read from stdin.
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("fmt: failed to read stdin");
            return 2;
        }
        return fmt_one(&src, "<stdin>", check, write, &mut had_failure);
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fmt: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = fmt_one(&src, path, check, write, &mut had_failure);
        if code == 2 {
            return 2;
        }
    }

    if had_failure {
        1
    } else {
        0
    }
}

/// Format a single source. Returns 2 on parse error (and sets `had_failure`
/// for check failures). Otherwise returns 0.
fn fmt_one(src: &str, path: &str, check: bool, write: bool, had_failure: &mut bool) -> i32 {
    if check {
        match handl_plugin_ext_fmt::check_source(src) {
            Ok(true) => 0,
            Ok(false) => {
                println!("{path}: not canonical");
                *had_failure = true;
                0
            }
            Err(err) => {
                let diag = handl_plugin_ext_fmt::format_error_to_diagnostic(&err, src);
                eprintln!("{}", render_diagnostic(&diag, path));
                2
            }
        }
    } else {
        match handl_plugin_ext_fmt::format_source(src) {
            Ok(formatted) => {
                if write {
                    if let Err(e) = write_atomic(path, &formatted) {
                        eprintln!("fmt: failed to write {path}: {e}");
                        return 2;
                    }
                } else {
                    print!("{formatted}");
                }
                0
            }
            Err(err) => {
                let diag = handl_plugin_ext_fmt::format_error_to_diagnostic(&err, src);
                eprintln!("{}", render_diagnostic(&diag, path));
                2
            }
        }
    }
}

// ---------------------------------------------------------------------------
// `hnl lint [--fix] [--fail-on-warn] [paths...]`
// ---------------------------------------------------------------------------

fn lint_command(args: &[String]) -> i32 {
    let mut fix = false;
    let mut fail_on_warn = false;
    let mut paths = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--fix" => fix = true,
            "--fail-on-warn" => fail_on_warn = true,
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("lint: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
    }

    let mut had_failure = false;

    if paths.is_empty() {
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("lint: failed to read stdin");
            return 2;
        }
        return lint_one(&src, "<stdin>", fix, fail_on_warn, &mut had_failure);
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("lint: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = lint_one(&src, path, fix, fail_on_warn, &mut had_failure);
        if code == 2 {
            return 2;
        }
    }

    if had_failure {
        1
    } else {
        0
    }
}

/// Lint a single source. Returns 2 on parse error; sets `had_failure` for
/// Error diagnostics (or Warning when `fail_on_warn`).
fn lint_one(
    src: &str,
    path: &str,
    fix: bool,
    fail_on_warn: bool,
    had_failure: &mut bool,
) -> i32 {
    let diags = match handl_plugin_ext_lint::lint_source(src) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("{path}: parse error: {err}");
            return 2;
        }
    };

    let rendered = render_diagnostics(&diags, path);
    if !rendered.is_empty() {
        println!("{rendered}");
    }

    let has_error = diags.iter().any(|d| d.severity == Severity::Error);
    let has_warn = diags.iter().any(|d| d.severity == Severity::Warning);
    if has_error || (fail_on_warn && has_warn) {
        *had_failure = true;
    }

    if fix {
        match handl_plugin_ext_lint::fix_source(src) {
            Ok(fixed) => {
                if let Err(e) = write_atomic(path, &fixed) {
                    eprintln!("lint: failed to write {path}: {e}");
                    return 2;
                }
            }
            Err(err) => {
                eprintln!("{path}: fix error: {err}");
                return 2;
            }
        }
    }

    0
}

// ---------------------------------------------------------------------------
// `hnl check [paths...]`
// ---------------------------------------------------------------------------

fn check_command(args: &[String]) -> i32 {
    let mut paths = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("check: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
    }

    let mut had_failure = false;

    if paths.is_empty() {
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("check: failed to read stdin");
            return 2;
        }
        return check_one(&src, "<stdin>", &mut had_failure);
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("check: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = check_one(&src, path, &mut had_failure);
        if code == 2 {
            return 2;
        }
    }

    if had_failure {
        1
    } else {
        0
    }
}

/// Typecheck a single source. Returns 2 on parse error; sets `had_failure`
/// for Error diagnostics.
fn check_one(src: &str, path: &str, had_failure: &mut bool) -> i32 {
    let report = match handl_plugin_engine_typecheck::check_module(src) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("{path}: parse error: {err}");
            return 2;
        }
    };

    let rendered = render_diagnostics(&report.diagnostics, path);
    if !rendered.is_empty() {
        println!("{rendered}");
    }

    if report.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        *had_failure = true;
    }
    0
}

// ---------------------------------------------------------------------------
// `hnl test [paths...]`
// ---------------------------------------------------------------------------

fn test_command(args: &[String]) -> i32 {
    let mut filter: Option<String> = None;
    let mut list = false;
    let mut report_kind = "human".to_string();
    let mut jobs: Option<String> = None;
    let mut cache = false;
    let mut tag: Option<String> = None;
    let mut changed = false;
    let mut paths = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--filter" => {
                i += 1;
                match args.get(i) {
                    Some(v) => filter = Some(v.clone()),
                    None => {
                        eprintln!("test: --filter requires a value");
                        return 2;
                    }
                }
            }
            "--tag" => {
                i += 1;
                match args.get(i) {
                    Some(v) => tag = Some(v.clone()),
                    None => {
                        eprintln!("test: --tag requires a value");
                        return 2;
                    }
                }
            }
            "--jobs" => {
                i += 1;
                match args.get(i) {
                    Some(v) => jobs = Some(v.clone()),
                    None => {
                        eprintln!("test: --jobs requires a value");
                        return 2;
                    }
                }
            }
            "--report" => {
                i += 1;
                match args.get(i) {
                    Some(v) => report_kind = v.clone(),
                    None => {
                        eprintln!("test: --report requires a value");
                        return 2;
                    }
                }
            }
            "--list" => list = true,
            "--changed" => changed = true,
            "--cache" => cache = true,
            "--json" => report_kind = "json".to_string(),
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("test: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
        i += 1;
    }

    // `--tag` and `--changed` require runner support that is not yet wired
    // (TestCase carries no tag metadata and there is no VCS/changed filter).
    if tag.is_some() {
        eprintln!("test: --tag is not yet implemented");
        return 2;
    }
    if changed {
        eprintln!("test: --changed is not yet implemented");
        return 2;
    }

    // Default paths: `tests/` if it exists, otherwise the current directory.
    if paths.is_empty() {
        if std::path::Path::new("tests").is_dir() {
            paths.push("tests".to_string());
        } else {
            paths.push(".".to_string());
        }
    }

    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let plan = match handl_plugin_engine_test::discover(&path_bufs) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("test: discovery error: {e}");
            return 2;
        }
    };

    // Report parse errors discovered while scanning test modules.
    let mut had_parse_error = false;
    for (file, msg) in &plan.parse_errors {
        eprintln!("{}: parse error: {msg}", file.display());
        had_parse_error = true;
    }

    if list {
        let predicate = handl_plugin_engine_test::filter_predicate(filter.as_deref().unwrap_or(""));
        for case in &plan.tests {
            if predicate.eval(case) {
                println!("{} ({})", case.name, case.module.display());
            }
        }
        return if had_parse_error { 1 } else { 0 };
    }

    // Build the run config from the CLI flags.
    let mut config = handl_plugin_engine_test::RunConfig::new();
    if let Some(pat) = &filter {
        config = config.with_filter(pat);
    }
    if let Some(j) = jobs {
        let policy = if j == "auto" {
            handl_plugin_lib_scheduler::SchedulePolicy::default()
        } else {
            match j.parse::<usize>() {
                Ok(n) => handl_plugin_lib_scheduler::SchedulePolicy::parallel(n),
                Err(_) => {
                    eprintln!("test: --jobs requires a positive integer or `auto`");
                    return 2;
                }
            }
        };
        config = config.with_policy(policy);
    }
    if cache {
        // The result cache is in-process only; it speeds up a single `hnl test`
        // invocation but is not written to disk between processes (the generic
        // execution cache lives in `plugin-exec-cache`).
        config = config.with_cache(handl_plugin_engine_test::TestRunCache::new());
    }

    let report = handl_plugin_engine_test::run_with_config(&plan, &mut config);

    // Render the report with the selected reporter.
    let summary = handl_plugin_engine_test::summarize(&report);
    let output = match report_kind.as_str() {
        "human" | "console" => {
            <handl_plugin_engine_test::HumanReporter as handl_plugin_engine_test::Reporter>::format(
                &handl_plugin_engine_test::HumanReporter,
                &report,
                &summary,
            )
        }
        "json" => {
            <handl_plugin_engine_test::JsonReporter as handl_plugin_engine_test::Reporter>::format(
                &handl_plugin_engine_test::JsonReporter,
                &report,
                &summary,
            )
        }
        other => {
            eprintln!("test: unknown report format `{other}`");
            return 2;
        }
    };
    if report_kind == "json" {
        println!("{output}");
    } else {
        print!("{output}");
    }

    if report.has_failures() || had_parse_error {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// `hnl run <file> [-- <args...>]`
// ---------------------------------------------------------------------------

fn run_file(path: Option<&str>, engine: Option<&str>) -> i32 {
    let path = match path {
        Some(p) => p,
        None => {
            eprintln!("usage: hnl run <file.handl>");
            return 2;
        }
    };

    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("run: failed to read {path}: {e}");
            return 2;
        }
    };

    let kit = match engine {
        Some(name) => match Kit::with_eval(name) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("run: {e}");
                return 1;
            }
        },
        None => match Kit::new() {
            Ok(k) => k,
            Err(e) => {
                eprintln!("run: {e}");
                return 1;
            }
        },
    };

    match kit.run(&source) {
        Ok(value) => {
            print_value(&value);
            0
        }
        Err(e) => {
            eprintln!("run: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// `hnl eval <expr>`
// ---------------------------------------------------------------------------

fn eval_expression(expr: Option<&str>) -> i32 {
    let expr = match expr {
        Some(e) => e,
        None => {
            eprintln!("usage: hnl eval <expression>");
            return 2;
        }
    };

    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("eval: {e}");
            return 1;
        }
    };

    match kit.eval(expr) {
        Ok(value) => {
            print_value(&value);
            0
        }
        Err(e) => {
            eprintln!("eval: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// `hnl typecheck <expr>` (single-expression typecheck, legacy)
// ---------------------------------------------------------------------------

fn typecheck_expression(expr: Option<&str>) -> i32 {
    let expr = match expr {
        Some(e) => e,
        None => {
            eprintln!("usage: hnl typecheck <expression>");
            return 2;
        }
    };

    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("typecheck: {e}");
            return 1;
        }
    };

    match kit.typecheck(expr) {
        Ok(typ) => {
            print_value(&typ);
            0
        }
        Err(e) => {
            eprintln!("typecheck: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// `hnl gen-grammar [output]`
// ---------------------------------------------------------------------------

fn gen_grammar(kit_name: &str, output_path: Option<&str>) -> i32 {
    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("gen-grammar: {e}");
            return 1;
        }
    };
    let _ = kit_name;
    let rules = handl_host_tooling::grammar_rules_from_language(kit.runtime().lang());
    let grammar = handl_host_tooling::generate_textmate(
        &rules,
        kit.scope_name(),
        &kit.file_extensions(),
    );

    let json = match serde_json::to_string_pretty(&grammar) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gen-grammar: {e}");
            return 1;
        }
    };

    match output_path {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &json) {
                eprintln!("gen-grammar: failed to write {path}: {e}");
                return 1;
            }
            eprintln!("Generated TextMate grammar: {path}");
        }
        None => {
            println!("{json}");
        }
    }
    0
}

// ---------------------------------------------------------------------------
// `cli` capability dispatch (unknown commands)
// ---------------------------------------------------------------------------

fn cli_dispatch(
    applier_id: &str,
    config_path: &str,
    kit_name: &str,
    command: &str,
    command_args: &[String],
) -> i32 {
    let mut host = DefaultHost::new();
    host.register_plugin(BootstrapPlugin::new(
        config_path.to_string(),
        kit_name.to_string(),
        applier_id.to_string(),
    ));
    host.register_plugin(BasicCli::new());
    if let Err(e) = host.load() {
        eprintln!("composition error: {e}");
        return 2;
    }
    if let Err(e) = host.resolve_composition() {
        eprintln!("composition error: {e}");
        return 2;
    }

    let resp = host.run(&CapabilityRequest {
        capability: "cli".into(),
        payload: json!({
            "command": command,
            "args": command_args,
        }),
    });

    match resp {
        Ok(r) => {
            if let Some(s) = r.0.as_str() {
                println!("{s}");
            }
            0
        }
        Err(e) => {
            eprintln!("{command}: {e}");
            2
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Write `content` to `path` atomically: write to a temp file in the same
/// directory, then rename over the target.
fn write_atomic(path: &str, content: &str) -> std::io::Result<()> {
    let target = Path::new(path);
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".hnl-fmt-tmp-{}",
        std::process::id()
    ));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// Print a `serde_json::Value` the way the CLI presents eval/run results: bare
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

/// Convert a [`LangError`] to a [`PlatformError`] (kept for compatibility with
/// any external callers of the legacy dispatch path).
#[allow(dead_code)]
fn lang_to_platform(e: LangError) -> PlatformError {
    PlatformError::Plugin(e.to_string())
}
