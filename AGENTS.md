# AGENTS.md — Operating Rules (LAC, qwen3.8-27B, M5 Ultra 96GB)

Distilled from the Local Agentic Coding Build Plan. This file is read by OpenCode
as project instructions. Refine it after every failure — treat it as a product.

## Model

- Primary: `ollama/qwen3.8-27b` everywhere. Q4 default (16.1GB, fast), Q8 via
  `model-swap` for quality work. Never substitute another model without human approval.
- One model resident per session. Do not load two models concurrently on 96GB
  unless KV budgets are verified first (`ollama ps`).
- Skill invocation from CLI: `opencode2 run 'Use the <skill-id> skill ...'`.
  There is no `opencode2 skill` subcommand; skills load model-side by exact ID.
- Served context 64K minimum. Defaults of 4K silently break tool use — always set
  `num_ctx` / `-c` explicitly (32K standard, 16K cap via `context-cap` skill).

## Sampling (do not change without reason)

- `temp 0.6, top_p 0.95, top_k 20`; penalties at 0 so MTP stays exact.
- KV cache FP16/INT8 — don't over-quantize, the chip is compute-bound.
- Thinking mode OFF for routine coding. Enable only for deep analysis tasks.

## Task discipline

- Scope tasks tightly; chunk them. One well-defined task per loop run.
- Work from progress files (kanban): `~/todo/lac-tasks.yaml` for queue,
  `~/todo/lac-loops/*.yaml` for loop definitions.
- Vague tasks fail — refine before queueing ("fix X in file Y", never "improve auth").

## Reliability gates (mandatory for multi-file changes)

- Tests-as-gate: re-run affected tests after every apply phase. No test pass, no commit.
- Separate auditor: `@reviewer` (read-only, edit denied) checks the work. Max 2 rounds
  (implement→review→apply, then re-review). Round 3+ means the requirement is ambiguous —
  stop and ask the human.
- Apply only critical+major findings. Minor/suggestion: report accept/reject with reasons,
  don't touch code.
- Never auto-commit. Gate phase always ends with human `git diff` review + commit.
  Reviewer reads `git diff`, which requires changes uncommitted in the working tree.

## Parallelism

- `git worktrees` + parallel agents to saturate the box (`worktree-fanout` skill, jj colocated).
- Backfill idle time with indexing/embeddings (`index-embeddings`) and test generation.
- `kv-cache-manage --truncate` between tasks for clean context.

## Session hygiene (non-stop operation)

- `context-cap --set --max 16000` at session start; `kv-cache-manage --auto --threshold 24000`
  as safety net. Slow TTFT = KV pressure = truncate immediately.
- `agent-resume --auto --interval 300` during long loops. Save before model-swap,
  before throttling, before every loop phase.
- `thermal-monitor --check` before 8hr+ runs. Serious → throttle Q8→Q4. Critical →
  pause 5 min, save state, resume when Nominal.
- Keep one model resident; keep OS + working set internal, weights library external
  (`/Volumes/AIModels`). Never run the live model from external if avoidable.

## Calibration

- Local coders are last-gen-flagship, not frontier. Keep an occasional hosted call
  for the hardest 5% — escalate, don't grind.
- LM Studio is a GUI/manager, not the speed layer — serve through mlx/llama-server.
- Sandbox OpenClaw hard if used. Hermes fits solo local coding better.
