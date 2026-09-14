//! End-to-end smoke tests for `hnl compile` (ADR-0114 Phase 14).

use std::env;
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
    let output = Command::new(&bin).args(args).output().expect("run hnl");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn compile_js_deno_default_emits_export() {
    let path = fixture("run_expr.handl");
    let (code, stdout, stderr) =
        run_hnl(&["compile", "js", path.to_str().unwrap(), "--format", "esm"]);
    assert_eq!(
        code, 0,
        "expected exit 0, got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("export"), "expected `export` in: {stdout}");
}

#[test]
fn compile_llvm_target_exits_successfully() {
    let path = fixture("run_expr.handl");
    let (code, _stdout, stderr) = run_hnl(&["compile", "llvm", path.to_str().unwrap()]);
    assert_eq!(code, 0, "expected exit 0, got {code}\nstderr: {stderr}");
}

#[test]
fn compile_usage_error_with_missing_args() {
    let (code, _stdout, stderr) = run_hnl(&["compile"]);
    assert_eq!(code, 2, "expected exit 2, got {code}");
    assert!(!stderr.is_empty(), "expected usage error on stderr");
}
