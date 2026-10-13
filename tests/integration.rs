//! CLI integration tests.
//!
//! `cli_loads_native_plugin_and_runs_echo` was **removed** here. It drove the
//! `--plugin` flag, which no longer exists (the canonical CLI is the ADR-0090
//! `hnl` subcommand surface), *and* it returned early whenever it could not find
//! the native-echo fixture cdylib — so it verified nothing either way. The
//! capability it aimed at (a host loading a native plugin and dispatching one of
//! its commands) is covered at the right layer by
//! `host-tooling/tests/loader.rs`.

use std::env;
use std::process::Command;

/// `hnl --kit hello eval 'hello.greet("world")'` runs end to end.
///
/// This was parked with *"Pratt engine does not yet support function call syntax
/// with arguments"*; that is long obsolete — call syntax with arguments works —
/// and the test passes unmodified.
#[test]
fn cli_lib_hello_eval() {
    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .arg("--kit")
        .arg("hello")
        .arg("eval")
        .arg("hello.greet(\"world\")")
        .output()
        .expect("run hnl");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "hnl exited with {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("Hello, world!"),
        "expected 'Hello, world!' in output, got: {stdout}"
    );
}
