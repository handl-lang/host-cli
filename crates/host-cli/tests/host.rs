//! Integration tests for the `hnl` host surface (ADR-0149).
//!
//! The host knows no tools, so these tests cover host behavior only: meta
//! commands, discovery reporting, and the unknown-command path. Real
//! extension dispatch (`hnl fmt` reaching a `libhnl_ext_fmt.so`) is
//! exercised end-to-end against `ext-fmt`; a fixture cdylib cannot be built
//! from inside `cargo test`.

use std::env;
use std::process::Command;

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

/// Run `hnl` in a controlled environment: no `HANDL_EXT_DIR`, so discovery
/// falls through to `~/.handl/ext` + the dev scan.
fn run_hnl_no_ext_env(args: &[&str]) -> (i32, String, String) {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .args(args)
        .env_remove("HANDL_EXT_DIR")
        .output()
        .expect("run hnl");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn version_prints_and_exits_zero() {
    let (code, stdout, _stderr) = run_hnl(&["version"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("hnl "), "got: {stdout}");
}

#[test]
fn help_lists_host_commands_and_discovery_rules() {
    let (code, stdout, _stderr) = run_hnl(&["--help"]);
    assert_eq!(code, 0);
    for needle in [
        "usage: hnl",
        "ext list",
        "ext doctor",
        "version",
        "$HANDL_EXT_DIR",
        "~/.handl/ext/",
    ] {
        assert!(stdout.contains(needle), "help missing `{needle}`:\n{stdout}");
    }
}

#[test]
fn ext_list_with_empty_dir_reports_none() {
    let dir = env::temp_dir().join(format!("hnl-ext-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (code, stdout, _stderr) = run_hnl(&["--ext-dir", dir.to_str().unwrap(), "ext", "list"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("no extensions loaded"), "got: {stdout}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ext_doctor_reports_candidate_dirs() {
    let (code, stdout, _stderr) = run_hnl_no_ext_env(&["ext", "doctor"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("discovery dirs:"), "got: {stdout}");
    assert!(stdout.contains("loaded:"), "got: {stdout}");
}

#[test]
fn unknown_command_is_a_clean_error() {
    let (code, _stdout, stderr) = run_hnl(&["nonexistent-command"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("unknown command `nonexistent-command`"),
        "got: {stderr}"
    );
}

#[test]
fn no_args_is_a_usage_error() {
    let (code, _stdout, stderr) = run_hnl(&[]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage: hnl"), "got: {stderr}");
}
