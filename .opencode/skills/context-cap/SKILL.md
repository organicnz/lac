---
name: Context Cap
description: Enforce steady-state context limits to prevent gradual window exhaustion
---

## What it does

Complements `kv-cache-manage` (emergency truncation) with steady-state enforcement. Monitors token usage every turn and auto-truncates at configurable limits before hitting model max. Prevents slow degradation where context grows 2K→8K→16K→32K→OOM over hours.

While `kv-cache-manage` handles RED alerts (>24K), `context-cap` maintains GREEN (<16K) through continuous hygiene.

## Workflow

### 1. Set cap for session

```bash
opencode2 run 'Use the context-cap skill --set --max 16000 --keep-last 6000'
# Enforces: total context never exceeds 16K; keeps system + tools + last 6K
```

### 2. Check current usage

```bash
opencode2 run 'Use the context-cap skill --check'
# Reports: current tokens, % of cap, % of model max (32K), turns until cap
```

### 3. Enforce (called automatically each turn if auto enabled)

```bash
opencode2 run 'Use the context-cap skill --auto --max 16000'
```

Enforcement logic:
1. If `current_tokens < max`: do nothing, report GREEN
2. If `current_tokens >= max`: truncate oldest non-essential messages
3. Always preserve: system prompt (~2000) + tool defs (~3000) + last N (default 6000)
4. Summarize removed section to 300-token digest, prepend as `[history summary]`
5. Log truncation to `/Volumes/AIModels/hf/context-truncation/<timestamp>.json`

## Difference vs kv-cache-manage

| Aspect | `context-cap` | `kv-cache-manage` |
|---|---|---|
| Purpose | Steady-state hygiene | Emergency OOM prevention |
| Threshold | Low (16K default) | High (24K default) |
| Frequency | Every turn | On-demand / RED alert |
| Action | Gentle trim | Aggressive truncate + summarize |
| Use together | Yes — cap keeps GREEN, kv-manage handles RED spikes | |

**Recommended:** Enable both. `context-cap --auto --max 16000` for steady state, `kv-cache-manage --auto --threshold 24000` as safety net.

## Config

```json
"skills": {
  "context-cap": {
    "max_tokens": 16000,
    "keep_last_tokens": 6000,
    "preserve_system": true,
    "summarize_removed": true,
    "log_dir": "/Volumes/AIModels/hf/context-truncation"
  }
}
```

## When to use

- **Start of every long session**: `--set --max 16000` to establish hygiene
- **With kv-cache-manage**: Run both; cap for steady state, kv-manage for emergencies
- **After model-swap Q4→Q8**: Lower cap by 20% (Q8 KV larger per token)
- **For 32K model max**: Cap at 16K (50%) leaves headroom for output + tools

## Integration

Works with:
- `kv-cache-manage` — cap for GREEN, kv-manage for RED; complementary thresholds
- `agent-resume` — save state before truncation (state includes pre-truncation summary)
- `loop-orchestrator` — each loop iteration checks cap before starting
