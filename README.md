# host-cli

The `hnl` CLI host (ADR-0149: *dumb host*).

- `crates/host-cli` — the `hnl` binary. It links exactly one sibling
  crate, `handl-ext` (the extension ABI from `handl-lang/kernel`), and
  knows no tools: it discovers `libhnl_ext_*.{so,dylib,dll}` libraries at
  runtime, reads their manifests, and dispatches argv to the command each
  extension declares.

The tools (`fmt`, `lint`, `typecheck`, `run`, `eval`, `test`, `compile`,
`sdkgen`, …) are **not** built into `hnl` — each is an `hnl-ext-*` cdylib
in its own `ext-*` repo, discovered via `--ext-dir`, `$HANDL_EXT_DIR`,
`~/.handl/ext/`, or a dev `target/debug/` next to the binary. The former
`plugin-ext-cli` capability adapter was pruned by ADR-0149: dispatch now
goes straight to the extension, not through a `cli` capability.

```bash
cargo build -p handl-host-cli          # target/debug/hnl
hnl --help                             # meta commands + loaded extensions
hnl ext list                           # discovered extensions
hnl ext doctor                         # diagnose discovery dirs
HANDL_EXT_DIR=../ext-fmt/target/debug hnl fmt x.hnl
```

The project docs live on the `project` branch, mounted at `project/`
(ADR-0129).

```bash
git worktree list        # main + project
git -C project log       # docs history
```
