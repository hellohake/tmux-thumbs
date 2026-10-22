# Copy-Mode Viewport and Table URL Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `prefix+f` and copy-mode `f` capture the exact invoking tmux viewport and copy one complete parenthesized URL when TraeX hard-wraps it inside a table cell.

**Architecture:** Pass the source pane ID through every launcher layer, parse pane state into validated live/copy-mode viewport variants, and fail closed when copy-mode coordinates are unavailable. Independently extend the existing URL/path normalizer with a candidate-local bracket-and-column lookahead that proves a table URL boundary before relaxing normal cross-line checks.

**Tech Stack:** Rust 2018, clap 2, regex, unicode-width, Bash, tmux 3.5a, isolated real-tmux integration tests.

## Global Constraints

- A picker invoked from copy mode must represent exactly the visible copy-mode viewport, or fail without opening an overlay.
- Snapshot capture must use the invoking pane, not mutable global active-window state.
- Copy mode with unavailable viewport coordinates must display `tmux-thumbs: copy-mode viewport unavailable; retry` and must never fall back to live capture.
- A table URL may join only when its outer bracket, continuation column, consecutive lines, and final matching close bracket are all proven.
- Independent P90 and AVG URLs on the same logical row must remain separate candidates.
- Existing live-pane, soft-wrap, path, filename, custom-regexp, clipboard, zoom, concurrent-invocation, and private-runtime-directory behavior must remain unchanged.
- Runtime directories remain `0700`; snapshot files remain `0600`; shell paths remain POSIX-quoted.
- Real tmux tests must use `tmux -L <socket> -f /dev/null`.
- Do not push commits. Keep plugin and dotfiles commits local.

---

### Task 1: Validate an Explicit Source Pane and Viewport

**Files:**
- Modify: `src/swapper.rs:58-336`
- Test: `src/swapper.rs:656-840`

**Interfaces:**
- Consumes: `source_pane_id: String` supplied to `Swapper::new`.
- Produces: `enum Viewport { Live, CopyMode { scroll_position: i32 } }`.
- Produces: `PaneState { pane_id, viewport, height, width, history_size, zoomed }`.
- Produces: `SnapshotError::{SourcePaneUnavailable, ViewportUnavailable}` in addition to the existing errors.
- Produces: `read_source_pane_state(&mut self) -> Result<PaneState, SnapshotError>`.

- [ ] **Step 1: Add failing unit tests for explicit targeting and fail-closed copy mode**

Replace the test-only pane-state fixture with exact one-pane outputs and add tests with these assertions:

```rust
fn copy_state(pane_id: &str, scroll: &str, history_size: usize) -> String {
  format!("{}:copy-mode:24:120:{}:{}:0", pane_id, scroll, history_size)
}

fn live_state(pane_id: &str, history_size: usize) -> String {
  format!("{}::24:120::{}:0", pane_id, history_size)
}

#[test]
fn queries_the_explicit_source_pane() {
  let stable = copy_state("%97", "10", 100);
  let mut executor = TestShell::new(vec![stable.clone(), "".to_string(), stable]);
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    "%97".to_string(),
    "/tmp/tmux-thumbs".to_string(),
    "".to_string(),
    "".to_string(),
    "".to_string(),
    false,
  );

  assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
  swapper.cleanup_runtime_dir();
  drop(swapper);

  let state_commands = executor
    .executed
    .iter()
    .filter(|command| command.get(1).map(String::as_str) == Some("display-message"))
    .collect::<Vec<_>>();
  assert_eq!(state_commands.len(), 2);
  assert!(state_commands
    .iter()
    .all(|command| command.windows(2).any(|pair| pair == ["-t", "%97"])));
}

#[test]
fn rejects_copy_mode_without_scroll_position() {
  let invalid = copy_state("%97", "", 100);
  let mut executor = TestShell::new(vec![invalid]);
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    "%97".to_string(),
    "/tmp/tmux-thumbs".to_string(),
    "".to_string(),
    "".to_string(),
    "".to_string(),
    false,
  );

  assert_eq!(
    swapper.capture_stable_snapshot(),
    Err(SnapshotError::ViewportUnavailable)
  );
  drop(swapper);
  assert!(executor
    .executed
    .iter()
    .all(|command| command.get(1).map(String::as_str) != Some("capture-pane")));
}

#[test]
fn rejects_invalid_copy_mode_scroll_positions() {
  for scroll in ["-1", "invalid"] {
    let mut executor = TestShell::new(vec![copy_state("%97", scroll, 100)]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    assert_eq!(
      swapper.capture_stable_snapshot(),
      Err(SnapshotError::ViewportUnavailable),
      "scroll={}",
      scroll
    );
  }
}

#[test]
fn rejects_a_state_for_another_pane() {
  let mut executor = TestShell::new(vec![copy_state("%98", "10", 100)]);
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    "%97".to_string(),
    "/tmp/tmux-thumbs".to_string(),
    "".to_string(),
    "".to_string(),
    "".to_string(),
    false,
  );

  assert_eq!(
    swapper.capture_stable_snapshot(),
    Err(SnapshotError::SourcePaneUnavailable)
  );
}

#[test]
fn accepts_live_viewport_without_scroll_range() {
  let stable = live_state("%97", 100);
  let mut executor = TestShell::new(vec![stable.clone(), "".to_string(), stable]);
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    "%97".to_string(),
    "/tmp/tmux-thumbs".to_string(),
    "".to_string(),
    "".to_string(),
    "".to_string(),
    false,
  );

  assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
  swapper.cleanup_runtime_dir();
  drop(swapper);
  let capture = executor
    .executed
    .iter()
    .find(|command| command.get(1).map(String::as_str) == Some("capture-pane"))
    .expect("capture command");
  assert!(!capture.iter().any(|argument| argument == "-S" || argument == "-E"));
}

#[test]
fn rejects_unsupported_pane_modes() {
  let mut executor = TestShell::new(vec!["%97:clock-mode:24:120::100:0".to_string()]);
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    "%97".to_string(),
    "/tmp/tmux-thumbs".to_string(),
    "".to_string(),
    "".to_string(),
    "".to_string(),
    false,
  );
  assert_eq!(
    swapper.capture_stable_snapshot(),
    Err(SnapshotError::ViewportUnavailable)
  );
}
```

Update existing `Swapper::new` calls in the test module to supply `%97` without changing their assertions.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --bin tmux-thumbs queries_the_explicit_source_pane
cargo test --bin tmux-thumbs rejects_copy_mode_without_scroll_position
cargo test --bin tmux-thumbs rejects_invalid_copy_mode_scroll_positions
cargo test --bin tmux-thumbs rejects_a_state_for_another_pane
cargo test --bin tmux-thumbs accepts_live_viewport_without_scroll_range
cargo test --bin tmux-thumbs rejects_unsupported_pane_modes
```

Expected: compilation fails because `Swapper::new` has no pane-ID parameter and the new viewport/error variants do not exist.

- [ ] **Step 3: Implement the validated viewport model**

In `src/swapper.rs`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
enum Viewport {
  Live,
  CopyMode { scroll_position: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneState {
  pane_id: String,
  viewport: Viewport,
  height: i32,
  width: usize,
  history_size: usize,
  zoomed: bool,
}
```

Add `source_pane_id: String` to `Swapper`, accept it immediately after the executor in `Swapper::new`, and replace `read_active_pane_state` with a fallible `read_source_pane_state`. Query exactly one pane:

```rust
let command = vec![
  "tmux".to_string(),
  "display-message".to_string(),
  "-p".to_string(),
  "-t".to_string(),
  self.source_pane_id.clone(),
  "#{pane_id}:#{pane_mode}:#{pane_height}:#{pane_width}:#{scroll_position}:#{history_size}:#{window_zoomed_flag}".to_string(),
];
```

Parse an empty `pane_mode` as `Viewport::Live`, parse `copy-mode` only when `scroll_position` is a non-negative `i32`, and return `ViewportUnavailable` for copy mode without a valid scroll position or for another non-empty mode. Return `SourcePaneUnavailable` when tmux fails, output is malformed, or the returned `pane_id` differs from `source_pane_id`.

Update `capture_snapshot_files` to derive `-S/-E` only from `Viewport::CopyMode`:

```rust
if let Viewport::CopyMode { scroll_position } = state.viewport {
  command.extend([
    "-S".to_string(),
    (-scroll_position).to_string(),
    "-E".to_string(),
    (state.height - scroll_position - 1).to_string(),
  ]);
}
```

Make `capture_stable_snapshot` propagate state-read errors, preserve the three-attempt mutation retry, and clean up the owned runtime directory on every error path.

- [ ] **Step 4: Run the swapper test suite and verify GREEN**

Run:

```bash
cargo test --bin tmux-thumbs
```

Expected: all swapper unit tests pass; the new tests prove explicit `-t %97` targeting and no capture on a missing copy-mode scroll position.

- [ ] **Step 5: Commit the viewport model**

```bash
git add src/swapper.rs
git commit -m "fix: validate copy-mode viewport snapshots"
```

---

### Task 2: Pass the Pane ID Through Every Launcher

**Files:**
- Modify: `src/swapper.rs:989-1065`
- Modify: `tmux-thumbs.sh:1-53`
- Modify: `tmux-thumbs.tmux:1-11`
- Test: `tests/copy_mode_snapshot_tmux.sh:37-120`

**Interfaces:**
- Consumes: `Swapper::new(executor, source_pane_id, ...)` from Task 1.
- Produces: required CLI option `--pane-id <tmux-pane-id>`.
- Produces: launcher contract `tmux-thumbs.sh <pane-id>`.
- Produces: tmux binding that expands `#{pane_id}` before starting the background shell.

- [ ] **Step 1: Make the real-tmux test fail without explicit pane propagation**

Extend `tests/copy_mode_snapshot_tmux.sh` to create a distractor window whose live page contains `/tmp/live-distractor.txt`, select that window before launch, and still invoke the swapper for `SOURCE_PANE`. Change the launcher to the desired interface:

```bash
launcher="PATH=$(printf '%q' "${TMP_DIR}:${PATH}") REAL_TMUX=$(printf '%q' "${REAL_TMUX}") TMPDIR=$(printf '%q' "${RUNTIME_TMP_DIR}") $(printf '%q' "${ROOT_DIR}/target/release/tmux-thumbs") --pane-id $(printf '%q' "${SOURCE_PANE}") --dir $(printf '%q' "${ROOT_DIR}")"
```

Assert both viewport identity and target isolation:

```bash
grep -Fq 'L015 ' <<<"${overlay}" || fail 'overlay drifted away from the visible copy-mode page'
if grep -Fq '/tmp/live-distractor.txt' <<<"${overlay}"; then
  fail 'overlay captured the mutable active window instead of the invoking pane'
fi
```

In the same failing-test step, extend the fake `tmux` wrapper with an
environment-controlled `display-message` branch that removes the copy-mode
scroll field only for the swapper state query:

```bash
  display-message)
    if [ "${TMUX_THUMBS_DROP_SCROLL:-0}" = 1 ] && [[ "$*" == *'#{pane_mode}'* ]]; then
      output="$("${REAL_TMUX}" "$@")"
      IFS=: read -r pane mode height width _scroll history zoom <<<"${output}"
      printf '%s:%s:%s:%s::%s:%s\n' "${pane}" "${mode}" "${height}" "${width}" "${history}" "${zoom}"
      exit 0
    fi
    exec "${REAL_TMUX}" "$@"
    ;;
```

Launch a second invocation with `TMUX_THUMBS_DROP_SCROLL=1`, the same explicit
`--pane-id "${SOURCE_PANE}"`, and the same private `TMPDIR`. Wait until `tmux
show-messages` contains the retry text, then assert that no `[thumbs]` window
was created:

```bash
if "${REAL_TMUX}" -L "${SOCKET_NAME}" list-windows -F '#{window_name}' | grep -Fxq '[thumbs]'; then
  fail 'invalid copy-mode viewport opened a picker window'
fi
"${REAL_TMUX}" -L "${SOCKET_NAME}" show-messages \
  | grep -Fq 'tmux-thumbs: copy-mode viewport unavailable; retry' \
  || fail 'missing viewport retry message'
```

- [ ] **Step 2: Build and run the integration test to verify RED**

Run:

```bash
cargo build --release --bins
bash tests/copy_mode_snapshot_tmux.sh
```

Expected: FAIL because `tmux-thumbs` does not recognize `--pane-id`, or because launcher state still depends on active-pane discovery.

- [ ] **Step 3: Implement the CLI and shell launcher contract**

Add a required clap option in `src/swapper.rs`:

```rust
.arg(
  Arg::with_name("pane_id")
    .help("Source tmux pane to capture")
    .long("pane-id")
    .takes_value(true)
    .required(true),
)
```

Read it in `main`, pass it to `Swapper::new`, and map the Task 1 errors to exact messages:

```rust
SnapshotError::SourcePaneUnavailable => "tmux-thumbs: source pane unavailable; retry",
SnapshotError::ViewportUnavailable => "tmux-thumbs: copy-mode viewport unavailable; retry",
```

In `tmux-thumbs.sh`, require and forward the first argument:

```bash
SOURCE_PANE="${1:-}"
if [ -z "${SOURCE_PANE}" ]; then
  tmux display-message 'tmux-thumbs: source pane unavailable; retry'
  exit 0
fi

PARAMS=(--dir "${CURRENT_DIR}" --pane-id "${SOURCE_PANE}")
```

In `tmux-thumbs.tmux`, expand the invoking pane in the command alias:

```bash
tmux set-option -ag command-alias "thumbs-pick=run-shell -b ${CURRENT_DIR}/tmux-thumbs.sh '#{pane_id}'"
```

- [ ] **Step 4: Run unit and real-tmux tests and verify GREEN**

Run:

```bash
cargo test --bin tmux-thumbs
cargo build --release --bins
bash tests/copy_mode_snapshot_tmux.sh
```

Expected: all commands pass; the overlay contains the older visible marker and excludes the distractor/live marker.

- [ ] **Step 5: Commit the explicit launcher target**

```bash
git add src/swapper.rs tmux-thumbs.sh tmux-thumbs.tmux tests/copy_mode_snapshot_tmux.sh
git commit -m "fix: bind picker launch to source pane"
```

---

### Task 3: Join Proven Parenthesized Table URLs

**Files:**
- Modify: `src/url_path.rs:52-260`
- Test: `src/url_path.rs:330-945`
- Test: `tests/hard_wrap_tmux.sh:160-279`

**Interfaces:**
- Consumes: `Match`, `ScreenSpan`, `continuation_start`, `continuation_span`, `boundary_len`, and `closes` already defined in `src/url_path.rs`.
- Produces: `ParenthesizedCellUrl { continuation_column: usize, close_line: usize }`.
- Produces: `parenthesized_cell_url(lines, candidate, soft_wrapped) -> Option<ParenthesizedCellUrl>`.

- [ ] **Step 1: Add failing URL reconstruction tests based on the recording**

Add a positive test whose URL contains internal parentheses and arbitrary mid-token breaks:

```rust
#[test]
fn joins_parenthesized_metrics_urls_across_table_cell_lines() {
  let input = concat!(
    "  Summary  P90 (https://metrics.example/plot#now-1h,China-             same\n",
    "           North6,false;max:stage.ms%7Bcar\n",
    "           d_type=*%7D%7Bstage=literal_or(summary),status=literal_or(\n",
    "           success)%7D%5Bpct90%5D;0) / AVG (https://metrics.example/plot#now-\n",
    "           1h,China-North6,false;avg:stage.ms%7Bstage=literal_or(summary)\n",
    "           %7D%5Bavg%5D;0)\n",
  );
  let lines = input.split('\n').collect::<Vec<_>>();
  let results = State::new(&lines, "abcd", &vec![], Some(120)).matches(false, false);
  let urls = results
    .iter()
    .filter(|candidate| candidate.pattern == "url")
    .collect::<Vec<_>>();

  assert_eq!(
    urls.iter().map(|candidate| candidate.text.as_str()).collect::<Vec<_>>(),
    [
      "https://metrics.example/plot#now-1h,China-North6,false;max:stage.ms%7Bcard_type=*%7D%7Bstage=literal_or(summary),status=literal_or(success)%7D%5Bpct90%5D;0",
      "https://metrics.example/plot#now-1h,China-North6,false;avg:stage.ms%7Bstage=literal_or(summary)%7D%5Bavg%5D;0",
    ]
  );
  assert_eq!(urls[0].spans.len(), 4);
  assert_eq!(urls[1].spans.len(), 3);
}
```

Add negative cases that must retain single-line fragments:

```rust
#[test]
fn rejects_ambiguous_parenthesized_table_url_continuations() {
  let cases = [
    "row (https://host/path-\n     continuation-without-close",
    "row (https://host/path-\n       wrong-column) ",
    "row (https://host/path-\n\n     continuation)",
    "row (https://host/path-\n     https://independent.example/next)",
    "row (https://host/path-\n  ─────────────────────",
  ];

  for input in cases {
    let lines = input.split('\n').collect::<Vec<_>>();
    let results = State::new(&lines, "abcd", &vec![], Some(120)).matches(false, false);
    assert!(results
      .iter()
      .filter(|candidate| candidate.pattern == "url")
      .all(|candidate| candidate.spans.len() == 1), "{:?}", input);
  }
}
```

Add a real-tmux cell with the same two independent URLs in the Python section
of `tests/hard_wrap_tmux.sh`:

```python
p90 = "https://metrics.example/plot#now-1h,China-North6,false;max:stage.ms%7Bcard_type=*%7D%7Bstage=literal_or(summary),status=literal_or(success)%7D%5Bpct90%5D;0"
avg = "https://metrics.example/plot#now-1h,China-North6,false;avg:stage.ms%7Bstage=literal_or(summary)%7D%5Bavg%5D;0"
table_urls = (
    "  links (https://metrics.example/plot#now-1h,China-                         note\r\n"
    "         North6,false;max:stage.ms%7Bcard_\r\n"
    "         type=*%7D%7Bstage=literal_or(summary),status=literal_or(\r\n"
    "         success)%7D%5Bpct90%5D;0) / (https://metrics.example/plot#now-\r\n"
    "         1h,China-North6,false;avg:stage.ms%7Bstage=literal_or(summary)\r\n"
    "         %7D%5Bavg%5D;0)\r\n"
)
run_case("table-url-p90", table_urls, p90, 120, hint="a")
run_case("table-url-avg", table_urls, avg, 120, hint="b")
```

Update the existing `run_case` launcher at the same time so every hard-wrap
case uses the required source-pane interface from Task 2:

```python
launcher = shlex.join([
    root + "/target/release/tmux-thumbs",
    "--pane-id", source,
    "--dir", root,
    "--command", command,
])
```

If candidate ordering assigns different deterministic hints, update only the
fixture's explicit hint values after inspecting the overlay; never weaken the
expected full copied strings.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test --bin thumbs joins_parenthesized_metrics_urls_across_table_cell_lines
cargo test --bin thumbs rejects_ambiguous_parenthesized_table_url_continuations
cargo build --release --bins
bash tests/hard_wrap_tmux.sh
```

Expected: the positive unit test and both new real-tmux selections fail because
the URLs remain separate first-line candidates; the negative test should
either pass immediately or expose an existing over-join that must remain
fixed.

- [ ] **Step 3: Implement candidate-local bracket and column lookahead**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParenthesizedCellUrl {
  continuation_column: usize,
  close_line: usize,
}
```

Implement `parenthesized_cell_url` with this exact decision sequence:

1. Require `candidate.pattern == "url"` and a single initial span.
2. Read the character immediately before the initial span and require a supported opener.
3. For each consecutive following line, reject tmux soft wraps, blank lines, separator rows, list/prompt prefixes, metadata fields, and new URL prefixes.
4. Obtain the fragment with the existing `continuation_start` and `continuation_span` helpers.
5. Require the first continuation display column to be earlier than the initial URL column; require later fragments to use that same display column.
6. Append fragments to a probe string and use the existing delimiter stack behavior in `boundary_len`. Accept only when the first boundary-shortening character is the external closer matching the saved opener.
7. Return the continuation column and close-line index; otherwise return `None`.

In `join_candidate`, calculate this proof once before the loop. For lines up to its `close_line`, allow the proven table URL to bypass both `reaches_wrap_boundary` and `is_confident_continuation`. Keep all existing path and ordinary URL rules unchanged. Continue using `boundary_len` to trim the external closer from the last physical span.

- [ ] **Step 4: Run the URL/path unit suite and verify GREEN**

Run:

```bash
cargo test --bin thumbs url_path::tests
cargo build --release --bins
bash tests/hard_wrap_tmux.sh
```

Expected: all new and existing URL/path tests pass, the integration script
copies both exact full URLs, and parenthesized table paths, tool gutters,
punctuation boundaries, and soft wraps remain green.

- [ ] **Step 5: Commit semantic table URL joining**

```bash
git add src/url_path.rs tests/hard_wrap_tmux.sh
git commit -m "fix: join parenthesized table URLs"
```

---

### Task 4: Update Local Bindings and Complete Verification

**Files:**
- Modify: `/data00/home/lihao.hellohake/.config/nvim/dotfiles/tmux/.tmux.conf:181-187`
- Verify: `target/release/thumbs`
- Verify: `target/release/tmux-thumbs`

**Interfaces:**
- Consumes: `tmux-thumbs.sh <pane-id>` from Task 2.
- Produces: both direct copy-mode `f` bindings pass `#{pane_id}`.
- Produces: rebuilt release binaries used by the live plugin.

- [ ] **Step 1: Update both direct copy-mode bindings**

Change the two commands to:

```tmux
bind-key -T copy-mode-vi f run-shell -b "$HOME/.tmux/plugins/tmux-thumbs/tmux-thumbs.sh '#{pane_id}'"
bind-key -T copy-mode f run-shell -b "$HOME/.tmux/plugins/tmux-thumbs/tmux-thumbs.sh '#{pane_id}'"
```

Keep the existing comments and all unrelated tmux mappings unchanged.

- [ ] **Step 2: Run formatting and the complete automated verification set**

Run:

```bash
cargo fmt -- --check
cargo test
cargo build --release --bins
bash tests/copy_mode_snapshot_tmux.sh
bash tests/hard_wrap_tmux.sh
git diff --check
```

Expected: formatting passes, every Rust test passes, both real-tmux scripts print `ok`, both release binaries exist, and `git diff --check` is silent.

- [ ] **Step 3: Check for internal or workstation-specific fixture leakage**

Run:

```bash
rg -n 'code\.byted|/data00|lihao\.hellohake' src tests docs/superpowers/plans/2026-10-22-copy-mode-viewport-table-url-reliability.md
```

Expected: only the intentional absolute local-config path in this implementation plan may match; source and test fixtures contain no internal repository paths or usernames.

- [ ] **Step 4: Reload tmux and verify installed bindings**

Run:

```bash
tmux source-file /data00/home/lihao.hellohake/.config/nvim/dotfiles/tmux/.tmux.conf
tmux list-keys -T prefix | rg 'tmux-thumbs.*#\{pane_id\}'
tmux list-keys -T copy-mode-vi | rg 'tmux-thumbs.*#\{pane_id\}'
tmux list-keys -T copy-mode | rg 'tmux-thumbs.*#\{pane_id\}'
```

Expected: prefix `f` and both copy-mode `f` bindings call `tmux-thumbs.sh` with an expanded pane-ID format argument.

- [ ] **Step 5: Commit the local tmux binding update in its owning repository**

Run from `/data00/home/lihao.hellohake/.config/nvim`:

```bash
git add dotfiles/tmux/.tmux.conf
git commit -m "fix: pass source pane to tmux thumbs"
```

- [ ] **Step 6: Record final local state without pushing**

Run:

```bash
git -C /data00/home/lihao.hellohake/.tmux/plugins/tmux-thumbs status --short
git -C /data00/home/lihao.hellohake/.tmux/plugins/tmux-thumbs log -5 --oneline
git -C /data00/home/lihao.hellohake/.config/nvim status --short
git -C /data00/home/lihao.hellohake/.config/nvim log -3 --oneline
```

Expected: both worktrees are clean, the plugin log contains the viewport, launcher, URL, and integration commits, the dotfiles log contains the binding commit, and nothing has been pushed.
