# Terminals — WARP vs Ghostty vs Others for LAC on M5 Ultra

Your current base: **WARP** (Rust-based, AGPL-3.0 since April 2026).

This doc contrasts WARP against alternatives for the specific use case of
local Qwen 27–32B agentic coding on Mac Studio M5 Ultra 96GB.

## 1. WARP 2.0 (your current terminal)

| Aspect | Fact |
|---|---|
| Stack | Rust 98.1% + custom GPU UI framework + block model. Open-sourced AGPL-3.0 2026-04-28. |
| Version | `v0.2026.06.08.09.48.dev_00` (current channel), stable `v0.2026.04.29.08.56.stable_00`. |
| Agentic features | Universal input, multi-agent threading, Code/Agents/Terminal/Drive. Warp Agent CLI supports BYO inference (OpenAI-compat `baseURL`). |
| Local-model support | **Proxied only.** Warp backend calls your endpoint → streams back. `localhost/127.0.0.1` **rejected** without tunnel (ngrok/Cloudflare/Tailscale). No pure-local mode as of Sep 2026. |
| Pros | Best parallel ADE, block UI, frontier models (Fable/Opus/GPT-5.6 via Warp), now open-source AGPL. |
| Cons | Server-side proxy defeats local privacy/latency purpose; tunnel required for local LLMs; auto-routers ignore private IPs. |
| Best for | Cloud frontier work, mixed cloud+local, Warp Agent workflow, teams using Warp's orchestration platform. |
| Verdict for this build | **Keep installed** for cloud work. Do **not** use for pure local Qwen 27-32B inference. |

Sources: `warp.dev/blog/reimagining-coding-agentic-development-environment`, `warp.dev/blog/warp-is-now-open-source`, `warp.dev/blog/block-model-behind-warps-agentic-development-environment`, `docs.warp.dev/agents/inference/custom-inference-endpoint`, `github.com/warpdotdev/warp/discussions/9619`.

---

## 2. Ghostty 1.3.1 — recommended for local LAC

| Aspect | Fact |
|---|---|
| Stack | Zig + Metal, native AppKit UI. ~60.7k stars, ~1M dl/wk. |
| Version | **1.3.1** (Mar 2026), **1.4** due Sep 2026 tip. macOS 13+ (last 13 release). |
| Agentic integration | **No agentic built-in** by design — pure terminal emulator. Agent comes from OpenCode/Aider/Claude Code running *inside* Ghostty. |
| Local-model support | **Yes — direct `localhost`**. No tunnel required. Ghostty + OpenCode/Aider/Claude Code → `http://127.0.0.1:11434/v1` works out of the box. |
| Pros | Fastest Mac feel 2026, zero-config, native scrollbars, AppleScript preview, 120Hz ProMotion, MIT license, no subscription, privacy-respecting. |
| Cons | No native agentic features; agent is external (OpenCode/Claude Code). |
| Verdict | **Best for local Qwen 27-32B on M5 Ultra.** Pair with OpenCode + Ollama/`mlx_lm.server`. |

Sources: `ghostty.org/docs/install/release-notes/1-3-0`, `ghostty.org/download`, `releasealert.dev Ghostty Sep 2026 tip`.

---

## 3. Other terminals — how they compare for local LLMs

| Terminal | Stack / Version | Native local LLM support? | Verdict for M5 Ultra + Qwen 27-32B |
|---|---|---|---|
| **Alacritty** | Rust v0.17.0 Apr 2026 / OpenGL | Yes via OpenCode/Aider — same `baseURL` story. | No advantage over Ghostty. No tabs/splits; pair with tmux/zellij. |
| **WezTerm** | Rust nightly `20260824-f93d90` / Lua config | Yes via OpenCode/Aider. | Hard to recommend: stable gap (Feb 2024 → Sep 2026), 1.4k open issues. |
| **Kitty** | C v0.48.2 Jul 2026 / GPU-accel | Yes via OpenCode/Aider. | No Rust, less macOS-native in 2026. Pick if you need built-in mux. |
| **Zed** | Rust v1.18.1 Sep 2026 / GPU | **Yes — built-in agent panel + terminal threads.** | Best if you want editor+terminal+agent in one. Not a pure terminal replacement. |
| **Wave** | Go+Electron/React v0.14.2-beta.1 Mar 2026 | **Yes direct** `aibaseurl`+`aimodel`, any `/v1/chat/completions`. | Electron heavier; assistant-only (not autonomous coder). Good if you want integrated AI-terminal. |
| **warpterm** (WARP legacy) | — | Same as current WARP — proxy required. | — |

Sources: `github.com/kovidgoyal/kitty/releases v0.48.2`, `github.com/wezterm/wezterm/releases 20260824`, `zed.dev/docs/ai/use-a-local-model`, `github.com/wavetermdev/waveterm/releases`, `docs.waveterm.dev`, `waveai.dev`.

---

## 4. Verdict — which terminal for this build?

| Goal | Recommended terminal |
|---|---|
| **Local Qwen 27-32B agentic coding (this build)** | **Ghostty 1.3.1** + OpenCode inside it |
| **Cloud + local mixed work** | **WARP 2.0** (keep installed; use Ghostty for local-only sessions) |
| **All-in-one editor+terminal+agent** | **Zed** (Rust+GPU, built-in agent, local model support) |
| **Integrated AI-terminal zero-config** | **Wave** (direct local, no account, but assistant-only) |
| **Terminal-only, max performance, no compromise** | **Ghostty** |

### How to switch (if you want Ghostty):

```bash
# Install
brew install --cask ghostty

# Make it default (optional)
open -a Ghostty

# The LAC repo ships opencode.jsonc + skills + Brewfile — just run:
./rust-src/target/release/bootstrap  # ensures HF_HOME, ollama symlink, zprofile hooks

# Then in Ghostty:
opencode2 run --model ollama/qwen3.8-27b "verify stack"

# No tunnel needed. Direct localhost:11434 (Ollama) or :8080 (mlx/llama).
```

### Keeping WARP + adding Ghostty for local work:

You can keep WARP as your primary terminal and launch Ghostty alongside for local LLM sessions:

```bash
# In WARP terminal:
open -a Ghostty  # or `ghostty` if in $PATH

# Then in Ghostty:
opencode2 run --model ollama/qwen3.8-27b "..."
```

No config conflict — each terminal has its own isolation. WARP for cloud/orchestration, Ghostty for private local inference.

---

## 5. Terminal cheat-sheet for LAC

| Action | WARP | Ghostty |
|---|---|---|
| Start OpenCode | `opencode2` | `opencode2` |
| Set model for session | `/models` → `ollama/qwen3.8-27b` | `/models` → `ollama/qwen3.8-27b` |
| Toggle model (skill) | ask for `model-swap` | ask for `model-swap` |
| Verify stack health | ask for `local-verify` | ask for `local-verify` |
| Index code embeddings | ask for `index-embeddings` | ask for `index-embeddings` |
| Create git worktree | ask for `worktree-fanout` | ask for `worktree-fanout` |
| Audit code | ask for `auditor` | ask for `auditor` |

Both terminals pass the same OpenCode commands identically. The difference is:
- **WARP**: backend proxies `localhost` → requires tunnel for local LLMs
- **Ghostty**: passes `localhost` directly → no tunnel, lower latency, private

---

## 6. Recommendation for your M5 Ultra build

1. **Keep WARP installed** — it's the best terminal for cloud frontier work, Warp Agent, and mixed cloud+local orchestration. The AGPL open-source release means you already have it.

2. **Also install Ghostty** — use it for your daily local Qwen 27-32B agentic coding. It's faster, more macOS-native in 2026, and the critical difference: **no tunnel required** for `http://127.0.0.1:11434/v1` or `:8080`.

3. **Launch pattern**: Keep WARP as your terminal baseline. When you need local agentic coding, open Ghostty (or split if you prefer) and run `opencode2` there. When you need cloud models, Warp, or Warp Agent, stay in WARP.

4. **No need to choose one exclusively** — the Rust binaries in this repo (`./rust-src/target/release/*) work identically from either terminal. The only difference is the `baseURL` reachability.

---

**Bottom line**: For this specific build (Mac Studio M5 Ultra 96GB + Qwen3 27-32B local), Ghostty + OpenCode gives you the most capable, private, lowest-latency local agentic coding experience. WARP stays for cloud/orchestration work. Both can coexist on your M5 Ultra.