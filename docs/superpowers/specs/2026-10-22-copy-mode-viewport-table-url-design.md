# Copy-Mode Viewport and Table URL Reliability Design

## Problem

`tmux-thumbs` currently has two independent failure modes behind the same
`prefix+f` workflow.

First, a picker opened from tmux copy mode can show a newer live page instead
of the page visible at invocation time. The existing stable-snapshot work
prevents a resize or history mutation from invalidating a snapshot after the
pane state has been read. It does not protect the initial target lookup or the
case where a pane is in copy mode but has no usable `scroll_position`.
`scroll_position` is optional today; when it is absent, capture silently omits
`-S` and `-E` and therefore captures the live pane.

Second, TraeX renders Markdown tables with hard newlines at cell boundaries.
A parenthesized Metrics URL may be split at arbitrary characters such as
`China-`, `car|d_type`, or an encoded query value. The existing URL joiner only
accepts terminal-edge wrapping. The table-cell fallback only handles paths
ending in `/` whose matching close bracket appears on the next line. The URL
therefore receives multiple hints and selecting any one hint copies only a
fragment.

The two symptoms share an entry point but not a root cause. Viewport capture
belongs to `src/swapper.rs`; logical URL reconstruction belongs to
`src/url_path.rs`.

## Goals

- A picker invoked from copy mode must represent exactly the visible copy-mode
  viewport, or fail without opening an overlay.
- Snapshot capture must use the invoking pane, not a pane inferred from mutable
  global active-window state.
- A parenthesized URL hard-wrapped inside one table cell must become one logical
  candidate with one hint and an exact copied value.
- Independent URLs on the same row must remain independent candidates.
- Existing live-pane, soft-wrap, path, filename, custom-regexp, clipboard,
  zoom, and concurrent-invocation behavior must remain unchanged.

## Non-Goals

- Reconstruct arbitrary table cells or prose across lines.
- Join custom regular expressions across lines.
- Remove all newlines from copied content.
- Change OSC52 or the configured final copy command.
- Replace the detached-window picker architecture.

## Considered Approaches

### A. Explicit pane identity, fail-closed viewport, semantic URL lookahead

Bind the invocation to the source pane, represent live and copy-mode viewports
as distinct validated states, and reject missing copy-mode coordinates. Extend
the existing URL/path scanner with a lookahead that proves a parenthesized
table-cell URL reaches its matching close bracket before joining arbitrary
hard-wrapped fragments.

This is the selected approach. It fixes each failure at its source and keeps
the matching scope narrow.

### B. Keep active-pane discovery and add retries

Retrying the current lookup may hide transient failures, but a consistently
wrong active-pane context remains stable and would still be accepted. It also
does not distinguish a valid live pane from a copy-mode pane whose scroll
position was lost. This approach is rejected because it preserves the silent
fallback.

### C. Capture all history and rediscover the visible page later

The picker could capture the complete history and search for content near the
copy cursor. This adds latency and ambiguity when TUI frames repeat, as they do
in the reported recording. It also enlarges the candidate set beyond what the
user can see. This approach is rejected.

## Design: Source Pane and Viewport Contract

The tmux binding will expand `#{pane_id}` at invocation time and pass it as the
first argument to `tmux-thumbs.sh`. The script will forward it to a new
`tmux-thumbs --pane-id <id>` option. An empty target is an error; the script
will not fall back to `$TMUX_PANE` because tmux `run-shell` can inherit a stale
server-start environment value rather than the invoking pane.

The compiled swapper will query state with `tmux display-message -p -t
<pane-id>` and capture that same target. It will no longer scan a default
window and choose whichever row currently reports `pane_active`. The user's
direct copy-mode bindings will be updated to pass `#{pane_id}` through the
same launcher contract as the normal prefix binding.

Pane state will distinguish these cases:

```text
Live { pane_id, width, height, history_size, zoomed }
CopyMode { pane_id, width, height, history_size, scroll_position, zoomed }
```

`pane_mode=copy-mode` requires a non-empty, non-negative
`scroll_position`. A pane reporting copy mode without that coordinate is an
invalid snapshot source, not a live pane. Other unsupported pane modes also
fail closed rather than capturing unrelated live contents.

For a valid copy-mode state, physical and joined captures use the same range:

```text
start = -scroll_position
end   = pane_height - scroll_position - 1
```

The existing before/capture/after loop remains, but compatibility now includes
the explicit target identity, exact pane mode, dimensions, history size, and
copy-mode scroll position. A state change retries the entire pair of physical
and joined captures. Missing or invalid viewport data does not retry as live
capture.

After three unstable attempts, or immediately for an invalid copy-mode state,
the swapper removes its private runtime directory, opens no `[thumbs]` window,
and displays a concise retry message:

```text
tmux-thumbs: copy-mode viewport unavailable; retry
```

Normal live-pane invocation still captures the current visible pane without
`-S` or `-E`.

## Design: Parenthesized Table URL Joining

The scanner will add a URL-specific table-cell proof before relaxing the
normal terminal-edge and token-boundary checks. A URL qualifies only when all
of these conditions hold:

1. The first URL span begins immediately after `(`, `[`, `（`, or `【`.
2. The first continuation returns to an earlier display column, proving a
   table-cell continuation rather than ordinary prose following the URL.
3. Later continuation fragments begin at the same table-cell display column.
4. Every intermediate fragment is non-empty URL text with no blank line,
   separator row, list item, prompt, metadata field, or new URL prefix.
5. Looking ahead through consecutive continuation lines finds the matching
   external close bracket. Internal balanced parentheses such as
   `literal_or(summary)` do not close the wrapper.
6. The external close bracket is reached before the row or cell continuation
   becomes ambiguous.

Once this proof succeeds, the existing `join_candidate` loop may accept
arbitrary ASCII URL fragments until the proven close line. Only physical
newlines and continuation indentation are removed. The external wrapper is not
copied, while balanced URL-internal delimiters are preserved.

The proof is candidate-local. A line containing `P90 (<url>) / AVG (<url>)`
produces two independent candidates, each with its own spans and hint.

If lookahead cannot prove the closing boundary, matching fails closed and
retains the existing single-line fragments.

## Error Handling

- Missing or nonexistent invoking pane: display
  `tmux-thumbs: source pane unavailable; retry` and create no picker window.
- Copy mode with missing or invalid scroll position: show the viewport retry
  message and do not capture the live pane.
- State mutation during capture: retain the existing bounded three-attempt
  retry behavior.
- Capture or picker creation failure: retain the existing cleanup and messages.
- Ambiguous table continuation: do not join it; preserve current candidates.

## Verification

### Swapper unit tests

- Parse a valid live state and a valid copy-mode state.
- Reject copy mode with an empty, negative, or malformed scroll position.
- Prove every state query and capture command targets the supplied pane ID.
- Prove copy-mode range calculation emits the expected `-S` and `-E` values.
- Prove an invalid viewport never calls `capture-pane` or `new-window`.

### Real tmux snapshot tests

- Use an isolated `tmux -L <socket> -f /dev/null` server.
- Create distinct source and distractor panes so active-pane discovery cannot
  accidentally pass.
- Put a marker on an older copy-mode page and a different marker on the live
  page; assert the overlay contains only the older visible marker.
- Simulate an unavailable copy-mode scroll field; assert no `[thumbs]` pane is
  created and the failure is reported.
- Retain the existing delayed-capture resize test to cover the original race.

### URL matcher tests

- Reproduce the Metrics table shape from the recording, including arbitrary
  splits, encoded braces, and internal `literal_or(...)` parentheses.
- Join one URL across three or more table-cell lines into one exact value.
- Keep the P90 and AVG URLs on the same logical row independent.
- Keep right-hand explanatory columns out of the copied value.
- Reject missing close brackets, changing continuation columns, blank lines,
  separator rows, prompts, metadata fields, and a new URL prefix.
- Extend the real-tmux hard-wrap integration test to select the reconstructed
  URL and assert the exact tmux buffer.

Finally run the full Rust test suite, the hard-wrap integration test, and the
copy-mode snapshot integration test, then rebuild both release binaries used by
the plugin.
