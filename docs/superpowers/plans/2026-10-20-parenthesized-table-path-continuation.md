# Parenthesized Table Path Continuation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Join parenthesized paths that wrap at a table-cell boundary while keeping multiple wrapped paths independent.

**Architecture:** Preserve the existing terminal-edge path join. Add one semantic fallback that recognizes an opening wrapper before the first span, an earlier continuation column, a slash-terminated prefix, and a matching closer around the continuation. Keep privacy-only fixture renames separate from behavior changes.

**Tech Stack:** Rust, `unicode-width`, Cargo tests, isolated tmux shell integration tests.

## Global Constraints

- Preserve the existing unmatched-wrapper boundary fix in `src/url_path.rs`.
- Never join without a matching wrapper pair and a slash-terminated first segment.
- Stop each continuation before following prose or another path candidate.
- Use only neutral `/workspace/...` paths in new and updated fixtures.
- Do not commit or push without explicit user authorization.

---

### Task 1: Recognize parenthesized table-cell continuations

**Files:**
- Modify: `src/url_path.rs:52-161`
- Test: `src/url_path.rs` test module

**Interfaces:**
- Consumes: current line, current `ScreenSpan`, next line, next `ScreenSpan`, and accumulated path text.
- Produces: `is_parenthesized_cell_continuation(...) -> bool`, used as a fallback beside `reaches_wrap_boundary`.

- [x] **Step 1: Add the failing multi-path test**

Use a literal three-line fixture containing two wrapped paths. Assert that the
ordered path candidates are exactly:

```rust
[
  "src/engine/materials/data_loader/data_loader_helper.go:47",
  "src/engine/tracer/request_tracer.go:70",
]
```

Assert that both candidates have two screen spans. Add a separate negative
fixture without closing parentheses and assert no candidate spans multiple
lines.

- [x] **Step 2: Verify RED**

Run:

```bash
cargo test url_path::tests::joins_multiple_parenthesized_paths_across_table_cell_lines -- --exact
```

Expected: failure showing the two slash-terminated first fragments instead of
the two complete joined paths.

- [x] **Step 3: Implement the minimal semantic fallback**

Add this interface:

```rust
fn is_parenthesized_cell_continuation(
  current: &str,
  current_span: &ScreenSpan,
  next: &str,
  next_span: &ScreenSpan,
  accumulated: &str,
) -> bool
```

Return true only when the accumulated text ends in `/`, the character directly
before `current_span.start` is an opening bracket, the next span begins at an
earlier display column, and either the last character inside the next span or
the first character after it closes that bracket. OR this result with the
existing `reaches_wrap_boundary` result before confidence checks.

- [x] **Step 4: Verify GREEN and related boundaries**

Run:

```bash
cargo test url_path::tests::joins_multiple_parenthesized_paths_across_table_cell_lines -- --exact
cargo test url_path::tests
```

Expected: the focused case and all path-boundary tests pass.

### Task 2: Neutralize repository-specific fixtures

**Files:**
- Modify: `src/url_path.rs`
- Modify: `tests/hard_wrap_tmux.sh`

**Interfaces:**
- Consumes: existing literal path fixtures.
- Produces: behaviorally equivalent `/workspace/...` fixtures with no company domain, account path, repository name, or service identifier.

- [x] **Step 1: Replace fixture values**

Use
`/workspace/example-org/example-product/example-service/example-repository/api/spec/changes/example-service-feature-compatibility`
for wrapped proposal paths and
`/workspace/example-org/example-product/example-service/example-repository/worker/optimize-pipeline-job`
for metadata-boundary paths in both Rust and shell tests. These neutral values
preserve the original fixture lengths and wrap geometry.

- [x] **Step 2: Verify fixture behavior and privacy**

Run:

```bash
cargo test url_path::tests
rg -n '/workspace/' src/url_path.rs tests/hard_wrap_tmux.sh
```

Expected: path tests pass and every absolute fixture reported by `rg` uses a
neutral `/workspace/...` value.

### Task 3: Verify the real tmux flow and rebuild runtime binaries

**Files:**
- Modify: `tests/hard_wrap_tmux.sh`
- Build: `target/release/thumbs`, `target/release/tmux-thumbs`

**Interfaces:**
- Consumes: the real picker entrypoint and a three-line table fixture.
- Produces: independently selectable complete `THUMB` values for both paths.

- [x] **Step 1: Extend the real-tmux harness**

Allow the existing Python `run_case` helper to select a supplied hint key.
Run the same three-line fixture twice, selecting `a` for the first path and `b`
for the second path.

- [x] **Step 2: Run complete verification**

Run:

```bash
cargo fmt --all -- --check
cargo test
cargo build --release
bash tests/hard_wrap_tmux.sh
bash tests/copy_mode_snapshot_tmux.sh
git diff --check
```

Expected: all commands exit zero. Existing compiler warnings may remain only if
they are identical to the clean baseline.

- [x] **Step 3: Verify the reported interaction**

Launch `tmux-thumbs.sh` in an isolated `tmux -L ... -f /dev/null` server with a
neutral equivalent of the reported table. Select both hints in separate runs
and assert the two full path values.

- [x] **Step 4: Review status without committing**

Inspect `git status --short --branch` and the scoped diff. Leave all changes
uncommitted and do not modify the separate nvim repository worktree.
