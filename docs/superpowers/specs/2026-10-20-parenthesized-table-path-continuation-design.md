# Parenthesized Table Path Continuation Design

## Problem

Table renderers can wrap a long path at the edge of a left-hand cell rather
than at the terminal edge. The first physical line may contain another column,
and a continuation line may return to the start of the left cell. The existing
matcher only joins candidates that reach the visible terminal line end, so it
copies fragments such as `src/engine/materials/data_loader/`.

A single table cell can contain more than one wrapped path:

```text
handleResult:47 (src/engine/materials/data_loader/          reports metrics
  data_loader_helper.go:47) -> relay:70 (src/engine/tracer/
  request_tracer.go:70)
```

The two paths must remain independent candidates.

## Intended behavior

The matcher returns exactly these two values:

```text
src/engine/materials/data_loader/data_loader_helper.go:47
src/engine/tracer/request_tracer.go:70
```

It must not copy the wrapper parentheses, the arrow, the relay label, or the
right-hand table column.

## Design

Keep the existing terminal-edge rule and add a conservative semantic boundary
inside `join_candidate`. A non-edge continuation is accepted only when:

- the accumulated path ends in `/`;
- the path begins immediately after an opening bracket;
- the next candidate starts at an earlier display column, indicating a return
  to the left cell;
- the next candidate ends with, or is immediately followed by, the matching
  closing bracket.

The check runs per path candidate. Existing tokenization stops the first
continuation at whitespace after its closing bracket, so following prose and a
second wrapped path are scanned independently.

## Fixture privacy

Replace repository-specific absolute paths in `src/url_path.rs` and
`tests/hard_wrap_tmux.sh` with neutral `/workspace/...` fixtures while keeping
the same path lengths, wrap shapes, metadata boundaries, and expected values.

## Verification

- A focused Rust test must fail before the production change and pass after it.
- A negative Rust case preserves separate fragments when no matching closer is
  present.
- The real-tmux integration test selects both wrapped paths independently.
- Formatting, the complete Rust suite, existing tmux suites, release build, and
  an isolated reproduction using the reported table shape must pass.
