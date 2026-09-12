---
name: Task Runner
description: Simple job queue processor for autonomous task batches
---

## What it does

Processes queued coding tasks one by one without manual intervention per task. Reads `~/todo/lac-tasks.yaml`, pops first `pending` entry, runs it via `loop-orchestrator`, marks `complete` or `failed`, moves to next. Enables "process these 5 tickets overnight and stop" autonomy.

Lower priority than loop safety, but essential for true non-stop operation where human queues work and machine drains queue.

## Workflow

### 1. Queue tasks (human does this)

```yaml
# ~/todo/lac-tasks.yaml
- id: "1"
  task: "refactor auth login function for timing attack"
  priority: high
  status: pending
  loop: feature-auth-fix.yaml
  created: 2026-09-10T10:00:00Z

- id: "2"
  task: "fix memory leak in data processor"
  priority: medium
  status: pending
  loop: bug-fix.yaml
  created: 2026-09-10T10:05:00Z

- id: "3"
  task: "add unit tests for token validator"
  priority: low
  status: pending
  loop: daily-coding.yaml
  created: 2026-09-10T10:10:00Z
```

### 2. Run queue (machine does this)

```bash
opencode2 run 'Use the task-runner skill --drain --max 5'
# Processes up to 5 pending tasks in priority order, then stops
```

Execution per task:
1. Pop highest priority `pending` task, mark `in_progress`
2. `agent-resume --save` checkpoint
3. `loop-orchestrator --run <loop>` with task description
4. On success: mark `complete`, log duration, tokens used
5. On failure: mark `failed`, save error + state, continue to next (do not stop queue)
6. After `--max` tasks or queue empty: stop, report summary
7. `agent-resume --save` final checkpoint

### 3. Monitor queue

```bash
opencode2 run 'Use the task-runner skill --status'
# Reports: pending count, in_progress, complete, failed, estimated time remaining
```

## Safety

- **Max tasks per run**: Required `--max` flag (default 3). Prevents infinite overnight runs.
- **Failure isolation**: Failed task does not stop queue. Error saved, next task starts with clean context (`kv-cache-manage --truncate` between tasks).
- **Human gate preserved**: Each task's loop still ends with human gate (no auto-commit). Queue drains to "ready for human review" state, not "auto-pushed".
- **Thermal/context gates**: Between tasks, check `thermal-monitor` and `context-cap`. Pause queue if Serious/Critical.

## Config

```json
"skills": {
  "task-runner": {
    "queue_path": "~/todo/lac-tasks.yaml",
    "default_max": 3,
    "continue_on_failure": true,
    "truncate_between_tasks": true
  }
}
```

## When to use

- **Overnight batches**: Queue 3-5 well-defined tasks, run `--drain --max 5`, review diffs in morning
- **Well-refined tasks only**: Queue entries must be specific ("fix X in file Y") not vague ("improve auth"). Vague tasks fail and waste queue time.
- **Never for exploratory work**: Task-runner is for execution, not discovery. Human refines task first, then queues.

## Integration

Calls per task:
1. `agent-resume --save` (pre-task checkpoint)
2. `kv-cache-manage --truncate` (clean context between tasks)
3. `loop-orchestrator --run` (execute with review safety)
4. `agent-resume --save` (post-task checkpoint)
5. Update `lac-tasks.yaml` status
