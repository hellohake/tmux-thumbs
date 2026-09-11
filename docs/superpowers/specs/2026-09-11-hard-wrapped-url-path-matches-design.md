# Hard-Wrapped URL and Path Matches Design

## Goal

Make one logical tmux-thumbs candidate out of a URL or file path that a TUI
has hard-wrapped across physical lines, while preserving the existing behavior
for custom patterns, commands, hashes, numbers, and other single-line matches.
URL and path candidates must also exclude surrounding ASCII/CJK punctuation and
prose.

## Scope

This change applies only to the built-in URL and path categories. It supports:

- HTTP, Git, SSH, FTP, and file URLs already recognized by tmux-thumbs.
- Relative, absolute, `~/`, `./`, and `../` paths.
- File locations of the form `path:line` and `path:line:column`.
- One logical candidate spanning two or more consecutive physical lines.
- Balanced parentheses and brackets inside a URL or path, including
  `wiki/Foo_(bar)` and `app/(main)/[id]/index.tsx`.
- ASCII and CJK surrounding brackets and sentence punctuation.

The change does not join ordinary text, shell commands, hashes, numbers, or
arbitrary custom regular expressions across lines. It does not require a path
to exist on disk. It does not broaden the existing path matcher into a general
Unicode filename parser.

## Observed Failures

The current pipeline splits captured pane content before matching:

1. `src/swapper.rs` runs `tmux capture-pane -J`.
2. `src/main.rs` splits the result on `\n`.
3. `src/state.rs` applies every pattern to one physical line at a time.
4. `Match` stores only one `(x, y)` position and one borrowed `&str`.
5. `src/view.rs` paints one contiguous string and assigns a hint to that
   single-line match.

`capture-pane -J` can join tmux soft wraps, but it cannot undo a newline written
by the Coding Agent TUI. A hard-wrapped path therefore becomes two candidates
before a regular expression can see the complete value.

The built-in URL expression has a separate boundary bug because it consumes
every non-space character. The three screenshots in the `debug` Feishu Wiki
document (Wiki node `AGtwwgh0CiC4m3kZoWwckdPhn4e`, Docx
`QKrpdlOyKoFRmex1ectcc4cznAf`, revision 1264) demonstrate both problem classes.
In particular, these source strings are currently overmatched:

```text
(https://github.com/fcsonline/tmux-thumbs)最后一次合并提交是 2023-03-21。
(https://github.com/fcsonline/tmux-thumbs/releases/tag/0.8.0)，发布于 2023-03-11。
```

The selected values must end before the outer `)` / `），` and must not contain
the immediately adjacent Chinese prose. The path shown in the same document:

```text
(.ai_doc/records/inbox/
  life-card-admin-20260911-01a08c45/handoff.md)。
```

must produce exactly one candidate whose copied value is:

```text
.ai_doc/records/inbox/life-card-admin-20260911-01a08c45/handoff.md
```

## Considered Approaches

### A. Merge existing per-line matches

This is the smallest patch, but it cannot recover a continuation that is not a
valid standalone path or URL. It also leaves coordinate and punctuation logic
spread across unrelated matching results. This approach is rejected.

### B. Dedicated URL/path scanner with screen spans

Keep the physical screen lines unchanged for rendering, but scan built-in URLs
and paths with a dedicated component that may return multiple screen spans for
one owned value. Keep all other patterns on the existing per-line path. This is
the selected approach because it solves extraction, copying, hint allocation,
and highlighting without replacing the entire matching engine.

### C. Whole-screen offset model

Flatten every pattern into a whole-screen coordinate space, similar in spirit
to upstream PR #140. This is more general but expands the regression surface
for every existing match category and requires substantially more rendering
work. This approach is rejected for the fork's first fix.

## Data Model

`Match` becomes an owned logical candidate with one or more physical spans:

```rust
struct ScreenSpan {
  line: usize,
  start: usize,
  end: usize,
}

struct Match {
  pattern: &'static str,
  text: String,
  spans: Vec<ScreenSpan>,
  hint: Option<String>,
}
```

`start` and `end` are UTF-8 byte offsets into the corresponding original
physical line. Byte offsets make slicing exact; terminal columns are derived
only when rendering with `unicode-width`. A match owns `text` because removing
a newline and continuation indentation means the selected value is no longer a
single slice of the captured input.

Single-line legacy matches are represented by one span and an owned copy of
their captured text. This keeps hint selection and output formatting uniform.

## Pane Width Propagation

Hard-wrap recognition needs the source pane width, not the width of the temporary
thumbs window. `Swapper::capture_active_pane` will request `#{pane_width}` with
the existing pane metadata and store it. `Swapper::execute_thumbs` will pass the
value to a new `thumbs --pane-width <columns>` argument.

When `--pane-width` is absent, zero, or invalid, cross-line joining is disabled
and matching remains single-line. This preserves direct `thumbs` usage and
makes the fallback fail closed.

## Candidate Extraction

Matching has two paths:

1. Exclude patterns, custom regular expressions, and all non-URL/path built-ins
   retain the current line-by-line priority behavior.
2. Built-in URL/path candidates are produced by the dedicated scanner.

The scanner first finds an anchored URL or path on one physical line. It may
continue onto the next physical line only when all of these signals hold:

- The candidate reaches the last non-whitespace byte of the current line.
- The current line's CJK display width reaches the supplied pane width.
- The next line is consecutive, non-empty, and begins with indentation.
- The content after indentation begins with a character valid for the current
  URL/path category.
- The next line is not a list item, status bullet, shell prompt, command prompt,
  new URL, or unambiguously independent rooted path.
- For an ambiguous path boundary, the join is accepted only when syntax on one
  side makes continuation clear, such as a trailing `/`, `.`, `:`, `-`, `_`,
  or an incomplete path/location component. Ambiguous input fails closed as two
  candidates.

The same decision is repeated for a third or later physical line. The scanner
removes only the physical newline and continuation indentation when building
`text`; it records the exact visible portion of every line as a `ScreenSpan`.

After all candidates are collected, a cross-line URL/path candidate suppresses
single-line fragments fully covered by its spans. Remaining candidates retain
screen order and the existing custom-pattern priority for non-overlapping
matches.

## URL and Path Boundaries

URL scanning uses an ASCII URL character set plus percent escapes instead of a
blanket `[^\s]+`. Non-ASCII prose terminates the URL; internationalized values
remain representable through standard percent encoding. This intentionally
prevents Chinese prose adjacent to a URL from becoming part of the candidate.

Parentheses and square brackets are tracked while scanning:

- A closing bracket with a matching opening bracket inside the candidate is
  retained, so `https://host/wiki/Foo_(bar)` remains complete.
- A closing bracket with no matching opening bracket inside the candidate is an
  outer delimiter and terminates the candidate. This covers both `)` and `）`.
- Bracket tracking is applied before trailing punctuation cleanup.

At the end of a candidate, unmatched surrounding brackets and sentence-ending
`，。；：！、,.;!` are removed. A terminal `?` is retained when it starts or
belongs to a URL query and otherwise treated as sentence punctuation. Path
location suffixes `:line[:column]` are parsed before punctuation cleanup so the
numeric suffix is preserved.

The span endpoints are shortened together with `text`; punctuation is never
highlighted merely because it was trimmed from the copied value.

## Rendering and Selection

`View::render` will repaint each `ScreenSpan` using the exact substring from the
original line. It will calculate a span's terminal column as the CJK display
width of the line prefix, rather than deriving a column from UTF-8 byte counts.
This covers Chinese prefixes without changing stored offsets.

Only one hint is assigned to a logical match:

- `left` and `off_left` anchor at the first span.
- `right` and `off_right` anchor at the final span.
- Every span receives the same selected, multi-selected, or normal highlight
  color, but only the anchor span receives the hint text.

Selection and unique-match handling compare the owned logical `text`. The
chosen value written to the target file is the normalized logical value, so no
change is needed in `swapper`'s final copy command.

`render` will write through its supplied `Write` object rather than bypassing it
with `print!`. This makes the multi-span terminal output testable and preserves
the same production stdout behavior.

## Compatibility and Configuration

The fork remains based on `ae91d5f` and will not upgrade dependencies as part of
this work. `unicode-width` is already present. Existing custom regular
expressions remain single-line and retain their documented priority.

The user's current `@thumbs-regexp-1`, `@thumbs-regexp-2`, and
`@thumbs-regexp-3` can still shadow the improved built-in URL/path categories.
The fork will be implemented and verified first. Switching the real tmux config
to the fork and removing or narrowing those three expressions is a separate,
explicit rollout step. The live TPM checkout is not modified during feature
development.

## Testing

Tests are written before production changes and cover:

1. The two-line `.ai_doc` path from the screenshot, including `)。`.
2. Hard-wrapped absolute, relative, `~/`, `./`, and `../` paths.
3. Hard-wrapped URLs surrounded by ASCII and CJK brackets/punctuation.
4. The exact `)最后一次合并提交` and `)，发布于` URL overmatches from the
   Feishu screenshots.
5. Internal URL parentheses and path `(main)` / `[id]` components.
6. `path/file.go:42:7` across a physical line boundary.
7. Adjacent independent paths, list items, bullets, prompts, and prose that must
   not join.
8. A CJK prefix whose byte count differs from its terminal display width.
9. One logical URL/path across three lines with one hint and three highlighted
   spans.
10. Pane-width discovery and command propagation in `swapper`.
11. All 34 existing tests.

Final verification is:

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
```

An isolated tmux socket will then create a narrow pane, print hard-newline URL
and path fixtures, run the fork's release binary, select the sole logical hint,
and assert the tmux buffer. The check must confirm the copied text, one hint per
logical candidate, and correct highlighting of every physical span.

## Delivery Boundaries

Implementation should remain reviewable as focused changes: candidate model and
scanner, multi-span rendering, boundary cleanup, and integration verification.
Do not cherry-pick all of PR #140, do not perform dependency upgrades or broad
refactors, and do not edit or delete the user's live TPM directory.
