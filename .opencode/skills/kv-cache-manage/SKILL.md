---
name: KV Cache Manage
description: Truncate context before OOM kills the server during sustained agentic coding
---

## What it does

Prevents `mlx-lm.server` Metal OOM crashes by truncating conversation context at configurable thresholds while preserving essential system prompt + tool definitions. The #1 cause of server death during long agentic loops is unbounded KV cache growth.

Per Dan MacKinlay (2026): *"The KV cache grows unboundedly as the conversation gets longer"* and *"libc++abi: terminating due to uncaught exception... [METAL] Command buffer execution failed: Insufficient Memory."*

## Workflow

Rust binary implements checks locally (no model call needed):
`./rust-src/target/release/kv-manage check [--tokens N] [--threshold N] [--kv-q4|--kv-q8|--kv-fp16]`
`./rust-src/target/release/kv-manage truncate --tokens N [--keep N]`
Archive: `/Volumes/AIModels/hf/kv_cache_history/` on Studio, `~/.lac/kv_cache_history/` fallback.

### 1. Check current context usage

```bash
opencode2 run 'Use the kv-cache-manage skill --check'
# Reports: current tokens, KV cache estimate, free RAM, risk level
```

The skill will:
1. Query `opencode2 api get /api/health` for session token count
2. Estimate KV cache: `tokens * layers * width * 2 bytes / 1GB`
3. Check free RAM via `vm_stat` or `memory_pressure`
4. Report risk: GREEN (<16K tokens), YELLOW (16K-24K), RED (>24K, truncate now)

### 2. Truncate when needed

```bash
opencode2 run 'Use the kv-cache-manage skill --truncate --keep 8000'
# Preserves: system prompt + tool defs + last 8000 tokens
# Archives: truncated history to /Volumes/AIModels/hf/kv_cache_history/
```

The skill will:
1. Read current session history
2. Preserve: system prompt (first ~2000 tokens) + tool definitions (~3000 tokens) + last N tokens (default 8000)
3. Summarize truncated middle section into 500-token digest via qwen3.8-27B
4. Write full truncated history to `/Volumes/AIModels/hf/kv_cache_history/<timestamp>.json`
5. Replace session history with: `[system] + [tools] + [summary of truncated] + [last N tokens]`
6. Report new token count and estimated KV savings

### 3. Auto-mode for long loops

```bash
opencode2 run 'Use the kv-cache-manage skill --auto --threshold 24000'
# Monitors every turn; auto-truncates when exceeding threshold
```

## Config

```json
"skills": {
  "kv-cache-manage": {
    "threshold_tokens": 24000,
    "keep_last_tokens": 8000,
    "preserve_system": true,
    "preserve_tools": true,
    "archive_path": "/Volumes/AIModels/hf/kv_cache_history",
    "summarize_truncated": true
  }
}
```

## When to use

- **Before long agentic loops**: Run `--check` to establish baseline
- **Every 30-45 min during sustained coding**: Run `--truncate` proactively
- **When server feels slow**: TTFT increasing = KV cache pressure; truncate immediately
- **After OOM crash**: Server restarted via `serve-launchd`; run `--truncate` before resuming via `agent-resume`

## Integration

Works with:
- `agent-resume` — truncate before saving state, resume with clean context
- `context-cap` — kv-cache-manage handles emergency truncation; context-cap handles steady-state limits
- `thermal-monitor` — high temp + high KV = double risk; coordinate throttling
