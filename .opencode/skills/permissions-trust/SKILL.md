---
name: Permissions Trust
description: Manage trusted permission rules so repeated operations don't need re-approval
---

## What it does

Maintains `~/.lac/permissions-trusted.json` — a list of shell/edit/skill patterns the human has approved N times and now trusts for non-stop operation. OpenCode checks trusted rules before prompting, so overnight loops don't stall on `git commit` or `cargo test` approvals.

Without this, every `task-runner --drain` batch stops at the first `ask` permission.

## Workflow

### 1. Trust a pattern after repeated approval

```bash
opencode2 run 'Use the permissions-trust skill --allow "git commit -m *"'
opencode2 run 'Use the permissions-trust skill --allow "cargo test *"'
opencode2 run 'Use the permissions-trust skill --allow "skill:local-verify"'
```

Appends to `~/.lac/permissions-trusted.json`:
```json
[
  {"action": "shell", "resource": "git commit -m *", "effect": "allow", "times_approved": 47, "trusted_since": "2026-09-10"},
  {"action": "skill", "resource": "local-verify", "effect": "allow", "times_approved": 23, "trusted_since": "2026-09-10"}
]
```

### 2. Deny a pattern explicitly

```bash
opencode2 run 'Use the permissions-trust skill --deny "rm -rf *"'
opencode2 run 'Use the permissions-trust skill --deny "git push --force *"'
```

Deny always wins over allow (last matching rule wins in OpenCode).

### 3. List / revoke

```bash
opencode2 run 'Use the permissions-trust skill --list'
opencode2 run 'Use the permissions-trust skill --revoke "cargo test *"'
```

### 4. Sync to opencode.jsonc

```bash
opencode2 run 'Use the permissions-trust skill --sync'
# Merges trusted rules into opencode.jsonc permissions[] (trusted first, then project rules)
```

## Safety rules (never trust these)

The skill refuses to trust:
- `rm -rf /`, `rm -rf ~`, `rm -rf /*`
- `git push --force *`, `git reset --hard *`
- `sudo *`, `chmod 777 *`
- `edit: /etc/*`, `edit: ~/.ssh/*`

## Config

```json
"skills": {
  "permissions-trust": {
    "store": "~/.lac/permissions-trusted.json",
    "min_approvals_before_suggest": 5
  }
}
```

## When to use

- **Before overnight batches**: `--list` to confirm no `ask` will stall the queue
- **After 5+ manual approvals of same pattern**: skill suggests trusting it
- **After incident**: `--deny` the pattern + `--revoke` related allows

## Integration

Works with:
- `task-runner` — pre-flight checks trusted list; warns if queue tasks need untrusted permissions
- `loop-orchestrator` — each phase checks trusted before running (no mid-loop prompts)
- `agent-resume` — trusted list hash saved in state file for audit
