# Filename Candidate Scanner Design

This document extends the accepted hard-wrap design in
`2026-09-11-hard-wrapped-url-path-matches-design.md`. Where filename candidate
classification is concerned, this document is authoritative.

## Goal

Recognize complete file references even when they do not contain a slash, while preserving the established behavior for URLs, rooted and relative paths, custom patterns, SHA values, numbers, prose, domains, versions, tree output, and hard-wrapped candidates.

The motivating failure is:

```text
skill-eval-review-report-20260912.md
```

The existing path matcher requires `/`, so it does not see this token. The embedded `20260912` is eight hexadecimal characters, so the earlier `sha` fallback receives the hint. The selected value is therefore `20260912` instead of the complete filename.

## Scope

The scanner recognizes three file-reference shapes:

1. URL: a recognized scheme such as `https://`, `ssh://`, or `file:///`.
2. Path: a token with a path root or separator, including absolute paths, `~/`, `./`, `../`, Unicode components, globs, and `:line[:column]` suffixes.
3. Bare filename: a filename token without `/`, such as `README.md`, `main.rs`, `archive.tar.gz`, or `skill-eval-review-report-20260912.md`.

Bare filenames are single-line candidates in this change. Cross-line joining remains limited to URL and path candidates; a line ending in `report-` followed by `20260912.md` is ambiguous and fails closed.

The scanner does not treat arbitrary dotted text as a filename. In particular, it must reject:

- domain names such as `host.example.com`;
- versions such as `1.2.3` or `v1.2.3`;
- decimal values and IP addresses;
- ordinary prose or identifiers without filename evidence;
- metadata values and tree decorations as part of a preceding path.

## Selected Architecture

Use a dedicated `filename` module rather than another monolithic regular expression.

```rust
pub struct FilenameMatch {
  pub start: usize,
  pub end: usize,
}

pub fn scan(line: &str) -> Vec<FilenameMatch>;
```

The module performs four explicit stages:

1. Tokenize candidate spans using delimiter-aware character scanning.
2. Trim outer punctuation while preserving balanced internal parentheses and brackets.
3. Classify the token using both the complete token and its left/right context
   in the supplied line: basename, known-extension filename, strong-context
   unknown-extension filename, domain/version, or non-file.
4. Return exact UTF-8 byte spans; `State` turns accepted spans into normal `Match { pattern: "path", ... }` values.

This keeps selection, rendering, uniqueness, and copied output on the existing path contract while isolating filename-specific policy.

## Tokenization and Boundaries

The scanner walks every physical line and identifies maximal filename-like tokens. It does not consume delimiters, so multiple filenames on one line remain independently discoverable.

Allowed internal characters include ASCII letters and digits plus `_`, `-`, `.`, `@`, `+`, `$`, `%`, `*`, balanced `()` and `[]`, and an optional `:line[:column]` suffix. Bare filenames remain ASCII-only in v1; Unicode filenames continue to work when they appear in a rooted or slash-containing path.

Accepted left contexts:

- line start or whitespace;
- sentence punctuation or opening brackets;
- ordinary prose immediately preceding a filename when the token itself has strong filename evidence.

Tree or tool decorations such as `└`, `├`, `│`, and `┌` are not copied. A filename after tree decoration is still an independent candidate, as the user explicitly requested; it is never joined to a path on the previous line.

Accepted right contexts:

- line end or whitespace;
- sentence punctuation;
- closing ASCII/CJK brackets or quotes.

A candidate may not stop inside a longer alphanumeric/dotted token. This prevents `host.example.com` from being partially accepted as `host.example.c` and prevents a short extension from truncating `.jsx`, `.tsx`, `.cpp`, `.hpp`, or `.kts`.

## Classification Policy

### Known basenames

A centralized, small set covers conventional extensionless project files:

- `README`, `LICENSE`, `NOTICE`, `CHANGELOG`, `Makefile`, `Dockerfile`, `Gemfile`, `Rakefile`.
- Matching is case-insensitive where convention permits, while the copied text preserves original case.

### Known extensions

A centralized set covers common engineering artifacts:

- documentation/text: `md`, `markdown`, `txt`, `rst`, `adoc`, `log`;
- structured/config: `json`, `yaml`, `yml`, `toml`, `xml`, `csv`, `ini`, `conf`, `cfg`, `env`, `lock`;
- source/schema: `rs`, `go`, `py`, `js`, `jsx`, `ts`, `tsx`, `java`, `kt`, `kts`, `c`, `cc`, `cpp`, `h`, `hpp`, `sh`, `bash`, `zsh`, `fish`, `lua`, `vim`, `sql`, `proto`, `thrift`, `idl`, `html`, `css`, `scss`, `less`;
- build/module/archive: `mod`, `sum`, `tar`, `gz`, `tgz`, `zip`.

Composite names such as `archive.tar.gz` are accepted as one token. The list lives in one constant and is exercised by table-driven tests.

### Unknown extensions

An unknown extension is accepted only in a strong file context visible in the
same line, for example:

- Markdown link destination;
- explicit phrases such as `文件`, `文档`, `file`, or `written to` adjacent to the token;
- an existing URL/path candidate's final component; this is normally handled
  by the path scanner and does not create a second overlapping filename.

V1 does not add broad natural-language context inference. Unknown, standalone dotted tokens fail closed.

### Domain and version rejection

A token is rejected as a domain when its final label resembles a common domain suffix and the token contains multiple domain-like labels without file context. At minimum, reject common suffixes such as `com`, `org`, `net`, `io`, `dev`, `cn`, and `internal`.

A token is rejected as a version when all dot-separated components are numeric, optionally prefixed by `v`. Existing IP and numeric patterns retain their current priority.

## Candidate Priority and Overlap

Candidate extraction order is:

1. Exclude sequences and custom regexps.
2. Markdown URL and URL.
3. Slash-containing path.
4. Bare filename scanner.
5. Existing specialized values: docker digest, color, UUID, IPFS, SHA, IP, IPv6, address, and number.

All collectors return candidates without assigning hints. A single merge step
then resolves every overlap. When candidates overlap, the winner is selected
by:

1. lower screen start offset;
2. higher semantic priority from the order above;
3. longer span at the same start and priority.

A winning URL/path/filename suppresses every lower-priority candidate fully contained in its spans. Therefore `skill-eval-review-report-20260912.md` wins over the embedded `sha("20260912")` without changing standalone SHA behavior.

Custom regexp priority remains unchanged. If a custom regexp starts at the same position as a filename, the custom regexp wins.

## Tree and Tool Output

The scanner treats tree decoration separately from filename text:

```text
└ result.txt
├ config.yaml
│ README.md
```

Each filename is a standalone candidate with a span beginning after the decoration. It is not appended to a preceding directory candidate. This satisfies the explicit user decision while keeping hard-wrap joining fail closed.

Tool-output gutters such as `  │ ` retain their existing hard-wrap semantics only when line alignment and width prove a continuation. A filename discovered after a terminating tree branch (`└`/`├`) is never used as continuation evidence.

## Data Flow

```text
physical screen lines
  ├─ legacy regex collector: custom + URL + slash path + SHA/number/etc.
  ├─ filename::scan: independent bare filename candidates
  └─ joined capture mapping: tmux soft-wrap reconstruction
          ↓
normalize URL/path/filename boundaries
          ↓
merge by screen position + semantic priority + span length
          ↓
hard-wrap join for URL/slash-path only
          ↓
one logical candidate → one hint → exact copied text
```

The scanner does not alter `ScreenSpan` or rendering contracts.

## Failure Handling

- Ambiguous dotted tokens remain unmatched instead of becoming false-positive filenames.
- Unknown extensions remain unmatched unless strong context exists.
- Invalid UTF-8 boundaries are impossible because scanning advances by `char_indices` and stores byte offsets.
- Missing pane width disables cross-line joining but does not disable single-line filename recognition.
- Custom regexp compilation errors retain existing behavior.

## Testing

Required automated coverage:

1. The screenshot token `skill-eval-review-report-20260912.md` is one `path` candidate and suppresses the internal SHA.
2. The same terminal text selects and copies the complete filename in an isolated tmux server.
3. Multiple bare filenames on one line are independently matched.
4. Composite and prefix-related extensions: `archive.tar.gz`, `component.jsx`, `view.tsx`, `code.cpp`, `header.hpp`, `build.kts`.
5. Known extensionless basenames: `README`, `LICENSE`, `Makefile`, `Dockerfile`.
6. Outer punctuation is excluded; internal `report(final).md` remains intact.
7. `file.rs:42:7` retains its location suffix.
8. Domains `host.example.com`, versions `1.2.3` / `v1.2.3`, IPs, prose, and unknown dotted tokens do not become filenames.
9. `DOC_OK lines=268 bytes=14797 report-20260912` retains existing number/SHA candidates.
10. Tree entries such as `└ result.txt` are standalone candidates and are not joined to a previous directory.
11. Existing URL/path hard-wrap, CJK coordinate, custom regexp, priority, and isolated tmux tests remain green.

Final verification remains:

```bash
cargo fmt --check
cargo test --locked
cargo build --release --locked
bash tests/hard_wrap_tmux.sh
```

## Compatibility and Rollout

No tmux configuration change is required. Bare filenames are emitted with `pattern: "path"`, so existing copy commands, hint colors, reverse/unique settings, and URL/path rendering continue to work.

After verification:

1. Commit and push the fork.
2. Fast-forward `~/.tmux/plugins/tmux-thumbs` to the new fork commit.
3. Rebuild the release binary explicitly.
4. Re-run the isolated tmux test from the TPM checkout.
5. No tmux restart or `source-file` is required because the config and binding do not change.

## Delivery Boundaries

- Do not remove or weaken standalone SHA/number matching.
- Do not classify every dotted token as a file.
- Do not rely on filesystem existence.
- Do not add cross-line joining for bare filenames in this change.
- Do not modify the user's tmux or Neovim configuration for this change.
