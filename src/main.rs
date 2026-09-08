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
//! hnl typecheck [paths...]                     — typecheck files (`hnl check` is an alias)
//! hnl test  [--list] [--filter <pat>] [--tag <expr>] [--jobs N|auto]
//!           [--report human|json] [--changed] [--cache] [paths...] — run tests
//!           (--cache stores results in `.handl/cache/test-runner/cache.json`)
//! hnl run   <file.handl> [-- <args...>]        — run a file
//! hnl eval  '<expression>'                     — evaluate an expression
//! hnl typecheck '<expression>'                 — typecheck a single expression
//! hnl transform <pass> <file.hnl> [--pipeline a,b,c] [--plan] [--trace]
//!           [--stats] [--emit tree|json]       — run a transform pass (ADR-0094)
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
use handl_plugin_applier_bootstrap::BootstrapPlugin;
use handl_plugin_ext_cli::BasicCli;
use handl_plugin_ext_diag::{Severity, render_diagnostic, render_diagnostics};
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
        return Err("usage: hnl <command> [args...]\ncommands: fmt, lint, typecheck (alias: check), test, run, eval, transform, gen-grammar".into());
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

    if had_failure { 1 } else { 0 }
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

    if had_failure { 1 } else { 0 }
}

/// Lint a single source. Returns 2 on parse error; sets `had_failure` for
/// Error diagnostics (or Warning when `fail_on_warn`).
fn lint_one(src: &str, path: &str, fix: bool, fail_on_warn: bool, had_failure: &mut bool) -> i32 {
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

    let mut had_failure = false;

    if paths.is_empty() {
        let mut src = String::new();
        if std::io::stdin().read_to_string(&mut src).is_err() {
            eprintln!("typecheck: failed to read stdin");
            return 2;
        }
        return typecheck_one(&src, "<stdin>", &mut had_failure);
    }

    for path in &paths {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("typecheck: failed to read {path}: {e}");
                return 2;
            }
        };
        let code = typecheck_one(&src, path, &mut had_failure);
        if code == 2 {
            return 2;
        }
    }

    if had_failure { 1 } else { 0 }
}

/// Typecheck a single source. Returns 2 on parse error; sets `had_failure`
/// for Error diagnostics.
fn typecheck_one(src: &str, path: &str, had_failure: &mut bool) -> i32 {
    let report = match handl_plugin_engine_typecheck::check_module(src) {
        Ok(r) => r,
        Err(err) => {
            // `CheckError`'s Display already carries its category prefix
            // ("parse error: …" / "language error: …" — the latter covers
            // transform-rule validation failures at Language build, ADR-0094).
            eprintln!("{path}: {err}");
            return 2;
        }
    };

    let rendered = render_diagnostics(&report.diagnostics, path);
    if !rendered.is_empty() {
        println!("{rendered}");
    }

    if report
        .diagnostics
        .iter()
        .any(|d| d.severity == Severity::Error)
    {
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

/// Remove top-level `from ... import ...` and `import ...` declaration lines.
///
/// A `.hnl` file is a module with import declarations at the top; the body to
/// evaluate is what remains (cf. `dedent.hnl`).
fn strip_import_declarations(src: &str) -> String {
    src.lines()
        .filter(|line| !is_import_line(line.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_import_line(trimmed: &str) -> bool {
    if let Some(rest) = trimmed.strip_prefix("import") {
        if rest.is_empty()
            || rest.starts_with(' ')
            || rest.starts_with('\t')
            || rest.starts_with('"')
            || rest.starts_with('\'')
        {
            return true;
        }
    }
    if let Some(rest) = trimmed.strip_prefix("from") {
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            return true;
        }
    }
    false
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

    match kit.eval(&strip_import_declarations(&source)) {
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

    // The Pratt parser handles one top-level statement per call; wrap the
    // user expression in a `do` block so semicolon-separated commands such as
    // `from "transform" import * as transform; transform.map(...)` evaluate as a sequence.
    let wrapped = format!("do {{ {} }}", expr);
    match kit.eval(&wrapped) {
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
    /// `--emit tree|json` (default `tree`).
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
                        eprintln!("transform: --emit requires a value (tree|json)");
                        return 2;
                    }
                }
            }
            "--plan" => t.plan = true,
            "--trace" => t.trace = true,
            "--stats" => t.stats = true,
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

    if t.emit != "tree" && t.emit != "json" {
        eprintln!(
            "transform: unknown emit format `{}` (expected tree|json)",
            t.emit
        );
        return 2;
    }

    let file = match &t.file {
        Some(f) => f.clone(),
        None => {
            eprintln!(
                "usage: hnl transform <pass> <file.hnl> [--pipeline a,b,c] [--plan] [--trace] [--stats] [--emit tree|json]"
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

    let engine = handl_plugin_engine_transform_rewrite::RewriteEngine::new().with_desugar_natives();

    if t.plan {
        // Match-only: report match sites + rule ids without rewriting.
        let mut count = 0usize;
        for (n, ast) in stmts.iter().enumerate() {
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
    let mut had_error = false;

    for (n, ast) in stmts.iter().enumerate() {
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

        match t.emit.as_str() {
            "json" => {
                json_out.push(json!({
                    "statement": n + 1,
                    "root": out.root.index(),
                    "nodes": out.ast.nodes(),
                }));
            }
            _ => {
                if stmts.len() > 1 {
                    println!("-- statement {}", n + 1);
                }
                let mut buf = String::new();
                render_ast_tree(&out.ast, out.root.index(), 0, &mut buf);
                print!("{buf}");
            }
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
    }

    if had_error { 1 } else { 0 }
}

/// Raw-parse `src` into one [`handl_language::Ast`] per top-level statement,
/// WITHOUT the desugar pass (`Runtime::parse` always desugars — ADR-0082).
///
/// Statements are split at bracket-depth zero, respecting string literals
/// and `--` comments — the same convention as the bootstrap typechecker,
/// linter, and test runner.
fn transform_parse_raw(
    lang: &handl_language::Language,
    src: &str,
) -> Result<Vec<handl_language::Ast>, LangError> {
    use handl_plugin_engine_parse::ParserEngine;

    let mut asts = Vec::new();
    for chunk in split_statements(src) {
        let tokens = handl_plugin_engine_lex_pattern::lex(lang, &chunk)?;
        if tokens.as_slice().is_empty() {
            continue;
        }
        let req = handl_plugin_engine_parse::ParserRequest::new(tokens);
        let ast = handl_plugin_engine_parse_pratt::PrattEngine::new().parse(lang, &req)?;
        if !ast.is_empty() {
            asts.push(ast);
        }
    }
    Ok(asts)
}

/// Split `src` into top-level statement chunks at bracket-depth zero,
/// respecting string literals and `--` comments (mirrors the bootstrap
/// typechecker / linter / test runner convention — the Pratt parser parses
/// one statement per call).
fn split_statements(src: &str) -> Vec<String> {
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
            let trimmed = src[start..i].trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
            i += 1;
            start = i;
            continue;
        }

        i += 1;
    }

    let trimmed = src[start..].trim();
    if !trimmed.is_empty() {
        chunks.push(trimmed.to_string());
    }
    chunks
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
