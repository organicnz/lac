# Autonomous Loops & Kanban System Guide

## 1. Operating Discipline

Local Agentic Coding operates under strict non-stop reliability rules defined in `AGENTS.md`. Vague tasks fail; well-scoped, chunked tasks succeed.

- **Task Queue**: Managed via `~/todo/lac-tasks.yaml`.
- **Loop Definitions**: Stored in `~/todo/lac-loops/*.yaml`.
- **Reliability Gate**: Tests-as-gate + dual-model review + mandatory human git review.

---

## 2. The 2-Round Review Rule

Per cmaven research on multi-agent feedback loops:
```
Round 1: implement → review → apply  (Resolves 90%+ of genuine defects)
Round 2: re-review → apply           (Verifies Round 1 did not introduce regressions)
Round 3+: ABORT AND ESCALATE         (Ambiguous requirements; human intervention required)
```

1. **Implement**: `@coder` implements minimal diff and runs unit tests.
2. **Review**: `@reviewer` (read-only, edit denied) inspects `git diff` and categorizes findings into `Critical`, `Major`, `Minor`, `Suggestion`.
3. **Apply**: `@coder` applies **only** `Critical` and `Major` findings. `Minor` and `Suggestion` items are logged without touching code.
4. **Gate**: Loop halts before committing. Changes remain unstaged in the working tree for the human to review with `git diff`.

---

## 3. Pre-built Loop Catalog

| Loop File | Target Use Case |
|---|---|
| `daily-coding.yaml` | Standard routine feature or refactoring task |
| `bug-fix.yaml` | Bug reproduction, root-cause isolation, and regression verification |
| `feature-branch.yaml` | Multi-file feature developed in a dedicated `jj` / git worktree |
| `security-audit.yaml` | Deep vulnerability scanning, triage, and surgical patching |

---

## 4. CLI Workflow

```bash
# 1. Initialize loops and task queue in ~/todo/
lac loop init

# 2. Inspect available loops
lac loop list

# 3. Validate a loop definition
lac loop validate ~/todo/lac-loops/daily-coding.yaml

# 4. Check stack readiness before long autonomous runs
lac status
lac doctor

# 5. Run OpenCode with loop orchestrator
opencode2 run 'Use the loop-orchestrator skill --run ~/todo/lac-loops/daily-coding.yaml'
```
