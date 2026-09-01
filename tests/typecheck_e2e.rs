use std::env;
use std::process::Command;

fn run_typecheck(expr: &str) -> (bool, String) {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .arg("typecheck")
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
fn typecheck_number_e2e() {
    let (ok, out) = run_typecheck("42");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_arithmetic_e2e() {
    let (ok, out) = run_typecheck("1 + 2");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_bool_e2e() {
    let (ok, out) = run_typecheck("true");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.bool"), "expected type.bool, got: {out}");
}

#[test]
fn typecheck_if_e2e() {
    let (ok, out) = run_typecheck("if true then 1 else 2");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_if_non_bool_cond_errors_e2e() {
    let (ok, out) = run_typecheck("if 42 then 1 else 2");
    assert!(!ok, "expected error, got: {out}");
}

#[test]
fn typecheck_let_e2e() {
    let (ok, out) = run_typecheck("let x = 1 in x");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_fn_e2e() {
    let (ok, out) = run_typecheck("(x) -> x");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.function"), "expected type.function, got: {out}");
}

#[test]
fn typecheck_match_e2e() {
    let (ok, out) = run_typecheck("match 1 { 1: 2, _: 3 }");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_type_annot_e2e() {
    let (ok, out) = run_typecheck("2: type.number");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_type_annot_mismatch_errors_e2e() {
    let (ok, out) = run_typecheck("2: type.string");
    assert!(!ok, "expected error, got: {out}");
}

#[test]
fn typecheck_let_with_annotation_e2e() {
    let (ok, out) = run_typecheck("let x: type.number = 2 in x");
    assert!(ok, "expected success, got: {out}");
    assert!(out.contains("type.number"), "expected type.number, got: {out}");
}

#[test]
fn typecheck_let_annotation_mismatch_errors_e2e() {
    let (ok, out) = run_typecheck("let x: type.string = 2 in x");
    assert!(!ok, "expected error, got: {out}");
}
