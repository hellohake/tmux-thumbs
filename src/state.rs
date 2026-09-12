use regex::Regex;
use std::collections::HashMap;

const EXCLUDE_PATTERNS: [(&'static str, &'static str); 1] = [("bash", r"[[:cntrl:]]\[([0-9]{1,2};)?([0-9]{1,2})?m")];

const PATTERNS: [(&'static str, &'static str); 15] = [
  ("markdown_url", r"\[[^]\n]*\]\((?P<match>[^\s]+)"),
  (
    "url",
    r"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[\x21-\x7e]+)",
  ),
  (
    "diff_summary",
    r"diff --git a/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++) b/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++)",
  ),
  ("diff_a", r"--- a/([^ ]+)"),
  ("diff_b", r"\+\+\+ b/([^ ]+)"),
  ("docker", r"sha256:([0-9a-f]{64})"),
  (
    "path",
    r"(?P<match>([.\w\-@$~%+*\[\]()]*)?(/[.\w\-@$~%+*\[\]()]*)+(?::(?:\d+(?::\d*)?)?)?)",
  ),
  ("color", r"#[0-9a-fA-F]{6}"),
  ("uid", r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
  ("ipfs", r"Qm[0-9a-zA-Z]{44}"),
  ("sha", r"[0-9a-f]{7,40}"),
  ("ip", r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"),
  ("ipv6", r"[A-f0-9:]+:+[A-f0-9:]+[%\w\d]+"),
  ("address", r"0x[0-9a-fA-F]+"),
  ("number", r"[0-9]{4,}"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenSpan {
  pub line: usize,
  pub start: usize,
  pub end: usize,
}

#[derive(Clone, Debug)]
pub struct Match {
  pub pattern: &'static str,
  pub text: String,
  pub spans: Vec<ScreenSpan>,
  pub hint: Option<String>,
}

impl Match {
  pub fn anchor(&self) -> &ScreenSpan {
    self.spans.first().expect("match must have a screen span")
  }
}

impl PartialEq for Match {
  fn eq(&self, other: &Match) -> bool {
    self.spans == other.spans
  }
}

pub struct State<'a> {
  pub lines: &'a Vec<&'a str>,
  alphabet: &'a str,
  regexp: &'a Vec<&'a str>,
  pane_width: Option<usize>,
  joined_text: Option<&'a str>,
}

impl<'a> State<'a> {
  pub fn new(
    lines: &'a Vec<&'a str>,
    alphabet: &'a str,
    regexp: &'a Vec<&'a str>,
    pane_width: Option<usize>,
  ) -> State<'a> {
    State {
      lines,
      alphabet,
      regexp,
      pane_width,
      joined_text: None,
    }
  }

  pub fn with_joined_text(mut self, text: &'a str) -> Self {
    self.joined_text = Some(text);
    self
  }

  pub fn pane_width(&self) -> Option<usize> {
    self.pane_width
  }

  fn collect_matches(lines: &[&str], regexp: &[&str]) -> Vec<Match> {
    let mut matches = Vec::new();

    let exclude_patterns = EXCLUDE_PATTERNS
      .iter()
      .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
      .collect::<Vec<_>>();

    let custom_patterns = regexp
      .iter()
      .map(|regexp| ("custom", Regex::new(regexp).expect("Invalid custom regexp")))
      .collect::<Vec<_>>();

    let patterns = PATTERNS
      .iter()
      .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
      .collect::<Vec<_>>();

    // This order determines the priority of pattern matching
    let all_patterns = [exclude_patterns, custom_patterns, patterns].concat();

    for (index, line) in lines.iter().enumerate() {
      let mut chunk: &str = line;
      let mut offset: usize = 0;

      loop {
        // For this line we search which patterns match, all of them.
        let submatches = all_patterns
          .iter()
          .filter_map(|tuple| match tuple.1.find_iter(chunk).find(|m| m.start() < m.end()) {
            Some(m) => Some((tuple.0, tuple.1.clone(), m)),
            None => None,
          })
          .collect::<Vec<_>>();

        // Then, we search for the match with the lowest index
        let first_match_option = submatches.iter().min_by(|x, y| x.2.start().cmp(&y.2.start()));

        if let Some(first_match) = first_match_option {
          let (name, pattern, matching) = first_match;
          let text = matching.as_str();

          let mut consumed = matching.end();
          if let Some(captures) = pattern.captures(text) {
            let captures: Vec<(&str, usize)> = if let Some(capture) = captures.name("match") {
              [(capture.as_str(), capture.start())].to_vec()
            } else if captures.len() > 1 {
              captures
                .iter()
                .skip(1)
                .filter_map(|capture| capture)
                .map(|capture| (capture.as_str(), capture.start()))
                .collect::<Vec<(&str, usize)>>()
            } else {
              [(matching.as_str(), 0)].to_vec()
            };

            // Never hint or broke bash color sequences, but process it
            if *name != "bash" {
              for (subtext, substart) in captures.iter() {
                let (start, end) = if matches!(*name, "url" | "path" | "markdown_url") {
                  super::url_path::boundary_range(subtext, name)
                } else {
                  (0, subtext.len())
                };
                if matches!(*name, "url" | "path" | "markdown_url") {
                  consumed = matching.start() + *substart + end;
                }
                if start == end {
                  continue;
                }
                matches.push(Match {
                  pattern: name,
                  text: subtext[start..end].to_string(),
                  spans: vec![ScreenSpan {
                    line: index,
                    start: offset + matching.start() + *substart + start,
                    end: offset + matching.start() + *substart + end,
                  }],
                  hint: None,
                });
              }
            }

            consumed = consumed.max(matching.start() + text.chars().next().unwrap().len_utf8());
            chunk = chunk.get(consumed..).expect("Unknown chunk");
            offset += consumed;
          } else {
            panic!("No matching?");
          }
        } else {
          break;
        }
      }
    }

    matches
  }

  pub fn matches(&self, reverse: bool, unique: bool) -> Vec<Match> {
    let mut matches = Self::collect_matches(self.lines, self.regexp);
    let mut filenames = Vec::new();
    for (line_index, line) in self.lines.iter().enumerate() {
      for filename in super::filename::scan(line) {
        let span = ScreenSpan {
          line: line_index,
          start: filename.start,
          end: filename.end,
        };
        let overlaps_custom = matches
          .iter()
          .any(|candidate| candidate.pattern == "custom" && Self::spans_overlap(&candidate.spans, &[span.clone()]));
        if !overlaps_custom {
          filenames.push(Match {
            pattern: "path",
            text: line[filename.start..filename.end].to_string(),
            spans: vec![span],
            hint: None,
          });
        }
      }
    }
    for filename in filenames {
      matches.retain(|candidate| {
        candidate.pattern == "custom"
          || !candidate
            .spans
            .iter()
            .all(|span| Self::span_contains(filename.anchor(), span))
      });
      matches.push(filename);
    }
    let mut soft_wrapped = vec![false; self.lines.len()];
    if let Some(joined) = self.joined_text {
      let logical_lines: Vec<_> = joined.split('\n').collect();
      if let Some(mapping) = Self::map_logical_lines(self.lines, &logical_lines) {
        for spans in &mapping {
          for (_, span) in spans.iter().take(spans.len().saturating_sub(1)) {
            soft_wrapped[span.line] = true;
          }
        }
        for candidate in Self::collect_matches(&logical_lines, &[]) {
          if !matches!(candidate.pattern, "path" | "url" | "markdown_url") {
            continue;
          }
          let anchor = candidate.anchor();
          let spans: Vec<_> = mapping[anchor.line]
            .iter()
            .filter_map(|(base, span)| {
              let start = anchor.start.max(*base);
              let end = anchor.end.min(*base + span.end - span.start);
              (start < end).then(|| ScreenSpan {
                line: span.line,
                start: span.start + start - base,
                end: span.start + end - base,
              })
            })
            .collect();
          if spans.len() < 2
            || matches
              .iter()
              .any(|m| m.pattern == "custom" && Self::spans_overlap(&m.spans, &spans))
          {
            continue;
          }
          matches.retain(|m| !Self::spans_overlap(&m.spans, &spans));
          matches.push(Match { spans, ..candidate });
        }
      }
    }
    matches.sort_by_key(|m| (m.anchor().line, m.anchor().start));
    let mut matches = super::url_path::normalize(self.lines, matches, self.pane_width, &soft_wrapped);

    let alphabet = super::alphabets::get_alphabet(self.alphabet);
    let mut hints = alphabet.hints(matches.len());

    // This looks wrong but we do a pop after
    if !reverse {
      hints.reverse();
    } else {
      matches.reverse();
      hints.reverse();
    }

    if unique {
      let mut previous: HashMap<String, String> = HashMap::new();

      for mat in &mut matches {
        if let Some(previous_hint) = previous.get(&mat.text) {
          mat.hint = Some(previous_hint.clone());
        } else if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
          previous.insert(mat.text.clone(), hint.to_string().clone());
        }
      }
    } else {
      for mat in &mut matches {
        if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
        }
      }
    }

    if reverse {
      matches.reverse();
    }

    matches
  }

  fn spans_overlap(left: &[ScreenSpan], right: &[ScreenSpan]) -> bool {
    left.iter().any(|a| {
      right
        .iter()
        .any(|b| a.line == b.line && a.start < b.end && b.start < a.end)
    })
  }

  fn span_contains(container: &ScreenSpan, inner: &ScreenSpan) -> bool {
    container.line == inner.line && container.start <= inner.start && container.end >= inner.end
  }

  fn map_logical_lines(physical: &[&str], logical: &[&str]) -> Option<Vec<Vec<(usize, ScreenSpan)>>> {
    let mut row = 0;
    let mut result = Vec::new();
    for line in logical {
      let mut offset = 0;
      let mut spans = Vec::new();
      loop {
        let visible = physical.get(row)?.trim_end_matches(' ');
        if !line.get(offset..)?.starts_with(visible) {
          return None;
        }
        spans.push((
          offset,
          ScreenSpan {
            line: row,
            start: 0,
            end: visible.len(),
          },
        ));
        offset += visible.len();
        row += 1;
        if line.get(offset..)?.trim_matches(' ').is_empty() {
          break;
        }
        let next = physical.get(row)?.trim_end_matches(' ');
        let available = physical[row - 1].len() - visible.len();
        let whitespace = line[offset..].len() - line[offset..].trim_start_matches(' ').len();
        let spaces = (0..=available.min(whitespace)).find(|count| line[offset + count..].starts_with(next))?;
        spans.last_mut()?.1.end += spaces;
        offset += spaces;
      }
      result.push(spans);
    }
    if physical.get(row..)?.iter().any(|line| !line.trim().is_empty()) {
      return None;
    }
    Some(result)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn soft_wrap_mapping_preserves_wide_glyph_padding_and_row_coordinates() {
    let lines = vec!["/tmp/abcdefghij ", "中文设计.md", "FOLLOWING_ROW"];
    let custom = vec![];
    let result = State::new(&lines, "abcd", &custom, Some(16))
      .with_joined_text("/tmp/abcdefghij中文设计.md\nFOLLOWING_ROW")
      .matches(false, false);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].text, "/tmp/abcdefghij中文设计.md");
    assert_eq!(
      result[0].spans,
      vec![
        ScreenSpan {
          line: 0,
          start: 0,
          end: 15
        },
        ScreenSpan {
          line: 1,
          start: 0,
          end: "中文设计.md".len()
        },
      ]
    );
  }

  #[test]
  fn soft_wrap_mapping_does_not_join_across_real_whitespace() {
    let lines = vec!["/tmp/first.rs ", "/tmp/second.rs", "FOLLOWING_ROW"];
    let custom = vec![];
    let result = State::new(&lines, "abcd", &custom, Some(14))
      .with_joined_text("/tmp/first.rs /tmp/second.rs\nFOLLOWING_ROW")
      .matches(false, false);
    assert_eq!(
      result.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
      ["/tmp/first.rs", "/tmp/second.rs"]
    );
    assert!(result.iter().all(|m| m.spans.len() == 1));
  }

  #[test]
  fn soft_wrap_whitespace_is_not_reinterpreted_as_a_hard_wrap() {
    let lines = vec!["/tmp/     ", "  a/b.md"];
    let custom = vec![];
    let result = State::new(&lines, "abcd", &custom, Some(10))
      .with_joined_text("/tmp/       a/b.md")
      .matches(false, false);
    assert_eq!(
      result.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
      ["/tmp/", "a/b.md"]
    );
    assert!(result.iter().all(|m| m.spans.len() == 1));
  }

  #[test]
  fn inconsistent_joined_capture_keeps_physical_candidates() {
    let lines = vec!["/tmp/source.rs", "metadata"];
    let custom = vec![];
    let result = State::new(&lines, "abcd", &custom, Some(80))
      .with_joined_text("unrelated content")
      .matches(false, false);
    assert_eq!(result[0].text, "/tmp/source.rs");
    assert_eq!(result[0].spans.len(), 1);
  }

  #[test]
  fn zero_length_custom_matches_do_not_panic_or_hide_paths() {
    let lines = vec!["/tmp/file.rs"];
    let custom = vec!["", "^", "z*"];
    let result = State::new(&lines, "abcd", &custom, None).matches(false, false);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].text, "/tmp/file.rs");
  }

  #[test]
  fn single_line_match_owns_text_and_has_one_span() {
    let lines = split("前缀 /tmp/foo.rs 后缀");
    let custom = vec![];
    let result = State::new(&lines, "abcd", &custom, None).matches(false, false);
    let path = result.iter().find(|item| item.pattern == "path").unwrap();

    assert_eq!(path.text, "/tmp/foo.rs");
    assert_eq!(
      path.spans,
      vec![ScreenSpan {
        line: 0,
        start: 7,
        end: 18,
      }]
    );
  }

  #[test]
  fn bare_filename_outranks_sha_inside_it() {
    let lines = split("已写成本地 Markdown 文档： skill-eval-review-report-20260912.md");
    let custom = vec![];
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].pattern, "path");
    assert_eq!(results[0].text, "skill-eval-review-report-20260912.md");
    assert_eq!(results[0].hint.as_deref(), Some("a"));
  }

  #[test]
  fn custom_match_keeps_priority_over_a_bare_filename() {
    let lines = split("report-20260912.md");
    let custom = vec!["report-[0-9]+\\.md"];
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].pattern, "custom");
    assert_eq!(results[0].text, "report-20260912.md");
  }

  #[test]
  fn standalone_sha_and_number_behavior_is_unchanged() {
    let lines = split("DOC_OK lines=268 bytes=14797 commit=20260912");
    let custom = vec![];
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(
      results
        .iter()
        .map(|candidate| (candidate.pattern, candidate.text.as_str()))
        .collect::<Vec<_>>(),
      [("number", "14797"), ("sha", "20260912")]
    );
  }

  #[test]
  fn match_reverse() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "c");
  }

  #[test]
  fn match_unique() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, true);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn match_docker() {
    let lines = split("latest sha256:30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4 20 hours ago");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text,
      "30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4"
    );
  }

  #[test]
  fn match_bash() {
    let lines = split("path: [32m/var/log/nginx.log[m\npath: [32mtest/log/nginx-2.log:32[mfolder/.nginx@4df2.log");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "/var/log/nginx.log");
    assert_eq!(results.get(1).unwrap().text, "test/log/nginx-2.log:32");
    assert_eq!(results.get(2).unwrap().text, "folder/.nginx@4df2.log");
  }

  #[test]
  fn match_paths() {
    let lines = split("Lorem /tmp/foo/bar_lol, lorem\n Lorem /var/log/boot-strap.log lorem ../log/kern.log lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "/tmp/foo/bar_lol");
    assert_eq!(results.get(1).unwrap().text.clone(), "/var/log/boot-strap.log");
    assert_eq!(results.get(2).unwrap().text.clone(), "../log/kern.log");
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "/app/routes/$routeId/$objectId");
    assert_eq!(results.get(1).unwrap().text.clone(), "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "dev/api/[slug]/foo");
  }

  #[test]
  fn match_uids() {
    let lines = split("Lorem ipsum 123e4567-e89b-12d3-a456-426655440000 lorem\n Lorem lorem lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
  }

  #[test]
  fn match_shas() {
    let lines = split("Lorem fd70b5695 5246ddf f924213 lorem\n Lorem 973113963b491874ab2e372ee60d4b4cb75f717c lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "f924213");
    assert_eq!(
      results.get(3).unwrap().text.clone(),
      "973113963b491874ab2e372ee60d4b4cb75f717c"
    );
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "127.0.0.1");
    assert_eq!(results.get(1).unwrap().text.clone(), "255.255.10.255");
    assert_eq!(results.get(2).unwrap().text.clone(), "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split("Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fe80::2:202:fe4");
    assert_eq!(
      results.get(1).unwrap().text.clone(),
      "2001:67c:670:202:7ba8:5e41:1591:d723"
    );
    assert_eq!(results.get(2).unwrap().text.clone(), "fe80::2:1");
    assert_eq!(results.get(3).unwrap().text.clone(), "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(0).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(1).unwrap().text.clone(), "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split("Lorem ipsum https://www.rust-lang.org/tools lorem\n Lorem ipsumhttps://crates.io lorem https://github.io?foo=bar lorem ssh://github.io");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "https://www.rust-lang.org/tools");
    assert_eq!(results.get(0).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(1).unwrap().text.clone(), "https://crates.io");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(2).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(2).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(3).unwrap().text.clone(), "ssh://github.io");
    assert_eq!(results.get(3).unwrap().pattern.clone(), "url");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "0xfd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "0x5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "#fd7b56");
    assert_eq!(results.get(1).unwrap().text.clone(), "#FF00FF");
    assert_eq!(results.get(2).unwrap().text.clone(), "#00fF05");
    assert_eq!(results.get(3).unwrap().text.clone(), "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text.clone(),
      "QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ"
    );
  }

  #[test]
  fn match_process_port() {
    let lines =
      split("Lorem 5695 52463 lorem\n Lorem 973113 lorem 99999 lorem 8888 lorem\n   23456 lorem 5432 lorem 23444");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 8);
  }

  #[test]
  fn match_diff_a() {
    let lines = split("Lorem lorem\n--- a/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "samples/test1");
    assert_eq!(results.get(1).unwrap().text.clone(), "samples/test2");
  }

  #[test]
  fn priority() {
    let lines = split("Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem");
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();
    let results = State::new(&lines, "abcd", &custom, None).matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results.get(0).unwrap().text.clone(), "http://foo.bar");
    assert_eq!(results.get(1).unwrap().text.clone(), "CUSTOM-52463");
    assert_eq!(results.get(2).unwrap().text.clone(), "ISSUE-123");
    assert_eq!(results.get(3).unwrap().text.clone(), "/var/fd70b569/9999.log");
    assert_eq!(results.get(4).unwrap().text.clone(), "52463");
    assert_eq!(results.get(5).unwrap().text.clone(), "973113");
    assert_eq!(
      results.get(6).unwrap().text.clone(),
      "123e4567-e89b-12d3-a456-426655440000"
    );
    assert_eq!(results.get(7).unwrap().text.clone(), "8888");
    assert_eq!(results.get(8).unwrap().text.clone(), "https://crates.io/23456/fd70b569");
  }
}
