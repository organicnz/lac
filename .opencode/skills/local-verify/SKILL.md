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

1. Run `opencode2 api get /api/health` to verify OpenCode server alive
2. Run `curl -s http://127.0.0.1:11434/v1/models` (Ollama) or `curl -s http://127.0.0.1:8080/v1/models` (llama-server)
3. Parse model list, confirm expected model name present
4. Run a quick `chat/completions` ping: `POST /v1/chat/completions` with `{"messages":[{"role":"user","content":"ping"}],"temperature":0}`
5. Report: model name, tok/s (if available), context window, latency

## Usage

```bash
opencode2 run 'Use the local-verify skill and report stack health'
# or in TUI: ask for the local-verify skill by ID
```