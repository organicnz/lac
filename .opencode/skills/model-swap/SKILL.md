---
name: Model Swap
description: Swap between Qwen3.8-27B variants and backends
---

## What it does

Quickly switch the active OpenCode model between:
- `ollama/qwen3.8-27b` — quality mode for analysis/refactoring (16.1GB Q4, 32K ctx)
- `ollama/qwen3.8-27b` with Q8 via llama-server (32GB, very good quality)
- `ollama/qwen3.8-27b` via MLX 4bit (16.1GB, max tok/s on M5 Ultra)

Also toggles between inference backends:
- Ollama `:11434` (simplest, GGUF quantized)
- llama-server `:8080` (Metal, speculative decoding, long-ctx stable, Q8_0)
- mlx-lm `:8080` (fastest decode on Apple Silicon, M5 Ultra Metal)

## Workflow

### 1. via OpenCode config (per-session)

Edit `opencode.jsonc` model field, or use the model selection TUI:

```bash
# Select model interactively
opencode2 model select

# Or set via env on run
OLLAMA_MODEL=qwen3.8-27b opencode2 run "analyze this codebase"
```

### 2. via skill tool

```bash
opencode2 run 'Use the model-swap skill'
```

### 3. via `/models` picker

```
/models  →  ollama/qwen3.8-27b
```

## Backend toggle notes

| Backend | URL | Best for | Q4 footprint |
|---|---|---|---|
| Ollama | `:11434` | Simplicity, GGUF, any model | ~16GB |
| llama-server | `:8080` | Speculative dec, long ctx, Q8_0 grammar-constrained | ~19-22GB |
| mlx-lm | `:8080` | Max tok/s on M5 Ultra Metal | ~16-32GB |

To switch backends: stop current server, start new one, then `opencode2 model select` or repoint provider `baseURL`.