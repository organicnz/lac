# LAC — Local Agentic Coding (Apple Silicon Macs)

Target: **Apple Silicon Macs** (8GB+ supported; best on high-RAM Studio + TB5 external).  
Primary Model: **Qwen 3.8 27B** dense hybrid VLM with native MTP speculative decoding (~45 tok/s).  
Harness: **OpenCode V2** (@coder, @reviewer, @orchestrator).  
Orchestrator: **Hermes** + Kanban Loops (`~/todo/lac-loops/`).  
Gateway: **`lac-router` on `:8000`** (auto-routes MLX `:8080`, llama-server `:8081`, Ollama `:11434`).

---

## 10x Pro Quickstart

```bash
# 1. System diagnostics & status
./rust-src/target/release/lac doctor
./rust-src/target/release/lac status

# 2. Start the unified gateway on :8000
./rust-src/target/release/lac route --daemon

# 3. Start high-speed inference (MLX with native MTP)
./rust-src/target/release/lac serve mlx

# 4. Or launch the interactive Terminal UI (TUI)
./rust-src/target/release/lac tui

# 5. Run OpenCode against the unified local gateway
opencode2 run "Verify stack: list files and check git status"
```

### Remote via Tailscale (no public ports)
```bash
# On the Mac: join tailnet, set token, keep router on loopback, front it with TLS
brew install tailscale && sudo tailscaled && tailscale up
export LAC_API_TOKEN="$(openssl rand -hex 32)"
launchctl setenv LAC_API_TOKEN "$LAC_API_TOKEN"
tailscale serve --https 443 http://127.0.0.1:8000
# On any laptop: same tailnet, Studio → Ops → Gateway Connection →
# host mac.tailXXX.ts.net:443 + enable TLS + paste token, Test.
./rust-src/target/release/lac config   # bind + auth (redacted)
./rust-src/target/release/lac doctor   # remote_auth check
```

---

## Architecture & Layout

- `SwiftUI/` — **LAC Studio** native macOS desktop app (Apple Silicon native):
  - **Apple Liquid Glass**: Optically accurate specular refraction borders, ambient light sheen, dual-layer visionOS depth shadows, 120Hz ProMotion interpolating springs, and trackpad tactile haptics.
  - **Claude & Codex Ergonomics**: 780pt centered golden-ratio reading column, sparkle monogram avatar, syntax-highlighted code containers with animated copy buttons, collapsible sidebar (`⌘B`), `+ New Chat` (`⌘N`), and floating elevated bottom composer.
  - **Code Assistant (`⌘2`)**: Split coding canvas with multi-language editor on the left and agentic assistant on the right. Quick actions for `Explain`, `Refactor`, `Generate Tests`, `Audit Bugs`, and `Optimize`, with 1-click apply and undo.
  - **LM Studio Hugging Face Hub**: Live debounced search across 100,000+ HF models, Apple Silicon / MLX / GGUF filter capsules, unified memory RAM fit estimation, and 1-click model switching.
  - **Live Ops Dashboard**: Real-time daemon monitoring, one-click backend switches, port health telemetry, and self-healing error recovery.
- `rust-src/` — **100% Pure Native Rust Backend** (std library only, zero external crates, zero Python/shell scripts in the backend path):
  - `lac` — Swiss-army-knife CLI (`status`, `serve`, `route`, `code`, `doctor`, `loop`, `bench`, `kv`)
  - `lac-router` — Unbreakable intelligent reverse proxy on `:8000` (SIGPIPE-immune, thread panic-isolated, auto-routes MLX `:8080`, llama `:8081`, Ollama `:11434`)
  - `lac-tui` — Full-featured interactive ANSI terminal control center
  - `bootstrap` — Idempotent machine setup and volume provisioning
  - `serve-mlx` — MLX-LM server launcher with port drift detection and MTP on `:8080`
  - `serve-llama` — llama-server with Flash Attention & Q8_0 on `:8081`
  - `pull-models` — Automated Hugging Face model puller with RAM fit gating
  - `kv-manage` — Context cache risk analyzer & truncation engine
- `opencode.jsonc` — OpenCode V2 project config (gateway `:8000`, 64K context, 14 skills, qwen3.8-27b primary)
- `templates/` — Production Kanban queue (`lac-tasks.yaml`) & loops (`daily-coding`, `bug-fix`, `feature-branch`, `security-audit`)
- `.opencode/skills/` — 14 skills including `hermes-orchestrator`, `loop-orchestrator`, `kv-cache-manage`, `thermal-monitor`
- `launchd/` — KeepAlive daemons for `lac-router` (`org.lac.router.plist`) and `serve-mlx` (`org.lac.serve-mlx.plist`)
- `docs/` — Technical blueprints (`ARCHITECTURE.md`, `MTP_TUNING.md`, `LOOPS_AND_KANBAN.md`)

---

## Documentation

- [Operating Rules](docs/AGENTS.md)
- [Quickstart Guide](docs/QUICKSTART.md)
- [Architecture Blueprint](docs/ARCHITECTURE.md)
- [Speculative Decoding & MTP](docs/MTP_TUNING.md)
- [Kanban & Autonomous Loops](docs/LOOPS_AND_KANBAN.md)
- [Terminal Selection Guide](docs/TERMINALS.md)
