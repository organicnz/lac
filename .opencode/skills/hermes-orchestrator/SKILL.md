---
name: Hermes Orchestrator
description: High-order autonomous task driver and OpenCode coordinator
---

## What it does

Hermes serves as the autonomous coordinator for the Local Agentic Coding (LAC) stack. It sits above OpenCode as the orchestrator layer (Layer 4 in the LAC Build Plan).

While OpenCode executes individual coding turns with surgical tools, Hermes:
1. **Drives Continuous Execution**: Keeps local compute cores saturated by draining tasks from `~/todo/lac-tasks.yaml`.
2. **Dispatches Structured Loops**: Executes multi-phase loops (`daily-coding.yaml`, `feature-branch.yaml`, `bug-fix.yaml`) via `loop-orchestrator`.
3. **Coordinates Subagents**: Coordinates `@coder` (implementation) and `@reviewer` (auditing) under strict 2-round caps with tests-as-gate validation.
4. **Maintains System Hygiene**: Coordinates `context-cap` (16K steady-state), `kv-cache-manage` (emergency truncation), and `thermal-monitor` before long runs.
5. **Routes through LAC Gateway**: Connects via `http://127.0.0.1:8000/v1` for automatic load balancing across MLX (Q4 MTP) and llama-server (Q8 quality).

## Workflow

### 1. Autonomous Task Drain
```bash
# Launch Hermes to drain pending Kanban tasks through OpenCode
hermes run --task-queue ~/todo/lac-tasks.yaml --gateway http://127.0.0.1:8000/v1
```

### 2. Invocation from OpenCode
```bash
opencode2 run 'Use the hermes-orchestrator skill to inspect pending tasks and trigger the next loop'
```

### 3. Execution Cycle
```
[lac-tasks.yaml]
       │
       ▼ (Pop highest priority pending task)
[Preflight Checks: Thermals Nominal, KV Green]
       │
       ▼ (Trigger loop via OpenCode)
[Round 1: @coder implement] ──> [Round 1: @reviewer audit]
       │                                │
       ▼                                ▼
[Tests pass? Yes] ───────────────> [Round 1: apply critical]
       │                                │
       ▼ (If new defects surface)       │ (Clean)
[Round 2: re-review & apply] ───────────┴─> [Human Gate: git diff review]
```

## Configuration

In `hermes-agent/cli-config.yaml` or environment:
```yaml
inference:
  provider: openai-compatible
  base_url: "http://127.0.0.1:8000/v1"
  model: "qwen3.8-27b"
  max_context: 65536
  working_cap: 150000

lac:
  tasks_file: "~/todo/lac-tasks.yaml"
  loops_dir: "~/todo/lac-loops"
  safety:
    max_rounds: 2
    require_human_gate: true
    thermal_throttle_state: "Serious"
```

## Integration Commands

- `lac status`: Check gateway and backend availability.
- `lac loop list`: List all available loop workflows.
- `lac task drain`: Drain tasks using the orchestrator.
