---
name: Agent Resume
description: Persist agent state across interruptions and resume interrupted loops
---

## What it does

Saves agent loop state to `/Volumes/AIModels/hf/agent-state.json` (or the `~/.lac` fallback when the volume is absent) so that if OpenCode crashes, the server OOMs, or the Mac reboots, the agent can resume where it left off instead of starting from scratch.

State includes: last task, completed steps, pending steps, workspace snapshot path, model quant in use, last TTFT, git diff hash.

## Workflow

### 1. Save state (call before risky operations or periodically)

```bash
opencode2 run 'Use the agent-resume skill --save --task "refactor auth module"'
# Writes state to /Volumes/AIModels/hf/agent-state.json
```

The skill will:
1. Capture current task description from session
2. List completed steps (from conversation history: files read, edits made, tests run)
3. List pending steps (inferred from task + completed)
4. Snapshot workspace: `git diff > /Volumes/AIModels/hf/snapshots/<timestamp>.diff`
5. Record: model (`ollama/qwen3.8-27b`), quant (q4/q8), last TTFT, timestamp
6. Write JSON to `/Volumes/AIModels/hf/agent-state.json`

### 2. Resume state (call after restart/crash)

```bash
opencode2 run 'Use the agent-resume skill --resume'
# Reads state, restores context, continues loop
```

The skill will:
1. Read `/Volumes/AIModels/hf/agent-state.json`
2. Report: last task, completed steps, pending steps, time since save
3. Restore workspace if needed: `git apply /Volumes/AIModels/hf/snapshots/<timestamp>.diff`
4. Rebuild context: system prompt + task + completed summary + pending steps
5. Continue loop from first pending step
6. Clear state file after successful resume (or archive to `agent-state-history/`)

### 3. Auto-save mode

```bash
opencode2 run 'Use the agent-resume skill --auto --interval 300'
# Auto-saves every 300s (5 min) during long loops
```

## State file format (`agent-state.json`)

```json
{
  "last_task": "refactor auth module for security",
  "completed_steps": [
    "read git diff",
    "identified 3 security flaws in login function",
    "created worktree feature/auth-fix"
  ],
  "pending_steps": [
    "apply critical fixes to login function",
    "re-run auth tests",
    "commit with message"
  ],
  "workspace_snapshot": "/Volumes/AIModels/hf/snapshots/2026-09-10T14-30-00.diff",
  "model": "ollama/qwen3.8-27b",
  "quant": "q4",
  "last_ttft_ms": 2300,
  "saved_at": "2026-09-10T14:30:00Z",
  "git_head": "abc123"
}
```

## Config

```json
"skills": {
  "agent-resume": {
    "state_path": "/Volumes/AIModels/hf/agent-state.json",
    "snapshot_dir": "/Volumes/AIModels/hf/snapshots",
    "auto_save_interval_s": 300,
    "max_history": 10
  }
}
```

## When to use

- **Before long loops**: `--save` to establish checkpoint
- **After crash/restart**: `--resume` to continue (call this first in new session)
- **During sustained coding**: `--auto` for periodic checkpoints every 5 min
- **Before model-swap**: Save state, swap quant, resume with clean context

## Integration

Works with:
- `kv-cache-manage` — truncate context before saving state (smaller state file, faster resume)
- `serve-launchd` — after server auto-restart, call `--resume` to continue loop
- `loop-orchestrator` — each loop step calls `--save` before and after
- `thermal-monitor` — if throttling, save state before pausing
