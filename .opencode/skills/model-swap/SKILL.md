---
name: Model Swap
description: Swap between Qwen3.8-27B variants and backends
---

## What it does

Quickly switch the active OpenCode model between:
- `ollama/qwen3.8-27b` — quality mode for analysis/refactoring (16.1GB Q4, 32K ctx)
- `ollama/qwen3.8-27b` with Q8 via llama-server (32GB, very good quality)
- `ollama/qwen3.8-27b` via MLX 4bit (16.1GB, max tok/s on Apple Silicon)

Also toggles between inference backends:
- Ollama `:11434` (simplest, GGUF quantized)
- llama-server `:8081` (Metal, speculative decoding, long-ctx stable, Q8_0)
- mlx-lm `:8080` (fastest decode on Apple Silicon Metal; Apple Silicon only)

## Workflow

### 1. via LAC CLI (Instant Pure Rust Switch)

```bash
# Switch gateway preferred backend instantly via native Rust lac CLI:
lac switch mlx       # Switch to MLX (port :8080, fastest)
lac switch llama     # Switch to llama-server (port :8081, Q8_0 quality)
lac switch ollama    # Switch to Ollama (port :11434, fallback)
lac switch auto      # Dynamic policy-ordered failover
lac switch fastest   # Lowest measured EWMA latency routing
```

### 2. via LAC Studio (macOS Native App)

- Click the **Model Selector Pill** in the top header or browse the **Hugging Face Model Hub** tab.
- Click **"Use for Chat"** on any model to instantly retarget inference.

### 3. via Terminal Control Center (`lac-tui`)

```bash
lac tui
# Option 2) Serve menu -> Toggle Q4 / Q8 or switch preferred backend
```

## Backend toggle notes

| Backend | URL | Best for | Q4 footprint |
|---|---|---|---|
| Ollama | `:11434` | Simplicity, GGUF, any model | ~16GB |
| llama-server | `:8081` | Speculative dec, long ctx, Q8_0 grammar-constrained | ~19-22GB |
| mlx-lm | `:8080` (drifts to 8082+ if taken) | Max tok/s on Apple Metal | ~16-32GB |

The unified router on `:8000` automatically handles routing, model sniffing, and health checking across all backends.