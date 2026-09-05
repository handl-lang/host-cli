# RFC-0007: CLI in handl — Tool Integration via the handl Plugin System

## Status

Low priority. Realizes the end-state intent of
[ADR-0029 — CLI as Capability Orchestrator](../adrs/0029-cli-as-capability-orchestrator.md)
in handl, replacing the bootstrap CLI of
[ADR-0090](../adrs/0090-bootstrap-cli.md). Depends on
[RFC-0001](0001-plugin-system-in-handl.md) (the plugin system it loads) and
the tool RFCs [0002](0002-formatter-in-handl.md)–[0006](0006-lsp-server-in-handl.md),
which each ship a CLI integration.

## Summary

The final CLI is written in handl and has **no built-in tools**. It is a
thin, generic, documented orchestrator: it discovers and loads _tool
integrations_ contributed by plugins, and dispatches subcommands to them.
The formatter, linter, typechecker, test runner, and LSP integrations are
written by their respective RFCs as separate plugin modules — the CLI is
decoupled from all of them.

## Design goals (the contract every integration relies on)

- **Generic** — serves any handl language (ADR-0041 languages as values),
  not just the canonical arrow surface; integrations declare which
  languages/kit they serve.
- **Composable** — a command pipeline is arrow composition
  (`parse >>> lint >>> fix >>> format >>> write`); integrations contribute
  arrows, not opaque commands.
- **Extensible** — new subcommands, new flags on existing subcommands, and
  new output formats are plugin contributions; no CLI edit required.
- **Replaceable** — capabilities (formatter, linter, …) are resolved
  through the ADR-0029 capability registry, so `formatter = "fmt-pretty"`
  in project config selects among loaded integrations; one implementation
  can be swapped without touching others.
- **Modular** — CLI core knows only: config discovery, plugin loading
  (RFC-0001), capability registry, dispatch, exit codes, and the shared
  diagnostics formatter. Everything else is a plugin.
- **Documented** — the integration contract (the `Integration` record and
  capability ids) is a stable, versioned protocol with its own spec
  document and conformance tests; third parties can ship integrations
  without reading CLI source.

## Detailed design

- **Integration record:** each tool RFC ships a module exporting

  ```text
  integration = {
    id        = "tool.fmt",         -- namespaced capability id
    languages = [ "handl.canonical" ],
    commands  = [ { name = "fmt", ... } ],
    hooks     = [ ... ]             -- pre/post-run arrows
  }
  ```

- **Dispatch:** the CLI core maps the argv → capability request; the
  registry (RFC-0001 §Kit/Host) resolves the request to an integration's
  arrow and runs it with a context value (paths, flags, config, stdout
  sink). All I/O effects flow through the host handler, keeping the CLI
  sandboxable.
- **Package manager role** (from ADR-0029) is retained: `hnl plugin add`,
  lazy provider install, and pinned tool versions in project config.
- **Exit codes and diagnostics format** are unchanged from ADR-0090 so CI
  scripts migrate transparently.

## Migration

1. Define the integration protocol spec + conformance suite (pure handl
   tests).
2. Move `host-cli`'s subcommand logic into per-tool integration modules,
   consumed by the new CLI; run both binaries until parity.
3. Retire `crates/host-cli` as the primary; refactor `host-cli` to be a
   consumer of the handl capability contracts (per ADR-0085, the Rust
   substrate remains as bootstrap/fallback).

## Drawbacks

- Bootstrap CLI (ADR-0090) must exist first; this RFC is intentionally
  blocked on it.
- Process-protocol plugins (ADR-0029 §Plugin protocol) for non-handl
  integrations (prettier, third-party checkers) need the WASM/FFI loader
  story from RFC-0001 to be settled.

## Related

- ADR-0029, ADR-0085, ADR-0090, RFC-0001, RFC-0002–0006.
