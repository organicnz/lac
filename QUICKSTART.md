# LAC — Quick Start (Rust-First, Qwen3.8-27B Primary)

**All primary tools are the Rust binaries built in `rust-src/target/release/`.**
Standalone Rust launchers under `scripts/*.rs` (build with `make scripts`,
no dependencies, plain `rustc`) are **reference fallbacks** — verified
single-file programs for environments where the full suite isn't built.
There is no bash anywhere in the operational path.

The primary model throughout this build is **`qwen3.8-27b`** — the Qwen3.8-27B dense hybrid
VLM released Aug 14 2026 (27.78B params, 64 layers, 262K native / 1M YaRN, MTP head).
It is the better model per your preference, replacing the older `qwen3-coder`.

## 1. Initial arrival / day-one setup

```bash
# Clone & enter
git clone <repo-url> ~/dev/work/AI/LAC
cd ~/dev/work/AI/LAC

# Option A — Unified CLI (fastest & recommended)
./rust-src/target/release/lac doctor     # Verify stack integrity
./rust-src/target/release/lac status     # Real-time telemetry

# Option B — LAC Studio (Native Apple Silicon macOS Liquid Glass Desktop App)
./SwiftUI/package-app.sh --open          # Or run: lac visualize
# Features Claude/Codex ergonomics, Code Assistant split canvas (⌘2), live Hugging Face Model Hub, and Ops telemetry

# Option C — Code Assistant from CLI (Instant single-shot or piped refactoring)
lac code -f src/main.rs "Review for thread-safety and zero-allocation performance"
cat src/lib.rs | lac code "Generate comprehensive unit tests"

# Option C — TUI (interactive terminal dashboard)
./rust-src/target/release/lac-tui
# Menu: 1) health check  2) serve menu  3) smart serve  4) skills  5) pull models  6) preflight  7) bootstrap

# Option D — headless bootstrap (one-time, 100% pure Rust)
./rust-src/target/release/bootstrap
# This does (Rust-first, idempotent):
#   • Ensures Homebrew + brew bundle
#   • Installs Rust via rustup (stable) — NOT brew rust
#   • Installs uv, python 3.14, node 24 via mise
#   • Creates /Volumes/AIModels layout (APFS encrypted, GUID)
#   • Symlinks ~/.ollama/models -> /Volumes/AIModels/ollama
#   • Auto-installs Kanban loop templates into ~/todo/lac-loops/
#   • Appends zprofile hooks (HF_HOME, ollama, TM exclusions)
```

## 2. Pull Qwen3.8-27B model into Ollama (Rust binary)

```bash
# The single source of truth: pull qwen3.8-27b (the preferred model)
./rust-src/target/release/pull-models
```

This pulls (and only these — focused on your preferred model):
- `qwen3.8-27b` — the primary model (16.1GB Q4, 32K ctx, 27.78B dense hybrid VLM)

Models land in `/Volumes/AIModels/ollama` (symlinked from `~/.ollama/models` via
bootstrap's Rust `std::os::unix::fs::symlink`).

> **Note**: `qwen3-coder` is intentionally omitted from the pull list per your preference
> for `qwen3.8-27B`. The older model is not needed — qwen3.8-27b covers both coding and
> quality analysis use-cases.

## 3. Start the inference server (Rust binary — preferred)

### MLX (fastest decode on Apple Silicon)

```bash
# Rust binary — preferred method
./rust-src/target/release/serve-mlx

# Or specify the primary model:
./rust-src/target/release/serve-mlx ollama/qwen3.8-27b
```

Server runs on `http://127.0.0.1:8080/v1`. Press Ctrl+C to stop.

> The standalone equivalent `scripts/serve-mlx.rs` (build: `make scripts`)
> exists only as a reference fallback. Prefer the suite binary for new setups.

### llama.cpp (long-ctx / speculative decoding / Q8_0 grammar-constrained)

```bash
# Rust binary — preferred method, Q8_0 for quality
./rust-src/target/release/serve-llama unsloth/Qwen3.6-27B-MTP-GGUF:Q8_0 16384
# Or via lac CLI:
./rust-src/target/release/lac serve llama
```

Server runs on `http://127.0.0.1:8081/v1`. Press Ctrl+C to stop.

### Unified Gateway (lac-router on :8000)

```bash
# Start the unified intelligent router (routes to MLX :8080 or llama :8081 automatically)
./rust-src/target/release/lac route --daemon
```

Endpoint runs on `http://127.0.0.1:8000/v1`.

---

## 4. Launch OpenCode with the primary model

```bash
# Using OpenCode V2 (opencode2). Config in opencode.jsonc points at
# the unified gateway on :8000 with automatic backend failover.

opencode2 run "List the files in the project and summarize the codebase structure"
```

> The `opencode.jsonc` config in the project root points at `lac/qwen3.8-27b` 
> via the unified gateway at `http://127.0.0.1:8000/v1`.

## 5. Useful skills (ask by ID in a session, or via the TUI command catalog)

| Skill | ID | What it does |
|---|---|---|
| Local Verify | `local-verify` | Checks model stack health & responsiveness |
| Model Swap | `model-swap` | Toggle between Qwen3.8-27B variants / backends (Q4↔Q8) |
| KV Cache Manage | `kv-cache-manage` | Emergency truncate before OOM; archive + summarize |
| Context Cap | `context-cap` | Steady-state 16K hygiene; complements kv-cache-manage |
| Agent Resume | `agent-resume` | Persist state to agent-state.json; resume after crash |
| Thermal Monitor | `thermal-monitor` | Watch Mac temps; auto-throttle Q8→Q4 on Serious |
| Loop Orchestrator | `loop-orchestrator` | implement→review→apply, max 2 rounds, human gate |
| Task Runner | `task-runner` | Drain ~/todo/lac-tasks.yaml queue, max N tasks |
| Permissions Trust | `permissions-trust` | Trusted allow/deny rules so batches don't stall on prompts |
| Knowledge Recall | `knowledge-recall` | Recall indexed code + lessons before task; record after |
| Index Embeddings | `index-embeddings` | Build local vector index of codebase for semantic search |
| Worktree Fanout | `worktree-fanout` | Create jj/colocated git worktrees for parallel features |
| Auditor | `auditor` | Static code audit → `AUDIT_REPORT.md` |

### Example skill invocations (Ruth-first, qwen3.8-27b primary)

```bash
# Verify the stack is healthy
opencode2 run 'Use the local-verify skill'

# The model-swap skill now toggles Qwen3.8-27B variants:
opencode2 run 'Use the model-swap skill'

# Build semantic search index of the codebase
opencode2 run 'Use the index-embeddings skill'

# Create a new feature worktree
opencode2 run 'Use the worktree-fanout skill --feature auth'

# Run a code audit
opencode2 run 'Use the auditor skill'
```

## 5b. LAC Studio Native macOS Desktop App

Launch the native Apple Silicon desktop client with Apple Liquid Glass aesthetics, Claude Desktop & Codex ergonomics, split Code Assistant (`⌘2`), and Hugging Face Model Hub discovery:

```bash
# Launch LAC Studio (builds and packages on first run)
lac studio
# or
make studio

# Package as signed macOS application (.build/LAC Studio.app)
cd SwiftUI && ./package-app.sh --open
```

**Key Features:**
- **Executive Liquid Glass Aesthetic**: VisionOS-grade specular highlights, ambient sheen, light diffraction, and tactile haptic feedback.
- **Claude & Codex Ergonomics**: 780pt centered golden-ratio column, collapsible sidebar (`⌘B`), `+ New Chat` (`⌘N`), search filter across threads grouped by date ("Today", "Yesterday", "Previous 7 Days", "Older"), and floating elevated composer.
- **Code Assistant (`⌘2`)**: Split coding canvas with multi-file code editor, fast actions (`Explain`, `Refactor`, `Generate Tests`, `Audit Bugs`, `Optimize`), and 1-click diff application directly into the workbench.
- **LM Studio Hugging Face Hub (`⌘4`)**: Discover top models filtered by Apple Silicon, MLX, GGUF, Coding, Reasoning, and Multimodal/Vision; parameter size filters (≤8B, 14B–32B, 70B+); RAM fit estimations; and 1-click pull.

## 6. Daily workflow (example — Rust-first, qwen3.8-27B primary)

```bash
# Morning: TUI health check + start server
./rust-src/target/release/lac-tui   # 1) health check, then 2) serve Q4
# Or headless:
./rust-src/target/release/serve-mlx ollama/qwen3.8-27b &

# Set steady-state hygiene for the session
opencode2 run 'Use the context-cap skill --set --max 16000'
opencode2 run 'Use the agent-resume skill --auto --interval 300'

# Normal work
opencode2 run --model ollama/qwen3.8-27b "Review PR #42 changes"

# Quality work: swap to Q8, truncate first for clean context
opencode2 run 'Use the kv-cache-manage skill --truncate --keep 8000'
opencode2 run 'Use the model-swap skill'   # Q4 -> Q8
opencode2 run "Refactor auth module for deep security analysis"
opencode2 run 'Use the model-swap skill'   # Q8 -> Q4 (back to speed)

# End of day: index + save state
opencode2 run 'Use the index-embeddings skill'
opencode2 run 'Use the agent-resume skill --save --task "checkpoint end of day"'
```

## 6b. Non-stop operation (8hr+ autonomous loops)

```bash
# Pre-flight (every long run)
opencode2 run 'Use the thermal-monitor skill --check'
opencode2 run 'Use the context-cap skill --set --max 16000'
opencode2 run 'Use the agent-resume skill --save --task "<task description>"'

# Run structured loop (max 2 rounds, human gate, never auto-commits)
opencode2 run 'Use the loop-orchestrator skill --run ~/todo/lac-loops/<task>.yaml'

# Or drain a refined task queue (max N, failure-isolated)
opencode2 run 'Use the task-runner skill --drain --max 3'

# If server OOMs mid-loop: restart via TUI serve menu, then
opencode2 run 'Use the agent-resume skill --resume'
```

## 7. When you're done / shutdown

```bash
# Kill any running servers (if not backgrounded with &)
# Or press Ctrl+C in the foreground terminal

# Rust binaries are the canonical way to start/stop servers.
# The standalone scripts/ launchers are reference fallbacks — do not rely on them.

# Optional: export models to Thunderbolt external for archival
# (weights already loaded into unified memory; external is cold store only)

# Time Machine already excludes /Volumes/AIModels
# Keep FileVault on; keep ambient temp 18-24°C for sustained inference
```

---

## 8. Terminal note (WARP + Ghostty)

Your current base terminal is **WARP** (Rust-based, AGPL-3.0 since April 2026).

- **WARP** is kept installed for cloud frontier work, Warp Agent, and mixed cloud+local orchestration.
- **Ghostty 1.3.1** is recommended for daily local Qwen3.8-27B coding:
  - No tunnel required for `http://127.0.0.1:11434/v1` or `:8080`
  - Fastest Mac feel 2026, native Metal, zero-config
  - Rust binaries in this repo work identically from either terminal

> You can keep WARP and also install Ghostty. Launch Ghostty alongside WARP for
> local-only qwen3.8-27B sessions. The only difference: WARP proxies `localhost`
> (tunnel needed), Ghostty passes `localhost` directly (private, lower latency).

> No need to switch exclusively unless you want a cleaner local-only setup.

---

## 9. Project structure (key files — Rust-first, qwen3.8-27B primary)

```
LAC/
├─ Rust binaries (primary):
│  ├─ rust-src/target/release/lac             # Unified Swiss-army-knife CLI
│  ├─ rust-src/target/release/lac-router      # Intelligent reverse proxy on :8000
│  ├─ rust-src/target/release/lac-tui         # Full-featured TUI dashboard
│  ├─ rust-src/target/release/bootstrap       # Idempotent machine setup
│  ├─ rust-src/target/release/serve-mlx       # mlx-lm server :8080 (native MTP)
│  ├─ rust-src/target/release/serve-llama     # llama-server :8081 (Q8_0 quality)
│  ├─ rust-src/target/release/pull-models     # pull Qwen3.8-27B + MLX cache dirs
│  └─ rust-src/target/release/kv-manage       # KV check/truncate (GREEN/YELLOW/RED)
│
├─ OpenCode config (single source of truth):
│  ├─ opencode.jsonc                          # gateway :8000, 64K context, 14 skills, qwen3.8-27b
│  └─ .env.example                            # env vars & port definitions
│
├─ Templates (Production Kanban & Loops):
│  ├─ templates/tasks/lac-tasks.yaml          # Task queue template
│  ├─ templates/loops/daily-coding.yaml       # Fast implement-review-apply-gate
│  ├─ templates/loops/bug-fix.yaml            # Reproduce-isolate-fix-verify
│  ├─ templates/loops/feature-branch.yaml     # Worktree multi-file feature loop
│  └─ templates/loops/security-audit.yaml     # Deep static audit & remediation
│
├─ Skills (OpenCode V2, model-facing — 14 total):
│  ├─ .opencode/skills/hermes-orchestrator/   # high-order autonomous driver
│  ├─ .opencode/skills/loop-orchestrator/     # implement→review→apply, 2 rounds
│  ├─ .opencode/skills/task-runner/           # drain task queue
│  ├─ .opencode/skills/local-verify/          # stack health
│  ├─ .opencode/skills/model-swap/            # Q4↔Q8 toggle
│  ├─ .opencode/skills/kv-cache-manage/       # emergency OOM truncate
│  ├─ .opencode/skills/context-cap/           # steady-state 16K hygiene
│  ├─ .opencode/skills/agent-resume/          # persist + resume state
│  ├─ .opencode/skills/thermal-monitor/       # temp watch
│  ├─ .opencode/skills/permissions-trust/     # trusted rules, no mid-loop prompts
│  ├─ .opencode/skills/knowledge-recall/      # recall index + lessons pre-task
│  ├─ .opencode/skills/index-embeddings/      # vector index
│  ├─ .opencode/skills/worktree-fanout/       # jj worktrees
│  └─ .opencode/skills/auditor/               # code audit
│
├─ launchd (non-stop server survival):
│  ├─ launchd/org.lac.router.plist            # KeepAlive gateway :8000
│  └─ launchd/org.lac.serve-mlx.plist         # KeepAlive serve-mlx
│
├─ Documentation:
│  ├─ README.md                               # High-level overview
│  ├─ QUICKSTART.md                           # Quick start guide
│  ├─ docs/ARCHITECTURE.md                    # memory budget & 4-layer stack
│  ├─ docs/MTP_TUNING.md                      # Speculative decoding optimization
│  ├─ docs/LOOPS_AND_KANBAN.md                # Non-stop autonomous loops guide
│  └─ TERMINALS.md                            # Warp vs Ghostty decision guide
│
└─ scripts/                                   # ← standalone Rust launchers (make scripts)


> **Rule**: If it has `rust-src/target/release/` binary → use it.
> If it lives under `scripts/` → it's a reference fallback; the suite binary is authoritative.
> The primary model is **qwen3.8-27B** — the better model per your preference.

---

## 10. Troubleshooting (Rust-first, qwen3.8-27B focus)

| Symptom | Fix (Rust binary or config) |
|---|---|
| `ollama pull` stalls | Ensure `brew services start ollama`; check `pgrep ollama` |
| `./rust-src/target/release/serve-mlx` crash | Verify `pip install -U mlx-lm mlx-vlm`; model tag must match HF hub |
| OpenCode can't reach model | Check `opencode.jsonc` provider `baseURL`; ensure server is running |
| `num_ctx` errors | Adjust `OLLAMA_CONTEXT_LENGTH` or server `--max-model-len` |
| Time Machine backup stalls | `tmutil addexclusion /Volumes/AIModels` already applied by bootstrap |
| Rust build fails | `rustup update stable`; ensure Xcode CLT: `xcode-select --install` |
| Want to use qwen3-coder instead | Don't. Per your preference, qwen3.8-27b is the primary model. |
| Quality vs speed tradeoff | Qwen3.8-27B Q4 (16.1GB) is the default — use model-swap skill to toggle backends |
| Q8 for even better quality | `./rust-src/target/release/serve-llama ...:Q8_0` for Q8 via llama-server |