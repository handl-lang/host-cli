use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn target_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
}

fn find_example_cdylib() -> Option<PathBuf> {
    let dir = target_dir();
    if !dir.is_dir() {
        return None;
    }

    let candidates: &[&str] = &[
        "libnative_plugin_example.so",
        "libnative-plugin-example.so",
        "libnative_plugin_example.dylib",
        "libnative-plugin-example.dylib",
        "native_plugin_example.dll",
        "native-plugin-example.dll",
    ];

    for name in candidates {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }

    None
}

#[test]
fn cli_loads_native_plugin_and_runs_echo() {
    let Some(cdylib) = find_example_cdylib() else {
        eprintln!("native-plugin-example cdylib not found; skipping integration test");
        return;
    };

    let manifest = format!(
        r#"{{
  "id": "native-plugin-example",
  "version": "0.1.0",
  "provides": ["echo"],
  "requires": [],
  "optional": [],
  "extends": [],
  "conflicts": [],
  "replaces": [],
  "targets": [],
  "exclusive": [],
  "metadata": {{
    "path": {:?},
    "command": "echo",
    "description": "Echo plugin"
  }}
}}"#,
        cdylib.to_string_lossy()
    );

    let tmp = env::temp_dir().join("handl_native_plugin_test_manifest.json");
    fs::write(&tmp, &manifest).expect("write manifest");

    let bin = env::var("CARGO_BIN_EXE_hnl").expect("CARGO_BIN_EXE_hnl");
    let output = Command::new(&bin)
        .arg("--plugin")
        .arg(&tmp)
        .arg("echo")
        .arg("hello")
        .output()
        .expect("run hnl");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "hnl exited with {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(stdout.contains("hello"), "expected 'hello' in output, got: {stdout}");
}

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
