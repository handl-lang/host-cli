---
title: Handl REPL
type: adr
date: 2026-10-16
status: proposed
status_note: Proposed
---


# Handl REPL

## Decision


Add an interactive REPL mode to `handl-host-cli`, activated by `handl repl` (or running `handl` with no arguments).

### Architecture

```
handl repl
├── Prompt: handl>
├── Input: single-line or multi-line expressions
├── Evaluate: lex → parse → typecheck → eval
├── Output: result value + inferred refinement type
└── State: persistent session (bindings carry over between inputs)
```

### Commands

| Command         | Description                                                     |
| --------------- | --------------------------------------------------------------- |
| `:t <expr>`     | Show the inferred refinement type of an expression              |
| `:r <arrow>`    | Show the inferred contract (input/output/guarantee) of an arrow |
| `:l <file>`     | Load a `.hnl` file into the session                             |
| `:l <dir>`      | Load all arrows from a directory                                |
| `:q`            | Quit                                                            |
| `:help`         | Show available commands                                         |
| `:kit`          | Show the active kit and version                                 |
| `:kit <name>`   | Switch to a different kit                                       |
| `:env`          | Show all bindings in the session                                |
| `:trace <expr>` | Show step-by-step refinement inference for an expression        |
| `<expr>`        | Evaluate and print the result                                   |

### Session state

The REPL maintains a persistent environment across inputs:

- **Bindings**: `let x = expr` creates a binding visible in subsequent inputs
- **Loaded arrows**: `:l arrows/fn/map.hnl` makes `map` available as a value
- **Active kit**: determined by `.handl.toml` in the current directory (same resolution as the LSP server)

### Multi-line input

If a line ends with an incomplete expression (open bracket, unclosed `do` block, trailing operator), the REPL continues to the next line with a `... ` prompt until the expression is complete. This mirrors how GHCi and IEx handle multi-line input.

### Output format

```
handl> arr("double") >>> fold(0, +)
  ⇒ <arrow: In ~> Out>
  ⇝ In: List(Int) ~> Out: Int
  ⇝ Guarantee: AllSatisfy(λx. x ≥ 0) → Sum ≥ 0

handl> [1, 2, 3]
  ⇒ List(Int)
  ⇝ LengthEq(3)

handl> :t arr("double") >>> fold(0, +)
  ⇝ List(Int) ~> Int
```

- `⇒` shows the evaluated value (or value type)
- `⇝` shows the inferred refinement type

### Integration with existing infrastructure

The REPL uses the same kit loading as the CLI host and LSP server:

1. Resolve `.handl.toml` from the current directory
2. Load the kit's data files (lexer rules, tokens, grammar)
3. Create a `Kit` instance
4. For each input: lex → parse → typecheck → eval, using the kit's engines

No new crates needed. The REPL is a new command in `handl-host-cli`'s CLI module.

### History and readline

Use the `rustyline` crate for line editing, history, and tab completion. Tab completion completes:

- Arrow names from loaded files
- Combinator names (`arr`, `comp`, `first`, `choice`, `loop`, `id`)
- `:` commands

`rustyline` is a well-maintained, pure-Rust readline replacement with no system dependencies.

### Error handling

Parse errors, type errors, and refinement errors are printed in-place without killing the session:

```
handl> arr("double") >>> invalid_token
  ⚠ Parse error: unexpected token 'invalid_token' at line 1, col 16

handl> [1, "two", 3]
  ⚠ Type error: expected Int, got String at index 1
```


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

The full record (context, tables, phases, trade-offs): [`design/0005-handl-repl.md`](../design/0005-handl-repl.md).
