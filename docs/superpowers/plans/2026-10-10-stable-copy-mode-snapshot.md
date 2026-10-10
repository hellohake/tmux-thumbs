# Stable Copy-Mode Snapshot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make tmux-thumbs hint candidates come from the exact stable copy-mode page captured before any temporary-window pane swap or resize.

**Architecture:** Move both tmux captures into a synchronous pre-swap phase owned by `Swapper`. Compare pane state around the capture and retry unstable copy-mode snapshots before the detached picker window is created.

**Tech Stack:** Rust 2018, tmux 3.5-compatible CLI commands, Bash integration tests.

## Global Constraints

- Preserve the current matcher, renderer, clipboard commands, and unique runtime naming.
- Use test-first RED/GREEN cycles.
- Do not change global tmux window sizing as a workaround.
- Do not add fixed sleeps to production code.
- Do not commit or stage changes unless the user explicitly requests it.

---

### Task 1: Reproduce copy-mode page drift with real tmux

**Files:**
- Create: `tests/copy_mode_snapshot_tmux.sh`

**Interfaces:**
- Consumes: `target/release/tmux-thumbs --dir <repo>` and a private tmux socket.
- Produces: an executable regression test whose assertion compares the pre-swap visible page with the rendered overlay.

- [x] **Step 1: Write the failing integration test**

  Create a `120x30` pane containing long uniquely numbered lines, enter copy mode,
  move one page down from history top, and assert the source page contains
  `L015`. Run tmux-thumbs through a private `tmux` wrapper which delays
  `capture-pane` and resizes each newly-created picker window to `80x23`. Assert
  that the overlay contains `L015` rather than the later `L019` page.

- [x] **Step 2: Verify RED**

  Run:

  ```bash
  cargo build --release
  bash tests/copy_mode_snapshot_tmux.sh
  ```

  Expected: the assertion reports that the overlay drifted from the source page.

### Task 2: Freeze and validate snapshots before swapping panes

**Files:**
- Modify: `src/swapper.rs:59-320`
- Test: `src/swapper.rs:440-619`

**Interfaces:**
- Produces: `PaneState`, `capture_stable_snapshot() -> Result<(), SnapshotError>`,
  and frozen `screen` / `joined` inputs inside an invocation-private runtime
  directory consumed by `execute_thumbs()`.
- Consumes: the existing `Executor`, invocation-unique `signal`, and `tmp_file`.

- [x] **Step 1: Add unit tests for retry and fail-closed behavior**

  Use `TestShell` output sequences to prove that a changed copy-mode state causes
  a second capture attempt, a stable second state succeeds, and three unstable
  attempts return false without creating a picker window.

- [x] **Step 2: Verify the new unit tests fail**

  Run:

  ```bash
  cargo test --bin tmux-thumbs
  ```

  Expected: compilation or assertions fail because stable snapshot capture does
  not exist yet.

- [x] **Step 3: Implement the minimal snapshot phase**

  Add a comparable pane-state structure including pane id, mode, height, width,
  scroll position, history size, and zoom state. Capture `-N` and `-J` files
  synchronously before `new-window`, compare state after capture, retry at most
  three times, and remove the owned private runtime directory on failure. Reject
  failed capture commands instead of accepting empty or partial files.

- [x] **Step 4: Make the picker consume only frozen files**

  Remove `capture-pane`, `save-buffer`, and `delete-buffer` from the detached
  picker command. Keep pane restoration, zoom restoration, and the completion
  signal there; keep directory ownership and final cleanup in the parent.

- [x] **Step 5: Fail closed in `main`**

  If the snapshot remains unstable, capture fails, or `new-window` fails, display
  the corresponding tmux message, remove only parent-owned runtime data, and
  return before `swap_panes` or `wait-for`.

- [x] **Step 6: Verify GREEN for unit tests**

  Run:

  ```bash
  cargo test --bin tmux-thumbs
  ```

  Expected: all tmux-thumbs unit tests pass.

### Task 3: Verify the real regression and the full suite

**Files:**
- Test: `tests/copy_mode_snapshot_tmux.sh`
- Test: `tests/hard_wrap_tmux.sh`

**Interfaces:**
- Consumes: the release binaries built from the completed implementation.
- Produces: evidence that copy-mode geometry changes no longer alter candidates and existing matching behavior is unchanged.

- [x] **Step 1: Build release binaries and verify the regression**

  Run:

  ```bash
  cargo build --release
  bash tests/copy_mode_snapshot_tmux.sh
  ```

  Expected: the frozen overlay contains the same `L015` page as the source.

- [x] **Step 2: Run all Rust tests**

  Run:

  ```bash
  cargo test
  ```

  Expected: all tests pass; only pre-existing compiler warnings remain.

- [x] **Step 3: Run the existing tmux integration suite**

  Run:

  ```bash
  bash tests/hard_wrap_tmux.sh
  ```

  Expected: every hard-wrap, Unicode, URL, filename, and gutter case prints `ok`.

- [x] **Step 4: Inspect the final diff and runtime artifacts**

  Run:

  ```bash
  git diff --check
  git status --short
  tmux list-windows -a -F '#{window_name}' | rg '^\[thumbs\]$' || true
  ```

  Expected: no whitespace errors, only intended source/test/docs changes, and no leaked picker windows.
