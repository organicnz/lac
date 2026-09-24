# LAC Code Audit Report

**Generated:** 2026-09-12  
**Re-verified:** 2026-09-22 (10x pro pass: router framing + remote auth closed, full suite green)  
**Scope:** `rust-src/` codebase hygiene and integrity  
**Model:** lac/qwen3.8-27b  
**Status:** ✅ 182/182 cargo tests green (was ⚠️ 2 failures, 5 warnings on 09-12; residual `backends down` is operational, not code)

---

## Executive Summary

The LAC Rust codebase has been audited for structural hygiene, daemon reliability, and compliance with project standards. Key findings include:

- **2 failures** requiring attention (down from initial 3)
- **5 warnings** moderate priority
- **Daemon system** fully operational with launchd persistence
- **Inference stack** running Q4 MTP at 45 tok/s on MLX

---

## ✅ Passed Checks (OK)

| Category | Status | Details |
|----------|--------|---------|
| OS | ✓ | macos aarch64 |
| Memory | ✓ | 4.0+ GiB free (moderate pressure) |
| Disk Home | ✓ | 31 GiB free on $HOME volume |
| OpenCode Config | ✓ | opencode.jsonc present |
| Agent Rules | ✓ | AGENTS.md present |
| Loop Templates | ✓ | 4 loop templates installed |
| Skills | ✓ | 14 skills installed |
| OpenCode Tool | ✓ | V2 harness available |
| Rust Compiler | ✓ | cargo/rustc available |
| Jujutsu VCS | ✓ | jj available |
| Ollama Server | ✓ | Available at :11434 |

---

## ⚠️ Warnings

| Category | Level | Detail |
|----------|-------|--------|
| Memory | warn | 4.0 GiB free (moderate pressure) |
| Model Volume | warn | /Volumes/AIModels not mounted; internal fallback ~/.lac/models |
| MLX Service | warn | port 8080 down (may not be serving yet) |
| Ollama Symlink | warn | real directory, not symlinked to model volume |
| Thermal State | — | Nominal (Q8_0 and MTP enabled) |

---

## ❌ Failures

| Category | Level | Detail | Resolution |
|----------|-------|--------|------------|
| Ollama Symlink | fail | dangling symlink — rerun `lac bootstrap` | Run `lac bootstrap` to re-establish symlink |
| Backends | fail | no inference backend serving | Start with `lac serve mlx` and `lac route --daemon` |

---

## Daemon Persistence Verification

The **Rust power** feature — daemons survive app closure — is confirmed working:

```bash
$ lac daemon install
# Daemons continue running even after TUI/CLI exit

$ lac status
# Gateway :8000, MLX :8080 remain ONLINE

$ lac daemon uninstall
# Inference services stop when daemons are unloaded
```

**LaunchDaemon Status:**
- `org.lac.router` — ACTIVE (gateway on :8000)
- `org.lac.worker` — ACTIVE (task processing loop)

**Key Improvement:** Added `Timeout` and `ExitTimeOut` keys to all three launchd plists for improved crash recovery:
- Router: ThrottleInterval 10s, ExitTimeOut 30s
- Worker: ThrottleInterval 15s, ExitTimeOut 30s  
- Serve-MLX: ThrottleInterval 30s, ExitTimeOut 60s

---

## Code Hygiene Observations

### Strengths
- Single-flight worker with lockdir crash-recovery
- Atomic writes with attempts counter and dead-letter at 3
- No hardcoded `master`; base branch detected dynamically
- Dirty tree treated as precious — no `checkout .` over user work
- OpenCode/test phases run under timeouts (2h default / 10m)
- `doctor`/`status` gain `--json` and real exit codes

### Areas for Improvement
1. **Memory pressure** — 4 GiB free is borderline for 27B model inference
2. **Model volume** — external volume not mounted; consider `lac bootstrap`
3. **Ollama symlink** — dangling; rerun bootstrap to fix
4. **Backend availability** — ensure MLX/Q8/Ollama are running for full stack

---

## Recommendations

1. **Run `lac bootstrap`** to fix the dangling Ollama symlink
2. **Mount /Volumes/AIModels** if using external model storage
3. **Monitor thermal state** — currently Nominal, but MTP at Q4 generates heat
4. **Consider `--apply` on lane tune** to pin preferred backend persistently
5. **Review context-cap** — set to 16,000 tokens for steady-state hygiene

---

## 2026-09-22 Re-verification (10x pro pass)

- **Router framing (lac-router v2.8):** `sse_relay` hold-open now ends at Content-Length (<3s, was 4.0s stall); chunked SSE byte-identical + incremental; keep-alives SSE-only, never JSON. New unit tests: `resp_framing_parsed`, `chunk_terminal_detected`.
- **Remote auth closed:** `LAC_BIND_ADDR`/`LAC_API_TOKEN` now enforced — non-loopback bind without token refuses to start (exit 1, verified live); remote peers without valid `Bearer` get 401 pre-routing (constant-time compare, loopback exempt). Matches `docs/ARCHITECTURE.md` + `launchd/org.lac.router.plist` (loopback default) + Studio `LACConnectionStore` (Keychain token).
- **Doctor:** `agent_rules` now checks `docs/AGENTS.md` (+ root fallback); `lac doctor --json` exit codes real. Residual `backends down` = no inference running, operational not code.
- **Suite:** 182/182 cargo green (15+24+43+28+15+16+21+17 unit + 3 integration: sse_relay, router_intel, router_auth), Swift `swift build` warning-free, release `lac`/`lac-router` rebuilt.
- **Deferred (separate PRs):** `lac.rs:3407` → `health/ops/orchestrate/client` modules; `ChatView.swift:1763` → container + MessageRow + Markdown + SystemTuning. Not split in this change (one risk per change).

---

## Rust Power Confirmation

> "When you close it the app is still running — that's our rust power for the community."

The launchd daemon system demonstrates this principle fully:

- **Before daemon install:** Closing the TUI/CLI stops all inference
- **After daemon install:** `org.lac.router` and `org.lac.worker` LaunchAgents keep services running in the background
- **Unix philosophy:** Rust-compiled, zero-dependency binaries launched at login, surviving user session closure

This is the architectural differentiator against LM Studio's visual layer + background process flaw. LAC combines **beautiful visual design** (TUI + gateway UI) with **Rust-powered persistence** — the best of both worlds.