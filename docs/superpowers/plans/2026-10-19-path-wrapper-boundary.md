# Path Wrapper Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Exclude an unmatched outer opening bracket from location-bearing path candidates without damaging legitimate parenthesized path components.

**Architecture:** Keep the fix inside `url_path::boundary_range`, where URL and path candidates already receive punctuation normalization. Detect whether the first opening bracket has a matching closer inside the candidate; if not, advance the normalized start past that wrapper before applying the existing trailing-boundary logic.

**Tech Stack:** Rust, `regex`, Cargo unit tests, tmux integration shell tests.

## Global Constraints

- Preserve numeric `:line` and `:line:column` suffixes.
- Preserve balanced parentheses and brackets inside legitimate paths.
- Do not change user-level copy-command cleanup.
- Do not commit or push without explicit user authorization.

---

### Task 1: Normalize an unmatched leading wrapper

**Files:**
- Modify: `src/url_path.rs`
- Test: `src/url_path.rs`

**Interfaces:**
- Consumes: `boundary_range(text: &str, pattern: &str) -> (usize, usize)`.
- Produces: normalized path text and matching screen span through the existing `State::matches` pipeline.

- [x] **Step 1: Write the failing regression test**

Add this assertion to the existing path-boundary tests:

```rust
assert_path(
  "(src/engine/materials/data_loader/data_load_executor.go:120):",
  "src/engine/materials/data_loader/data_load_executor.go:120",
  1,
);
```

- [x] **Step 2: Run the focused test and verify RED**

Run: `cargo test url_path::tests::path_excludes_unmatched_outer_opener_before_location_closer -- --exact`

Expected: assertion failure showing an actual value beginning with `(`.

- [x] **Step 3: Implement the minimal boundary rule**

Replace the boolean end-only check with a helper that returns where the first
opening bracket closes:

```rust
fn first_matching_close_end(text: &str, first: char) -> Option<usize> {
  let mut stack = Vec::new();
  for (index, ch) in text.char_indices() {
    match ch {
      '(' | '[' | '（' | '【' => stack.push(ch),
      ')' | ']' | '）' | '】' => {
        let open = match stack.pop() {
          Some(open) if closes(open, ch) => open,
          _ => return None,
        };
        if open == first && stack.is_empty() {
          return Some(index + ch.len_utf8());
        }
      }
      _ => {}
    }
  }
  None
}
```

In `boundary_range`, strip both sides when the returned end equals the bounded
length, keep the opener when it closes inside a legitimate path component, and
strip only the opener when the helper returns `None`.

- [x] **Step 4: Run focused and related tests**

Run:

```bash
cargo test url_path::tests::path_excludes_unmatched_outer_opener_before_location_closer -- --exact
cargo test url_path::tests
```

Expected: all selected tests pass, including preservation of balanced internal
delimiters and `(path/file.md)` wrapper trimming.

- [x] **Step 5: Verify the complete runtime artifact**

Run:

```bash
cargo fmt --all -- --check
cargo test
bash tests/hard_wrap_tmux.sh
bash tests/copy_mode_snapshot_tmux.sh
cargo build --release
```

Then use an isolated `tmux -L ... -f /dev/null` session with the reproduced
input, launch `tmux-thumbs.sh`, select its only hint, and assert that
the resulting `THUMB` equals the expected literal without the leading `(`.

- [x] **Step 6: Review the final diff**

Run: `git diff --check && git diff -- src/url_path.rs docs/superpowers/specs/2026-10-19-path-wrapper-boundary-design.md docs/superpowers/plans/2026-10-19-path-wrapper-boundary.md`

Expected: only the approved boundary behavior, regression coverage, and these
two records are present; no commit or push is performed.
