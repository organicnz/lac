# LAC Architecture Blueprint — Apple Silicon Macs

## 1. System Overview

The Local Agentic Coding (LAC) stack targets Apple Silicon **Macs** (8GB+ supported; reference configuration below measured on a high-RAM Mac Studio). It runs **Qwen 3.8 27B** as the primary model across all coding, analysis, and auditing tasks. RAM-pressure thresholds and smart-serve tiers scale with detected total RAM (`common::mem_thresholds_gib`, `common::serve_ram_tiers_gib`), so smaller Macs get sane gates from the same code.

```
┌────────────────────────────────────────────────────────────────────────┐
│                        4. Orchestrator Layer                           │
│     Hermes Orchestrator      │  Kanban Loop Engine (`~/todo/lac-loops`) │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ Drives & monitors
┌───────────────────────────────────▼────────────────────────────────────┐
│                      3. Client & Harness Layer                         │
│ LAC Studio (macOS Liquid Glass)      │ OpenCode V2 (Agents & 14 Skills)    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ OpenAI API calls (:8000)
┌───────────────────────────────────▼────────────────────────────────────┐
│           2. LAC Unified Gateway & Engine (100% Pure Rust Native)      │
│   `lac` CLI  │  `lac-router` :8000 (Hot-swap, Auto-routing, Telemetry) │
│   `lac-tui`  │  `serve-mlx` :8080  │ `serve-llama` :8081               │
│   `kv-manage`│  `pull-models`      │ `bootstrap`                       │
└──────────┬────────────────────────┬────────────────────────┬───────────┘
           │ :8080/8082             │ :8081                  │ :11434
┌──────────▼───────────────┐ ┌──────▼───────────────┐ ┌──────▼───────────┐
│     MLX / MTPLX          │ │    llama-server      │ │      Ollama      │
│  Qwen3.8-27B 4-bit       │ │  Qwen3.8-27B Q8_0    │ │ Qwen3.8-27B GGUF │
│  (Native MTP ~45 tok/s)  │ │ (Quality / Spec Dec) │ │   (Fallback)     │
└──────────────────────────┴─┴──────────────────────┴─┴──────────────────┘
                                1. Inference Layer (Apple Silicon Macs)
```

---

## 2. Port Allocation & Gateway Map

| Component | Port | Interface | Purpose |
|---|---|---|---|
| **LAC Router** | `:8000` | HTTP / OpenAI API | Unified reverse proxy, load balancer, hot-swap endpoint |
| **MLX Server** | `:8080` | HTTP / OpenAI API | Fast Q4 inference with native Multi-Token Prediction (MTP) |
| **llama-server** | `:8081` | HTTP / OpenAI API | High-fidelity Q8_0 inference, grammar constraints, fallback |
| **Ollama** | `:11434` | HTTP / OpenAI API | Local tool use & fallback runner |

OpenCode connects to `http://127.0.0.1:8000/v1`. The LAC router transparently forwards requests to whichever engine is online, prioritizing MLX for speed, then llama-server for quality, then Ollama.

---

## 3. Unified Memory Budget (reference: 96GB Studio)

On Apple Silicon, unified memory is shared dynamically between CPU and GPU Metal cores. Worked example below is measured on a 96GB Studio; on other Macs the same rows apply with their own totals, and the code scales its gates automatically.

```
Reference Total Physical Memory: 96.0 GiB
┌───────────────────────────────┬─────────────────┬──────────────────────┐
│ Model Weights                 │ KV Cache        │ OS & Working Set     │
│ Q4_K_M: ~16.1 GiB             │ 32K ctx: ~8 GiB │ macOS 26: ~8-12 GiB  │
│ Q8_0  : ~29.5 GiB             │ 64K ctx: ~16 GiB│ Workspace: ~6 GiB    │
├───────────────────────────────┴─────────────────┴──────────────────────┤
│ Free Headroom: ~42.0 to 65.0 GiB (Wide open for compilation & tools)  │
└────────────────────────────────────────────────────────────────────────┘
```

- **Qwen3.8-27B (Q4 default)**: 16.1GB weights + 8GB KV (32K) + 12GB system = **~36GB total**. Over **60GB free** for compilation, IDE, and worktrees.
- **Qwen3.8-27B (Q8 quality)**: 29.5GB weights + 16GB KV (64K) + 12GB system = **~57.5GB total**. Over **38GB free**.
- **Working Context Cap**: 16K steady-state cap via `context-cap` skill; emergency truncation at 24K via `kv-manage`.

---

## 4. Process Supervision & Recovery

Inference servers are kept alive via Apple `launchd` daemons in `~/Library/LaunchAgents/`:
- `org.lac.router.plist`: Supervises `lac-router` on port `:8000`.
- `org.lac.serve-mlx.plist`: Supervises `serve-mlx` with crash backoff and restart throttle.

In case of Metal OOM (`Insufficient Memory`), `lac-router` returns HTTP 502 with error details, while `launchd` auto-restarts the inference backend within 10 seconds.

---

## 5. Native Orchestrator (Hermes)

Normative spec: `.opencode/skills/hermes-orchestrator/SKILL.md`. Summary:

There is no external Hermes process. `lac hermes` and `lac worker` are entry points into one native Rust loop (`cmd_worker` in `rust-src/src/lac.rs`):

- **Selection**: `judge_next()` asks the `:8000` gateway to pick the most urgent pending task from `~/todo/lac-tasks.yaml`, given thermal state. Single-candidate queues, a down gateway, `LAC_JUDGE=off`, or any model/gateway failure all resolve to deterministic file order — judgment augments selection, never reliability.
- **Execution per task**: thermal gate → KV checkpoint (`kv-manage truncate`) → isolated `task/<id>` worktree → `opencode2 run` (@coder) → `make test` fail-closed gate → native reviewer pass (`review_diff`: `git diff HEAD` plus capped untracked contents, verdict derived from `[critical]`/`[major]` markers; one @coder fix round, max 2 review rounds) → commit on pass, stash/remove failed worktree + attempt bump on failure, dead-letter (`failed`) after 3 attempts. Reviewer infra failure never blocks a test-passing commit (event-logged); round-2 leftovers commit flagged `review_flagged` for human merge review. Panic-isolated via `catch_unwind`; single-flight locked; the operator tree is never checked out, reset, or cleaned during recovery.
- **Status**: `lac hermes status` reports queue counts, gateway, thermals, and model from the live machine. `lac hermes run [--drain]` delegates to the worker loop. `LAC_JUDGE=off` / `LAC_REVIEW=off` force deterministic skip of each LLM gate.

---

## 6. Remote Access via Tailscale (tailnet-only, no public ports)

`lac-router` binds loopback by default. Remote Studio clients connect over the
tailnet (WireGuard-encrypted), never the public internet. No `:8000` ingress
rule on Oracle/VPS firewalls.

- Router: `LAC_BIND_ADDR` (default `127.0.0.1`; non-loopback binds are refused
  unless explicitly acknowledged) + `LAC_API_TOKEN`. Non-loopback bind without a token refuses to start
  (fail closed). When a token is configured it is required on every listener,
  including loopback, so a local reverse proxy cannot bypass the gate;
  forwarded proxy headers also require it. Requests without a valid
  `Authorization: Bearer` get 401 and are never forwarded. Token comparison is
  constant-time. See `lac config` (redacted) and `lac doctor` (`remote_auth` check).
- Studio: `LACConnectionStore` (`SwiftUI/.../LACConnection.swift`) — host
  (MagicDNS `mac.tailXXX.ts.net`), port, TLS toggle (required for remote
  profiles; use `tailscale serve --https`), Bearer token in Keychain
  (`org.lac.studio`). All clients (chat, code, loops, dashboard) route through
  `connection.url(path:)` + `authorize(&req)`. The store rejects arbitrary
  public hosts and plaintext remote profiles before creating a request, and
  clears the token whenever the origin changes. Local serve/daemon/host-fact
  actions self-disable while remote.
- Ops: one tailnet, device approval + key expiry + Tailnet Lock on,
  ACL `laptops => mac:8000`. `launchctl setenv LAC_API_TOKEN "$(openssl rand -hex 32)"`
  then `lac daemon install`. A `tailscale serve` reverse proxy must preserve the
  Bearer header; configure the token even when the router itself listens on
  loopback. Oracle box is a plain tailnet member (status/ping/logs), never a public relay.
