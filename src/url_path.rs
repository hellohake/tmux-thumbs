use super::state::{Match, ScreenSpan};
use unicode_width::UnicodeWidthStr;

pub fn normalize(
  lines: &[&str],
  mut matches: Vec<Match>,
  pane_width: Option<usize>,
  soft_wrapped: &[bool],
) -> Vec<Match> {
  matches.retain(|candidate| candidate.pattern != "path" || is_meaningful_path(&candidate.text));

  let width = match pane_width.filter(|width| *width > 0) {
    Some(width) => width,
    None => return matches,
  };

  let mut joined = Vec::new();
  for candidate in matches
    .iter()
    .filter(|candidate| candidate.pattern == "url" || candidate.pattern == "path")
  {
    if joined
      .iter()
      .any(|joined_candidate| overlaps(joined_candidate, candidate))
    {
      continue;
    }

    if let Some(joined_candidate) = join_candidate(lines, candidate, width, soft_wrapped) {
      joined.push(joined_candidate);
    }
  }

  if joined.is_empty() {
    return matches;
  }

  matches.retain(|candidate| {
    !joined
      .iter()
      .any(|joined_candidate| overlaps(joined_candidate, candidate))
  });
  matches.extend(joined);
  matches.sort_by_key(|candidate| {
    let anchor = candidate.anchor();
    (anchor.line, anchor.start)
  });

  matches
}

fn join_candidate(lines: &[&str], candidate: &Match, pane_width: usize, soft_wrapped: &[bool]) -> Option<Match> {
  let mut joined = candidate.clone();
  let mut current_line = candidate.spans.last()?.line;

  while let Some(next) = lines.get(current_line + 1) {
    if soft_wrapped.get(current_line).copied().unwrap_or(false) {
      break;
    }
    let current = lines[current_line];
    let mut pending = joined.clone();
    restore_join_separator(current, &mut pending);
    let start = match continuation_start(current, next) {
      Some(start) => start,
      None => break,
    };
    let span = match continuation_span(pending.pattern, next, start, &pending.text) {
      Some(span) => ScreenSpan {
        line: current_line + 1,
        ..span
      },
      None => break,
    };
    let token = &next[span.start..span.end];
    if !reaches_wrap_boundary(current, pending.spans.last().unwrap(), token, pane_width)
      || !is_confident_continuation(pending.pattern, &pending.text, token)
    {
      break;
    }

    let combined = format!("{}{}", pending.text, token);
    let end = boundary_len(&combined, pending.pattern);
    if end <= pending.text.len() {
      break;
    }
    let mut span = span;
    span.end = span.start + end - pending.text.len();
    pending.text = combined[..end].to_string();
    pending.spans.push(span);
    joined = pending;
    current_line += 1;
  }

  if joined.spans.len() == candidate.spans.len() {
    return None;
  }
  let (start, end) = boundary_range(&joined.text, joined.pattern);
  let mut leading = start;
  let mut trailing = joined.text.len() - end;
  for span in &mut joined.spans {
    let removed = leading.min(span.end - span.start);
    span.start += removed;
    leading -= removed;
  }
  for span in joined.spans.iter_mut().rev() {
    let removed = trailing.min(span.end - span.start);
    span.end -= removed;
    trailing -= removed;
  }
  joined.spans.retain(|span| span.start < span.end);
  joined.text = joined.text[start..end].to_string();
  (!joined.spans.is_empty()).then_some(joined)
}

fn continuation_start(current: &str, next: &str) -> Option<usize> {
  let trimmed = next.trim_start_matches([' ', '\t']);
  let indent = next.len() - trimmed.len();
  if indent == 0 || trimmed.is_empty() {
    return None;
  }
  if let Some(rest) = trimmed.strip_prefix("│ ") {
    let current_trimmed = current.trim_start_matches([' ', '\t']);
    let current_indent = current.len() - current_trimmed.len();
    let same_gutter = current_trimmed.starts_with("│ ") && current_indent == indent;
    let tool_header = current_trimmed.starts_with("◆ Ran ") && indent == current_indent + 2;
    if !same_gutter && !tool_header {
      return None;
    }
    return Some(next.len() - rest.trim_start_matches([' ', '\t']).len());
  }
  if current.trim_start().starts_with("│ ") {
    return None;
  }
  Some(indent)
}

fn restore_join_separator(line: &str, candidate: &mut Match) {
  if candidate.text.ends_with('.') || candidate.text.ends_with(':') {
    return;
  }

  let span = candidate.spans.last_mut().unwrap();
  let separator = line[span.end..].chars().next();
  let should_restore = match separator {
    Some('.') => true,
    Some(':') => candidate.pattern == "path",
    _ => false,
  };
  if should_restore {
    let separator = separator.unwrap();
    span.end += separator.len_utf8();
    candidate.text.push(separator);
  }
}

fn reaches_wrap_boundary(line: &str, span: &ScreenSpan, continuation: &str, pane_width: usize) -> bool {
  let visible = line.trim_end_matches(char::is_whitespace);
  span.end == visible.len()
    && (visible.width() >= pane_width
      || (has_strong_path_structure(continuation) && visible.width() + continuation.width() > pane_width))
}

fn continuation_span(pattern: &str, line: &str, start: usize, accumulated: &str) -> Option<ScreenSpan> {
  let trimmed = &line[start..];
  if trimmed.is_empty() || is_blocked_continuation(pattern, trimmed, accumulated) {
    return None;
  }

  let end_in_trimmed = trimmed
    .char_indices()
    .take_while(|(_, ch)| is_candidate_char(pattern, *ch))
    .last()
    .map(|(index, ch)| index + ch.len_utf8())?;
  let remainder = &trimmed[end_in_trimmed..];

  let token = &trimmed[..end_in_trimmed];
  if remainder.starts_with(char::is_whitespace) && !has_strong_path_structure(token) {
    return None;
  }

  Some(ScreenSpan {
    line: 0,
    start,
    end: start + end_in_trimmed,
  })
}

fn is_blocked_continuation(pattern: &str, text: &str, accumulated: &str) -> bool {
  let blocked_prefixes = ["- ", "* ", "+ ", "◆", "•", "$ ", "# ", "> ", "% "];
  if blocked_prefixes.iter().any(|prefix| text.starts_with(prefix)) {
    return true;
  }
  if starts_metadata_field(text) {
    return true;
  }

  if ["http://", "https://", "git://", "ssh://", "ftp://", "file:///", "git@"]
    .iter()
    .any(|prefix| text.starts_with(prefix))
  {
    return true;
  }

  if pattern != "path" {
    return false;
  }

  if text.starts_with('/') {
    let suffix = text.split_whitespace().next().unwrap_or(text);
    return accumulated.ends_with('/')
      || path_looks_complete(accumulated)
      || !path_looks_complete(suffix)
      || suffix[1..].contains('/');
  }

  text.starts_with("~/") || text.starts_with("./") || text.starts_with("../")
}

fn starts_metadata_field(text: &str) -> bool {
  let (label, value) = match text.split_once(':') {
    Some(parts) => parts,
    None => return false,
  };

  !label.is_empty()
    && label.chars().any(|ch| ch.is_ascii_alphabetic())
    && label
      .chars()
      .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | '_' | '-'))
    && (value.is_empty() || value.starts_with(char::is_whitespace))
}

fn path_looks_complete(path: &str) -> bool {
  let component = path.rsplit('/').next().unwrap_or(path);
  component
    .rfind('.')
    .map(|index| index > 0 && index + 1 < component.len())
    .unwrap_or(false)
}

fn is_candidate_char(pattern: &str, ch: char) -> bool {
  if pattern == "url" {
    ch.is_ascii_graphic()
  } else {
    ch.is_alphanumeric()
      || matches!(
        ch,
        '.' | '_' | '-' | '@' | '$' | '~' | '%' | '+' | '*' | '[' | ']' | '(' | ')' | '/' | ':'
      )
  }
}

fn is_confident_continuation(pattern: &str, accumulated: &str, token: &str) -> bool {
  if is_numeric_location_suffix(token) {
    return pattern == "path" && accumulated.ends_with(':');
  }
  if pattern == "path" {
    if accumulated.ends_with(':') {
      return false;
    }
    if !accumulated.ends_with(['/', ':', '.', '-', '_']) && !token.starts_with('/') {
      return false;
    }
    if path_looks_complete(accumulated) && token.contains('/') {
      return false;
    }
    return has_strong_path_structure(token) || (accumulated.ends_with(['.', '-', '_']) && !token.ends_with('.'));
  }
  has_strong_path_structure(token) || accumulated.ends_with(['/', '.', '-', '_', '=', '?', '&', '#', '%'])
}

fn has_strong_path_structure(text: &str) -> bool {
  let bounded = text.trim_end_matches([')', ']', '}', ',', ';', ':']);
  bounded.contains('/') || path_looks_complete(bounded) || is_numeric_location_suffix(bounded)
}

fn is_numeric_location_suffix(text: &str) -> bool {
  let mut parts = text.split(':');
  let line = parts.next().unwrap_or_default();
  let column = parts.next();

  !line.is_empty()
    && line.chars().all(|ch| ch.is_ascii_digit())
    && column
      .map(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
      .unwrap_or(true)
    && parts.next().is_none()
}

fn is_meaningful_path(text: &str) -> bool {
  text.chars().any(|ch| ch != '/')
}

fn overlaps(candidate: &Match, other: &Match) -> bool {
  other.spans.iter().all(|span| {
    candidate.spans.iter().any(|candidate_span| {
      candidate_span.line == span.line && candidate_span.start < span.end && span.start < candidate_span.end
    })
  })
}

pub fn boundary_range(text: &str, pattern: &str) -> (usize, usize) {
  let mut start = 0;
  if matches!(pattern, "path" | "markdown_url") {
    if let Some(slash) = text.find('/') {
      let prefix = &text[..slash];
      for root in ["..", ".", "~"] {
        if let Some(prose) = prefix.strip_suffix(root) {
          if prose.chars().any(|ch| !ch.is_ascii()) {
            start = slash - root.len();
            break;
          }
        }
      }
    }
  }
  let mut end = trim_terminal_punctuation(text, pattern);

  loop {
    let bounded = &text[start..end];
    let first = match bounded.chars().next() {
      Some(ch @ ('(' | '[' | '（' | '【')) => ch,
      _ => break,
    };

    if matching_close_at_end(bounded, first) {
      start += first.len_utf8();
      end -= bounded.chars().last().unwrap().len_utf8();
    } else {
      break;
    }
  }

  let bounded_len = boundary_len(&text[start..end], pattern);
  (start, start + bounded_len)
}

fn trim_terminal_punctuation(text: &str, pattern: &str) -> usize {
  let mut bounded = text;
  while let Some(ch) = bounded.chars().last() {
    if is_terminal_punctuation(ch, pattern, bounded) {
      bounded = &bounded[..bounded.len() - ch.len_utf8()];
    } else {
      break;
    }
  }
  bounded.len()
}

fn boundary_len(text: &str, pattern: &str) -> usize {
  let mut stack = Vec::new();
  let mut end = text.len();

  for (index, ch) in text.char_indices() {
    match ch {
      '(' | '[' | '（' | '【' => stack.push(ch),
      ')' | ']' | '）' | '】' => {
        if stack.last().copied().map(|open| closes(open, ch)).unwrap_or(false) {
          stack.pop();
        } else {
          end = index;
          break;
        }
      }
      '\''
        if text[..index].chars().last().map(char::is_alphanumeric).unwrap_or(false)
          && text[index + 1..]
            .chars()
            .next()
            .map(char::is_alphanumeric)
            .unwrap_or(false) => {}
      '\'' | '"' | '`' | '<' | '>' => {
        end = index;
        break;
      }
      _ => {}
    }
  }

  trim_terminal_punctuation(&text[..end], pattern)
}

fn matching_close_at_end(text: &str, first: char) -> bool {
  let mut stack = Vec::new();

  for (index, ch) in text.char_indices() {
    match ch {
      '(' | '[' | '（' | '【' => stack.push(ch),
      ')' | ']' | '）' | '】' => {
        let open = match stack.pop() {
          Some(open) if closes(open, ch) => open,
          _ => return false,
        };

        if open == first && stack.is_empty() {
          return index + ch.len_utf8() == text.len();
        }
      }
      _ => {}
    }
  }

  false
}

fn closes(open: char, close: char) -> bool {
  matches!((open, close), ('(', ')') | ('[', ']') | ('（', '）') | ('【', '】'))
}

fn is_terminal_punctuation(ch: char, pattern: &str, text: &str) -> bool {
  match ch {
    '，' | '。' | '；' | '：' | '！' | '、' | ',' | '.' | ';' | '!' => true,
    ':' => {
      pattern == "path"
        && !text
          .rsplit_once(':')
          .map(|(_, suffix)| !suffix.is_empty())
          .unwrap_or(false)
    }
    '?' => pattern != "url" || text.ends_with('?'),
    _ => false,
  }
}

#[cfg(test)]
mod tests {
  use super::super::state::{Match, State};
  use unicode_width::UnicodeWidthStr;

  fn matching<'a>(input: &'a str, pattern: &str) -> Match {
    matching_with_width(input, pattern, None)
  }

  fn matching_with_width<'a>(input: &'a str, pattern: &str, pane_width: Option<usize>) -> Match {
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];

    State::new(&lines, "abcd", &custom, pane_width)
      .matches(false, false)
      .into_iter()
      .find(|candidate| candidate.pattern == pattern && (pane_width.is_none() || candidate.spans.len() > 1))
      .unwrap_or_else(|| panic!("no {} candidate in {:?}", pattern, input))
  }

  fn pane_width(input: &str) -> usize {
    input.lines().next().unwrap().width_cjk()
  }

  fn assert_url(input: &str, expected: &str, start: usize) {
    let candidate = matching(input, "url");

    assert_eq!(candidate.text, expected);
    assert_eq!(candidate.spans[0].start, start);
    assert_eq!(candidate.spans[0].end, start + expected.len());
  }

  fn assert_path(input: &str, expected: &str, start: usize) {
    let candidate = matching(input, "path");

    assert_eq!(candidate.text, expected);
    assert_eq!(candidate.spans[0].start, start);
    assert_eq!(candidate.spans[0].end, start + expected.len());
  }

  #[test]
  fn url_stops_before_ascii_closer_and_chinese_prose() {
    assert_url(
      "(https://github.com/fcsonline/tmux-thumbs)最后一次合并提交是 2023-03-21。",
      "https://github.com/fcsonline/tmux-thumbs",
      1,
    );
  }

  #[test]
  fn url_stops_before_cjk_closer_comma_and_chinese_prose() {
    assert_url(
      "（https://github.com/fcsonline/tmux-thumbs/releases/tag/0.8.0），发布于 2023-03-11。",
      "https://github.com/fcsonline/tmux-thumbs/releases/tag/0.8.0",
      "（".len(),
    );
  }

  #[test]
  fn url_trims_a_sentence_question_mark_but_keeps_a_query() {
    assert_url("https://host/path?", "https://host/path", 0);
    assert_url("https://host/path?q=value", "https://host/path?q=value", 0);
  }

  #[test]
  fn balanced_internal_delimiters_are_preserved() {
    assert_url("https://host/wiki/Foo_(bar)。", "https://host/wiki/Foo_(bar)", 0);
    assert_path("app/(main)/[id]/index.tsx)。", "app/(main)/[id]/index.tsx", 0);
  }

  #[test]
  fn path_location_keeps_line_and_column() {
    assert_path("见 path/file.go:42:7。", "path/file.go:42:7", "见 ".len());
  }

  #[test]
  fn path_glob_is_one_candidate() {
    assert_path("specs/**/*.md 与 grill-spec.md", "specs/**/*.md", 0);
  }

  #[test]
  fn path_excludes_outer_parentheses() {
    assert_path(
      "(.ai_doc/records/inbox/handoff.md)。",
      ".ai_doc/records/inbox/handoff.md",
      1,
    );
    assert_path("(path/file.md).", "path/file.md", 1);
  }

  #[test]
  fn path_excludes_a_dangling_location_colon() {
    assert_path("path/file.go:", "path/file.go", 0);
  }

  #[test]
  fn standalone_slash_is_not_a_path_candidate() {
    let lines = vec!["use / to separate alternatives"];
    let custom = vec![];
    let matches = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert!(matches.iter().all(|candidate| candidate.pattern != "path"));
  }

  #[test]
  fn joins_screenshot_path_into_one_candidate() {
    let input = "(.ai_doc/records/inbox/\n  life-card-admin-20260911-01a08c45/handoff.md)。";
    let candidate = matching_with_width(input, "path", Some(pane_width(input)));

    assert_eq!(
      candidate.text,
      ".ai_doc/records/inbox/life-card-admin-20260911-01a08c45/handoff.md"
    );
    assert_eq!(candidate.spans.len(), 2);
    assert_eq!(candidate.hint.as_deref(), Some("a"));
  }

  #[test]
  fn joins_supported_path_prefixes_and_location_suffix() {
    let cases = [
      ("/var/log/\n  app/server.log", "/var/log/app/server.log"),
      ("~/workspace/\n  src/main.rs", "~/workspace/src/main.rs"),
      ("./workspace/\n  src/main.rs", "./workspace/src/main.rs"),
      ("../workspace/\n  src/main.rs", "../workspace/src/main.rs"),
      ("path/file.go:\n  42:7", "path/file.go:42:7"),
    ];

    for (input, expected) in cases {
      let candidate = matching_with_width(input, "path", Some(pane_width(input)));
      assert_eq!(candidate.text, expected, "input: {:?}", input);
      assert_eq!(candidate.spans.len(), 2, "input: {:?}", input);
    }
  }

  #[test]
  fn joins_a_numeric_location_suffix_wrapped_before_the_pane_edge() {
    let input = "path/file.go:\n  42:7";
    let candidate = matching_with_width(input, "path", Some(16));

    assert_eq!(candidate.text, "path/file.go:42:7");
    assert_eq!(candidate.spans.len(), 2);
  }

  #[test]
  fn joins_url_across_three_lines() {
    let input = "https://example.com/a/\n  long-path-segment/bb/\n  final.html";
    let candidate = matching_with_width(input, "url", Some(pane_width(input)));

    assert_eq!(candidate.text, "https://example.com/a/long-path-segment/bb/final.html");
    assert_eq!(candidate.spans.len(), 3);
  }

  #[test]
  fn joins_filename_before_following_prose() {
    let input = "path/to/\n  file.md 后续文字";
    let candidate = matching_with_width(input, "path", Some(pane_width(input)));

    assert_eq!(candidate.text, "path/to/file.md");
    assert_eq!(candidate.spans.len(), 2);
  }

  #[test]
  fn joins_after_a_dot_trimmed_by_single_line_normalization() {
    let cases = [
      ("path/file.\n  ext", "path", "path/file.ext"),
      ("https://example.\n  com/path", "url", "https://example.com/path"),
    ];

    for (input, pattern, expected) in cases {
      let candidate = matching_with_width(input, pattern, Some(pane_width(input)));
      assert_eq!(candidate.text, expected, "input: {:?}", input);
    }
  }

  #[test]
  fn joins_tui_word_wrapped_paths_before_the_pane_edge() {
    let base = "/data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_card_admin/openspec/changes/";
    let cases = [
      (
        format!(
          "      - {}life-service-card-admin-compatibility\n        /proposal.md：明确使用",
          base
        ),
        format!("{}life-service-card-admin-compatibility/proposal.md", base),
      ),
      (
        format!(
          "      - {}life-service-card-admin-\n        compatibility/design.md：命令实际解析",
          base
        ),
        format!("{}life-service-card-admin-compatibility/design.md", base),
      ),
      (
        format!(
          "  - {}life-service-card-admin-compatibility/\n    revise.md 已恢复",
          base
        ),
        format!("{}life-service-card-admin-compatibility/revise.md", base),
      ),
    ];

    for (input, expected) in cases {
      let candidate = matching_with_width(&input, "path", Some(136));
      assert_eq!(candidate.text, expected, "input: {:?}", input);
      assert_eq!(candidate.spans.len(), 2, "input: {:?}", input);
    }
  }

  #[test]
  fn does_not_join_a_complete_file_with_an_independent_rooted_path() {
    let input = "  - /workspace/complete.md\n    /another/independent.md";
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];
    let results = State::new(&lines, "abcd", &custom, Some(pane_width(input))).matches(false, false);

    assert!(results.iter().all(|candidate| candidate.spans.len() == 1));
  }

  #[test]
  fn does_not_join_a_complete_path_with_a_following_metadata_field() {
    let input = " Directory:            /data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits\n Permissions:          Full Access";
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];
    let results = State::new(&lines, "abcd", &custom, Some(136)).matches(false, false);
    let path = results
      .iter()
      .find(|candidate| candidate.pattern == "path" && candidate.text.starts_with("/data00/"))
      .expect("directory path candidate");

    assert_eq!(
      path.text,
      "/data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits"
    );
    assert_eq!(path.spans.len(), 1);
  }

  #[test]
  fn does_not_join_a_metadata_field_that_contains_path_punctuation() {
    let input = " Directory:            /data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits\n Agents.md:            AGENTS.md";
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];
    let results = State::new(&lines, "abcd", &custom, Some(130)).matches(false, false);
    let path = results
      .iter()
      .find(|candidate| candidate.pattern == "path" && candidate.text.starts_with("/data00/"))
      .expect("directory path candidate");

    assert_eq!(
      path.text,
      "/data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits"
    );
    assert_eq!(path.spans.len(), 1);
  }

  #[test]
  fn does_not_join_blocked_or_ambiguous_continuations() {
    let cases = [
      "path/to/\n  /another/independent/path",
      "path/to/\n  - list item",
      "path/to/\n  ◆ status item",
      "path/to/\n  $ shell prompt",
      "path/to/\n  https://new.example/path",
      "path/to/\n  natural language continues here",
      "path/to/\n  natural-language continues here",
      "path/to/\n\n  next/file.rs",
      "path/to/\nnext/file.rs",
    ];

    for input in cases {
      let lines = input.split('\n').collect::<Vec<_>>();
      let custom = vec![];
      let results = State::new(&lines, "abcd", &custom, Some(pane_width(input))).matches(false, false);
      assert!(
        results.iter().all(|candidate| candidate.spans.len() == 1),
        "unexpected join for {:?}: {:?}",
        input,
        results
      );
    }
  }

  #[test]
  fn explicit_roots_exclude_adjacent_prose_but_preserve_unicode_components() {
    let cases = [
      ("详细方案已写入./.ai_doc/design.md", "./.ai_doc/design.md"),
      ("请看../文档/设计.md", "../文档/设计.md"),
      ("已保存~/文档/设计.md", "~/文档/设计.md"),
      ("见 ./文档/设计.md", "./文档/设计.md"),
    ];
    for (input, expected) in cases {
      assert_path(input, expected, input.find(expected).unwrap());
    }
  }

  #[test]
  fn urls_share_boundaries_in_quotes_and_markdown() {
    let cases = [
      ("url=\"https://host/path\"", vec!["https://host/path"]),
      ("'https://host/path'", vec!["https://host/path"]),
      (
        "https://host/search?q=don't-stop",
        vec!["https://host/search?q=don't-stop"],
      ),
      ("\"https://host/author/O'Reilly\"", vec!["https://host/author/O'Reilly"]),
      (
        "[doc](https://host/wiki/Foo_(bar))",
        vec!["https://host/wiki/Foo_(bar)"],
      ),
      (
        "[a](https://host/a)[b](https://host/b)",
        vec!["https://host/a", "https://host/b"],
      ),
      ("[文件](./文档/设计.md)", vec!["./文档/设计.md"]),
    ];
    for (input, expected) in cases {
      let lines = vec![input];
      let custom = vec![];
      let result = State::new(&lines, "abcd", &custom, None).matches(false, false);
      assert_eq!(
        result.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        expected,
        "{:?}",
        input
      );
    }
  }

  #[test]
  fn joins_aligned_tool_gutters_without_copying_decoration() {
    let cases = [
      (
        "  │ /workspace/docs/\n  │ design.md",
        19,
        "path",
        "/workspace/docs/design.md",
      ),
      (
        "◆ Ran cat /workspace/docs/\n  │ design.md",
        25,
        "path",
        "/workspace/docs/design.md",
      ),
      (
        "  │ https://example.com/\n  │ docs/index.html",
        23,
        "url",
        "https://example.com/docs/index.html",
      ),
      (
        "  │ /workspace/long/\n  │ subdirectory/\n  │ file.rs",
        19,
        "path",
        "/workspace/long/subdirectory/file.rs",
      ),
    ];
    for (input, width, pattern, expected) in cases {
      let candidate = matching_with_width(input, pattern, Some(width));
      assert_eq!(candidate.text, expected, "{:?}", input);
      for span in candidate.spans.iter().skip(1) {
        assert_eq!(span.start, "  │ ".len());
      }
    }
  }

  #[test]
  fn does_not_join_independent_targets_prose_or_output() {
    let cases = [
      ("path/to/\n  Done.", vec!["path/to/"]),
      (
        "src/first.rs\n  tests/second.rs",
        vec!["src/first.rs", "tests/second.rs"],
      ),
      (
        "  - /workspace/project\n    /another/project",
        vec!["/workspace/project", "/another/project"],
      ),
      ("/workspace/build/\n  1234 5678", vec!["/workspace/build/"]),
      (
        "/workspace/README\n  docs/guide.md",
        vec!["/workspace/README", "docs/guide.md"],
      ),
      ("/workspace/README\n  设计.md", vec!["/workspace/README"]),
      ("path/file.go\n  42:7", vec!["path/file.go"]),
      (
        "  │ /workspace/docs/\n  └ result.txt",
        vec!["/workspace/docs/", "result.txt"],
      ),
      (
        "  │ /workspace/docs/\n    │ other/file.rs",
        vec!["/workspace/docs/", "other/file.rs"],
      ),
      (
        "  │ /workspace/docs/\n  │ && cat other/file.rs",
        vec!["/workspace/docs/", "other/file.rs"],
      ),
    ];
    for (input, expected) in cases {
      let lines = input.split('\n').collect::<Vec<_>>();
      let custom = vec![];
      let result = State::new(&lines, "abcd", &custom, Some(pane_width(input))).matches(false, false);
      let paths = result.iter().filter(|m| m.pattern == "path").collect::<Vec<_>>();
      assert_eq!(
        paths.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        expected,
        "{:?}",
        input
      );
      assert!(paths.iter().all(|m| m.spans.len() == 1), "{:?}", input);
    }
  }

  #[test]
  fn joins_unicode_path_components_at_hard_wraps() {
    for input in ["../文档/\n  设计.md", "  │ ../文档/\n  │ 设计.md"] {
      let candidate = matching_with_width(input, "path", Some(pane_width(input)));
      assert_eq!(candidate.text, "../文档/设计.md");
    }
  }

  #[test]
  fn does_not_join_without_width_or_before_pane_edge() {
    let input = "path/to/\n  next/file.rs";
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];

    let without_width = State::new(&lines, "abcd", &custom, None).matches(false, false);
    let before_edge = State::new(&lines, "abcd", &custom, Some(80)).matches(false, false);

    assert!(without_width.iter().all(|candidate| candidate.spans.len() == 1));
    assert!(before_edge.iter().all(|candidate| candidate.spans.len() == 1));
  }
}
