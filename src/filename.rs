#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilenameMatch {
  pub start: usize,
  pub end: usize,
}

const KNOWN_BASENAMES: &[&str] = &[
  "readme",
  "license",
  "notice",
  "changelog",
  "makefile",
  "dockerfile",
  "gemfile",
  "rakefile",
];

const KNOWN_EXTENSIONS: &[&str] = &[
  "md", "markdown", "txt", "rst", "adoc", "log", "json", "yaml", "yml", "toml", "xml", "csv", "ini", "conf", "cfg",
  "env", "lock", "rs", "go", "py", "js", "jsx", "ts", "tsx", "java", "kt", "kts", "c", "cc", "cpp", "h", "hpp", "sh",
  "bash", "zsh", "fish", "lua", "vim", "sql", "proto", "thrift", "idl", "html", "css", "scss", "less", "mod", "sum",
  "tar", "gz", "tgz", "zip",
];

pub fn scan(line: &str) -> Vec<FilenameMatch> {
  let mut matches = Vec::new();
  let mut token_start = None;

  for (index, ch) in line.char_indices().chain(std::iter::once((line.len(), '\0'))) {
    if is_token_char(ch) {
      token_start.get_or_insert(index);
      continue;
    }

    if let Some(start) = token_start.take() {
      if let Some(candidate) = classify_token(line, start, index) {
        matches.push(candidate);
      }
    }
  }

  matches
}

fn is_token_char(ch: char) -> bool {
  ch.is_ascii_alphanumeric()
    || matches!(
      ch,
      '_' | '-' | '.' | '@' | '+' | '$' | '%' | '*' | '(' | ')' | '[' | ']' | ':'
    )
}

fn classify_token(line: &str, raw_start: usize, raw_end: usize) -> Option<FilenameMatch> {
  if adjacent_to_slash(line, raw_start, raw_end) {
    return None;
  }

  let (start, end) = trim_token(line, raw_start, raw_end);
  if start >= end {
    return None;
  }

  let token = &line[start..end];
  let basename = strip_location(token);
  let lower = basename.to_ascii_lowercase();

  if is_version(&lower) || is_domain(&lower) {
    return None;
  }

  if KNOWN_BASENAMES.contains(&lower.as_str()) || known_extension(&lower) || has_explicit_file_context(line, start) {
    Some(FilenameMatch { start, end })
  } else {
    None
  }
}

fn has_explicit_file_context(line: &str, start: usize) -> bool {
  let prefix = line[..start].trim_end_matches(|ch: char| ch.is_whitespace() || matches!(ch, ':' | '：'));
  ["文件", "文档", "file", "written to"]
    .iter()
    .any(|marker| prefix.to_ascii_lowercase().ends_with(marker))
}

fn adjacent_to_slash(line: &str, start: usize, end: usize) -> bool {
  line[..start].chars().next_back() == Some('/') || line[end..].chars().next() == Some('/')
}

fn trim_token(line: &str, mut start: usize, mut end: usize) -> (usize, usize) {
  loop {
    let token = &line[start..end];
    let first = token.chars().next();
    let last = token.chars().next_back();
    let paired = matches!((first, last), (Some('('), Some(')')) | (Some('['), Some(']')));
    if !paired {
      break;
    }
    start += first.unwrap().len_utf8();
    end -= last.unwrap().len_utf8();
  }

  while let Some(ch) = line[start..end].chars().next_back() {
    if matches!(ch, '.' | ':' | ',' | ';' | '!' | '?') {
      end -= ch.len_utf8();
    } else {
      break;
    }
  }

  (start, end)
}

fn strip_location(token: &str) -> &str {
  let mut end = token.len();
  for _ in 0..2 {
    let prefix = &token[..end];
    let Some((head, tail)) = prefix.rsplit_once(':') else {
      break;
    };
    if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_digit()) {
      break;
    }
    end = head.len();
  }
  &token[..end]
}

fn known_extension(token: &str) -> bool {
  token
    .rsplit_once('.')
    .map(|(stem, extension)| !stem.is_empty() && KNOWN_EXTENSIONS.contains(&extension))
    .unwrap_or(false)
}

fn is_version(token: &str) -> bool {
  let token = token.strip_prefix('v').unwrap_or(token);
  token.contains('.')
    && token
      .split('.')
      .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_domain(token: &str) -> bool {
  const DOMAIN_SUFFIXES: &[&str] = &["com", "org", "net", "io", "dev", "cn", "internal"];
  token
    .rsplit_once('.')
    .map(|(labels, suffix)| labels.contains('.') && DOMAIN_SUFFIXES.contains(&suffix))
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn matched(line: &str) -> Vec<&str> {
    scan(line).iter().map(|item| &line[item.start..item.end]).collect()
  }

  #[test]
  fn scans_known_filenames_and_composite_extensions() {
    let line = "skill-eval-review-report-20260912.md README archive.tar.gz main.rs component.jsx view.tsx code.cpp header.hpp build.kts";

    assert_eq!(
      matched(line),
      [
        "skill-eval-review-report-20260912.md",
        "README",
        "archive.tar.gz",
        "main.rs",
        "component.jsx",
        "view.tsx",
        "code.cpp",
        "header.hpp",
        "build.kts",
      ]
    );
  }

  #[test]
  fn preserves_locations_and_internal_parentheses() {
    let line = "(report-20260912.md)，report(final).md file.rs:42:7";

    assert_eq!(
      matched(line),
      ["report-20260912.md", "report(final).md", "file.rs:42:7"]
    );
  }

  #[test]
  fn scans_tree_entries_without_their_decoration() {
    for line in ["└ result.txt", "├ config.yaml", "│ README.md"] {
      assert_eq!(matched(line), [line.split_whitespace().last().unwrap()], "{:?}", line);
    }
  }

  #[test]
  fn rejects_domains_versions_ips_prose_and_unknown_extensions() {
    let line = "host.example.com 1.2.3 v1.2.3 127.0.0.1 report-20260912 artifact.xyzabc ordinary-word";

    assert!(matched(line).is_empty());
  }

  #[test]
  fn accepts_unknown_extensions_only_with_explicit_file_context() {
    let cases = [
      ("文件 artifact.xyzabc", "artifact.xyzabc"),
      ("文档：report.customext", "report.customext"),
      ("file output.weirdext", "output.weirdext"),
      ("written to result.privateext", "result.privateext"),
    ];

    for (line, expected) in cases {
      assert_eq!(matched(line), [expected], "{:?}", line);
    }
    assert!(matched("artifact.xyzabc").is_empty());
  }
}
