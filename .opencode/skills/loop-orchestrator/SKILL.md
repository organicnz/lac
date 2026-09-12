---
name: Loop Orchestrator
description: Run defined agentic loops (implement-review-apply) with safety gates
---

## What it does

Orchestrates multi-step agentic coding loops with safety gates between phases. Prevents infinite implement↔review ping-pong (cap at 2 rounds per research). Each phase calls appropriate skills for state, context, and thermal safety.

Implements the proven pattern from cmaven dual-model review research:
```
Round 1: implement → review → apply  (most real problems surface here)
Round 2: re-review → apply           (only checks if round 1 introduced new problems)
Round 3+: human steps in; requirement is ambiguous
```

## Workflow

### 1. Define loop (YAML in ~/todo/lac-loops/)

```yaml
# ~/todo/lac-loops/feature-auth-fix.yaml
name: feature-auth-fix
task: "Fix 3 security flaws in login function"
max_rounds: 2
phases:
  - implement:
      model: ollama/qwen3.8-27b
      skills_pre: [context-cap, thermal-monitor]
      prompt: "Implement fix for: {{task}}. Run tests, do NOT commit."
  - review:
      model: ollama/qwen3.8-27b
      skills_pre: [agent-resume]
      prompt: "@reviewer Review git diff for: {{task}}. Report by severity."
  - apply:
      model: ollama/qwen3.8-27b
      prompt: "Apply only critical+major findings. Re-run tests."
  - gate:
      type: human
      prompt: "Read git diff. Commit if correct."
```

### 2. Run loop

```bash
opencode2 run 'Use the loop-orchestrator skill --run ~/todo/lac-loops/feature-auth-fix.yaml'
```

Execution:
1. **Pre-flight**: `context-cap --check`, `thermal-monitor --check`, `agent-resume --save`
2. **Round 1 implement**: Run implement phase, save state after
3. **Round 1 review**: Run review phase (read-only, no edits)
4. **Round 1 apply**: Apply critical+major only, re-run tests
5. **Round 2** (if needed): Re-review only for new problems introduced in Round 1
6. **Gate**: Stop, present git diff for human commit (never auto-commit)
7. **Post**: `agent-resume --save`, log loop completion

### 3. Safety gates (enforced, not optional)

- **Max 2 rounds**: Hard cap. Round 3+ requires human override flag `--force-round-3`
- **No auto-commit**: Gate phase always requires human `git commit`. Reviewer reads `git diff` which needs changes in working tree.
- **Severity filter**: Apply only critical+major. Minor/suggestion require explicit human accept per finding.
- **Thermal gate**: If `thermal-monitor` reports Serious/Critical, pause loop, save state, wait.
- **Context gate**: If `context-cap` reports >90% of max, truncate before next phase.

## Pre-built loops (`~/todo/lac-loops/`)

| Loop file | Purpose |
|---|---|
| `daily-coding.yaml` | Generic implement→review→apply for routine tasks |
| `feature-branch.yaml` | Feature branch → commits → PR description → review |
| `bug-fix.yaml` | Reproduce → root cause → fix → test → verify fix |

## Config

```json
"skills": {
  "loop-orchestrator": {
    "loops_dir": "~/todo/lac-loops",
    "max_rounds": 2,
    "require_human_gate": true,
    "auto_save_state": true
  }
}
```

## When to use

- **Any multi-file change**: Use loop instead of single `opencode2 run` to get review safety
- **Security-sensitive code**: Mandatory 2-round loop with reviewer
- **Late-night autonomous runs**: Loop with `--auto-save` + thermal/context gates for 8hr safety
- **Never for trivial single-file edits**: Overkill; use direct `opencode2 run`

## Integration

Calls in order:
1. `context-cap --check` + `thermal-monitor --check` (pre-flight)
2. `agent-resume --save` (checkpoint)
3. Implement phase (OpenCode primary agent)
4. `agent-resume --save` (post-implement)
5. Review phase (`@reviewer` subagent, read-only)
6. Apply phase (critical+major only)
7. `agent-resume --save` (post-apply)
8. Human gate (git diff review + commit)
