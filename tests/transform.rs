//! Integration tests for `hnl transform` (ADR-0094 deferred item).
//!
//! Binary-level tests run the real `hnl` binary (via `CARGO_BIN_EXE_hnl`)
//! following `cli_commands.rs` conventions. This file also contains the
//! arena-parity check: raw-parse + `RewriteEngine::run_pass("desugar")`
//! must produce an [`Ast`] identical to what `Runtime::parse` returns
//! internally (parse-time desugar, ADR-0082).

use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Path to a named fixture.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Run `hnl` with the given args, returning (exit code, stdout, stderr).
fn run_hnl(args: &[&str]) -> (i32, String, String) {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin).args(args).output().expect("run hnl");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

// ---------------------------------------------------------------------------
// transform
// ---------------------------------------------------------------------------

#[test]
fn transform_desugar_shows_rewrites() {
    let path = fixture("transform_desugar.hnl");
    let (code, stdout, stderr) =
        run_hnl(&["transform", "desugar", path.to_str().unwrap(), "--trace"]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    // `a ?? b` desugars to a `match`; `while` desugars to a `rec` loop.
    assert!(stdout.contains("match"), "expected match node: {stdout}");
    assert!(stdout.contains("rec"), "expected rec node: {stdout}");
    assert!(
        stdout.contains("native:desugar.nullish"),
        "expected nullish rewrite in trace: {stdout}"
    );
    assert!(
        stdout.contains("native:desugar.while"),
        "expected while rewrite in trace: {stdout}"
    );
}

#[test]
fn transform_plan_lists_matches_without_applying() {
    let path = fixture("transform_desugar.hnl");
    let (code, stdout, stderr) =
        run_hnl(&["transform", "desugar", path.to_str().unwrap(), "--plan"]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("match nullish"),
        "expected nullish match site: {stdout}"
    );
    assert!(
        stdout.contains("match while"),
        "expected while match site: {stdout}"
    );
    assert!(
        stdout.contains("plan: 2 match site(s)"),
        "expected match count: {stdout}"
    );
    assert!(
        stdout.contains('@'),
        "plan should include node byte span: {stdout}"
    );
    // Plan mode never emits the transformed tree.
    assert!(
        !stdout.contains("  arm"),
        "plan must not print rewritten output: {stdout}"
    );
}

#[test]
fn transform_stats_prints_counts() {
    let path = fixture("transform_desugar.hnl");
    let (code, stdout, stderr) =
        run_hnl(&["transform", "desugar", path.to_str().unwrap(), "--stats"]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("visited:"), "expected visited: {stdout}");
    assert!(
        stdout.contains("rewritten: 1"),
        "expected rewritten: {stdout}"
    );
    assert!(
        stdout.contains("rule native:desugar.nullish: 1"),
        "expected per-rule count: {stdout}"
    );
}

#[test]
fn transform_emit_json() {
    let path = fixture("transform_desugar.hnl");
    let (code, stdout, stderr) = run_hnl(&[
        "transform",
        "desugar",
        path.to_str().unwrap(),
        "--emit",
        "json",
    ]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let arr = v.as_array().expect("top-level array of statements");
    assert_eq!(arr.len(), 2, "expected two statements: {stdout}");
    assert!(arr[0]["nodes"].is_array());
    assert!(arr[0]["root"].is_u64());
}

#[test]
fn transform_unknown_pass_is_usage_error() {
    let path = fixture("transform_desugar.hnl");
    let (code, _stdout, stderr) = run_hnl(&["transform", "nosuchpass", path.to_str().unwrap()]);
    assert_eq!(code, 2, "expected exit 2, got {code}\nstderr: {stderr}");
    assert!(
        stderr.contains("unknown pass"),
        "expected 'unknown pass': {stderr}"
    );
}

#[test]
fn transform_missing_file() {
    let (code, _stdout, stderr) = run_hnl(&["transform", "desugar", "no-such-file.hnl"]);
    assert_eq!(code, 2, "expected exit 2, got {code}\nstderr: {stderr}");
    assert!(
        stderr.contains("failed to read"),
        "expected 'failed to read': {stderr}"
    );
}

#[test]
fn transform_missing_args() {
    let (code, _stdout, stderr) = run_hnl(&["transform"]);
    assert_eq!(code, 2, "expected exit 2, got {code}\nstderr: {stderr}");
    assert!(stderr.contains("usage"), "expected usage: {stderr}");
}

#[test]
fn transform_parse_error() {
    let path = fixture("parse_error.handl");
    let (code, _stdout, stderr) = run_hnl(&["transform", "desugar", path.to_str().unwrap()]);
    assert_eq!(code, 2, "expected exit 2, got {code}\nstderr: {stderr}");
    assert!(
        stderr.contains("parse error"),
        "expected parse error: {stderr}"
    );
}

#[test]
fn transform_pipeline() {
    let path = fixture("transform_desugar.hnl");
    let (code, stdout, stderr) = run_hnl(&[
        "transform",
        "--pipeline",
        "desugar,desugar",
        path.to_str().unwrap(),
        "--stats",
    ]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("visited:"), "expected stats: {stdout}");
}

// ---------------------------------------------------------------------------
// Arena parity: raw-parse + desugar == Runtime::parse
// ---------------------------------------------------------------------------

/// `Runtime::parse` applies the `"desugar"` pass at parse time (ADR-0082),
/// so `hnl transform desugar` must reproduce exactly the arena the runtime
/// sees: raw-parse via `lex` + `PrattEngine` (no desugar), then
/// `RewriteEngine::run_pass` — the same delegate `Runtime::parse` uses via
/// `handl_runtime::apply_desugar`.
#[test]
fn desugar_transform_matches_runtime_parse() {
    use handl_plugin_engine_parse::ParserEngine;

    let kit = handl_kit_canonical::Kit::new().expect("kit");
    let lang = kit.runtime().lang();

    for src in [
        "x = a ?? b",
        "while x > 0 { x = x - 1 }",
        "y = a?.b",
        "f = x ~> x * 2",
    ] {
        // What the runtime produces: lex → Pratt parse → desugar.
        let runtime_ast = kit.parse(src).expect("runtime parse");

        // What `hnl transform desugar` produces: raw Pratt parse (no
        // desugar) + the pass.
        let tokens = handl_plugin_engine_lex_pattern::lex(lang, src).expect("lex");
        let req = handl_plugin_engine_parse::ParserRequest::new(tokens);
        let raw = handl_plugin_engine_parse_pratt::PrattEngine::new()
            .parse(lang, &req)
            .expect("raw parse");
        let root = handl_language::AstRef::new(raw.len() - 1);
        let out = handl_plugin_engine_transform_rewrite::RewriteEngine::new()
            .with_desugar_natives()
            .run_pass(lang, &raw, root, "desugar")
            .expect("desugar pass");

        assert_eq!(
            out.ast, runtime_ast,
            "raw-parse+desugar must equal Runtime::parse output for: {src}"
        );
    }
}
