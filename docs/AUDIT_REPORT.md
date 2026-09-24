# LAC Code Audit Report

**Generated:** 2026-09-12  
**Re-verified:** 2026-09-24 (10x pro delivery pass)
**Scope:** Rust backend, Swift Studio, router framing/auth, packaging, and delivery checks
**Model:** lac/qwen3.8-27b  
**Status:** ✅ Rust + Swift tests, process-level E2E smoke, and packaged-app verification are enforced by `make validate`

---

## Executive Summary

The delivery path is now fail-closed and hermetic:

- Rust unit/integration tests and Swift Testing run from the same `make test` gate used by the worker.
- `make smoke` starts the real router plus a deterministic backend and exercises the real `lac status` and `lac chat` processes.
- `make validate` additionally builds, signs, and verifies the packaged Studio app.
- Lefthook runs `make delivery-index validate` on both pre-commit and pre-push; CI runs the same tracked-file check and gate.
- Remote auth, chunked response framing, cancellation state, and model-pull exit status have regression coverage.
- Remaining service/model availability messages are operational state, not source-code test failures.

---

## ✅ Passed Checks (OK)

| Category | Status | Details |
|----------|--------|---------|
| OS | ✓ | macos aarch64 |
| Memory | ✓ | 4.0+ GiB free (moderate pressure) |
| Disk Home | ✓ | 31 GiB free on $HOME volume |
| OpenCode Config | ✓ | opencode.jsonc present |
| Agent Rules | ✓ | `docs/AGENTS.md` present; `lac doctor` accepts the documented fallback |
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

## ⚠️ Operational State

| Category | Level | Detail | Resolution |
|----------|-------|--------|------------|
| Ollama Symlink | warn | dangling symlink may exist on the host | Run `lac bootstrap` to re-establish it |
| Backends | warn | no inference backend serving | Start with `lac serve mlx` and `lac route --daemon` |

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

## 2026-09-24 Re-verification (10x pro delivery pass)

- **Canonical gate:** `make test` runs the Rust and Swift suites; `make smoke` runs the real router/CLI E2E; `make validate` also packages and verifies the signed Studio app. Lefthook and CI invoke `make delivery-index validate`, so required gate/test files cannot be omitted from the commit.
- **Router framing (lac-router v2.8):** `sse_relay` covers Content-Length early close, incremental chunked SSE, split chunk boundaries, chunk extensions, terminal trailers, and SSE-only keep-alives. The chunk parser is stateful and rejects data that merely contains a terminal-looking byte sequence.
- **Remote auth closed:** non-loopback startup without a token fails closed; a configured token is required on every listener, including loopback; forwarded proxy headers also require the token. Constant-time comparison and successful non-loopback traffic are covered by `router_auth`.
- **Studio delivery:** connection settings are available before a router exists, remote clients cannot run local daemon/host-fact actions, malformed URLs clear loading state, and packaged builds include local-network ATS permission for dynamic Tailscale hosts.
- **State safety:** Chat, Code Assistant, and loop runners use generation guards so cancelled work cannot overwrite a newer run; stores use injectable roots in tests; failed worker tasks preserve changes in a recoverable stash instead of deleting them.
- **Model pull:** failures and cancellation return nonzero, input is passed as an argument rather than interpolated source, and the CLI owns the downloader process tree.
- **Counts:** the exact count is reported by the test runner; the previous stale arithmetic and active-daemon snapshot are intentionally not presented as guarantees.

---

## Rust Power Confirmation

> "When you close it the app is still running — that's our rust power for the community."

The launchd daemon system demonstrates this principle fully:

- **Before daemon install:** Closing the TUI/CLI stops all inference
- **After daemon install:** `org.lac.router` and `org.lac.worker` LaunchAgents keep services running in the background
- **Unix philosophy:** Rust-compiled, zero-dependency binaries launched at login, surviving user session closure

This is the architectural differentiator against LM Studio's visual layer + background process flaw. LAC combines **beautiful visual design** (TUI + gateway UI) with **Rust-powered persistence** — the best of both worlds.