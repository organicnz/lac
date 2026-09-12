# LAC — Local Agentic Coding (M5 Ultra 96GB)

Target: **Mac Studio M5 Ultra 30c/64c 96GB + 1TB internal + TB5 external**.  
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

---

## Architecture & Layout

- `opencode.jsonc` — OpenCode V2 project config (gateway `:8000`, 64K context, 14 skills, qwen3.8-27b primary)
- `rust-src/` — Native Rust toolsuite (compiled with `cargo build --release`):
  - `lac` — Swiss-army-knife CLI (`status`, `serve`, `route`, `doctor`, `loop`, `bench`, `kv`)
  - `lac-router` — Intelligent reverse proxy on `:8000` (zero config churn for OpenCode)
  - `lac-tui` — Full-featured interactive ANSI terminal control center
  - `bootstrap` — Idempotent machine setup and volume provisioning
  - `serve-mlx` — MLX-LM server with MTP speculative decoding on `:8080`
  - `serve-llama` — llama-server with Flash Attention & Q8_0 on `:8081`
  - `kv-manage` — Context cache risk analyzer & truncation engine
- `templates/` — Production Kanban queue (`lac-tasks.yaml`) & loops (`daily-coding`, `bug-fix`, `feature-branch`, `security-audit`)
- `.opencode/skills/` — 14 skills including `hermes-orchestrator`, `loop-orchestrator`, `kv-cache-manage`, `thermal-monitor`
- `launchd/` — KeepAlive daemons for `lac-router` (`org.lac.router.plist`) and `serve-mlx` (`org.lac.serve-mlx.plist`)
- `docs/` — Technical blueprints (`ARCHITECTURE.md`, `MTP_TUNING.md`, `LOOPS_AND_KANBAN.md`)

---

## Documentation

- [Operating Rules (AGENTS.md)](file:///Users/organic/dev/work/AI/LAC/AGENTS.md)
- [Quickstart Guide (QUICKSTART.md)](file:///Users/organic/dev/work/AI/LAC/QUICKSTART.md)
- [Architecture Blueprint (docs/ARCHITECTURE.md)](file:///Users/organic/dev/work/AI/LAC/docs/ARCHITECTURE.md)
- [Speculative Decoding & MTP (docs/MTP_TUNING.md)](file:///Users/organic/dev/work/AI/LAC/docs/MTP_TUNING.md)
- [Kanban & Autonomous Loops (docs/LOOPS_AND_KANBAN.md)](file:///Users/organic/dev/work/AI/LAC/docs/LOOPS_AND_KANBAN.md)
- [Terminal Selection Guide (TERMINALS.md)](file:///Users/organic/dev/work/AI/LAC/TERMINALS.md)
