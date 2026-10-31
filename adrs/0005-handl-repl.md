+++
id = "ADR-0005"
title = "Handl REPL"
status = "open"
priority = "medium"
+++

---
title: Handl REPL
type: adr
date: 2026-10-16
status: proposed
status_note: Proposed
---


# Handl REPL

## Decision

1. **Architecture.** handl repl ├── Prompt: handl> ├── Input: single-line or multi-line expressions ├── Evaluate: lex → parse → typecheck → eval ├── Output: result value + inferred refinement type └── State: persistent session (bindings carry over between inputs)
2. **Commands.**
3. **Session state.** The REPL maintains a persistent environment across inputs:
4. **Multi-line input.** If a line ends with an incomplete expression (open bracket, unclosed `do` block, trailing operator), the REPL continues to the next line with a `... ` prompt until the expression is complete. This mirrors how GHCi and IEx handle multi-line input.
5. **Output format.** handl> arr("double") >>> fold(0, +) ⇒ <arrow: In ~> Out> ⇝ In: List(Int) ~> Out: Int ⇝ Guarantee: AllSatisfy(λx. x ≥ 0) → Sum ≥ 0
6. **Integration with existing infrastructure.** The REPL uses the same kit loading as the CLI host and LSP server:
7. **History and readline.** Use the `rustyline` crate for line editing, history, and tab completion. Tab completion completes:
8. **Error handling.** Parse errors, type errors, and refinement errors are printed in-place without killing the session:

## Why

Handl development currently requires a full file write + server round-trip to evaluate expressions. There is no way to interactively experiment with arrows, inspect refinement types, or test combinators without creating a `.hnl` file and opening it in an editor.

## Not doing

- The REPL uses the same kit loading, parsing, and evaluation as the CLI host. Duplicating this logic in a new binary creates maintenance burden.
- `handl-host-cli` already has the infrastructure for loading kits and running commands. The REPL is just a new interactive mode.
- One binary is simpler for users: `handl repl` vs remembering a separate `handl-repl` command.
- Requires a JS/WASM backend to run handl in the browser, which doesn't exist yet (see ADR 0006).
- A terminal REPL is faster to build, works offline, and integrates with existing terminal workflows.
- A web playground can be built later as a separate effort.

## Consequences

- **Interactive development** — developers can experiment without creating files
- **Refinement inspection** — `:t` and `:trace` make refinement inference tangible
- **Arrow testing** — load arrows and compose them interactively
- **Kit switching** — `:kit` command lets developers compare syntax variants
- **Uses existing infrastructure** — no new engines, just a new CLI mode

## Enforcement

- Code and tests are named in the design doc.

## Design

The full record (mechanism, tables, proofs, trade-offs): [`design/0005-handl-repl.md`](../design/0005-handl-repl.md).
