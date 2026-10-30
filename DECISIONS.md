---
title: Decisions
type: index
---

# Decisions

One line per decision rule, generated from each ADR's `## Decision`
section with `host/scripts/gen-decisions.py`. The ADR is the argument;
this is the recall surface (ADR-0161 §8). Superseded records are excluded.

### Handl REPL

- **Bindings**: let x = expr creates a binding visible in subsequent inputs — [ADR-0005](adrs/0005-handl-repl.md)
- **Loaded arrows**: :l arrows/fn/map.hnl makes map available as a value — [ADR-0005](adrs/0005-handl-repl.md)
- **Active kit**: determined by .handl.toml in the current directory (same resolution as the LSP server) — [ADR-0005](adrs/0005-handl-repl.md)
- ⇒ shows the evaluated value (or value type) — [ADR-0005](adrs/0005-handl-repl.md)
- ⇝ shows the inferred refinement type — [ADR-0005](adrs/0005-handl-repl.md)
- Resolve .handl.toml from the current directory — [ADR-0005](adrs/0005-handl-repl.md)
- Load the kit's data files (lexer rules, tokens, grammar) — [ADR-0005](adrs/0005-handl-repl.md)
- Create a Kit instance — [ADR-0005](adrs/0005-handl-repl.md)
- For each input: lex → parse → typecheck → eval, using the kit's engines — [ADR-0005](adrs/0005-handl-repl.md)
- Arrow names from loaded files — [ADR-0005](adrs/0005-handl-repl.md)
- Combinator names (arr, comp, first, choice, loop, id) — [ADR-0005](adrs/0005-handl-repl.md)
- : commands — [ADR-0005](adrs/0005-handl-repl.md)

### CLI as Capability Orchestrator

- plugin and module discovery — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- project and configuration management — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- dispatching commands to the appropriate capability providers — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- compiler-native, compiler-js, compiler-wasm — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- formatter-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- typechecker-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- linter-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- debugger — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
