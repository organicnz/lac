# Qwen 3.8 Speculative Decoding & MTP Tuning Guide

## 1. What is MTP (Multi-Token Prediction)?

Qwen 3.8 27B is shipped with built-in Multi-Token Prediction (MTP) heads. Unlike classic speculative decoding which requires loading a separate draft model (wasting GPU memory and bandwidth), MTP generates speculative candidate tokens directly from auxiliary heads trained in the base architecture.

- **Speedup Factor**: ~2.24x speedup over standard autoregressive decoding.
- **Decoding Speed on M5 Ultra**:
  - 20K–30K context: **~45 tokens/second**
  - 128K context: **~22 tokens/second**
- **Memory Overhead**: Zero additional model weights needed; uses native MTP heads.

---

## 2. Sampling Parameters for Exact Speculative Verification

Speculative decoding depends on candidate token verification matching the target model distribution. Non-zero repetition penalties or aggressive penalties disrupt the MTP verification logits.

```yaml
# Strict Sampling Presets for Qwen 3.8 MTP:
temperature: 0.6
top_p: 0.95
top_k: 20
repetition_penalty: 1.0       # Must remain 1.0 (no penalty) so MTP stays exact
frequency_penalty: 0.0        # Penalties at 0
presence_penalty: 0.0
```

---

## 3. Server Flags & Optimizations

### MLX / MTPLX (`serve-mlx`)
```bash
# Optimal invocation for MLX
mlx_lm.server \
  --model mlx-community/Qwen3.8-27B-4bit \
  --host 127.0.0.1 \
  --port 8080 \
  --max-tokens 8192
```

### llama-server (`serve-llama`)
```bash
# Speculative decoding flags on Metal
llama-server \
  -m unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0 \
  --host 127.0.0.1 \
  --port 8081 \
  -c 16384 \
  -fa on \           # Flash Attention on Metal
  -b 2048 \          # Large batch size for prompt processing
  -ub 256 \          # Micro-batching for low latency
  -np 4 \            # Parallel slots for concurrent agent queries
  -ngl 999           # Full GPU offload to M5 Ultra Metal cores
```

---

## 4. Benchmarking MTP Performance

Use the unified `lac bench` command to measure roundtrip latency:
```bash
lac bench 8000   # Benchmark through router
lac bench 8080   # Benchmark direct to MLX
lac bench 8081   # Benchmark direct to llama-server
```
