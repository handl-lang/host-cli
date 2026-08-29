# `handl-host-cli`

The `hnl` binary is a thin, metadata-driven capability orchestrator. It does
not hard-code any concrete command. Instead, it loads the default
`handl_platform::DefaultHost`, reads `CapabilityMetadata` from the plugin
registry, and dynamically builds `clap` subcommands from the `"command"`,
`"description"`, and `"aliases"` fields of that metadata.

## Build

From the workspace root:

```sh
cargo build -p handl-host-cli
```

The binary is available as `target/debug/hnl`.

## How it works

1. `main.rs` creates a `DefaultHost` and registers all built-in capability
   plugins (`fmt`, `lint`, `typecheck`, `eval`, `compile`, `build`,
   `extension`, `kit`, `plugin`).
2. `handl_host_cli::run` builds a root `clap::Command` from the host's
   `plugins().capabilities()`.
3. Each capability becomes a subcommand. Trailing positional arguments are
   collected into a JSON payload `{"args": [...]}` and dispatched through the
   host's `run` method.
4. The plugin's `CapabilityResponse` is pretty-printed to `stdout`.

Because command semantics live entirely in the capability plugins, adding a new
capability only requires registering another plugin with appropriate metadata;
the CLI binary does not need to change.

## Usage

```sh
hnl <COMMAND> [ARGS]...
```

Examples:

```sh
hnl fmt
hnl eval 42
hnl plugin add remote
hnl kit add core fmt lint
```

## Running `.hnl` files

The `hnl` binary can parse and evaluate `.hnl` source files using the canonical VM backend:

```bash
cargo run -p handl-host-cli -- run examples/handl/lib-utils/hello.hnl
```

This reads the file, lexes and parses it with `Kit::parse_module`, converts each `KitNode` to a VM `Expr`, and evaluates it with `CanonicalVm` backed by `CliHost` (which provides `str.*`, `math.*`, `fs.*`, `env.*`, and other stdlib functions). The value of the last expression is printed to stdout.

Example:

```
$ cat examples/handl/lib-utils/hello.hnl
let greeting = str.concat("hello", " ", "world");
greeting

$ cargo run -p handl-host-cli -- run examples/handl/lib-utils/hello.hnl
hello world
```

## Tests

```sh
cargo test -p handl-host-cli
```
