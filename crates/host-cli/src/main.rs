//! `hnl` — the handl CLI host binary (ADR-0149).
//!
//! This is a dumb host: all logic lives in [`handl_host_cli`], which
//! discovers `libhnl_ext_*` extensions and dispatches argv to them. No tool
//! knowledge is linked into this binary — `cargo build` produces a host
//! that can only report its loaded extension set until extensions are
//! installed into a discovery dir (`--ext-dir`, `$HANDL_EXT_DIR`,
//! `~/.handl/ext/`, or a dev `target/debug/` next to the binary).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(handl_host_cli::run(&args));
}
