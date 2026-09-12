# Filename Candidate Scanner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recognize complete bare filenames as path candidates so internal SHA/number fragments cannot steal their hints, without classifying domains, versions, prose, or tool decorations as files.

**Architecture:** Add a pure `filename` scanner that tokenizes complete physical lines, classifies tokens from centralized basename/extension policy plus bounded context, and returns byte spans. Merge these candidates with legacy regex results before normalization and hint assignment using explicit semantic priority and span length; bare filenames remain single-line while existing URL/slash-path hard-wrap behavior remains unchanged.

**Tech Stack:** Rust 2018, existing `regex` and `unicode-width` dependencies, Cargo unit tests, isolated tmux integration tests.

## Global Constraints

- Preserve standalone SHA and number matching.
- Do not classify every dotted token as a filename.
- Reject domains, versions, IPs, ordinary prose, and ambiguous unknown extensions.
- Recognize tree entries as independent filenames without joining them to a prior directory.
- Do not require files to exist.
- Bare filenames do not gain cross-line joining in this change.
- Do not modify tmux or Neovim configuration.
- Follow red-green-refactor for every production behavior.

---

## File Structure

- Create `src/filename.rs`: tokenization, basename/extension policy, context classification, exact byte spans.
- Modify `src/main.rs`: register the filename module.
- Modify `src/state.rs`: merge scanner output with legacy candidates by explicit priority and overlap.
- Modify `tests/hard_wrap_tmux.sh`: reproduce the screenshot and assert the complete copied filename.

### Task 1: Pure Filename Scanner

**Files:** Create `src/filename.rs`; modify `src/main.rs`.

**Interfaces:**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilenameMatch {
  pub start: usize,
  pub end: usize,
}

pub fn scan(line: &str) -> Vec<FilenameMatch>;
```

- [ ] Add RED table tests for `skill-eval-review-report-20260912.md`, multiple filenames, composite extensions, prefix-related extensions, `:line:column`, known basenames, outer/internal parentheses, and tree entries.
- [ ] Add RED rejection tests for `host.example.com`, `1.2.3`, `v1.2.3`, IPs, `report-20260912`, and unknown standalone `artifact.xyzabc`.
- [ ] Run `cargo test --locked filename::tests -- --nocapture` and verify failures are caused by the absent module/API.
- [ ] Implement delimiter-aware `char_indices` scanning, centralized `KNOWN_BASENAMES` / `KNOWN_EXTENSIONS` / `DOMAIN_SUFFIXES`, punctuation trimming, balanced internal delimiters, and exact byte spans.
- [ ] Run filename tests and full `cargo test --locked`.
- [ ] Commit with `feat: scan bare filename candidates`.

### Task 2: Candidate Priority and Overlap Merge

**Files:** Modify `src/state.rs`; test in `src/state.rs`.

**Interfaces:** Consume `filename::scan(line)`. Produce normal `Match { pattern: "path", text, spans, hint }` candidates. Add one merge helper that compares screen start, semantic priority, and span length.

- [ ] Add RED test where `skill-eval-review-report-20260912.md` yields exactly one path candidate and no embedded `sha("20260912")`.
- [ ] Add RED tests proving custom regexp wins at the same start, URL/slash-path behavior is unchanged, and standalone `20260912` remains SHA.
- [ ] Run focused tests and verify the old collector returns the embedded SHA.
- [ ] Collect filename spans separately, normalize them through the existing path boundary contract, then merge before hint assignment. Use priority `custom > markdown_url/url > slash path > filename > specialized values` and prefer the longer candidate when start/priority tie.
- [ ] Ensure a winning filename suppresses lower-priority candidates fully contained in its span but does not remove adjacent candidates.
- [ ] Run focused tests and the full suite.
- [ ] Commit with `feat: prioritize complete filenames over fragments`.

### Task 3: Context and Tree Boundaries

**Files:** Modify `src/filename.rs` and tests.

**Interfaces:** Extend classification with left/right context. Tree decoration is excluded from the span; the returned filename remains independent and cannot participate in hard-wrap joining.

- [ ] Add RED tests for `└ result.txt`, `├ config.yaml`, and `│ README.md` as independent candidates.
- [ ] Add RED tests proving `/workspace/docs/\n  └ result.txt` remains two independent candidates and metadata labels do not join.
- [ ] Add strong-context unknown-extension tests using explicit `文件/文档/file/written to` wording, alongside unknown-extension rejection without context.
- [ ] Implement bounded context classification without filesystem access.
- [ ] Run scanner/state tests and full suite.
- [ ] Commit with `fix: classify filename context conservatively`.

### Task 4: Screenshot E2E and Deployment

**Files:** Modify `tests/hard_wrap_tmux.sh`.

**Interfaces:** Reuse the isolated `run_case` helper. The screenshot fixture must show one hint and the selected result must equal `skill-eval-review-report-20260912.md`.

- [ ] Add the exact screenshot text fixture to the tmux E2E script.
- [ ] Run against the pre-fix release binary and verify it copies `20260912`.
- [ ] Build the new release binary and verify it copies the complete filename.
- [ ] Run `cargo fmt --check`, `cargo test --locked`, `cargo build --release --locked`, `bash tests/hard_wrap_tmux.sh`, and `git diff --check`.
- [ ] Commit any final integration-only adjustment with `test: cover bare filenames in tmux`.
- [ ] Push `master`, fast-forward `~/.tmux/plugins/tmux-thumbs`, rebuild there, and rerun the isolated tmux script from the runtime checkout.
- [ ] Verify the source checkout and TPM checkout are clean and on the same commit. No tmux restart or configuration reload is required.
