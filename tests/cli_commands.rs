//! Integration tests for the `hnl` CLI subcommand surface (ADR-0090).
//!
//! These tests run the real `hnl` binary (via `CARGO_BIN_EXE_hnl`) and verify
//! argument parsing, dispatch, exit-code mapping, and output shaping for every
//! subcommand. Fixtures live in `tests/fixtures/`.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Path to the fixtures directory.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Path to a named fixture.
fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

/// Run `hnl` with the given args, returning (exit code, stdout, stderr).
fn run_hnl(args: &[&str]) -> (i32, String, String) {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .args(args)
        .output()
        .expect("run hnl");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// Copy a fixture into a temp directory and return the new path. Used by tests
/// that modify the file in place (`fmt --write`, `lint --fix`).
fn copy_fixture_to_temp(name: &str) -> PathBuf {
    let src = fixture(name);
    let tmp = env::temp_dir().join(format!("hnl-cli-test-{}-{name}", std::process::id()));
    fs::write(&tmp, fs::read_to_string(&src).expect("read fixture")).expect("write temp");
    tmp
}

// ---------------------------------------------------------------------------
// fmt
// ---------------------------------------------------------------------------

#[test]
fn fmt_check_clean() {
    let path = fixture("clean.handl");
    let (code, stdout, stderr) = run_hnl(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
}

#[test]
fn fmt_check_dirty() {
    let path = fixture("dirty.handl");
    let (code, stdout, stderr) = run_hnl(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(code, 1, "expected exit 1, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("not canonical"), "expected 'not canonical', got: {stdout}");
}

#[test]
fn fmt_write() {
    let path = copy_fixture_to_temp("dirty.handl");
    let (code, stdout, stderr) = run_hnl(&["fmt", "--write", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    let after = fs::read_to_string(&path).expect("read after write");
    assert!(
        after.contains("x * 2"),
        "expected canonical output with spaces, got: {after}"
    );
    // Idempotent: running --check on the fixed file should now pass.
    let (code2, _, _) = run_hnl(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(code2, 0, "expected --check to pass after --write");
}

#[test]
fn fmt_stdout() {
    let path = fixture("dirty.handl");
    let (code, stdout, stderr) = run_hnl(&["fmt", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("x * 2"),
        "expected formatted output with spaces, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// lint
// ---------------------------------------------------------------------------

#[test]
fn lint_clean() {
    let path = fixture("clean.handl");
    let (code, stdout, stderr) = run_hnl(&["lint", path.to_str().unwrap()]);
    // No Error diagnostics → exit 0 (warnings/info are non-fatal).
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
}

#[test]
fn lint_with_errors() {
    let path = fixture("unbound.handl");
    let (code, stdout, stderr) = run_hnl(&["lint", path.to_str().unwrap()]);
    assert_eq!(code, 1, "expected exit 1, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("error") && stdout.contains("unbound-wire"),
        "expected error diagnostic, got: {stdout}"
    );
}

#[test]
fn lint_fix() {
    let path = copy_fixture_to_temp("dirty.handl");
    let (code, stdout, stderr) = run_hnl(&["lint", "--fix", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    let after = fs::read_to_string(&path).expect("read after fix");
    assert!(
        after.contains("x * 2"),
        "expected fixed canonical output, got: {after}"
    );
}

#[test]
fn lint_fail_on_warn() {
    let path = fixture("dirty.handl");
    let (code, stdout, stderr) =
        run_hnl(&["lint", "--fail-on-warn", path.to_str().unwrap()]);
    // dirty.handl has a `fmt` warning → --fail-on-warn makes it exit 1.
    assert_eq!(code, 1, "expected exit 1, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("warning"),
        "expected a warning diagnostic, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_clean() {
    let path = fixture("clean.handl");
    let (code, stdout, stderr) = run_hnl(&["check", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
}

#[test]
fn check_with_errors() {
    let path = fixture("unbound.handl");
    let (code, stdout, stderr) = run_hnl(&["check", path.to_str().unwrap()]);
    assert_eq!(code, 1, "expected exit 1, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("error") && stdout.contains("unbound"),
        "expected error diagnostic, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// test
// ---------------------------------------------------------------------------

#[test]
fn test_pass() {
    let path = fixture("test_pass.handl");
    let (code, stdout, stderr) = run_hnl(&["test", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("PASS"), "expected PASS, got: {stdout}");
}

#[test]
fn test_fail() {
    let path = fixture("test_fail.handl");
    let (code, stdout, stderr) = run_hnl(&["test", path.to_str().unwrap()]);
    assert_eq!(code, 1, "expected exit 1, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("FAIL"), "expected FAIL, got: {stdout}");
}

#[test]
fn test_list() {
    let path = fixture("test_pass.handl");
    let (code, stdout, stderr) = run_hnl(&["test", "--list", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("test_addition"),
        "expected test name in listing, got: {stdout}"
    );
    // --list should not print PASS/FAIL summary lines.
    assert!(!stdout.contains("PASS"), "listing should not run tests: {stdout}");
}

#[test]
fn test_json() {
    let path = fixture("test_pass.handl");
    let (code, stdout, stderr) = run_hnl(&["test", "--json", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    // Must be valid JSON (parseable).
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("valid JSON output");
    assert!(parsed.is_object(), "expected JSON object, got: {parsed}");
}

#[test]
fn test_filter() {
    let path = fixture("test_mixed.handl");
    let (code, stdout, stderr) =
        run_hnl(&["test", "--filter", "test_pass", path.to_str().unwrap()]);
    // test_pass and test_pass_two match; test_fail is skipped → no failures.
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("SKIP"), "expected SKIP for filtered-out tests: {stdout}");
}

// ---------------------------------------------------------------------------
// run / eval
// ---------------------------------------------------------------------------

#[test]
fn run_file() {
    let path = fixture("run_expr.handl");
    let (code, stdout, stderr) = run_hnl(&["run", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("2"), "expected result 2, got: {stdout}");
}

#[test]
fn eval_expr() {
    let (code, stdout, stderr) = run_hnl(&["eval", "1 + 2"]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("3"), "expected result 3, got: {stdout}");
}

// ---------------------------------------------------------------------------
// Usage / exit-code mapping
// ---------------------------------------------------------------------------

#[test]
fn usage_error_no_args() {
    let (code, _stdout, stderr) = run_hnl(&[]);
    assert_eq!(code, 2, "expected exit 2 for no args, got {code}");
    assert!(!stderr.is_empty(), "expected usage message on stderr");
}

#[test]
fn exit_code_fmt_check_failure() {
    let path = fixture("dirty.handl");
    let (code, _, _) = run_hnl(&["fmt", "--check", path.to_str().unwrap()]);
    assert_eq!(code, 1, "fmt --check failure should exit 1");
}

#[test]
fn exit_code_parse_error() {
    let path = fixture("parse_error.handl");
    let (code, _stdout, stderr) = run_hnl(&["check", path.to_str().unwrap()]);
    assert_eq!(code, 2, "parse error should exit 2, got {code}");
    assert!(!stderr.is_empty(), "expected parse error message on stderr");
}
