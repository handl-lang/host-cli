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
//! hnl typecheck [paths...]                   — typecheck files (`hnl check` is an alias)
//! hnl test  [--list] [--filter <pat>] [--tag <expr>] [--jobs N|auto]
//!           [--report human|json] [--changed] [--cache] [paths...] — run tests
//!           (--cache stores results in `.handl/cache/test-runner/cache.json`)
//! hnl run   <file.handl> [-- <args...>]        — run a file
//! hnl eval  '<expression>'                     — evaluate an expression
//! hnl transform <pass> <file.hnl> [--pipeline a,b,c] [--plan] [--trace]
//!           [--stats] [--emit tree|json]       — run a transform pass (ADR-0094)
//! hnl gen-grammar [output_path]                — generate TextMate grammar
//! hnl sdkgen c <header> --output <dir> [--target <triple>] [--include <glob>] [--exclude <glob>] [--config <yaml>]
//!                                                — generate C port + adapter
//! ```
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | 0    | success |
//! | 1    | check/lint/test/fmt-check failure |
//! | 2    | usage or parse error |

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use handl_kit_canonical::Kit;
use handl_language::{AstRef, LangError, Language, Source};
use handl_platform::{CapabilityRequest, DefaultHost, PlatformError};
use handl_plugin_applier_bootstrap::BootstrapPlugin;
use handl_plugin_engine_lex_pattern::DataLexerEngine;
use handl_plugin_engine_parse_pratt::PrattEngine;
use handl_plugin_ext_cli::BasicCli;
use handl_plugin_ext_diag::{Severity, render_diagnostic, render_diagnostics};
use handl_plugin_ext_refinement::{Dialect, detect_dialect};
use handl_runtime::{EngineRegistry, Runtime, RuntimeConfig};
use serde_json::{Value, json};

use handl_kit_arrow_lower::lower_ast;

mod transform_categorical;

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
    let mut kit_name = std::env::var("HANDL_KIT").unwrap_or_else(|_| "hello".to_string());
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
            "--help" | "-h" | "help" => {
                println!(
                    "usage: hnl <command> [args...]\n\
                     \n\
                     commands:\n\
                     \x20 fmt [--check] [--write] [paths...]      format files\n\
                     \x20 lint [--fix] [--fail-on-warn] [paths...] lint files\n\
                     \x20 typecheck [paths...] (alias: check)      typecheck files\n\
                     \x20 test [--list] [--filter <pat>] [paths...] run tests\n\
                     \x20 run <file.hnl>                           run a file\n\
                     \x20 eval '<expression>'                      evaluate an expression\n\
                     \x20 transform <pass> <file.hnl>              run a transform pass\n\
                     \x20 gen-grammar [output_path]                generate TextMate grammar\n\
                     \n\
                     global flags: --applier <id> --config <path> --kit <name> --engine <name>"
                );
                std::process::exit(0);
            }

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
        return Err("usage: hnl <command> [args...]\ncommands: fmt, lint, typecheck (alias: check), test, run, eval, transform, compile, gen-grammar, sdkgen".into());
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
        // `check` is an alias: ADR-0094's "runs under `hnl check`" wording.
        "typecheck" | "check" => typecheck_command(&command_args),
        "test" => test_command(&command_args),
        "run" => run_file(command_args.first().map(String::as_str), engine.as_deref()),
        "eval" => eval_expression(command_args.first().map(String::as_str)),
        "transform" => transform_command(&command_args),
        "compile" => compile_command(&command_args),
        "gen-grammar" => gen_grammar(&kit_name, command_args.first().map(String::as_str)),
        // SDK generator: `hnl sdkgen c <header> --output <dir> [...]`.
        "sdkgen" => handl_plugin_ext_sdkgen::sdkgen_command(&command_args),
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
// `hnl fmt [--check] [--write] [--prelude <path>] [--language <name>] [paths...]`
// ---------------------------------------------------------------------------

fn load_prelude(path: &str) -> Result<HashSet<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read prelude file {path}: {e}"))?;
    Ok(text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn load_canonical_language() -> Result<Language, String> {
    Ok(handl_kit_data_canonical::assemble()
        .map_err(|e| format!("failed to assemble canonical language: {e}"))?
        .lang()
        .clone())
}

fn fmt_command(args: &[String]) -> i32 {
    let mut check = false;
    let mut write = false;
    let mut language: Option<String> = None;
    let mut paths = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check = true,
            "--write" => write = true,
            "--prelude" => {
                // Accepted for CLI consistency with `lint`; it has no effect on formatting.
                i += 1;
                if i >= args.len() {
                    eprintln!("fmt: --prelude requires a value");
                    return 2;
                }
            }
            "--language" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("fmt: --language requires a value");
                    return 2;
                }
                language = Some(args[i].clone());
            }
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("fmt: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
        i += 1;
    }

    if check && write {
        eprintln!("fmt: --check and --write are mutually exclusive");
        return 2;
    }
    if let Some(lang) = language {
        if lang != "canonical" {
            eprintln!(
                "custom languages are not yet supported by the CLI; use the programmatic `_with_language` APIs"
            );
            return 2;
        }
    }

    let mut had_failure = false;

    if paths.is_empty() {
        // Read from stdin.
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("fmt: failed to read stdin");
            return 2;
        }
        let code = fmt_one(&src, "<stdin>", check, write, &mut had_failure);
        if code == 2 {
            return 2;
        }
        return if had_failure { 1 } else { 0 };
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

    if had_failure { 1 } else { 0 }
}

/// Format a single source. Returns 2 on parse error (and sets `had_failure`
/// for check failures). Otherwise returns 0.
fn fmt_one(src: &str, path: &str, check: bool, write: bool, had_failure: &mut bool) -> i32 {
    if detect_dialect(src) != Dialect::Canonical {
        eprintln!("skipping dialect file: {path}");
        return 0;
    }

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
// `hnl lint [--fix] [--fail-on-warn] [--prelude <path>] [--language <name>] [paths...]`
// ---------------------------------------------------------------------------

fn lint_command(args: &[String]) -> i32 {
    let mut fix = false;
    let mut fail_on_warn = false;
    let mut prelude_path: Option<String> = None;
    let mut language: Option<String> = None;
    let mut paths = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fix" => fix = true,
            "--fail-on-warn" => fail_on_warn = true,
            "--prelude" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("lint: --prelude requires a value");
                    return 2;
                }
                prelude_path = Some(args[i].clone());
            }
            "--language" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("lint: --language requires a value");
                    return 2;
                }
                language = Some(args[i].clone());
            }
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("lint: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
        i += 1;
    }

    if let Some(ref lang) = language {
        if lang != "canonical" {
            eprintln!(
                "custom languages are not yet supported by the CLI; use the programmatic `_with_language` APIs"
            );
            return 2;
        }
    }

    let use_with_language = prelude_path.is_some() || language.is_some();
    let prelude = match prelude_path {
        Some(path) => match load_prelude(&path) {
            Ok(set) => set,
            Err(msg) => {
                eprintln!("lint: {msg}");
                return 2;
            }
        },
        None => typecheck_wire_prelude().clone(),
    };
    let lang = if use_with_language {
        match load_canonical_language() {
            Ok(l) => Some(l),
            Err(msg) => {
                eprintln!("lint: {msg}");
                return 2;
            }
        }
    } else {
        None
    };

    let mut had_failure = false;

    if paths.is_empty() {
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("lint: failed to read stdin");
            return 2;
        }
        let code = lint_one(
            &src,
            "<stdin>",
            fix,
            fail_on_warn,
            &mut had_failure,
            &prelude,
            lang.as_ref(),
        );
        if code == 2 {
            return 2;
        }
        return if had_failure { 1 } else { 0 };
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("lint: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = lint_one(
            &src,
            path,
            fix,
            fail_on_warn,
            &mut had_failure,
            &prelude,
            lang.as_ref(),
        );
        if code == 2 {
            return 2;
        }
    }

    if had_failure { 1 } else { 0 }
}

/// Lint a single source. Returns 2 on parse error; sets `had_failure` for
/// Error diagnostics (or Warning when `fail_on_warn`).
fn lint_one(
    src: &str,
    path: &str,
    fix: bool,
    fail_on_warn: bool,
    had_failure: &mut bool,
    prelude: &HashSet<String>,
    lang: Option<&Language>,
) -> i32 {
    if detect_dialect(src) != Dialect::Canonical {
        eprintln!("skipping dialect file: {path}");
        return 0;
    }

    // `unbound-wire` is delegated to the bootstrap typechecker's scope pass;
    // seed it with the wire prelude supplied by the host (canonical or user).
    let lint_result = match lang {
        Some(lang) => {
            handl_plugin_ext_lint::lint_source_with_prelude_and_language(lang, src, prelude)
        }
        None => handl_plugin_ext_lint::lint_source_with_prelude(src, prelude),
    };
    let diags = match lint_result {
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
        match handl_plugin_ext_lint::fix_source_with_prelude(src, prelude) {
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
// `hnl typecheck [paths...]`
// ---------------------------------------------------------------------------

fn typecheck_command(args: &[String]) -> i32 {
    let mut paths = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("typecheck: unknown flag `{s}`");
                return 2;
            }
            s => paths.push(s.to_string()),
        }
    }

    // The canonical `Language` is assembled once here (single YAML→Language
    // path via `handl-kit-canonical`) and injected into the checker.
    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("typecheck: {e}");
            return 2;
        }
    };

    let mut had_failure = false;

    if paths.is_empty() {
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("typecheck: failed to read stdin");
            return 2;
        }
        let code = typecheck_one(&kit, &src, "<stdin>", &mut had_failure);
        if code == 2 {
            return 2;
        }
        return if had_failure { 1 } else { 0 };
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("typecheck: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = typecheck_one(&kit, &src, path, &mut had_failure);
        if code == 2 {
            return 2;
        }
    }

    if had_failure { 1 } else { 0 }
}

/// The process-wide unbound-wire prelude for `hnl typecheck`: the names
/// the canonical kit's module chain supplies at eval time (ADR-0096).
fn typecheck_wire_prelude() -> &'static std::collections::HashSet<String> {
    static PRELUDE: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    PRELUDE.get_or_init(handl_kit_canonical::canonical_wire_names)
}

/// Typecheck a single source. Returns 2 on parse error; sets `had_failure`
/// for Error diagnostics.
///
/// The canonical-dialect path always routes through `Kit::typecheck_handl`
/// (the Handl-authored `.hnl` model adapter, ADR-0105). The dialect routing
/// (Contract/Arrow) is unchanged — only canonical sources reach the Handl
/// engine.
fn typecheck_one(kit: &Kit, src: &str, path: &str, had_failure: &mut bool) -> i32 {
    // Route non-canonical dialects to their own loaders (ADR-0077).
    match detect_dialect(src) {
        Dialect::Contract => {
            return match handl_plugin_ext_refinement::parse_contracts(src) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("{path}: contract error: {e}");
                    2
                }
            };
        }
        Dialect::Arrow => {
            // A successful dialect parse means the file is valid in the
            // dialect that owns it; `hnl typecheck` is the canonical
            // checker, so arrow modules are accepted as-is.
            return 0;
        }
        Dialect::Canonical => {}
    }

    // Handl engine path (ADR-0105). The Handl model adapter lexes, parses,
    // and typechecks via the embedded `.hnl` model. On success it returns
    // the synthesized type; on failure the `LangError` is rendered and
    // recorded as a failure.
    match kit.typecheck_handl(src) {
        Ok(typ) => {
            println!("{path}: ok: {typ}");
            0
        }
        Err(err) => {
            // Lex failures and other parse-stage engine errors are exit 2.
            // `typecheck_handl` reports typecheck failures with the prefix
            // "typecheck errors: ..."; those are exit 1 and are printed to
            // stdout so the diagnostic is visible to the `hnl typecheck`
            // tests.
            if matches!(err, LangError::LexError { .. }) {
                eprintln!("{path}: {err}");
                return 2;
            }
            if err.to_string().contains("typecheck errors:") {
                println!("{path}: {err}");
                *had_failure = true;
                return 0;
            }
            eprintln!("{path}: {err}");
            2
        }
    }
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
    let mut tags: Vec<String> = Vec::new();
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
                    Some(v) => tags.push(v.clone()),
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

    // Default paths: `tests/` if it exists, otherwise the current directory.
    if paths.is_empty() {
        if std::path::Path::new("tests").is_dir() {
            paths.push("tests".to_string());
        } else {
            paths.push(".".to_string());
        }
    }

    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();

    // Discover with the filesystem provider by default, or the git-changed
    // provider when `--changed` is requested.
    let plan = if changed {
        let provider = handl_plugin_engine_test::ChangedProvider;
        match handl_plugin_engine_test::discover_with(&provider, &path_bufs, None) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("test: discovery error: {e}");
                return 2;
            }
        }
    } else {
        match handl_plugin_engine_test::discover(&path_bufs) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("test: discovery error: {e}");
                return 2;
            }
        }
    };

    // Report parse errors discovered while scanning test modules.
    let mut had_parse_error = false;
    for (file, msg) in &plan.parse_errors {
        eprintln!("{}: parse error: {msg}", file.display());
        had_parse_error = true;
    }

    // Compose the selection predicate: name filter AND every `--tag`
    // expression. Each `--tag` value is a tag expression — `a,b` selects
    // tests tagged `a` OR `b`, `!a` selects tests NOT tagged `a` — and
    // repeated `--tag` flags AND together.
    let mut predicate = handl_plugin_engine_test::filter_predicate(filter.as_deref().unwrap_or(""));
    for expr in &tags {
        predicate = predicate.and(handl_plugin_engine_test::tag_expr_predicate(expr));
    }

    if list {
        for case in &plan.tests {
            if predicate.eval(case) {
                println!("{} ({})", case.name, case.module.display());
            }
        }
        return if had_parse_error { 1 } else { 0 };
    }

    // Build the run config from the CLI flags.
    let mut config = handl_plugin_engine_test::RunConfig::new();
    config.predicate = predicate;
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

    // Disk-persisted result cache (ADR-0089). The cache lives at
    // `.handl/cache/test-runner/cache.json` under the working directory.
    let cache_path = PathBuf::from(".handl/cache/test-runner/cache.json");
    if cache {
        let cache = if cache_path.exists() {
            match handl_plugin_engine_test::TestRunCache::load(&cache_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "test: warning: failed to load cache at {}: {e}",
                        cache_path.display()
                    );
                    handl_plugin_engine_test::TestRunCache::new()
                }
            }
        } else {
            handl_plugin_engine_test::TestRunCache::new()
        };
        config = config.with_cache(cache);
    }

    let report = handl_plugin_engine_test::run_with_config(&plan, &mut config);

    if cache {
        if let Some(cache) = config.cache.as_ref() {
            if let Err(e) = cache.save(&cache_path) {
                eprintln!(
                    "test: warning: failed to save cache at {}: {e}",
                    cache_path.display()
                );
            }
        }
    }

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
// Import declaration stripping for `hnl run`
// ---------------------------------------------------------------------------

/// Remove top-level import *declaration* lines — the forms that name a
/// *host module* (resolved via the module chain / `HandlModule` wiring),
/// which the file-based `ModuleLoader` does not resolve (ADR-0096
/// §"Module-name imports"). The canonical kit's module chain already
/// supplies those bindings, so the lines are decorative here.
///
/// Kept (executable import ops resolved through the `ModuleLoader`):
/// - `import "<spec>"` — always;
/// - `from "<spec>" import ...` when `<spec>` resolves to an existing
///   file under `bases` (the entry directory, then the module root, then
///   the process working directory — mirroring `ModuleLoader`
///   resolution) — e.g. a `from "./m.hnl" import * as m` line inside a
///   multi-line `do { ... }` block is a real statement.
///
/// Stripped (host-module declarations):
/// - `import <name> ...` / `import <name> from "<spec>"` — the
///   module-declaration / arrow-dialect order, which the canonical grammar
///   cannot execute;
/// - `from "<bare-name>" import ...` — `<spec>` names a host module
///   (`str`, `fn`, `dedent`, ...) or a specifier with no backing file —
///   the module chain supplies the bindings either way.
fn strip_import_declarations(src: &str, bases: &[&Path]) -> String {
    src.lines()
        .filter(|line| !is_import_line(line.trim_start(), bases))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_import_line(trimmed: &str, bases: &[&Path]) -> bool {
    if let Some(rest) = trimmed.strip_prefix("import") {
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            // `import "spec"` / `import 'spec'` is the executable form —
            // keep it. `import <name> ...` (e.g. `import insert from
            // "collection/insert"`) is a module declaration — strip it.
            let inner = rest.trim_start();
            return !inner.starts_with('"') && !inner.starts_with('\'');
        }
        // `import` directly followed by a non-space char is an identifier
        // (`imported_and`) or the quoted executable form (`import"x"` —
        // kept either way): never a strippable declaration.
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix("from") {
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            // Strip iff the specifier names a host module — i.e. does not
            // resolve to an existing file. An unparseable line is kept so
            // the parser reports the error.
            return match first_quoted_spec(rest) {
                Some(spec) => !spec_resolves_to_file(spec, bases),
                None => false,
            };
        }
    }
    false
}

/// Whether `<spec>` resolves to an existing file under any of `bases`
/// (checked in order — mirroring `ModuleLoader` relative resolution:
/// importing module's directory, module root, working directory). A
/// `.handl.toml` `[dependencies]` specifier that resolves outside these
/// bases is stripped like any other host-module declaration; the
/// declaration is decorative here either way and a referenced name fails
/// loudly downstream.
///
/// A directory is treated as a package if it contains an `index.hnl` or
/// a `<dir>/<name>.hnl` main file; a bare name without `.hnl` is tried
/// with `.hnl` appended.
fn spec_resolves_to_file(spec: &str, bases: &[&Path]) -> bool {
    if Path::new(spec).is_absolute() {
        return file_candidate(Path::new(spec), spec).is_some();
    }
    bases
        .iter()
        .any(|base| file_candidate(&base.join(spec), spec).is_some())
}

fn file_candidate(joined: &Path, spec: &str) -> Option<PathBuf> {
    if joined.is_dir() {
        let index = joined.join("index.hnl");
        if index.is_file() {
            return Some(index);
        }
        if let Some(name) = joined.file_name().and_then(|s| s.to_str()) {
            let main = joined.join(format!("{}.hnl", name));
            if main.is_file() {
                return Some(main);
            }
        }
    }
    if !spec.ends_with(".hnl") {
        let with_ext = joined.with_extension("hnl");
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    if joined.is_file() {
        Some(joined.to_path_buf())
    } else {
        None
    }
}

/// Extract the first `'`- or `"`-quoted string literal's contents from a
/// source fragment (e.g. the specifier in `from "..." import ...`).
fn first_quoted_spec(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'"' || b == b'\'')?;
    let quote = bytes[start];
    let rest = &s[start + 1..];
    let end = rest.as_bytes().iter().position(|&b| b == quote)?;
    Some(&rest[..end])
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

    // Project-aware kit: discover `.handl.toml` by walking up from the
    // source file's directory; its `[dependencies]` table feeds the module
    // loader and its directory becomes the module root for `import "path"`.
    let start = Path::new(path);
    let kit = match engine {
        Some(name) => match Kit::discover_with_eval(start, name) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("run: {e}");
                return 1;
            }
        },
        None => match Kit::discover(start) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("run: {e}");
                return 1;
            }
        },
    };

    // Typecheck before running. Canonical-dialect sources go through the
    // embedded Handl model; non-canonical dialects (Contract/Arrow) are
    // checked by their own loaders, same routing as `hnl typecheck`.
    if detect_dialect(&source) == Dialect::Canonical {
        if let Err(err) = kit.typecheck_handl(&source) {
            eprintln!("run: typecheck failed: {err}");
            return 1;
        }
    }

    // Resolution bases for `from "<spec>"` declaration lines, mirroring
    // `ModuleLoader` order: the entry file's directory, the module root,
    // then the process working directory. A `from` line whose specifier
    // resolves to a file under one of these is kept (it is the executable
    // import op); one that names a host module is stripped.
    let mut bases: Vec<&Path> = Vec::new();
    let dot = Path::new(".");
    match start.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => bases.push(dir),
        _ => bases.push(dot),
    }
    if let Some(root) = kit.module_root() {
        if !bases.contains(&root) {
            bases.push(root);
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if !bases.iter().any(|b| *b == cwd.as_path()) {
        bases.push(&cwd);
    }

    let source = strip_import_declarations(&source, &bases);
    let trimmed = source.trim_start();
    let wrapped = if trimmed.starts_with("do") {
        source.to_string()
    } else {
        format!("do {{\n{}\n}}", source)
    };

    // Parse, apply the module's own ~[2]~> rules, then eval the optimized AST.
    let ast = match kit.parse(&wrapped) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("run: parse failed: {e}");
            return 1;
        }
    };
    let lang = kit.runtime().lang();
    // Bound names = module bindings + the wire names the module chain
    // supplies at eval time (stripped `from "pkg" import ...` lines are
    // not in the AST, but their names are still literal references, not
    // pattern variables).
    let mut bound = transform_categorical::bound_names(&ast, lang);
    bound.extend(typecheck_wire_prelude().iter().cloned());
    let env = transform_categorical::dim_env(&ast, lang);
    let rules = transform_categorical::collect_rules(&ast, &bound, &env, lang);
    let protected = Arc::new(transform_categorical::dim_arrow_subtrees(&ast));
    let engine = handl_plugin_engine_transform_rewrite::RewriteEngine::new().with_native_registry(
        transform_categorical::native(rules, protected, Arc::new(env)),
    );
    let out = match engine.run_pass(lang, &ast, AstRef::new(ast.len() - 1), "n_dim_opt") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("run: optimization pass failed: {e}");
            return 1;
        }
    };

    match kit.runtime().eval(&out.ast) {
        Ok(value) => {
            print_value(&value, lang, Some(wrapped.as_str()));
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

    // Project-aware kit: discover `.handl.toml` from the CWD upward so
    // `import "dep/..."` resolves against `[dependencies]`.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let kit = match Kit::discover(&cwd) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("eval: {e}");
            return 1;
        }
    };

    // The Pratt parser handles one top-level statement per call; wrap the
    // user expression in a `do` block so semicolon-separated commands such as
    // `from "transform" import * as transform; transform.map(...)` evaluate as a sequence.
    let wrapped = format!("do {{ {} }}", expr);
    let lang = kit.runtime().lang();
    match kit.eval(&wrapped) {
        Ok(value) => {
            print_value(&value, lang, Some(wrapped.as_str()));
            0
        }
        Err(e) => {
            eprintln!("eval: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// `hnl transform <pass> <file> [--pipeline a,b,c] [--plan] [--trace] [--stats] [--emit tree|json]`
// ---------------------------------------------------------------------------
//
// Runs a named transform pass (ADR-0094) over the RAW, pre-desugar AST.
// `Runtime::parse` applies the `desugar` pass at parse time (ADR-0082), so
// this command parses via `lex` + `PrattEngine` directly — the same raw-parse
// path the bootstrap typechecker uses — then runs the pass through
// `RewriteEngine` with `with_desugar_natives` so `TransformRule::Native` rules
// (and the `desugar.*` native fallback table) resolve.
//
// Pass validation: a pass is known iff `lang.transforms` declares at least
// one rule for it (or it is `"desugar"`, which also has the native fallback
// table). An unknown pass is a **usage error** (exit 2) — a misspelled pass
// name should not silently succeed. A known pass that simply matches no
// nodes exits 0 with empty output (valid: the pass ran, nothing fired).

/// Parsed `hnl transform` arguments.
struct TransformArgs {
    /// The pass name (first positional) — ignored when `pipeline` is set.
    pass: Option<String>,
    /// The input file (second positional).
    file: Option<String>,
    /// `--pipeline a,b,c`: ordered pass list (overrides `pass`).
    pipeline: Option<Vec<String>>,
    /// `--plan`: report match sites without applying.
    plan: bool,
    /// `--trace`: print each rewrite.
    trace: bool,
    /// `--stats`: print `PassStats`.
    stats: bool,
    /// `--write`: emit preserving source and overwrite the file.
    write: bool,
    /// `--emit tree|json|source` (default `tree`).
    emit: String,
}

fn transform_command(args: &[String]) -> i32 {
    let mut t = TransformArgs {
        pass: None,
        file: None,
        pipeline: None,
        plan: false,
        trace: false,
        stats: false,
        write: false,
        emit: "tree".to_string(),
    };

    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pipeline" => {
                i += 1;
                match args.get(i) {
                    Some(v) => {
                        t.pipeline = Some(
                            v.split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect(),
                        )
                    }
                    None => {
                        eprintln!("transform: --pipeline requires a comma-separated value");
                        return 2;
                    }
                }
            }
            "--emit" => {
                i += 1;
                match args.get(i) {
                    Some(v) => t.emit = v.clone(),
                    None => {
                        eprintln!("transform: --emit requires a value (tree|json|source)");
                        return 2;
                    }
                }
            }
            "--plan" => t.plan = true,
            "--trace" => t.trace = true,
            "--stats" => t.stats = true,
            "--write" => t.write = true,
            "--" => {}
            s if s.starts_with("--") => {
                eprintln!("transform: unknown flag `{s}`");
                return 2;
            }
            s => positional.push(s.to_string()),
        }
        i += 1;
    }

    // Positionals: `<pass> <file>`. When `--pipeline` is given, the pass
    // positional is optional — the last positional is always the file.
    if t.pipeline.is_some() {
        t.file = positional.pop();
        t.pass = positional.pop();
        if !positional.is_empty() {
            eprintln!("transform: too many positional arguments");
            return 2;
        }
    } else {
        let mut it = positional.into_iter();
        t.pass = it.next();
        t.file = it.next();
        if it.next().is_some() {
            eprintln!("transform: too many positional arguments");
            return 2;
        }
    }

    // Categorical two-file mode: `hnl transform <rules.hnl> <program.hnl>`.
    // The first positional ends in `.hnl` and exists, so route to the
    // prototype categorical rewrites rather than the named-pass engine.
    if let (Some(rules_path), Some(prog_path)) = (&t.pass, &t.file) {
        if rules_path.ends_with(".hnl") && prog_path.ends_with(".hnl") {
            return transform_categorical::run(rules_path, prog_path);
        }
    }

    if t.emit != "tree" && t.emit != "json" && t.emit != "source" {
        eprintln!(
            "transform: unknown emit format `{}` (expected tree|json|source)",
            t.emit
        );
        return 2;
    }

    if t.write && t.emit != "tree" && t.emit != "source" {
        eprintln!("transform: --write requires `--emit source`");
        return 2;
    }

    let file = match &t.file {
        Some(f) => f.clone(),
        None => {
            eprintln!(
                "usage: hnl transform <pass> <file.hnl> [--pipeline a,b,c] [--plan] [--trace] [--stats] [--emit tree|json|source] [--write]"
            );
            return 2;
        }
    };
    if t.pass.is_none() && t.pipeline.is_none() {
        eprintln!("transform: missing <pass> (or --pipeline)");
        return 2;
    }

    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: failed to read {file}: {e}");
            return 2;
        }
    };

    let kit = match Kit::new() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("transform: {e}");
            return 1;
        }
    };
    let lang = kit.runtime().lang();

    // Validate the requested pass(es): every pass must declare at least one
    // rule in `lang.transforms`. `"desugar"` additionally has the native
    // fallback table, but it always has declarative rules too.
    let passes: Vec<String> = match &t.pipeline {
        Some(ps) => ps.clone(),
        None => vec![t.pass.clone().unwrap()],
    };
    for p in &passes {
        if !lang.transforms.iter().any(|d| d.pass == *p) {
            let mut known: Vec<String> = lang
                .transforms
                .iter()
                .map(|d| d.pass.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            known.sort();
            eprintln!(
                "transform: unknown pass `{p}` (known passes: {})",
                known.join(", ")
            );
            return 2;
        }
    }

    // Raw-parse each top-level statement (split at bracket-depth zero, the
    // same convention as the typechecker/linter/test runner) — no desugar.
    let stmts = match transform_parse_raw(lang, &src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("transform: {file}: parse error: {e}");
            return 2;
        }
    };

    // Build the handl-native rewrite registry (ADR-0094) from the kit's
    // `.hnl` modules, layer the `desugar.*` natives underneath as a
    // fallback, and wire both into the engine. Without this, a
    // `transforms.yaml` rule with `rule: !Native "handl:<mod>.<fn>"` errors
    // as "unknown native" — the adapter is the production seam.
    let handl_natives = Arc::new(kit.handl_rewrite_adapter().with_fallback(Arc::new(
        handl_plugin_engine_transform_rewrite::native::desugar_native_registry(),
    )));
    let engine = handl_plugin_engine_transform_rewrite::RewriteEngine::new()
        .with_desugar_and_handl_natives(handl_natives);

    if t.plan {
        // Match-only: report match sites + rule ids without rewriting.
        let mut count = 0usize;
        for (n, (ast, _)) in stmts.iter().enumerate() {
            let root = handl_language::AstRef::new(ast.len() - 1);
            let sites: Vec<_> = passes
                .iter()
                .flat_map(|p| engine.find_matches(lang, ast, root, p))
                .collect();
            if sites.is_empty() {
                continue;
            }
            if stmts.len() > 1 {
                println!("-- statement {}", n + 1);
            }
            for m in sites {
                let node = ast.get(m.at);
                let kind = node.map(|nd| nd.kind.as_str()).unwrap_or("?");
                let span = node
                    .and_then(|nd| nd.span)
                    .map(|s| format!(" @{}..{}", s.start, s.end))
                    .unwrap_or_default();
                println!("match {kind}{span} <- {}", m.rule_id);
                count += 1;
            }
        }
        println!("plan: {count} match site(s)");
        return 0;
    }

    let pass_refs: Vec<&str> = passes.iter().map(String::as_str).collect();
    let mut json_out: Vec<serde_json::Value> = Vec::new();
    let mut source_parts: Vec<String> = Vec::new();
    let mut had_error = false;

    for (n, (ast, info)) in stmts.iter().enumerate() {
        let root = handl_language::AstRef::new(ast.len() - 1);
        let out = if t.pipeline.is_some() {
            engine.run_pipeline(lang, ast, root, &pass_refs)
        } else {
            engine.run_pass(lang, ast, root, &pass_refs[0])
        };
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                eprintln!("transform: {file}: pass failed: {e}");
                had_error = true;
                continue;
            }
        };

        if t.write || t.emit == "source" {
            if out.trace.rewrites.is_empty() {
                source_parts.push(src[info.start..info.end].to_string());
            } else {
                if let Err(kinds) = check_emittable(lang, &out.ast, out.root) {
                    eprintln!(
                        "transform: {file}: pass '{}' produced internal-form node(s): {}",
                        passes.join(", "),
                        kinds.join(", ")
                    );
                    eprintln!(
                        "  --emit source and --write are only meaningful for surface-preserving passes"
                    );
                    had_error = true;
                    continue;
                }
                let changed: Vec<handl_language::AstRef> =
                    out.trace.rewrites.iter().map(|r| r.to).collect();
                let emitter = match handl_plugin_engine_fmt::PreserveEmitter::new(
                    &src, &out.ast, out.root, &changed,
                ) {
                    Ok(e) => e,
                    Err(e) => {
                        eprintln!(
                            "transform: {file}: emit failed for statement {}: {e}",
                            n + 1
                        );
                        had_error = true;
                        continue;
                    }
                };
                match emitter.format(lang, &out.ast, out.root) {
                    Ok(s) => source_parts.push(s),
                    Err(e) => {
                        eprintln!(
                            "transform: {file}: emit failed for statement {}: {e}",
                            n + 1
                        );
                        had_error = true;
                        continue;
                    }
                }
            }
        } else if t.emit == "json" {
            json_out.push(json!({
                "statement": n + 1,
                "root": out.root.index(),
                "nodes": out.ast.nodes(),
            }));
        } else {
            if stmts.len() > 1 {
                println!("-- statement {}", n + 1);
            }
            let mut buf = String::new();
            render_ast_tree(&out.ast, out.root.index(), 0, &mut buf);
            print!("{buf}");
        }

        if t.trace {
            for r in &out.trace.rewrites {
                let from_kind = ast.get(r.from).map(|nd| nd.kind.as_str()).unwrap_or("?");
                let to_kind = out.ast.get(r.to).map(|nd| nd.kind.as_str()).unwrap_or("?");
                println!(
                    "rewrite #{} {from_kind} -> #{} {to_kind} [{}]",
                    r.from.index(),
                    r.to.index(),
                    r.rule_id
                );
            }
        }

        if t.stats {
            println!("visited: {}", out.stats.visited);
            println!("rewritten: {}", out.stats.rewritten);
            println!(
                "duration_ms: {:.3}",
                out.stats.duration.as_secs_f64() * 1000.0
            );
            for (rule, count) in &out.stats.per_rule {
                println!("rule {rule}: {count}");
            }
        }
    }

    if t.emit == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_out).unwrap_or_default()
        );
    } else if t.write {
        if had_error {
            return 1;
        }
        let mut out = String::new();
        if !stmts.is_empty() {
            out.push_str(&src[0..stmts[0].1.start]);
            for (i, emitted) in source_parts.iter().enumerate() {
                out.push_str(emitted);
                if i + 1 < stmts.len() {
                    out.push_str(&src[stmts[i].1.end..stmts[i + 1].1.start]);
                } else {
                    out.push_str(&src[stmts[i].1.end..]);
                }
            }
        }
        if let Err(e) = write_atomic(&file, &out) {
            eprintln!("transform: failed to write {file}: {e}");
            return 2;
        }
    } else if t.emit == "source" {
        if had_error {
            return 1;
        }
        let mut out = String::new();
        if !stmts.is_empty() {
            out.push_str(&src[0..stmts[0].1.start]);
            for (i, emitted) in source_parts.iter().enumerate() {
                out.push_str(emitted);
                if i + 1 < stmts.len() {
                    out.push_str(&src[stmts[i].1.end..stmts[i + 1].1.start]);
                } else {
                    out.push_str(&src[stmts[i].1.end..]);
                }
            }
        }
        print!("{out}");
    }

    if had_error { 1 } else { 0 }
}

/// Source span of a single top-level statement within the original file.
///
/// `start` and `end` are byte offsets of the trimmed statement text; the
/// `source` field holds that text for parsing.
struct StatementInfo {
    source: String,
    start: usize,
    end: usize,
}

/// Raw-parse `src` into one [`handl_language::Ast`] per top-level statement,
/// WITHOUT the desugar pass (`Runtime::parse` always desugars — ADR-0082).
///
/// Statements are split at bracket-depth zero, respecting string literals
/// and `--` comments — the same convention as the bootstrap typechecker,
/// linter, and test runner. Each returned AST's spans are offset back into
/// the original `src` so the preserving emitter can splice from it.
fn transform_parse_raw(
    lang: &handl_language::Language,
    src: &str,
) -> Result<Vec<(handl_language::Ast, StatementInfo)>, LangError> {
    use handl_plugin_engine_parse::ParserEngine;

    let mut asts = Vec::new();
    for info in split_statements(src) {
        let tokens = handl_plugin_engine_lex_pattern::lex(lang, &info.source)?;
        if tokens.as_slice().is_empty() {
            continue;
        }
        let req = handl_plugin_engine_parse::ParserRequest::new(tokens);
        let mut ast = handl_plugin_engine_parse_pratt::PrattEngine::new().parse(lang, &req)?;
        if !ast.is_empty() {
            offset_spans(&mut ast, info.start)?;
            asts.push((ast, info));
        }
    }
    Ok(asts)
}

/// Offset every span in `ast` by `delta`.
fn offset_spans(ast: &mut handl_language::Ast, delta: usize) -> Result<(), LangError> {
    for i in 0..ast.len() {
        let r = handl_language::AstRef::new(i);
        let node = ast
            .get_mut(r)
            .ok_or(handl_language::LangError::AstIndexOutOfBounds { index: i })?;
        if let Some(span) = node.span.as_mut() {
            span.start += delta;
            span.end += delta;
        }
    }
    Ok(())
}

/// Split `src` into top-level statement chunks at bracket-depth zero,
/// respecting string literals and `--` comments (mirrors the bootstrap
/// typechecker / linter / test runner convention — the Pratt parser parses
/// one statement per call).
fn split_statements(src: &str) -> Vec<StatementInfo> {
    let bytes = src.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut depth: i32 = 0;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        // String literal — skip to the closing quote (respecting escapes).
        if b == b'"' {
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => {
                        i += 2;
                        continue;
                    }
                    b'"' => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            continue;
        }

        // Comment: `--` to end of line.
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }

        if depth == 0 && (b == b'\n' || b == b';') {
            let (trim_start, trim_end) = trim_offsets(src, start, i);
            if trim_start < trim_end {
                chunks.push(StatementInfo {
                    source: src[trim_start..trim_end].to_string(),
                    start: trim_start,
                    end: trim_end,
                });
            }
            i += 1;
            start = i;
            continue;
        }

        i += 1;
    }

    let (trim_start, trim_end) = trim_offsets(src, start, src.len());
    if trim_start < trim_end {
        chunks.push(StatementInfo {
            source: src[trim_start..trim_end].to_string(),
            start: trim_start,
            end: trim_end,
        });
    }
    chunks
}

/// Return the byte offsets of the non-whitespace content within `src[a..b]`.
fn trim_offsets(src: &str, a: usize, b: usize) -> (usize, usize) {
    let bytes = src.as_bytes();
    let mut start = a;
    while start < b && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = b;
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    (start, end)
}

/// Check whether `ast` can be emitted back to surface syntax without loss.
///
/// Created (`span: None`) nodes whose kind has no grammar row and no useful
/// leaf payload are internal forms (e.g. `rec`, `if__continue`) and cannot be
/// turned into valid source. Returns the list of offending kinds.
fn check_emittable(
    lang: &handl_language::Language,
    ast: &handl_language::Ast,
    root: handl_language::AstRef,
) -> Result<(), Vec<String>> {
    let grammar_kinds: std::collections::HashSet<String> = lang
        .grammar
        .iter()
        .map(|r| r.child_shape.node_kind.clone())
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![root.index()];
    let mut bad = Vec::new();
    while let Some(i) = stack.pop() {
        if !seen.insert(i) {
            continue;
        }
        let Some(node) = ast.get(handl_language::AstRef::new(i)) else {
            continue;
        };
        if node.span.is_none() && !grammar_kinds.contains(&node.kind) && !has_leaf_payload(node) {
            bad.push(node.kind.clone());
        }
        if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
            for c in children {
                if let Some(idx) = c.as_u64() {
                    stack.push(idx as usize);
                }
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        bad.sort();
        bad.dedup();
        Err(bad)
    }
}

/// Whether a node has a leaf-level payload that can be rendered as text.
fn has_leaf_payload(node: &handl_language::AstNode) -> bool {
    if node.payload.get("value").is_some() || node.payload.get("name").is_some() {
        return true;
    }
    !node.payload.is_object() && !node.payload.is_null()
}

/// Render an [`handl_language::Ast`] subtree as an indented tree:
/// `kind` plus a compact payload summary (the `children` index array is
/// elided — it is rendered structurally as indentation).
fn render_ast_tree(ast: &handl_language::Ast, idx: usize, depth: usize, out: &mut String) {
    let Some(node) = ast.get(handl_language::AstRef::new(idx)) else {
        return;
    };
    let indent = "  ".repeat(depth);

    // Payload summary: everything except the `children` index array.
    let summary = match &node.payload {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(map) => {
            let rest: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, _)| k.as_str() != "children")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if rest.is_empty() {
                String::new()
            } else {
                format!(" {}", serde_json::Value::Object(rest))
            }
        }
        other => format!(" {other}"),
    };

    let span = node
        .span
        .map(|s| format!("  ; @{}..{}", s.start, s.end))
        .unwrap_or_default();
    out.push_str(&format!("{indent}{}{summary}{span}\n", node.kind));

    if let Some(children) = node.payload.get("children").and_then(|c| c.as_array()) {
        for c in children {
            if let Some(ci) = c.as_u64() {
                render_ast_tree(ast, ci as usize, depth + 1, out);
            }
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
    let grammar =
        handl_host_tooling::generate_textmate(&rules, kit.scope_name(), &kit.file_extensions());

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
// ---------------------------------------------------------------------------
// `hnl compile <target> <file> [--output <path>] [--source-map] [--debug] [--format ...] [--to hughes] [--emit json|pretty]`
//
// Supported targets: `js`, `llvm`, `handl`, `term`, `dot`, `sexp`.
// ---------------------------------------------------------------------------

fn compile_command(args: &[String]) -> i32 {
    let request = match handl_plugin_ext_compile::CompileFileRequest::from_args(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("compile: {e}");
            return 2;
        }
    };

    let src = match std::fs::read_to_string(&request.path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("compile: failed to read {}: {e}", request.path.display());
            return 2;
        }
    };

    let data_kit = match handl_kit_data_canonical::assemble() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("compile: failed to assemble canonical language: {e}");
            return 2;
        }
    };

    let mut registry = EngineRegistry::new();
    registry.register_lex("data", Box::new(DataLexerEngine::new()));
    registry.register_parse("pratt", PrattEngine::new());

    let mut runtime_config = RuntimeConfig::new().with_lex("data").with_parse("pratt");
    runtime_config.eval.clear();
    runtime_config.typecheck.clear();

    let lang = data_kit.lang().clone();
    let runtime = match Runtime::from_registry(lang, &Rc::new(registry), &runtime_config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("compile: failed to build runtime: {e}");
            return 2;
        }
    };

    let source = Source::new(src.clone());
    let tokens = match runtime.lex(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("compile: lex error: {e}");
            return 2;
        }
    };
    let ast = match runtime.parse(&tokens) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("compile: parse error: {e}");
            return 2;
        }
    };

    if ast.is_empty() {
        eprintln!("compile: empty AST");
        return 2;
    }

    let root = AstRef::new(ast.len() - 1);
    let term = match lower_ast(&ast, runtime.lang(), root) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("compile: lower error: {e}");
            return 2;
        }
    };

    let report = handl_plugin_engine_typecheck::TypecheckReport::default();
    let input = handl_plugin_compile::CompilerInput::new(
        src,
        ast,
        root,
        runtime.lang().clone(),
        term,
        report,
    );

    let config: Box<dyn handl_plugin_compile::CompilerConfig> = match request.target.as_str() {
        "js" => build_js_config(&request),
        "llvm" => build_llvm_config(&request),
        "handl" => build_handl_config(&request),
        "term" => build_term_config(&request),
        "dot" => build_dot_config(&request),
        "sexp" => build_sexp_config(&request),
        other => {
            eprintln!("compile: unsupported target `{other}`");
            return 2;
        }
    };

    let dispatcher = handl_plugin_ext_compile::CompileDispatcher::new();
    let output = match dispatcher.compile(&request.target, input, config.as_ref()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("compile: {e}");
            return 1;
        }
    };

    if let Some(path) = &request.output {
        if let Err(e) = std::fs::write(path, output.primary_output().unwrap_or_default()) {
            eprintln!("compile: failed to write {}: {e}", path.display());
            return 2;
        }
    } else if let Err(e) = std::io::Write::write_all(
        &mut std::io::stdout(),
        output.primary_output().unwrap_or_default(),
    ) {
        eprintln!("compile: failed to write stdout: {e}");
        return 2;
    }

    0
}

fn build_js_config(
    req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_js::{
        CommonJs, Deno, Esm, Iife, Inline, JsConfig, JsOutputFormat, JsTarget,
    };

    let target: Box<dyn JsTarget> = Box::new(Deno);
    let format: Box<dyn JsOutputFormat> = match req.format.as_deref().unwrap_or("esm") {
        "esm" => Box::new(Esm),
        "commonjs" => Box::new(CommonJs),
        "iife" => Box::new(Iife),
        "inline" => Box::new(Inline),
        other => {
            // Unsupported format is treated like the default `esm`; the
            // dispatcher will still run and the output can be inspected.
            eprintln!("compile: unknown js format `{other}`, defaulting to esm");
            Box::new(Esm)
        }
    };

    Box::new(JsConfig {
        target,
        format,
        debug: req.debug,
        no_inline_foreign: false,
        source_map: req.source_map,
    })
}

fn build_llvm_config(
    req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_llvm::{LlvmConfig, X86_64};

    let mut cfg = LlvmConfig::new(Box::new(X86_64));
    cfg.debug = req.debug;
    cfg.emit_object = req.format.as_deref().map(|f| f == "o").unwrap_or(false);
    Box::new(cfg)
}

fn build_handl_config(
    req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_handl::HandlConfig;

    let to = req.to.as_deref().unwrap_or("hughes").to_string();
    Box::new(HandlConfig::new(to))
}

fn build_term_config(
    req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_term::TermConfig;

    let emit = req.emit.as_deref().unwrap_or("json").to_string();
    Box::new(TermConfig::new(emit))
}

fn build_dot_config(
    _req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_dot::DotConfig;

    Box::new(DotConfig::new())
}

fn build_sexp_config(
    _req: &handl_plugin_ext_compile::CompileFileRequest,
) -> Box<dyn handl_plugin_compile::CompilerConfig> {
    use handl_plugin_compile_sexp::SexpConfig;

    Box::new(SexpConfig::new())
}

// Helpers
// ---------------------------------------------------------------------------

/// Write `content` to `path` atomically: write to a temp file in the same
/// directory, then rename over the target.
fn write_atomic(path: &str, content: &str) -> std::io::Result<()> {
    let target = Path::new(path);
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".hnl-fmt-tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// Print a `serde_json::Value` the way the CLI presents eval/run results: bare
/// strings are printed without quotes, arrow values as their compact term
/// (`left >>> right` for composites, the `body_def` source for literals),
/// everything else as pretty-printed JSON.
fn print_value(value: &Value, lang: &Language, source: Option<&str>) {
    if let Some(s) = value.as_str() {
        println!("{s}");
    } else if let Some(s) = arrow_display(value, lang, source, None, 0) {
        println!("{s}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&compact_arrows(value, lang, source)).unwrap_or_default()
        );
    }
}

/// Recursively replace arrow values nested inside records/arrays with
/// their compact term display, so `hnl run` output stays readable.
fn compact_arrows(value: &Value, lang: &Language, source: Option<&str>) -> Value {
    if let Some(s) = arrow_display(value, lang, source, None, 0) {
        return Value::String(s);
    }
    match value {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), compact_arrows(v, lang, source)))
                .collect(),
        ),
        Value::Array(a) => {
            Value::Array(a.iter().map(|v| compact_arrows(v, lang, source)).collect())
        }
        _ => value.clone(),
    }
}

/// Compact display for an arrow value.
///
/// - A composite built by `>>>` captures its components in `env` as
///   `__left__`/`__right__` and prints as `left >>> right`. Components that
///   were unbound names evaluate to their name string (first-class name
///   references), so `fetch >>> serialize >>> parse` prints as written.
/// - A graded arrow (`{"__tag__": "Arrow", "grade": N, "inputs", "output"}`)
///   prints as `(inputs) ~[N]~> (output)`.
/// - A leaf arrow prints as the name it is bound under when `scope` still
///   holds that name for the same definition, otherwise as
///   `params ~> body` rendered from `body_def` (source text when spans are
///   available, else the data-driven formatter).
fn arrow_display(
    value: &Value,
    lang: &Language,
    source: Option<&str>,
    scope: Option<&serde_json::Map<String, Value>>,
    depth: usize,
) -> Option<String> {
    if depth > 16 {
        return Some("...".to_string());
    }
    // First-class name references: unbound vars evaluate to their name string.
    if let Value::String(s) = value {
        return Some(s.clone());
    }
    let obj = value.as_object()?;
    let is_plain = obj.get("__arrow__").and_then(|v| v.as_bool()) == Some(true);
    let is_graded = obj.get("__tag__").and_then(|v| v.as_str()) == Some("Arrow")
        && obj.get("grade").is_some()
        && obj.get("inputs").is_some()
        && obj.get("output").is_some();
    if !is_plain && !is_graded {
        return None;
    }
    let env = obj.get("env").and_then(|e| e.as_object());
    // A name bound in scope for this same definition wins over expansion.
    for e in scope.into_iter().chain(env.iter().copied()) {
        for (name, val) in e {
            if !name.starts_with("__") && def_eq(val, value, 0) {
                return Some(name.clone());
            }
        }
    }
    if is_graded {
        // Graded arrows carry no env of their own; the evaluated `inputs`
        // side does, and it captured the ambient scope — use it so named
        // arrows on either side print by name.
        let inner_scope = env.or_else(|| {
            obj.get("inputs")
                .and_then(|i| i.get("env"))
                .and_then(|e| e.as_object())
        });
        // The `dim` slot is a normal expression — `~[a >>> b]~>`, `~[N]~>`,
        // `~[f(x)]~>` — so the grade may be any value, not just a literal.
        let grade = term_display(
            obj.get("grade").unwrap(),
            lang,
            source,
            inner_scope,
            depth + 1,
        );
        let i = term_display(&obj["inputs"], lang, source, inner_scope, depth + 1);
        let o = term_display(&obj["output"], lang, source, inner_scope, depth + 1);
        return Some(format!(
            "{} ~[{grade}]~> {}",
            paren_if_term(&i),
            paren_if_term(&o)
        ));
    }
    let left = env.and_then(|e| e.get("__left__"));
    let right = env.and_then(|e| e.get("__right__"));
    if let (Some(l), Some(r)) = (left, right) {
        let ls = term_display(l, lang, source, env, depth + 1);
        let rs_raw = term_display(r, lang, source, env, depth + 1);
        return Some(format!("{ls} >>> {}", paren_if_term(&rs_raw)));
    }
    // Leaf arrow: `params ~> body`.
    let params = obj
        .get("params")
        .and_then(|p| p.as_array())
        .map(|p| {
            p.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    if let Some(def) = obj.get("body_def").and_then(handl_language::as_ast) {
        if !def.is_empty() {
            let root = AstRef::new(def.len() - 1);
            let body = source
                .zip(def.at(root).ok().and_then(|n| n.span))
                .and_then(|(src, span)| src.get(span.start..span.end))
                .map(|s| s.trim().to_string())
                .or_else(|| {
                    handl_plugin_engine_fmt::FormatEngine::new()
                        .format(lang, &def, root)
                        .ok()
                        .map(|s| s.trim().to_string())
                });
            if let Some(b) = body {
                return Some(format!("{params} ~> {b}"));
            }
        }
    }
    Some("<arrow>".to_string())
}

/// Display a term-position value: arrows get [`arrow_display`]; a bare
/// string is a name reference and prints raw; other values resolve to a
/// bound name via the captured scope (e.g. `a` for `2` in `a >>> b`), or
/// fall back to a compact literal.
fn term_display(
    value: &Value,
    lang: &Language,
    source: Option<&str>,
    scope: Option<&serde_json::Map<String, Value>>,
    depth: usize,
) -> String {
    if let Some(s) = arrow_display(value, lang, source, scope, depth) {
        return s;
    }
    if let Value::String(s) = value {
        return s.clone();
    }
    if let Some(env) = scope {
        for (name, val) in env {
            if !name.starts_with("__") && def_eq(val, value, 0) {
                return name.clone();
            }
        }
    }
    match value {
        Value::Null => "null".to_string(),
        v => serde_json::to_string(v).unwrap_or_else(|_| "<value>".to_string()),
    }
}

/// Parenthesize a displayed subterm when it is a composite or arrow literal
/// (contains `>>>` or `~>`); names and atoms print bare.
fn paren_if_term(s: &str) -> String {
    if s.contains(">>>") || s.contains("~>") {
        format!("({s})")
    } else {
        s.to_string()
    }
}

/// Structural equality of arrow definitions, ignoring captured `env` (context,
/// not identity) except for the `__left__`/`__right__` composite components,
/// which are compared recursively.
fn def_eq(a: &Value, b: &Value, depth: usize) -> bool {
    if depth > 16 {
        return true;
    }
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            for (k, av) in ao {
                if k == "env" || k == "span" {
                    continue;
                }
                if !bo.get(k).is_some_and(|bv| def_eq(av, bv, depth + 1)) {
                    return false;
                }
            }
            for k in bo.keys() {
                if k != "env" && k != "span" && !ao.contains_key(k) {
                    return false;
                }
            }
            let al = a.pointer("/env/__left__");
            let ar = a.pointer("/env/__right__");
            let bl = b.pointer("/env/__left__");
            let br = b.pointer("/env/__right__");
            (al.is_none() && bl.is_none()
                || def_eq(
                    al.unwrap_or(&Value::Null),
                    bl.unwrap_or(&Value::Null),
                    depth + 1,
                ))
                && (ar.is_none() && br.is_none()
                    || def_eq(
                        ar.unwrap_or(&Value::Null),
                        br.unwrap_or(&Value::Null),
                        depth + 1,
                    ))
        }
        (Value::Array(aa), Value::Array(ba)) => {
            aa.len() == ba.len()
                && aa
                    .iter()
                    .zip(ba.iter())
                    .all(|(av, bv)| def_eq(av, bv, depth + 1))
        }
        _ => a == b,
    }
}

/// Convert a [`LangError`] to a [`PlatformError`] (kept for compatibility with
/// any external callers of the legacy dispatch path).
#[allow(dead_code)]
fn lang_to_platform(e: LangError) -> PlatformError {
    PlatformError::Plugin(e.to_string())
}
