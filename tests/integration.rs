use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn hnl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hnl"))
}

#[test]
fn fmt_file_writes_formatted_source_to_stdout() {
    let path = PathBuf::from(std::env::temp_dir())
        .join(format!("handl_fmt_test_{}.handl", std::process::id()));
    fs::write(&path, "let a = 1;").unwrap();
    let output = hnl().arg("fmt").arg(&path).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "fmt should succeed: {}",
        stderr
    );
    assert!(
        stdout.contains("let a = 1"),
        "expected formatted source in stdout, got: {}",
        stdout
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn eval_expr_prints_json_values() {
    let output = hnl().args(["eval", "42"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "eval should succeed: {}",
        stderr
    );
    assert!(
        stdout.contains("42"),
        "expected evaluated value in stdout, got: {}",
        stdout
    );
}

#[test]
fn typecheck_file_reports_errors_with_exit_code_1() {
    let path = PathBuf::from(std::env::temp_dir())
        .join(format!("handl_typecheck_bad_{}.handl", std::process::id()));
    fs::write(&path, "let a = ;").unwrap();
    let output = hnl().arg("typecheck").arg(&path).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected typecheck to fail for invalid source"
    );
    assert!(
        stderr.to_lowercase().contains("error")
            || stderr.to_lowercase().contains("diagnostic"),
        "expected error message on stderr, got: {}",
        stderr
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn lsp_help_shows_stdio_info() {
    let output = hnl().args(["lsp", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "lsp --help should succeed: {}",
        stderr
    );
    assert!(
        stdout.to_lowercase().contains("start the stdio lsp server"),
        "expected LSP description in help, got stdout: {}",
        stdout
    );
}

#[test]
fn lsp_help_long_flag() {
    let output = hnl().args(["lsp", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "lsp --help should succeed: {}",
        stderr
    );
    assert!(
        stdout.to_lowercase().contains("lsp"),
        "expected LSP help text, got: {}",
        stdout
    );
}

#[test]
fn build_reports_success() {
    let output = hnl().arg("build").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "build should succeed: {}",
        stderr
    );
    assert!(
        stdout.contains("built") || stdout.contains("true"),
        "expected build success in stdout, got: {}",
        stdout
    );
}

#[test]
fn compile_reports_success() {
    let output = hnl().arg("compile").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "compile should succeed: {}",
        stderr
    );
    assert!(
        stdout.contains("compiled") || stdout.contains("true"),
        "expected compile success in stdout, got: {}",
        stdout
    );
}
