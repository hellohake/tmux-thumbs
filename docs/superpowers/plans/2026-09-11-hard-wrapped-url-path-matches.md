# Hard-Wrapped URL and Path Matches Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make hard-wrapped URLs and file paths one candidate with one hint, exact multi-line highlighting, normalized copied text, and correct ASCII/CJK punctuation boundaries.

**Architecture:** Preserve physical lines for rendering and the line-oriented matcher for non-URL/path patterns. Represent every logical match as owned text plus byte-addressed screen spans, then add a URL/path normalizer that joins adjacent lines only when source pane width and conservative continuation gates prove a hard wrap.

**Tech Stack:** Rust 2018, `regex`, `unicode-width`, `termion`, Cargo unit tests, tmux 3.5a isolated-socket checks.

## Global Constraints

- Only built-in URL/path candidates may join physical lines.
- Custom regexps, commands, hashes, numbers, and all other candidates remain single-line.
- Joining fails closed without a valid pane width or with ambiguous continuation.
- Paths need not exist.
- Preserve balanced internal parentheses/brackets; remove outer ASCII/CJK delimiters and punctuation.
- The screenshot cases `)最后一次合并提交` and `)，发布于` stop at the URL boundary.
- Do not upgrade dependencies, cherry-pick PR #140, or touch `~/.tmux/plugins/tmux-thumbs`.
- Use red-green-refactor for each behavior.

---

## File Structure

- `src/state.rs`: match/span model, legacy matching, candidate merge, hint assignment.
- `src/url_path.rs`: boundary normalization and conservative line joining.
- `src/view.rs`: byte-span/display-column mapping and one-hint multi-span painting.
- `src/main.rs`: optional pane-width parsing.
- `src/swapper.rs`: source pane-width discovery and propagation.
- `tests/hard_wrap_tmux.sh`: isolated tmux integration check.

### Task 1: Owned Logical Matches

**Files:** Modify `src/state.rs`, `src/view.rs`, `src/main.rs`.

**Interfaces:** Produce `ScreenSpan { line, start, end }` and `Match { pattern, text: String, spans, hint }`. Add `Match::anchor()`. Preserve all 34 baseline tests.

- [ ] **Step 1: Write the failing representation test**

```rust
#[test]
fn single_line_match_owns_text_and_has_one_span() {
  let lines = split("前缀 /tmp/foo.rs 后缀");
  let custom = vec![];
  let result = State::new(&lines, "abcd", &custom, None).matches(false, false);
  let path = result.iter().find(|item| item.pattern == "path").unwrap();
  assert_eq!(path.text, "/tmp/foo.rs");
  assert_eq!(path.spans, vec![ScreenSpan { line: 0, start: 7, end: 18 }]);
}
```

- [ ] **Step 2: Verify RED**

```bash
cargo test --locked state::tests::single_line_match_owns_text_and_has_one_span -- --exact
```

Expected: compile failure because `State::new` has no width and `Match` has no spans.

- [ ] **Step 3: Implement the owned model**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenSpan { pub line: usize, pub start: usize, pub end: usize }

#[derive(Clone, Debug)]
pub struct Match {
  pub pattern: &'static str,
  pub text: String,
  pub spans: Vec<ScreenSpan>,
  pub hint: Option<String>,
}
```

Store `pane_width: Option<usize>` in `State`; make every legacy capture one span; use `HashMap<String, String>` for unique hints; adapt view to the first span; pass `None` at all constructors.

- [ ] **Step 4: Verify GREEN**

```bash
cargo test --locked state::tests::single_line_match_owns_text_and_has_one_span -- --exact
cargo test --locked
```

- [ ] **Step 5: Commit**

```bash
git add src/state.rs src/view.rs src/main.rs
git commit -m "refactor: represent matches with screen spans"
```

### Task 2: URL and Path Boundaries

**Files:** Create `src/url_path.rs`; modify `src/main.rs` and `src/state.rs`.

**Interfaces:** Produce `normalize(lines: &[&str], matches: Vec<Match>, pane_width: Option<usize>) -> Vec<Match>`. Text and final span endpoint both exclude outer delimiters/prose.

- [ ] **Step 1: Write failing boundary tests**

```rust
#[test]
fn url_stops_before_ascii_closer_and_chinese_prose() {
  assert_url(
    "(https://github.com/fcsonline/tmux-thumbs)最后一次合并提交是 2023-03-21。",
    "https://github.com/fcsonline/tmux-thumbs",
  );
}

#[test]
fn url_stops_before_cjk_closer_comma_and_chinese_prose() {
  assert_url(
    "（https://github.com/fcsonline/tmux-thumbs/releases/tag/0.8.0），发布于 2023-03-11。",
    "https://github.com/fcsonline/tmux-thumbs/releases/tag/0.8.0",
  );
}

#[test]
fn balanced_internal_delimiters_are_preserved() {
  assert_url("https://host/wiki/Foo_(bar)。", "https://host/wiki/Foo_(bar)");
  assert_path("app/(main)/[id]/index.tsx)。", "app/(main)/[id]/index.tsx");
}
```

Also assert exact span endpoints, `path/file.go:42:7`, and ASCII/CJK punctuation.

- [ ] **Step 2: Verify RED**

```bash
cargo test --locked url_path::tests -- --nocapture
```

Expected: missing module or current overmatch containing `)最后一次合并提交是`.

- [ ] **Step 3: Implement normalization**

Register `mod url_path;`. Replace the URL tail with printable ASCII:

```rust
("url", r"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[\x21-\x7e]+)")
```

Support only agreed path characters plus `:line[:column]` and trailing `/`. Track ASCII/full-width bracket balance; stop before an unmatched closer; trim `，。；：！、,.;!`; keep balanced closers. Shorten the final span whenever text is trimmed. Call normalization before hint allocation.

- [ ] **Step 4: Verify GREEN**

```bash
cargo test --locked url_path::tests -- --nocapture
cargo test --locked state::tests
cargo test --locked
```

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/state.rs src/url_path.rs
git commit -m "fix: stop URL and path matches at outer punctuation"
```

### Task 3: Conservative Cross-Line Joining

**Files:** Modify `src/url_path.rs` and `src/state.rs`.

**Interfaces:** Extend `normalize` to join only with `Some(width)` and all gates satisfied; output one match with ordered spans; preserve ambiguous candidates separately.

- [ ] **Step 1: Write positive tests**

Cover the two-line screenshot path, absolute/relative/`~/`/`./`/`../` paths, URL, `path/file.go:42:7`, and one three-line candidate. Assert:

```rust
assert_eq!(candidate.text, ".ai_doc/records/inbox/life-card-admin-20260911-01a08c45/handoff.md");
assert_eq!(candidate.spans.len(), 2);
assert_eq!(candidate.hint.as_deref(), Some("a"));
assert_eq!(results.iter().filter(|item| item.pattern == "path").count(), 1);
```

- [ ] **Step 2: Write negative tests**

Use full-width first lines followed by independent rooted paths, `- item`, `◆ item`, shell prompts, new URLs, prose, blanks, and non-indented lines. Assert no join for `None`/zero width or a short first line.

- [ ] **Step 3: Verify RED**

```bash
cargo test --locked url_path::tests::joins_ -- --nocapture
cargo test --locked url_path::tests::does_not_join_ -- --nocapture
```

- [ ] **Step 4: Implement join gates**

```rust
fn reaches_pane_edge(line: &str, span: &ScreenSpan, width: usize) -> bool;
fn continuation_span(pattern: &str, line: &str, accumulated: &str) -> Option<ScreenSpan>;
fn is_blocked_continuation(trimmed: &str) -> bool;
fn is_confident_continuation(pattern: &str, accumulated: &str, token: &str) -> bool;
fn covered_by(candidate: &Match, span: &ScreenSpan) -> bool;
```

Require: match reaches last non-whitespace byte; `line.width_cjk() >= width`; next line is indented/non-empty; token is category-valid; next line is not list/status/prompt/new-scheme/independent-root; syntax makes continuation confident. Repeat for later lines. Concatenate exact slices without newline/indentation, suppress covered fragments, restore screen order, then assign hints.

- [ ] **Step 5: Verify GREEN**

```bash
cargo test --locked url_path::tests -- --nocapture
cargo test --locked
```

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/url_path.rs
git commit -m "feat: join hard-wrapped URL and path candidates"
```

### Task 4: Multi-Span Rendering

**Files:** Modify `src/view.rs`.

**Interfaces:** Consume ordered spans; paint every fragment; paint one hint at first span for `left/off_left` or last span for `right/off_right`; compute columns with CJK display width.

- [ ] **Step 1: Write failing render tests**

Render a manual two-span match into `Vec<u8>`. Assert both cursor positions and one unique hint. Include `中文 https://host/very` followed by `  long/path`, and test both left/right positions.

- [ ] **Step 2: Verify RED**

```bash
cargo test --locked view::tests::renders_all_spans_with_one_hint -- --exact
cargo test --locked view::tests::uses_cjk_display_columns -- --exact
```

- [ ] **Step 3: Implement span rendering**

Write through the supplied writer rather than `print!`. Paint `&line[span.start..span.end]`. Compute:

```rust
fn display_column(line: &str, byte_offset: usize) -> u16 {
  line[..byte_offset].width_cjk() as u16 + 1
}
```

Use signed hint-offset arithmetic and paint typed-hint feedback only at the anchor.

- [ ] **Step 4: Verify GREEN**

```bash
cargo test --locked view::tests -- --nocapture
cargo test --locked
```

- [ ] **Step 5: Commit**

```bash
git add src/view.rs
git commit -m "feat: render multi-line candidates with one hint"
```

### Task 5: Source Pane Width Propagation

**Files:** Modify `src/main.rs` and `src/swapper.rs`.

**Interfaces:** Produce `Swapper::active_pane_width: Option<usize>` and `thumbs --pane-width <usize>`.

- [ ] **Step 1: Write failing tests**

Extend pane fixtures with width, assert storage, and assert generated command contains `--pane-width 120`. Test missing/zero/invalid CLI values become `None`.

- [ ] **Step 2: Verify RED**

```bash
cargo test --locked --bin tmux-thumbs tests::retrieve_active_pane -- --exact
cargo test --locked --bin tmux-thumbs tests::passes_pane_width_to_thumbs -- --exact
```

- [ ] **Step 3: Implement propagation**

Use:

```text
#{pane_id}:#{?pane_in_mode,1,0}:#{pane_height}:#{pane_width}:#{scroll_position}:#{window_zoomed_flag}:#{?pane_active,active,nope}
```

Store index 3, shift later indices, add Clap `--pane-width`, parse nonzero `usize`, pass it to `State::new`, and append it to the generated command only when present.

- [ ] **Step 4: Verify GREEN**

```bash
cargo test --locked --bin tmux-thumbs
cargo test --locked --bin thumbs
cargo test --locked
```

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/swapper.rs
git commit -m "feat: pass source pane width to candidate matching"
```

### Task 6: Isolated tmux Integration and Final Verification

**Files:** Create `tests/hard_wrap_tmux.sh`; modify `README.md` only if direct `--pane-width` usage needs documentation.

**Interfaces:** A repeatable unique-`tmux -L` check that proves exact buffer text, one hint, multi-span highlighting, and no default-server interaction.

- [ ] **Step 1: Write the failing integration script**

Use `set -euo pipefail`, a socket name containing `$$`, and a trap killing only that socket. Start a narrow detached session; print exact hard-newline path and URL/prose fixtures; launch this checkout's release binary; capture the temporary pane with escapes; assert two highlighted fragments and one hint; send the hint; assert `show-buffer` equals the normalized value. Never reference the default socket or live TPM directory.

- [ ] **Step 2: Verify RED**

```bash
bash tests/hard_wrap_tmux.sh
```

Expected: before final integration, a missing binary or assertion identifies the unverified behavior.

- [ ] **Step 3: Fix only evidenced integration defects**

For each defect, add a focused failing Rust unit test before production changes. Never weaken the integration assertion.

- [ ] **Step 4: Run final verification**

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
bash tests/hard_wrap_tmux.sh
git diff --check
git status --short
```

- [ ] **Step 5: Commit integration coverage**

```bash
git add tests/hard_wrap_tmux.sh README.md
git commit -m "test: cover hard-wrapped candidates in tmux"
```

- [ ] **Step 6: Perform independent review**

Invoke `superpowers:requesting-code-review`. Review priority regressions, byte/column confusion, Unicode boundaries, false joins, hint duplication, and accidental live configuration changes. Address accepted findings through failing tests first.

