---
name: Local Verify
description: Verify the local model stack is running and responsive
---

## What it does

Checks that:
1. OpenCode config references an available provider
2. Ollama / mlx-lm / llama-server is reachable at configured URL
3. At least one model is listed and responding
4. Context window is as expected

## Workflow

1. Run `lac doctor` or `./rust-src/target/release/lac doctor` (pure Rust diagnostic suite)
2. Run `curl -s http://127.0.0.1:8000/lac/status` to inspect `lac-router` gateway and backend health
3. Run `lac status` or check the telemetry in **LAC Studio** / `lac-tui`
4. Run a quick `chat/completions` ping against the gateway: `POST http://127.0.0.1:8000/v1/chat/completions` with `{"model":"qwen3.8-27b","messages":[{"role":"user","content":"ping"}],"temperature":0}`
5. Report: active backend (MLX/llama/Ollama), TTFT, free RAM, and thermal state

## Usage

```bash
opencode2 run 'Use the local-verify skill and report stack health'
# or in TUI: ask for the local-verify skill by ID
```