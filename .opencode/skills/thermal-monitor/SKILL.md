---
name: Thermal Monitor
description: Watch Mac temperature during sustained inference, auto-throttle on overheat
---

## What it does

Monitors Apple Silicon SoC temperature during 24/7 local inference. Reads thermal state via `pmset -g thermals` and Apple Silicon sensors. Logs to `/Volumes/AIModels/hf/thermal-log/` (or `~/.lac/models` fallback when the volume is absent). Triggers alerts and auto-throttling if sustained temp exceeds safe thresholds for non-stop coding.

Per research (measured on Mac Studio): Keep vents clear, 18-24°C ambient, don't stack TB5 enclosures on exhaust. Sustained ~55W for 7B-class, ~75W for 27B-class desktop chassis. LLM decode is GPU/memory-bandwidth bound, CPU 5-15%.

## Workflow

### 1. Check current thermals

```bash
opencode2 run 'Use the thermal-monitor skill --check'
# Reports: CPU temp, GPU temp, fan RPM, thermal pressure, recommendation
```

The skill will:
1. Run `pmset -g thermals` and parse thermal pressure (Nominal/Fair/Serious/Critical)
2. Run `sudo powermetrics --samplers smc -n 1` for CPU/GPU die temps (if available)
3. Check ambient via heuristic (fan RPM vs temp curve)
4. Report: current state, trend (rising/stable/falling), recommendation (continue/throttle/pause)

### 2. Continuous monitoring (for 24/7 loops)

```bash
opencode2 run 'Use the thermal-monitor skill --watch --interval 60 --log-dir /Volumes/AIModels/hf/thermal-log'
# Logs every 60s; alerts on Serious/Critical
```

Log format (`thermal-log/<date>.csv`):
```csv
timestamp,cpu_temp_c,gpu_temp_c,fan_rpm,thermal_pressure,model,quant,action
2026-09-10T14:30:00,72,68,1800,Nominal,qwen3.8-27b,q4,continue
2026-09-10T14:31:00,78,74,2200,Fair,qwen3.8-27b,q4,continue
2026-09-10T14:32:00,85,81,2800,Serious,qwen3.8-27b,q4,throttle-to-q4
```

### 3. Auto-throttle on overheat

When thermal pressure hits Serious/Critical:
1. Save agent state via `agent-resume --save`
2. If running Q8, swap to Q4 via `model-swap` (lower power, less heat)
3. If already Q4 and still Serious, pause loop for 5 min (sleep, fans cool)
4. Log action, resume via `agent-resume --resume` when Nominal

```bash
opencode2 run 'Use the thermal-monitor skill --auto-throttle'
```

## Thresholds (Apple Silicon Macs; calibrated on Mac Studio)

| Thermal Pressure | CPU Temp | Action |
|---|---|---|
| Nominal | <75°C | Continue normally |
| Fair | 75-85°C | Log, continue, watch trend |
| Serious | 85-95°C | Throttle Q8→Q4, or pause 5 min if already Q4 |
| Critical | >95°C | Pause immediately, save state, wait for Nominal |

## Config

```json
"skills": {
  "thermal-monitor": {
    "check_interval_s": 60,
    "log_dir": "/Volumes/AIModels/hf/thermal-log",
    "throttle_on_serious": true,
    "pause_on_critical": true,
    "pause_duration_s": 300
  }
}
```

## When to use

- **Before 8+ hour loops**: `--check` to establish baseline
- **During sustained coding**: `--watch` in background terminal
- **After moving the Mac**: Re-check airflow (vents clear? enclosure on exhaust?)
- **Summer / warm ambient**: Lower thresholds by 5°C

## Integration

Works with:
- `agent-resume` — save state before throttling/pausing
- `model-swap` — Q8→Q4 swap on Serious (lower power)
- `serve-launchd` — if Critical persists, restart server after cooldown
- `loop-orchestrator` — each loop iteration checks thermals before starting
