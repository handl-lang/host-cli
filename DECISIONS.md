---
title: Decisions
type: index
---

# Decisions

One line per decision rule, generated from each ADR's `## Decision`
section with `host/scripts/gen-decisions.py`. The ADR is the argument;
this is the recall surface (ADR-0161 §8). Superseded records are excluded.

### Handl REPL

- **Architecture.** handl repl ├── Prompt: handl> ├── Input: single-line or multi-line expressions ├── Evaluate: lex →… — [ADR-0005](adrs/0005-handl-repl.md)
- **Commands.** — [ADR-0005](adrs/0005-handl-repl.md)
- **Session state.** The REPL maintains a persistent environment across inputs: — [ADR-0005](adrs/0005-handl-repl.md)
- **Multi-line input.** If a line ends with an incomplete expression (open bracket, unclosed do block, trailing… — [ADR-0005](adrs/0005-handl-repl.md)
- **Output format.** handl> arr("double") >>> fold(0, +) ⇒ <arrow: In ~> Out> ⇝ In: List(Int) ~> Out: Int ⇝ Guarantee:… — [ADR-0005](adrs/0005-handl-repl.md)
- **Integration with existing infrastructure.** The REPL uses the same kit loading as the CLI host and LSP server: — [ADR-0005](adrs/0005-handl-repl.md)
- **History and readline.** Use the rustyline crate for line editing, history, and tab completion. Tab completion… — [ADR-0005](adrs/0005-handl-repl.md)
- **Error handling.** Parse errors, type errors, and refinement errors are printed in-place without killing the session: — [ADR-0005](adrs/0005-handl-repl.md)

### CLI as Capability Orchestrator

- plugin and module discovery — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- project and configuration management — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- dispatching commands to the appropriate capability providers — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- compiler-native, compiler-js, compiler-wasm — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- formatter-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- typechecker-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- linter-* — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
- debugger — [ADR-0029](adrs/0029-cli-as-capability-orchestrator.md)
