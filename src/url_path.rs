use super::state::{Match, ScreenSpan};
use unicode_width::UnicodeWidthStr;

pub fn normalize(lines: &[&str], mut matches: Vec<Match>, pane_width: Option<usize>) -> Vec<Match> {
  for candidate in matches.iter_mut() {
    if candidate.pattern == "url" || candidate.pattern == "path" {
      trim_candidate(candidate);
    }
  }
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

    if let Some(joined_candidate) = join_candidate(lines, candidate, width) {
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

fn join_candidate(lines: &[&str], candidate: &Match, pane_width: usize) -> Option<Match> {
  let mut joined = candidate.clone();
  let mut current_line = candidate.anchor().line;

  loop {
    let current = *lines.get(current_line)?;
    restore_join_separator(current, &mut joined);
    let next_line = current_line + 1;
    let next = match lines.get(next_line) {
      Some(next) => *next,
      None => break,
    };
    let span = match continuation_span(joined.pattern, next, &joined.text) {
      Some(span) => ScreenSpan {
        line: next_line,
        ..span
      },
      None => break,
    };
    let token = &next[span.start..span.end];
    let current_span = joined.spans.last().unwrap();
    if !reaches_wrap_boundary(current, current_span, token, pane_width) {
      break;
    }
    if !is_confident_continuation(joined.pattern, &joined.text, token) {
      break;
    }

    joined.text.push_str(token);
    joined.spans.push(span);
    current_line = next_line;
  }

  if joined.spans.len() == candidate.spans.len() {
    return None;
  }

  trim_candidate(&mut joined);
  Some(joined)
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
  let visible_end = line.trim_end_matches(char::is_whitespace).len();
  span.end == visible_end
    && (line.width_cjk() >= pane_width
      || (has_strong_path_structure(continuation) && line.width_cjk() + continuation.width_cjk() > pane_width))
}

fn continuation_span(pattern: &str, line: &str, accumulated: &str) -> Option<ScreenSpan> {
  let trimmed = line.trim_start_matches(char::is_whitespace);
  if trimmed.len() == line.len() || trimmed.is_empty() || is_blocked_continuation(pattern, trimmed, accumulated) {
    return None;
  }

  let start = line.len() - trimmed.len();
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
    return accumulated.ends_with('/') || path_looks_complete(accumulated);
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
    ch.is_ascii_alphanumeric()
      || matches!(
        ch,
        '.' | '_' | '-' | '@' | '$' | '~' | '%' | '+' | '*' | '[' | ']' | '(' | ')' | '/' | ':'
      )
  }
}

fn is_confident_continuation(_pattern: &str, accumulated: &str, token: &str) -> bool {
  accumulated
    .chars()
    .last()
    .map(|ch| matches!(ch, '/' | ':' | '.' | '-' | '_' | '=' | '?' | '&' | '#' | '%'))
    .unwrap_or(false)
    || has_path_structure(token)
}

fn has_path_structure(text: &str) -> bool {
  text
    .chars()
    .any(|ch| matches!(ch, '/' | ':' | '.' | '-' | '_' | '=' | '?' | '&' | '#' | '%'))
}

fn has_strong_path_structure(text: &str) -> bool {
  text.chars().any(|ch| matches!(ch, '/' | '.')) || is_numeric_location_suffix(text)
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

fn trim_candidate(candidate: &mut Match) {
  let (start, end) = boundary_range(&candidate.text, candidate.pattern);

  if start > 0 || end < candidate.text.len() {
    let removed_from_end = candidate.text.len() - end;
    candidate.text = candidate.text[start..end].to_string();
    let first_span = candidate
      .spans
      .first_mut()
      .expect("URL/path candidate must have a screen span");
    first_span.start += start;
    let final_span = candidate
      .spans
      .last_mut()
      .expect("URL/path candidate must have a screen span");
    final_span.end -= removed_from_end;
  }
}

fn boundary_range(text: &str, pattern: &str) -> (usize, usize) {
  let mut start = 0;
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
