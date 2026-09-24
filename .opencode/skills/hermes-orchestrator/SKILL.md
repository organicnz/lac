---
name: Hermes Orchestrator
description: High-order autonomous task driver and OpenCode coordinator
---

## What it does

Hermes is the native orchestrator for the Local Agentic Coding (LAC) stack — the judgment layer over the 24/7 worker (Layer 4 in the LAC Build Plan). There is no external Hermes process: `lac hermes` and `lac worker` are the same native Rust binary (`rust-src/src/lac.rs`).

While OpenCode executes individual coding turns with surgical tools, Hermes:
1. **Drives Continuous Execution**: Drains tasks from `~/todo/lac-tasks.yaml` via `lac worker` (continuous) or `lac worker --drain` (batch). Single-flight locked, crash-recoverable.
2. **Judges Task Selection**: Picks the next task with LLM judgment over pending candidates plus thermal state (`judge_next` in `lac.rs`, via the `:8000` gateway). Any model/gateway failure falls back to deterministic file order — judgment never makes the daemon less reliable.
3. **Dispatches Work**: Each task runs on an isolated `task/<id>` branch via `opencode2 run` (@coder), gated by `make test` (fail-closed), then a native **reviewer pass** (`review_diff` in `lac.rs`: `git diff HEAD` plus capped untracked-file contents, judged via the gateway, verdict derived from `[critical]`/`[major]` finding markers). Critical/major findings go back to @coder for one fix round (max 2 review rounds per AGENTS.md); round-2 leftovers commit to the task branch flagged `review_flagged` for human merge review. Minor/suggestion findings are never applied. Reviewer infra failure (gateway down, timeout) never blocks a test-passing commit — event-logged, queue keeps moving.
4. **Maintains System Hygiene**: Thermal gate before every task (Critical → sleep 300s, Serious → throttle 60s), `kv-manage truncate` checkpoint per task, `context-cap` 16K steady-state.
5. **Routes through LAC Gateway**: Connects via `http://127.0.0.1:8000/v1` for automatic load balancing across MLX (Q4 MTP) and llama-server (Q8 quality).

Note: the worker path is implement (@coder) → test (`make test`) → review (`review_diff`: critical/major only, max 2 rounds) → commit to the `task/<id>` branch. Human merge review of task branches stays mandatory.

## Workflow

### 1. Autonomous Task Drain
```bash
# Continuous 24/7 watch (same loop under both names)
lac hermes run
lac worker

# Batch drain and exit
lac hermes run --drain
lac worker --drain

# Native status (queue, gateway, thermals — no external process)
lac hermes status
```

### 2. Invocation from OpenCode
```bash
opencode2 run 'Use the hermes-orchestrator skill to inspect pending tasks and trigger the next loop'
```

### 3. Execution Cycle
```
[lac-tasks.yaml]
       │
       ▼ (Judge next task: LLM pick, file-order fallback)
[Preflight: router up, backend live, queue present; per-task thermal gate]
       │
       ▼ (Isolated task/<id> branch + KV checkpoint)
[@coder implement via opencode2] ──> [make test gate, fail-closed]
       │                                        │
       ▼ (pass)                                 ▼ (fail: reset branch, bump attempts, dead-letter at 3)
[Reviewer audit diff: approve | fix→1 @coder fix round→re-test→re-review]
       │
       ▼ (approve / reviewer down / 2 rounds spent → commit feat(<id>),
          round-2 leftovers flagged review_flagged for human merge review)
[Next task ← judge again]
```

Set `LAC_JUDGE=off` to force deterministic file-order selection without a model call (tests, deterministic ops).
Set `LAC_REVIEW=off` to skip the reviewer pass (deterministic ops).

## Configuration

Environment (no config file — there is no `hermes-agent/cli-config.yaml`):
```bash
LAC_CHAT_MODEL=qwen3.8-27b        # judging + chat model (else opencode.jsonc)
LAC_CHAT_TIMEOUT_SECS=300         # judging call budget (post_chat)
LAC_JUDGE=off                     # force deterministic file order
LAC_TASK_TIMEOUT_SECS=7200        # per-task opencode2 budget
```

Queue and loops:
- tasks: `~/todo/lac-tasks.yaml` (`lac loop init` installs the template)
- loops: `~/todo/lac-loops` (templates in `templates/loops/`)

Safety (enforced in code, mirroring `AGENTS.md`):
- max attempts per task: 3, then dead-letter (`failed`)
- thermal: Serious → 60s throttle, Critical → 300s pause
- dirty working tree is precious: worker never resets user work

## Integration Commands

- `lac hermes status`: Queue, gateway, thermal, and model state.
- `lac hermes run [--drain]`: Native drain (delegates to the worker loop).
- `lac worker [--drain]`: Same loop under its primary name.
- `lac status`: Check gateway and backend availability.
- `lac loop list`: List all available loop workflows.
