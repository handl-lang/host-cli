# Handl REPL

## Context

Handl development currently requires a full file write + server round-trip to evaluate expressions. There is no way to interactively experiment with arrows, inspect refinement types, or test combinators without creating a `.hnl` file and opening it in an editor.

A REPL (Read-Eval-Print Loop) would let developers:

- Evaluate handl expressions interactively
- Inspect inferred refinement types for expressions
- Test arrow compositions without writing files
- Explore the standard library
- Debug refinement inference step by step

The platform already has the infrastructure:

- `handl-kit-data-canonical` — lexer, parser, evaluator, typechecker (all data-driven)
- `handl-host-cli` — a CLI host that loads kits and runs commands
- `handl-plugin-engine-exec-tree-walk` — tree-walking evaluator
- `handl-plugin-engine-typecheck-nbe` — normalization-by-evaluation typechecker
- `handl-plugin-ext-refinement` — refinement inference engine

A REPL is a new mode for the existing CLI host, not a new binary.

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

## Alternatives Considered

### Alternative A: Separate `handl-repl` binary

Create a new crate `handl-host-repl` as a standalone binary.

**Rejected because:**

- The REPL uses the same kit loading, parsing, and evaluation as the CLI host. Duplicating this logic in a new binary creates maintenance burden.
- `handl-host-cli` already has the infrastructure for loading kits and running commands. The REPL is just a new interactive mode.
- One binary is simpler for users: `handl repl` vs remembering a separate `handl-repl` command.

### Alternative B: Web-based playground

Build a web REPL (like TypeScript Playground) instead of a terminal REPL.

**Rejected because:**

- Requires a JS/WASM backend to run handl in the browser, which doesn't exist yet (see ADR 0006).
- A terminal REPL is faster to build, works offline, and integrates with existing terminal workflows.
- A web playground can be built later as a separate effort.

### Alternative C: Embedded REPL in the LSP server

Add a `handl/repl` custom LSP command that the editor sends expressions to.

**Rejected because:**

- LSP is designed for document editing, not interactive evaluation. The request/response model doesn't fit a stateful REPL session.
- A terminal REPL works in any terminal, independent of the editor.
- Mixing REPL state with LSP document state would be confusing.

## Consequences

### Positive

- **Interactive development** — developers can experiment without creating files
- **Refinement inspection** — `:t` and `:trace` make refinement inference tangible
- **Arrow testing** — load arrows and compose them interactively
- **Kit switching** — `:kit` command lets developers compare syntax variants
- **Uses existing infrastructure** — no new engines, just a new CLI mode

### Negative

- **`rustyline` dependency** — adds one crate dependency to `handl-host-cli`. Minimal impact (pure Rust, no system deps).
- **Session state complexity** — the REPL must manage a persistent environment (bindings, loaded arrows, active kit). This is new state management logic not present in the current CLI.
- **Multi-line input parsing** — detecting incomplete expressions requires partial parse logic, which may need a helper in the parser.

### Neutral

- **No impact on the LSP server or editor extensions** — the REPL is a separate CLI mode.

## Status

Proposed
