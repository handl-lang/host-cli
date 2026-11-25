# host-cli

The `hnl` CLI host (ADR-0149: *dumb host*).

## Layout

| path | role |
| --- | --- |
| `crates/host-cli` | the `hnl` binary — links only `handl-ext` (the extension ABI from `handl-lang/kernel`) |
| `scripts/` | dev/release helper scripts |
| `xtask/` | vendored gate crate (`fmt`, `clippy`, `test`, `doc`, `gate`) |
| `workflows/gate.yaml` | per-push gate — `handl-gates` rust campaign via bupha |

## Contract

- `hnl` knows no tools: it discovers `libhnl_ext_*.{so,dylib,dll}`
  libraries at runtime, reads their manifests, and dispatches argv to
  the command each extension declares — via `--ext-dir`,
  `$HANDL_EXT_DIR`, `~/.handl/ext/`, or a dev `target/debug/` next to
  the binary.
- The tools (`fmt`, `lint`, `typecheck`, `run`, `eval`, `test`,
  `compile`, `codegen`, `codemod`, …) are **not** built in — each is an
  `hnl-ext-*` cdylib in its own `ext-*` repo. The former
  `plugin-ext-cli` capability adapter was pruned by ADR-0149.
- Release tarballs ship the host plus `eval`/`fmt`/`typecheck`
  extensions (linux-x86_64) with an `install.sh`.

## Verification

```bash
cargo xtask gate        # fmt → clippy → test → doc
cargo build -p handl-host-cli   # or: cargo build --bin hnl
hnl eval '1 + 2'        # smoke test once built
```

<!-- myco:agent-awareness -->

## Agent awareness

- Session start / any work discussion: run `myco status --prompt`
  before proposing next steps — know the board first.
- User describes new work → offer to capture it (`myco todo add`,
  `issue new`, `proposal new`). Don't wait to be asked.
- Picking up a task: `myco resume <ID>` is the briefing.
- Wrapping up: `myco handoff <ID>` anything resumable; keep statuses
  honest (claimed → in-progress → blocked/done).

<!-- /myco:agent-awareness -->
