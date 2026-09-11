use super::state::Match;

pub fn normalize(_lines: &[&str], mut matches: Vec<Match>, _pane_width: Option<usize>) -> Vec<Match> {
  for candidate in matches.iter_mut() {
    if candidate.pattern == "url" || candidate.pattern == "path" {
      trim_candidate(candidate);
    }
  }

  matches
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
  let mut end = text.len();

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

  let mut bounded = &text[..end];
  while let Some(ch) = bounded.chars().last() {
    if is_terminal_punctuation(ch, pattern, bounded) {
      bounded = &bounded[..bounded.len() - ch.len_utf8()];
    } else {
      break;
    }
  }

  bounded.len()
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
    '?' => pattern != "url" || !text.contains('?'),
    _ => false,
  }
}

#[cfg(test)]
mod tests {
  use super::super::state::{Match, State};

  fn matching<'a>(input: &'a str, pattern: &str) -> Match {
    let lines = input.split('\n').collect::<Vec<_>>();
    let custom = vec![];

    State::new(&lines, "abcd", &custom, None)
      .matches(false, false)
      .into_iter()
      .find(|candidate| candidate.pattern == pattern)
      .unwrap_or_else(|| panic!("no {} candidate in {:?}", pattern, input))
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
  fn balanced_internal_delimiters_are_preserved() {
    assert_url("https://host/wiki/Foo_(bar)。", "https://host/wiki/Foo_(bar)", 0);
    assert_path("app/(main)/[id]/index.tsx)。", "app/(main)/[id]/index.tsx", 0);
  }

  #[test]
  fn path_location_keeps_line_and_column() {
    assert_path("见 path/file.go:42:7。", "path/file.go:42:7", "见 ".len());
  }

  #[test]
  fn path_excludes_outer_parentheses() {
    assert_path(
      "(.ai_doc/records/inbox/handoff.md)。",
      ".ai_doc/records/inbox/handoff.md",
      1,
    );
  }
}
