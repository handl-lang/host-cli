use std::env;
use std::process::Command;

fn run_eval(expr: &str) -> (bool, String) {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .arg("eval")
        .arg(expr)
        .output()
        .expect("run hnl");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stdout.is_empty() {
        stderr
    } else {
        stdout
    };
    (output.status.success(), combined)
}

#[test]
fn eval_if_true_branch_e2e() {
    let (ok, out) = run_eval("if true then 1 else 2");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("1"), "expected 1, got: {out}");
}

#[test]
fn eval_if_false_branch_e2e() {
    let (ok, out) = run_eval("if false then 1 else 2");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("2"), "expected 2, got: {out}");
}
