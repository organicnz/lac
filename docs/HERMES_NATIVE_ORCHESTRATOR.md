# Native Orchestrator — Replacing the Hermes Stub
### Companion to STUDIO_PARITY_ROADMAP.md — scoped to one component

> Status (implemented): the stub described below is gone. `hermes_dir()` and the Python shim were deleted; `lac hermes status` reports native state and `lac hermes run` delegates to `cmd_worker`, which selects tasks via `judge_next()` (LLM judgment with deterministic file-order fallback). See `docs/ARCHITECTURE.md` §5 and `.opencode/skills/hermes-orchestrator/SKILL.md` for the as-built description. This file is retained as the historical plan record.

**Finding this plan is built on:** `lac hermes run` (rust-src/src/lac.rs, `cmd_hermes`/`hermes_dir`) is a stub. It searches for an external `hermes-agent` Python checkout and, if found, runs `python3 cli.py --help` against it — nothing more. It is not installed via `Brewfile` or `mise.toml` and is not part of the actual environment. There is no real dependency to migrate away from; this is a build-it-properly task, not a migration.

**What "Hermes" means today, concretely:**
- A documented role (`.opencode/skills/hermes-orchestrator/SKILL.md`) that Qwen3.8 reads and acts on when OpenCode invokes it — the model performs the judgment, guided by the skill file.
- Deterministic plumbing already implemented natively: `loop-orchestrator`, `task-runner`, `permissions-trust`, `agent-resume`, `worktree-fanout`, `thermal-monitor`, `kv-cache-manage`, `context-cap`.
- A CLI stub (`cmd_hermes`) that does nothing real.

Ground Rules from the parity roadmap apply unchanged here (zero external crates, tests-as-gate, never auto-commit, one worktree per phase).

---

## Design

Two layers, cleanly separated:

**Judgment** — the only part that needs actual reasoning. Reuses primitives that already exist in `lac.rs`:
- `post_chat()` — already used by `cmd_chat`/`cmd_code` to hit `http://127.0.0.1:8000/v1`. Reuse directly; no new HTTP code needed.
- `extract_json_string()` — already used to pull fields out of chat responses. Reusable as-is if the decision format below stays simple enough for it, or extend minimally.

**Decision format:** since there's no JSON crate, don't ask the model for JSON — ask for a strict line-based format that's trivial to hand-parse and hard for the model to get subtly wrong:
```
ACTION: dispatch_loop | escalate_hosted | wait | skip_task
LOOP: bug-fix | feature-branch | daily-coding | security-audit
TASK_ID: <id from lac-tasks.yaml>
REASON: <one line>
```
Parse with a simple line-prefix scan (`line.strip_prefix("ACTION: ")`, etc.) — same style already used elsewhere in `lac.rs`, no new parsing abstraction needed.

**Dispatch** — already built. Once Judgment decides `ACTION: dispatch_loop`, the orchestrator shells out exactly the way `AGENTS.md` already documents manual invocation: `opencode2 run 'Use the loop-orchestrator skill ...'`. No new dispatch logic — just call the existing path programmatically instead of by hand.

**Hygiene gates** — already built. Before each Judgment call, the loop checks `thermal-monitor`, `kv-cache-manage`, and `context-cap` the same way `AGENTS.md`'s session-hygiene section already mandates. This is the one place the stub-replacement adds real value beyond "connect existing pieces" — today nothing *automatically* sequences these checks; a human (or the model, if reminded) has to remember to invoke them.

---

## Phase 1 — Remove the dead stub

- Delete `hermes_dir()` and the Python-shim branch of `cmd_hermes`.
- Keep the `lac hermes` command name (it's used throughout `AGENTS.md`, the skill file, and your docs — renaming costs documentation churn for no benefit; the fix is making the command real, not renaming it).

**Definition of Done:** `lac hermes status` no longer references an external Python path; `cargo test` still passes.

---

## Phase 2 — Judgment loop

- New function `fn judge(state: &OrchestratorState) -> Decision` in `lac.rs` (or a new `orchestrate.rs` module if `lac.rs`'s 2,767 lines are getting unwieldy — reasonable point to split).
- `OrchestratorState` assembled from: `~/todo/lac-tasks.yaml` contents, active worktrees (reuse whatever `worktree-fanout` already tracks), and current thermal/KV state (reuse existing check functions).
- Builds a compact prompt from that state, calls `post_chat()`, parses the line-based `Decision` format above.

**Definition of Done:** given a fixture `lac-tasks.yaml` with 3 pending tasks and one overheating-thermal state, `judge()` returns `ACTION: wait` — a test that the hygiene state actually influences the decision, not just the task queue.

---

## Phase 3 — Dispatch wiring

- `fn dispatch(decision: Decision) -> io::Result<()>` — turns a `Decision::DispatchLoop` into the equivalent of the manual `opencode2 run 'Use the loop-orchestrator skill ...'` invocation, via `Command::new`.
- Route this dispatch through Phase 1 of the parity roadmap's sandboxing work if that's landed by the time this is built — this is exactly the kind of command LAC itself issues that should run sandboxed, not just agent-issued commands.

**Definition of Done:** a test fixture task, when judged as `dispatch_loop`, actually invokes the correct loop template and the resulting `git diff` matches what running that loop manually would produce.

---

## Phase 4 — The actual loop, wired to `lac worker`

- `~/Library/LaunchAgents/org.lac.worker.plist` already runs `lac worker` under launchd `KeepAlive` supervision — check what `cmd_worker` (whatever it currently does) already covers before adding a parallel loop; this may be the natural home for `judge() → dispatch()` rather than a new subcommand, or it may need to call into `cmd_hermes` from inside its existing loop. Resolve this by reading `cmd_worker`'s current implementation first — don't assume.
- Loop structure: hygiene check → `judge()` → `dispatch()` if applicable → sleep/backoff → repeat. `wait` and `skip_task` decisions just cycle without dispatching.

**Definition of Done:** `lac worker` (or `lac hermes run --daemon`, whichever it ends up being) drains a 3-task fixture queue over a bounded test run without human intervention, respecting the 2-round review cap already defined elsewhere in `AGENTS.md`.

---

## Phase 5 — Documentation cleanup

- Update `hermes-orchestrator/SKILL.md`, `AGENTS.md`, and `docs/ARCHITECTURE.md` to describe what's actually there now (a real native loop) instead of the aspirational description that predates this build.

**Definition of Done:** no doc in the repo still implies an external Hermes process exists.

---

## Note for whoever runs this (Astra or otherwise)

Start by reading `cmd_worker` in full before writing Phase 4 — that's the one open question in this plan I can't resolve without seeing code I haven't read yet. Everything else here is grounded in what's actually in the repo today, not assumption.
