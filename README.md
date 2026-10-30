---
title: '`project/` — what goes where'
type: readme
date: 2026-10-29
---

# `project/` — what goes where

The rule is ADR-0162 (`host/project/adrs/0162-project-folder-taxonomy.md`);
this file is the map. Core entries mean the same thing in every repo.

| entry | answers | lifetime |
|---|---|---|
| `adrs/` | *why* we decided, and what we rejected | append-only |
| `design/` | *how* it works — mechanism, algorithms | until the code is the spec |
| `rfcs/` | *when* we port a piece to Handl | until the port lands |
| `tasks/`, `TODO.md` | *who does what next* | consumed |
| `BACKLOG.md` | *not now* — accepted, unscheduled | reaped quarterly |
| `parked.yaml` | *retired* — deliberately not built | permanent, with unpark trigger |
| `notes/` | *what we found* | promoted or deleted |
| `reports/` | *what we measured* | dated, immutable |
| `retros/` | *what went wrong* | dated, immutable |
| `references.md` | *what we depend on* — other repos' ADR ids | maintained |
| `DECISIONS.md` | *what was decided* — one line per rule | generated, never hand-edited |
| `docs-split.yaml` | *where things live* — assignment manifest | maintained |
| `principles.md` | *org principles* (org `project` repo only) | maintained |

Present here: `DECISIONS.md`, `adrs`, `design`, `references.md`, `rfcs`, `tasks`.

Published documentation is not here — explanations, tutorials, how-to and
reference live in the org `docs` repo (each repo's `docs/site/`, vendored into
the hub). See ADR-0162 §1.

## Local entries

- none.
