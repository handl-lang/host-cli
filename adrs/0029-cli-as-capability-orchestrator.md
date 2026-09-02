# CLI as Capability Orchestrator

## Context

`handl` already splits the language implementation into small, composable crates. The [Pluggable Module Composition ADR](0028-pluggable-module-composition.md) introduced `module.toml` manifests and a `Composer` to assemble per-module pipelines. However, the CLI still resembles a monolithic compiler: `lang build`, `lang fmt`, `lang check` each embed assumptions about which crate or target implements the work.

To support multiple targets, tools, and language implementations without the CLI becoming a tightly coupled control plane, the CLI itself should become an orchestration shell.

## Decision

The `lang` CLI will be a **package manager + capability orchestrator**, not a compiler.

It will be responsible for:

- plugin and module discovery
- project and configuration management
- dispatching commands to the appropriate capability providers

The actual work will be performed by providers that implement stable protocols:

- `compiler-native`, `compiler-js`, `compiler-wasm`
- `formatter-*`
- `typechecker-*`
- `linter-*`
- `debugger`

This means commands like:

```text
lang build --target native
lang compile --target js
lang fmt
lang check
lang lint
```

are dispatched through a **capability registry** instead of being hard-wired to a single implementation.

### Capability registry

```text
                lang
                 │
          Capability Registry
                 │
      ┌──────────┼──────────┐
      ↓          ↓          ↓
   compile      fmt       check
      │          │          │
   ┌──┴──┐       │       ┌──┴──┐
 native  js    prettier   tc1  tc2
```

Capabilities are not restricted to compiler passes. Any tool that can satisfy a capability can register itself:

```text
language-toolchain-native
language-toolchain-js
language-toolchain-wasm

language-formatter
language-typechecker
language-linter
language-analyzer
language-debugger
language-repl
```

A project can declare which providers it prefers:

```toml
[tools]
formatter = "fmt-pretty"
typechecker = "types-fast"
compiler = ["native", "wasm"]
linter = "lint-standard"
```

Providers can be installed lazily:

```text
$ lang build --target native
→ native compiler not installed
→ install compiler-native@...
→ compile
```

Or installed explicitly:

```text
$ lang plugin add compiler-native
$ lang plugin add compiler-js
```

### Language core versus tooling protocols

`language-kit` defines the LEGO pieces and stable protocols. The CLI is one consumer of those protocols; language implementation libraries can be used without the CLI.

```text
                    language-kit
                         │
             ┌───────────┴───────────┐
             │                       │
        Language Core            Tooling Protocol
             │                       │
       ┌─────┼─────┐       ┌────────┼────────┐
       ↓     ↓     ↓       ↓        ↓        ↓
     lexer parser types   fmt      lint     LSP
       │           │
       └─────┬─────┘
             ↓
             IR
             │
       ┌─────┼──────┐
       ↓     ↓      ↓
     native  JS    Wasm
```

This lets the same language implementation be embedded in other applications, not just the CLI.

### Plugin protocol

To avoid Rust dynamic linking and ABI/versioning problems, the plugin interface will be a **process/protocol boundary**, such as JSON-RPC or MessagePack, over stdin/stdout or sockets.

```text
CLI
 │
 │ JSON-RPC / MessagePack / similar
 ↓
plugin process
 │
 └── implementation
```

This makes it possible to write providers in Rust, Go, TypeScript, Python, or any other language. Native Rust plugins can still be used internally for performance, but they will use the same protocol.

### Summary

```text
                ┌─────────────────┐
                │   language-kit  │
                │   LEGO pieces   │
                └────────┬────────┘
                         │
                 stable protocols
                         │
                ┌────────▼────────┐
                │      lang       │
                │ CLI / registry  │
                │ orchestration   │
                └────────┬────────┘
                         │
              ┌──────────┼──────────┐
              ↓          ↓          ↓
           compiler    tooling    runtime
           plugins     plugins    plugins
              │          │          │
         native/JS/   fmt/lint/   VM/GC/
         Wasm/etc.    check/LSP   debugger
```

## Consequences

- The CLI becomes a thin, stable control plane.
- `lang` can be used as a package manager and capability orchestrator for the whole `handl` ecosystem.
- `language-kit` can be consumed independently of `lang`, making the language implementation available as libraries.
- Plugin providers can be written in any language as long as they speak the stable process protocol.
- A capability registry and plugin manifest format must be defined.
- Plugin installation, lazy loading, and versioning must be implemented in `lang`.
- Targets, formatters, linters, and debuggers can be released and installed independently.
- Replacing one implementation of a capability does not require changes to the CLI or other plugins.

## Key constraint

No layer should require the layer above it:

- `language-kit` works without `lang`.
- A compiler plugin works without the CLI.
- The CLI works with third-party implementations.
- Tooling can consume language protocols without knowing the compiler.
- Targets can be installed independently.
- Implementations can be replaced independently.

## Status

Proposed
