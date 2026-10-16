# cli

The `hnl` CLI distribution (ADR-0128 Phase 8):

- `crates/host-cli` — the `hnl` binary; its git-dep list *is* the
  binary's target set.
- `crates/plugin-ext-cli` — the `cli` capability adapter (its only
  consumer is `host-cli`).

Platform crates are git dependencies on `handl-lang/platform`; the
canonical kit comes from `handl-lang/kit-canonical` and the language
data from `handl-lang/lang`. The workspace-root `[patch]` tables resolve
them from sibling `../platform`, `../lang`, `../kit-canonical` and
`../stdlib` checkouts for local development; see the comment in
`Cargo.toml` for making the git sources resolve locally before the
remotes are published.

Part of the `handl-lang` org split (ADR-0128). The project docs live on
the `project` branch, mounted at `project/` (ADR-0129).

```bash
git worktree list        # main + project
git -C project log       # docs history
```
