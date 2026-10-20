# Path Wrapper Boundary Design

## Problem

When a location path is wrapped by prose punctuation, for example:

```text
(src/engine/materials/data_loader/data_load_executor.go:120):
```

the path regular expression stops after the numeric location suffix, before the
closing `)`. The current boundary normalizer therefore receives an unmatched
leading `(` and copies it as part of the path.

## Intended behavior

Copy the path and its location suffix without the surrounding punctuation:

```text
src/engine/materials/data_loader/data_load_executor.go:120
```

Balanced parentheses that belong to a path component, such as
`(generated)/main.go` or `app/(main)/index.tsx`, must remain intact.

## Design

Keep path recognition and copy cleanup in the built-in matcher. Extend
`boundary_range` so a leading bracket is treated as outer decoration when the
candidate has no matching closer for that first bracket. Existing balanced
outer wrappers continue to be removed as a pair, while balanced path components
remain unchanged.

## Verification

- A focused Rust unit test reproduces the location-suffix case and asserts both
  copied text and screen span.
- Existing parenthesis and internal-delimiter tests protect legitimate path
  components.
- The complete Rust suite and the real-tmux integration suites remain green.
- An isolated tmux session confirms the release binary highlights and copies the
  exact expected value.
