# tmux-thumbs Command Alias Argument Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Prefix+f` pass the invoking pane ID to `tmux-thumbs.sh` so the picker opens instead of reporting that the source pane is unavailable.

**Architecture:** Keep the existing `thumbs-pick` command alias and explicit pane-ID flow. Change only the alias serialization so `run-shell` receives the script invocation and `#{pane_id}` as one shell-command argument, and lock that argument boundary into the existing isolated tmux integration test.

**Tech Stack:** Bash, tmux 3.5a command aliases, Rust/Cargo integration binaries

## Global Constraints

- Preserve the existing `thumbs-pick` command alias and configured prefix key.
- Preserve explicit source-pane capture; do not fall back to `$TMUX_PANE`.
- Leave unrelated duplicate historical aliases and picker behavior unchanged.
- Production code must not change until the new regression assertion fails for the expected argument-grouping reason.

---

### Task 1: Preserve the pane ID inside one `run-shell` command argument

**Files:**
- Modify: `tests/copy_mode_snapshot_tmux.sh:164-176`
- Modify: `tmux-thumbs.tmux:10`

**Interfaces:**
- Consumes: `THUMBS_COMMAND`, whose value is stored in tmux's `command-alias` option and then expanded by `tmux bind-key "${THUMBS_KEY}" thumbs-pick`.
- Produces: a prefix binding serialized as `run-shell -b "/absolute/path/tmux-thumbs.sh '#{pane_id}'"`, with one shell-command argument.

- [ ] **Step 1: Add the failing command-boundary assertion**

After the existing `prefix_binding` checks in `tests/copy_mode_snapshot_tmux.sh`, add:

```bash
expected_shell_command="run-shell -b \"${ROOT_DIR}/tmux-thumbs.sh '#{pane_id}'\""
grep -Fq "${expected_shell_command}" <<<"${prefix_binding}" \
  || {
    printf '%s\n%s\n' "${alias_state}" "${prefix_binding}" >&2
    fail 'plugin binding split the pane ID from the run-shell command'
  }
```

This assertion uses tmux's own `list-keys` serialization to verify the behavior-critical argument boundary rather than only checking that both substrings are present.

- [ ] **Step 2: Run the integration test and verify RED**

Run:

```bash
bash tests/copy_mode_snapshot_tmux.sh
```

Expected: FAIL with `plugin binding split the pane ID from the run-shell command`; the diagnostic binding has the broken form `run-shell -b /path/tmux-thumbs.sh "#{pane_id}"`.

- [ ] **Step 3: Group the full shell command in the alias**

Change `THUMBS_COMMAND` in `tmux-thumbs.tmux` to preserve quotes around the complete script invocation:

```bash
THUMBS_COMMAND="thumbs-pick=run-shell -b \"${CURRENT_DIR}/tmux-thumbs.sh '#{pane_id}'\""
```

Do not change alias lookup, replacement, key binding, or the launch script fallback behavior.

- [ ] **Step 4: Run focused and full automated verification**

Run:

```bash
bash tests/copy_mode_snapshot_tmux.sh
bash tests/hard_wrap_tmux.sh
cargo test
```

Expected: both tmux integration scripts print their `ok` result and all Cargo tests pass without failures.

- [ ] **Step 5: Reproduce the real key path in an isolated tmux server**

Start an isolated tmux server with `prefix C-f`, load `tmux-thumbs.tmux`, attach a temporary client, and send `Ctrl+f f`. Verify that the picker overlay renders hint letters and `show-messages` does not contain `tmux-thumbs: source pane unavailable; retry`. Exit the picker and kill only the isolated debug server.

- [ ] **Step 6: Commit the bug fix**

```bash
git add tests/copy_mode_snapshot_tmux.sh tmux-thumbs.tmux
git commit -m "fix: preserve pane id in thumbs alias"
```
