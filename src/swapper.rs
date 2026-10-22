extern crate clap;

use self::clap::{App, Arg};
use clap::crate_version;
use regex::Regex;
use std::fs::{self, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

trait Executor {
  fn execute(&mut self, args: Vec<String>) -> String;
  fn last_executed(&self) -> Option<Vec<String>>;
  fn last_succeeded(&self) -> bool;
}

struct RealShell {
  executed: Option<Vec<String>>,
  succeeded: bool,
}

impl RealShell {
  fn new() -> RealShell {
    RealShell {
      executed: None,
      succeeded: true,
    }
  }
}

impl Executor for RealShell {
  fn execute(&mut self, args: Vec<String>) -> String {
    let execution = Command::new(args[0].as_str())
      .args(&args[1..])
      .output()
      .expect("Couldn't run it");

    self.executed = Some(args);
    self.succeeded = execution.status.success();

    let output: String = String::from_utf8_lossy(&execution.stdout).into();

    output.trim_end().to_string()
  }

  fn last_executed(&self) -> Option<Vec<String>> {
    self.executed.clone()
  }

  fn last_succeeded(&self) -> bool {
    self.succeeded
  }
}

static INSTANCE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_SNAPSHOT_ATTEMPTS: usize = 3;

fn shell_quote(value: &str) -> String {
  format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Viewport {
  Live,
  CopyMode { scroll_position: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneState {
  pane_id: String,
  viewport: Viewport,
  height: i32,
  width: usize,
  history_size: usize,
  zoomed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotError {
  RuntimeSetupFailed,
  CaptureFailed,
  PaneChanged,
  SourcePaneUnavailable,
  ViewportUnavailable,
}

impl PaneState {
  fn is_stable_after(&self, other: &PaneState) -> bool {
    if self.pane_id != other.pane_id
      || self.height != other.height
      || self.width != other.width
      || self.zoomed != other.zoomed
    {
      return false;
    }

    match (&self.viewport, &other.viewport) {
      (Viewport::Live, Viewport::Live) => true,
      (
        Viewport::CopyMode {
          scroll_position: before,
        },
        Viewport::CopyMode { scroll_position: after },
      ) => before == after && self.history_size == other.history_size,
      _ => false,
    }
  }
}

#[allow(dead_code)]
fn dbg(msg: &str) {
  let mut file = std::fs::OpenOptions::new()
    .create(true)
    .write(true)
    .append(true)
    .open("/tmp/thumbs.log")
    .expect("Unable to open log file");

  writeln!(&mut file, "{}", msg).expect("Unable to write log file");
}

pub struct Swapper<'a> {
  executor: Box<&'a mut dyn Executor>,
  source_pane_id: String,
  dir: String,
  command: String,
  upcase_command: String,
  multi_command: String,
  osc52: bool,
  active_pane_id: Option<String>,
  active_pane_width: Option<usize>,
  active_pane_zoomed: Option<bool>,
  thumbs_pane_id: Option<String>,
  content: Option<String>,
  signal: String,
  runtime_dir: PathBuf,
  tmp_file: String,
}

impl<'a> Swapper<'a> {
  fn new(
    executor: Box<&'a mut dyn Executor>,
    source_pane_id: String,
    dir: String,
    command: String,
    upcase_command: String,
    multi_command: String,
    osc52: bool,
  ) -> Swapper {
    let since_the_epoch = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("Time went backwards");
    let instance = format!(
      "{}-{}-{}",
      std::process::id(),
      since_the_epoch.as_nanos(),
      INSTANCE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let signal = format!("thumbs-finished-{}", instance);
    let temp_root = std::env::temp_dir();
    let temp_root = fs::canonicalize(&temp_root).unwrap_or(temp_root);
    let runtime_dir = temp_root.join(format!("tmux-thumbs-{}", instance));
    let tmp_file = runtime_dir.join("selection").to_string_lossy().into_owned();

    Swapper {
      executor,
      source_pane_id,
      dir,
      command,
      upcase_command,
      multi_command,
      osc52,
      active_pane_id: None,
      active_pane_width: None,
      active_pane_zoomed: None,
      thumbs_pane_id: None,
      content: None,
      signal,
      runtime_dir,
      tmp_file,
    }
  }

  fn read_source_pane_state(&mut self) -> Result<PaneState, SnapshotError> {
    let state_command = vec![
      "tmux".to_string(),
      "display-message".to_string(),
      "-p".to_string(),
      "-t".to_string(),
      self.source_pane_id.clone(),
      "#{pane_id}:#{pane_mode}:#{pane_height}:#{pane_width}:#{scroll_position}:#{history_size}:#{window_zoomed_flag}"
        .to_string(),
    ];
    let output = self.executor.execute(state_command);
    if !self.executor.last_succeeded() {
      return Err(SnapshotError::SourcePaneUnavailable);
    }

    let fields = output.split(':').collect::<Vec<_>>();
    if fields.len() != 7 || fields[0] != self.source_pane_id {
      return Err(SnapshotError::SourcePaneUnavailable);
    }

    let viewport = match fields[1] {
      "" => Viewport::Live,
      "copy-mode" => {
        let scroll_position = fields[4]
          .parse::<i32>()
          .ok()
          .filter(|value| *value >= 0)
          .ok_or(SnapshotError::ViewportUnavailable)?;
        Viewport::CopyMode { scroll_position }
      }
      _ => return Err(SnapshotError::ViewportUnavailable),
    };
    let height = fields[2].parse().map_err(|_| SnapshotError::SourcePaneUnavailable)?;
    let width = fields[3].parse().map_err(|_| SnapshotError::SourcePaneUnavailable)?;
    let history_size = fields[5].parse().map_err(|_| SnapshotError::SourcePaneUnavailable)?;

    Ok(PaneState {
      pane_id: fields[0].to_string(),
      viewport,
      height,
      width,
      history_size,
      zoomed: fields[6] == "1",
    })
  }

  fn use_pane_state(&mut self, state: &PaneState) {
    self.active_pane_id = Some(state.pane_id.clone());
    self.active_pane_width = Some(state.width);
    self.active_pane_zoomed = Some(state.zoomed);
  }

  pub fn capture_active_pane(&mut self) {
    let state = self.read_source_pane_state().expect("Unable to retrieve source pane");
    self.use_pane_state(&state);
  }

  fn snapshot_path(&self, name: &str) -> PathBuf {
    self.runtime_dir.join(name)
  }

  fn prepare_snapshot_file(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().create_new(true).write(true).mode(0o600).open(path)?;
    file.set_permissions(Permissions::from_mode(0o600))
  }

  fn prepare_runtime_dir(&self) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(&self.runtime_dir)?;
    let result = Self::prepare_snapshot_file(&self.snapshot_path("screen"))
      .and_then(|_| Self::prepare_snapshot_file(&self.snapshot_path("joined")));
    if result.is_err() {
      self.cleanup_runtime_dir();
    }
    result
  }

  pub fn cleanup_runtime_dir(&self) {
    let _ = fs::remove_dir_all(&self.runtime_dir);
  }

  fn capture_snapshot_files(&mut self, state: &PaneState) -> bool {
    let screen_path = self.snapshot_path("screen");
    let joined_path = self.snapshot_path("joined");
    let screen_path = screen_path.to_string_lossy().into_owned();
    let joined_path = joined_path.to_string_lossy().into_owned();

    let screen_buffer = format!("{}-screen", self.signal);
    let joined_buffer = format!("{}-joined", self.signal);
    let mut command = vec![
      "tmux".to_string(),
      "capture-pane".to_string(),
      "-N".to_string(),
      "-b".to_string(),
      screen_buffer.clone(),
      "-t".to_string(),
      state.pane_id.clone(),
    ];
    if let Viewport::CopyMode { scroll_position } = state.viewport {
      command.extend([
        "-S".to_string(),
        (-scroll_position).to_string(),
        "-E".to_string(),
        (state.height - scroll_position - 1).to_string(),
      ]);
    }
    command.extend([
      ";".to_string(),
      "capture-pane".to_string(),
      "-J".to_string(),
      "-b".to_string(),
      joined_buffer.clone(),
      "-t".to_string(),
      state.pane_id.clone(),
    ]);
    if let Viewport::CopyMode { scroll_position } = state.viewport {
      command.extend([
        "-S".to_string(),
        (-scroll_position).to_string(),
        "-E".to_string(),
        (state.height - scroll_position - 1).to_string(),
      ]);
    }
    command.extend([
      ";".to_string(),
      "save-buffer".to_string(),
      "-b".to_string(),
      screen_buffer.clone(),
      screen_path,
      ";".to_string(),
      "save-buffer".to_string(),
      "-b".to_string(),
      joined_buffer.clone(),
      joined_path,
      ";".to_string(),
      "delete-buffer".to_string(),
      "-b".to_string(),
      screen_buffer,
      ";".to_string(),
      "delete-buffer".to_string(),
      "-b".to_string(),
      joined_buffer,
    ]);
    self.executor.execute(command);
    self.executor.last_succeeded()
  }

  fn capture_stable_snapshot(&mut self) -> Result<(), SnapshotError> {
    if self.prepare_runtime_dir().is_err() {
      return Err(SnapshotError::RuntimeSetupFailed);
    }

    for _ in 0..MAX_SNAPSHOT_ATTEMPTS {
      let before = match self.read_source_pane_state() {
        Ok(state) => state,
        Err(error) => {
          self.cleanup_runtime_dir();
          return Err(error);
        }
      };
      self.use_pane_state(&before);
      if !self.capture_snapshot_files(&before) {
        self.cleanup_runtime_dir();
        return Err(SnapshotError::CaptureFailed);
      }
      let after = match self.read_source_pane_state() {
        Ok(state) => state,
        Err(error) => {
          self.cleanup_runtime_dir();
          return Err(error);
        }
      };
      if before.is_stable_after(&after) {
        self.use_pane_state(&before);
        return Ok(());
      }
    }

    self.cleanup_runtime_dir();
    Err(SnapshotError::PaneChanged)
  }

  pub fn execute_thumbs(&mut self) -> bool {
    let options_command = vec!["tmux", "show", "-g"];
    let params: Vec<String> = options_command.iter().map(|arg| arg.to_string()).collect();
    let options = self.executor.execute(params);
    let lines: Vec<&str> = options.split('\n').collect();

    let pattern = Regex::new(r#"^@thumbs-([\w\-0-9]+)\s+"?([^"]+)"?$"#).unwrap();

    let args = lines
      .iter()
      .flat_map(|line| {
        if let Some(captures) = pattern.captures(line) {
          let name = captures.get(1).unwrap().as_str();
          let value = captures.get(2).unwrap().as_str();

          let boolean_params = vec!["reverse", "unique", "contrast"];

          if boolean_params.iter().any(|&x| x == name) {
            return vec![format!("--{}", name)];
          }

          let string_params = vec![
            "alphabet",
            "position",
            "fg-color",
            "bg-color",
            "hint-bg-color",
            "hint-fg-color",
            "select-fg-color",
            "select-bg-color",
            "multi-fg-color",
            "multi-bg-color",
          ];

          if string_params.iter().any(|&x| x == name) {
            return vec![format!("--{}", name), format!("'{}'", value)];
          }

          if name.starts_with("regexp") {
            return vec!["--regexp".to_string(), format!("'{}'", value.replace("\\\\", "\\"))];
          }

          vec![]
        } else {
          vec![]
        }
      })
      .collect::<Vec<String>>();

    let active_pane_id = self.active_pane_id.as_mut().unwrap().clone();

    let active_pane_zoomed = self.active_pane_zoomed.as_mut().unwrap().clone();
    let zoom_command = if active_pane_zoomed {
      format!("tmux resize-pane -t {} -Z;", active_pane_id)
    } else {
      "".to_string()
    };

    let pane_width_param = self
      .active_pane_width
      .filter(|width| *width > 0)
      .map(|width| format!(" --pane-width {}", width))
      .unwrap_or_default();
    let screen_path = shell_quote(&self.snapshot_path("screen").to_string_lossy());
    let joined_path = shell_quote(&self.snapshot_path("joined").to_string_lossy());
    let result_path = shell_quote(&self.tmp_file);
    let thumbs_binary = shell_quote(&format!("{}/target/release/thumbs", self.dir));

    let pane_command = format!(
        "umask 077; {thumbs_binary} -f '%U:%H' -t {result_path}{pane_width_param} --joined-capture {joined_path} {args} < {screen_path}; picker_status=$?; tmux swap-pane -t {active_pane_id}; {zoom_command} tmux wait-for -S {signal}; exit $picker_status",
        active_pane_id = active_pane_id,
        thumbs_binary = thumbs_binary,
        result_path = result_path,
        pane_width_param = pane_width_param,
        screen_path = screen_path,
        joined_path = joined_path,
        args = args.join(" "),
        zoom_command = zoom_command,
        signal = self.signal
    );

    let thumbs_command = vec![
      "tmux",
      "new-window",
      "-P",
      "-F",
      "#{pane_id}",
      "-d",
      "-n",
      "[thumbs]",
      pane_command.as_str(),
    ];

    let params: Vec<String> = thumbs_command.iter().map(|arg| arg.to_string()).collect();
    let thumbs_pane_id = self.executor.execute(params);
    if !self.executor.last_succeeded() || thumbs_pane_id.is_empty() {
      return false;
    }

    self.thumbs_pane_id = Some(thumbs_pane_id);
    true
  }

  pub fn swap_panes(&mut self) {
    let active_pane_id = self.active_pane_id.as_mut().unwrap().clone();
    let thumbs_pane_id = self.thumbs_pane_id.as_mut().unwrap().clone();

    let swap_command = vec![
      "tmux",
      "swap-pane",
      "-d",
      "-s",
      active_pane_id.as_str(),
      "-t",
      thumbs_pane_id.as_str(),
    ];

    let params = swap_command
      .iter()
      .filter(|&s| !s.is_empty())
      .map(|arg| arg.to_string())
      .collect();

    self.executor.execute(params);
  }

  pub fn resize_pane(&mut self) {
    let active_pane_zoomed = self.active_pane_zoomed.as_mut().unwrap().clone();

    if !active_pane_zoomed {
      return;
    }

    let thumbs_pane_id = self.thumbs_pane_id.as_mut().unwrap().clone();

    let resize_command = vec!["tmux", "resize-pane", "-t", thumbs_pane_id.as_str(), "-Z"];

    let params = resize_command
      .iter()
      .filter(|&s| !s.is_empty())
      .map(|arg| arg.to_string())
      .collect();

    self.executor.execute(params);
  }

  pub fn wait_thumbs(&mut self) {
    let wait_command = vec!["tmux", "wait-for", self.signal.as_str()];
    let params = wait_command.iter().map(|arg| arg.to_string()).collect();

    self.executor.execute(params);
  }

  pub fn notify_failure(&mut self, message: &str) {
    let message_command = vec!["tmux".to_string(), "display-message".to_string(), message.to_string()];
    self.executor.execute(message_command);
  }

  pub fn retrieve_content(&mut self) {
    let retrieve_command = vec!["cat", self.tmp_file.as_str()];
    let params = retrieve_command.iter().map(|arg| arg.to_string()).collect();

    self.content = Some(self.executor.execute(params));
  }

  pub fn send_osc52(&mut self) {}

  pub fn execute_command(&mut self) {
    let content = self.content.clone().unwrap();
    let items: Vec<&str> = content.split('\n').collect();

    if items.len() > 1 {
      let text = items
        .iter()
        .map(|item| item.splitn(2, ':').last().unwrap())
        .collect::<Vec<&str>>()
        .join(" ");

      self.execute_final_command(&text, &self.multi_command.clone());

      return;
    }

    // Only one item
    let item: &str = items.first().unwrap();

    let mut splitter = item.splitn(2, ':');

    if let Some(upcase) = splitter.next() {
      if let Some(text) = splitter.next() {
        if self.osc52 {
          let base64_text = base64::encode(text.as_bytes());
          let osc_seq = format!("\x1b]52;0;{}\x07", base64_text);
          let tmux_seq = format!("\x1bPtmux;{}\x1b\\", osc_seq.replace("\x1b", "\x1b\x1b"));

          // FIXME: Review if this comment is still rellevant
          //
          // When the user selects a match:
          // 1. The `rustbox` object created in the `viewbox` above is dropped.
          // 2. During its `drop`, the `rustbox` object sends a CSI 1049 escape
          //    sequence to tmux.
          // 3. This escape sequence causes the `window_pane_alternate_off` function
          //    in tmux to be called.
          // 4. In `window_pane_alternate_off`, tmux sets the needs-redraw flag in the
          //    pane.
          // 5. If we print the OSC copy escape sequence before the redraw is completed,
          //    tmux will *not* send the sequence to the host terminal. See the following
          //    call chain in tmux: `input_dcs_dispatch` -> `screen_write_rawstring`
          //    -> `tty_write` -> `tty_client_ready`. In this case, `tty_client_ready`
          //    will return false, thus preventing the escape sequence from being sent.
          //
          // Therefore, for now we wait a little bit here for the redraw to finish.
          std::thread::sleep(std::time::Duration::from_millis(100));

          std::io::stdout().write_all(tmux_seq.as_bytes()).unwrap();
          std::io::stdout().flush().unwrap();
        }

        let execute_command = if upcase.trim_end() == "true" {
          self.upcase_command.clone()
        } else {
          self.command.clone()
        };

        // The command we run has two arguments:
        //  * The first arg is the (trimmed) text. This gets stored in a variable, in order to
        //    preserve quoting and special characters.
        //
        //  * The second argument is the user's command, with the '{}' token replaced with an
        //    unquoted reference to the variable containing the text.
        //
        // The reference is unquoted, unfortunately, because the token may already have been
        // spliced into a string (e.g 'tmux display-message "Copied {}"'), and it's impossible (or
        // at least exceedingly difficult) to determine the correct quoting level.
        //
        // The alternative of literally splicing the text into the command is bad and it causes all
        // kinds of harmful escaping issues that the user cannot reasonable avoid.
        //
        // For example, imagine some pattern matched the text "foo;rm *" and the user's command was
        // an innocuous "echo {}". With literal splicing, we would run the command "echo foo;rm *".
        // That's BAD. Without splicing, instead we execute "echo ${THUMB}" which does mostly the
        // right thing regardless the contents of the text. (At worst, bash will word-separate the
        // unquoted variable; but it won't _execute_ those words in common scenarios).
        //
        // Ideally user commands would just use "${THUMB}" to begin with rather than having any
        // sort of ad-hoc string splicing here at all, and then they could specify the quoting they
        // want, but that would break backwards compatibility.
        self.execute_final_command(text.trim_end(), &execute_command);
      }
    }
  }

  pub fn execute_final_command(&mut self, text: &str, execute_command: &str) {
    let final_command = str::replace(execute_command, "{}", "${THUMB}");
    let retrieve_command = vec![
      "bash",
      "-c",
      "THUMB=\"$1\"; eval \"$2\"",
      "--",
      text,
      final_command.as_str(),
    ];

    let params = retrieve_command.iter().map(|arg| arg.to_string()).collect();

    self.executor.execute(params);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct TestShell {
    outputs: Vec<String>,
    successes: Vec<bool>,
    last_success: bool,
    executed: Vec<Vec<String>>,
  }

  impl TestShell {
    fn new(outputs: Vec<String>) -> TestShell {
      TestShell {
        successes: vec![true; outputs.len()],
        executed: Vec::new(),
        last_success: true,
        outputs,
      }
    }

    fn with_results(results: Vec<(String, bool)>) -> TestShell {
      let (outputs, successes): (Vec<String>, Vec<bool>) = results.into_iter().unzip();
      TestShell {
        outputs,
        successes,
        last_success: true,
        executed: Vec::new(),
      }
    }
  }

  impl Executor for TestShell {
    fn execute(&mut self, args: Vec<String>) -> String {
      self.executed.push(args);
      self.last_success = self.successes.pop().unwrap();
      self.outputs.pop().unwrap()
    }

    fn last_executed(&self) -> Option<Vec<String>> {
      self.executed.last().cloned()
    }

    fn last_succeeded(&self) -> bool {
      self.last_success
    }
  }

  fn pane_state(scroll_position: i32, history_size: usize) -> String {
    copy_state("%97", &scroll_position.to_string(), history_size)
  }

  fn copy_state(pane_id: &str, scroll_position: &str, history_size: usize) -> String {
    format!("{}:copy-mode:24:120:{}:{}:0", pane_id, scroll_position, history_size)
  }

  fn live_state(pane_id: &str, history_size: usize) -> String {
    format!("{}::24:120::{}:0", pane_id, history_size)
  }

  #[test]
  fn queries_the_explicit_source_pane() {
    let stable = copy_state("%97", "10", 100);
    let mut executor = TestShell::new(vec![stable.clone(), "".to_string(), stable]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
    swapper.cleanup_runtime_dir();
    drop(swapper);

    let state_commands = executor
      .executed
      .iter()
      .filter(|command| command.get(1).map(String::as_str) == Some("display-message"))
      .collect::<Vec<_>>();
    assert_eq!(state_commands.len(), 2);
    assert!(state_commands
      .iter()
      .all(|command| command.windows(2).any(|pair| pair == ["-t", "%97"])));
  }

  #[test]
  fn rejects_copy_mode_without_scroll_position() {
    let invalid = copy_state("%97", "", 100);
    let mut executor = TestShell::new(vec![invalid]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(
      swapper.capture_stable_snapshot(),
      Err(SnapshotError::ViewportUnavailable)
    );
    drop(swapper);
    assert!(executor
      .executed
      .iter()
      .all(|command| command.get(1).map(String::as_str) != Some("capture-pane")));
  }

  #[test]
  fn rejects_invalid_copy_mode_scroll_positions() {
    for scroll_position in ["-1", "invalid"] {
      let mut executor = TestShell::new(vec![copy_state("%97", scroll_position, 100)]);
      let mut swapper = Swapper::new(
        Box::new(&mut executor),
        "%97".to_string(),
        "/tmp/tmux-thumbs".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        false,
      );
      assert_eq!(
        swapper.capture_stable_snapshot(),
        Err(SnapshotError::ViewportUnavailable),
        "scroll_position={}",
        scroll_position
      );
    }
  }

  #[test]
  fn rejects_a_state_for_another_pane() {
    let mut executor = TestShell::new(vec![copy_state("%98", "10", 100)]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(
      swapper.capture_stable_snapshot(),
      Err(SnapshotError::SourcePaneUnavailable)
    );
  }

  #[test]
  fn accepts_live_viewport_without_scroll_range() {
    let stable = live_state("%97", 100);
    let mut executor = TestShell::new(vec![stable.clone(), "".to_string(), stable]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
    swapper.cleanup_runtime_dir();
    drop(swapper);
    let capture = executor
      .executed
      .iter()
      .find(|command| command.get(1).map(String::as_str) == Some("capture-pane"))
      .expect("capture command");
    assert!(!capture.iter().any(|argument| argument == "-S" || argument == "-E"));
  }

  #[test]
  fn rejects_unsupported_pane_modes() {
    let mut executor = TestShell::new(vec!["%97:clock-mode:24:120::100:0".to_string()]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    assert_eq!(
      swapper.capture_stable_snapshot(),
      Err(SnapshotError::ViewportUnavailable)
    );
  }

  #[test]
  fn shell_quotes_runtime_paths_with_spaces_quotes_and_metacharacters() {
    assert_eq!(
      shell_quote("/tmp/space dir/it's;fine"),
      "'/tmp/space dir/it'\"'\"'s;fine'"
    );
  }

  #[test]
  fn rejects_failed_snapshot_command_and_removes_private_runtime_dir() {
    let mut executor = TestShell::with_results(vec![("".to_string(), false), (pane_state(10, 100), true)]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    let runtime_dir = swapper.runtime_dir.clone();

    assert_eq!(swapper.capture_stable_snapshot(), Err(SnapshotError::CaptureFailed));
    assert!(!runtime_dir.exists());
  }

  #[test]
  fn does_not_remove_a_preexisting_runtime_path_it_does_not_own() {
    let mut executor = TestShell::new(Vec::new());
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    std::fs::create_dir(&swapper.runtime_dir).unwrap();
    let marker = swapper.runtime_dir.join("not-owned");
    std::fs::write(&marker, b"keep").unwrap();

    assert_eq!(
      swapper.capture_stable_snapshot(),
      Err(SnapshotError::RuntimeSetupFailed)
    );
    assert!(marker.exists());
    std::fs::remove_dir_all(&swapper.runtime_dir).unwrap();
  }

  #[test]
  fn creates_private_snapshot_directory_and_files() {
    let stable = pane_state(10, 100);
    let mut executor = TestShell::new(vec![stable.clone(), "".to_string(), stable]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
    assert_eq!(
      std::fs::metadata(&swapper.runtime_dir).unwrap().permissions().mode() & 0o777,
      0o700
    );
    assert_eq!(
      std::fs::metadata(swapper.snapshot_path("screen"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o600
    );
    assert_eq!(
      std::fs::metadata(swapper.snapshot_path("joined"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o600
    );
    swapper.cleanup_runtime_dir();
  }

  #[test]
  fn rejects_failed_picker_window_creation() {
    let mut executor = TestShell::with_results(vec![("".to_string(), false), ("".to_string(), true)]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    swapper.active_pane_id = Some("%97".to_string());
    swapper.active_pane_width = Some(120);
    swapper.active_pane_zoomed = Some(false);

    assert!(!swapper.execute_thumbs());
    assert!(swapper.thumbs_pane_id.is_none());
  }

  #[test]
  fn retries_snapshot_when_copy_mode_state_changes_during_capture() {
    let before_first = pane_state(10, 100);
    let after_first = pane_state(11, 101);
    let before_second = after_first.clone();
    let after_second = after_first.clone();
    let mut executor = TestShell::new(vec![
      after_second,
      "".to_string(),
      before_second,
      after_first,
      "".to_string(),
      before_first,
    ]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    assert_eq!(swapper.capture_stable_snapshot(), Ok(()));
    swapper.cleanup_runtime_dir();
    drop(swapper);

    let capture_attempts = executor
      .executed
      .iter()
      .filter(|command| command.get(1).map(String::as_str) == Some("capture-pane"))
      .count();
    assert_eq!(capture_attempts, 2);
  }

  #[test]
  fn rejects_snapshot_after_three_copy_mode_state_changes() {
    let mut executor = TestShell::new(vec![
      pane_state(13, 103),
      "".to_string(),
      pane_state(12, 102),
      pane_state(12, 102),
      "".to_string(),
      pane_state(11, 101),
      pane_state(11, 101),
      "".to_string(),
      pane_state(10, 100),
    ]);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    let runtime_dir = swapper.runtime_dir.clone();
    assert_eq!(swapper.capture_stable_snapshot(), Err(SnapshotError::PaneChanged));
    drop(swapper);

    let capture_attempts = executor
      .executed
      .iter()
      .filter(|command| command.get(1).map(String::as_str) == Some("capture-pane"))
      .count();
    assert_eq!(capture_attempts, 3);
    assert!(!runtime_dir.exists());
  }

  #[test]
  fn retrieve_active_pane() {
    let last_command_outputs = vec![copy_state("%97", "1", 100)];
    let mut executor = TestShell::new(last_command_outputs);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    swapper.capture_active_pane();

    assert_eq!(swapper.active_pane_id.unwrap(), "%97");
    assert_eq!(swapper.active_pane_width, Some(120));
  }

  #[test]
  fn passes_pane_width_to_thumbs() {
    let last_command_outputs = vec!["%100".to_string(), "".to_string()];
    let mut executor = TestShell::new(last_command_outputs);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    swapper.active_pane_id = Some("%97".to_string());
    swapper.active_pane_width = Some(120);
    swapper.active_pane_zoomed = Some(false);

    swapper.execute_thumbs();

    let command = executor.last_executed().unwrap();
    assert!(command.last().unwrap().contains("--pane-width 120"));
  }

  #[test]
  fn concurrent_swappers_use_unique_runtime_names() {
    let mut first_executor = TestShell::new(vec!["%100".to_string(), "".to_string()]);
    let mut second_executor = TestShell::new(vec!["%101".to_string(), "".to_string()]);
    let mut first = Swapper::new(
      Box::new(&mut first_executor),
      "%97".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    first.active_pane_id = Some("%97".to_string());
    first.active_pane_width = Some(120);
    first.active_pane_zoomed = Some(false);
    first.execute_thumbs();
    let first_runtime_dir = first.runtime_dir.to_string_lossy().into_owned();
    let first_command = first_executor.last_executed().unwrap().last().unwrap().clone();

    let mut second = Swapper::new(
      Box::new(&mut second_executor),
      "%98".to_string(),
      "/tmp/tmux-thumbs".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );
    second.active_pane_id = Some("%98".to_string());
    second.active_pane_width = Some(120);
    second.active_pane_zoomed = Some(false);
    second.execute_thumbs();
    let second_runtime_dir = second.runtime_dir.to_string_lossy().into_owned();
    let second_command = second_executor.last_executed().unwrap().last().unwrap().clone();

    assert!(first_command.contains(&first_runtime_dir));
    assert!(second_command.contains(&second_runtime_dir));
    assert_ne!(first_runtime_dir, second_runtime_dir);
    assert_ne!(first_command, second_command);
  }

  #[test]
  fn swap_panes() {
    let last_command_outputs = vec![
      "".to_string(),
      "%100".to_string(),
      "".to_string(),
      copy_state("%98", "1", 100),
    ];
    let mut executor = TestShell::new(last_command_outputs);
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%98".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      "".to_string(),
      false,
    );

    swapper.capture_active_pane();
    swapper.execute_thumbs();
    swapper.swap_panes();

    let expectation = vec!["tmux", "swap-pane", "-d", "-s", "%98", "-t", "%100"];

    assert_eq!(executor.last_executed().unwrap(), expectation);
  }

  #[test]
  fn quoted_execution() {
    let last_command_outputs = vec!["Blah blah blah, the ignored user script output".to_string()];
    let mut executor = TestShell::new(last_command_outputs);

    let user_command = "echo \"{}\"".to_string();
    let upcase_command = "open \"{}\"".to_string();
    let multi_command = "open \"{}\"".to_string();
    let mut swapper = Swapper::new(
      Box::new(&mut executor),
      "%97".to_string(),
      "".to_string(),
      user_command,
      upcase_command,
      multi_command,
      false,
    );

    swapper.content = Some(format!(
      "{do_upcase}:{thumb_text}",
      do_upcase = false,
      thumb_text = "foobar;rm *",
    ));
    swapper.execute_command();

    let expectation = vec![
      "bash",
      // The actual shell command:
      "-c",
      "THUMB=\"$1\"; eval \"$2\"",
      // $0: The non-existent program name.
      "--",
      // $1: The value assigned to THUMB above.
      //     Not interpreted as a shell expression!
      "foobar;rm *",
      // $2: The user script, with {} replaced with ${THUMB},
      //     and will be eval'd with THUMB in scope.
      "echo \"${THUMB}\"",
    ];

    assert_eq!(executor.last_executed().unwrap(), expectation);
  }
}

fn app_args<'a>() -> clap::ArgMatches<'a> {
  App::new("tmux-thumbs")
    .version(crate_version!())
    .about("A lightning fast version of tmux-fingers, copy/pasting tmux like vimium/vimperator")
    .arg(
      Arg::with_name("dir")
        .help("Directory where to execute thumbs")
        .long("dir")
        .default_value(""),
    )
    .arg(
      Arg::with_name("pane_id")
        .help("Source tmux pane to capture")
        .long("pane-id")
        .takes_value(true)
        .required(true),
    )
    .arg(
      Arg::with_name("command")
        .help("Command to execute after choose a hint")
        .long("command")
        .default_value("tmux set-buffer -- \"{}\" && tmux display-message \"Copied {}\""),
    )
    .arg(
      Arg::with_name("upcase_command")
        .help("Command to execute after choose a hint, in upcase")
        .long("upcase-command")
        .default_value("tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Copied {}\""),
    )
    .arg(
      Arg::with_name("multi_command")
        .help("Command to execute after choose multiple hints")
        .long("multi-command")
        .default_value("tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Multi copied {}\""),
    )
    .arg(
      Arg::with_name("osc52")
        .help("Print OSC52 copy escape sequence in addition to running the pick command")
        .long("osc52")
        .short("o"),
    )
    .get_matches()
}

fn main() -> std::io::Result<()> {
  let args = app_args();
  let source_pane_id = args.value_of("pane_id").unwrap();
  let dir = args.value_of("dir").unwrap();
  let command = args.value_of("command").unwrap();
  let upcase_command = args.value_of("upcase_command").unwrap();
  let multi_command = args.value_of("multi_command").unwrap();
  let osc52 = args.is_present("osc52");

  if dir.is_empty() {
    panic!("Invalid tmux-thumbs execution. Are you trying to execute tmux-thumbs directly?")
  }

  let mut executor = RealShell::new();
  let mut swapper = Swapper::new(
    Box::new(&mut executor),
    source_pane_id.to_string(),
    dir.to_string(),
    command.to_string(),
    upcase_command.to_string(),
    multi_command.to_string(),
    osc52,
  );

  if let Err(error) = swapper.capture_stable_snapshot() {
    let message = match error {
      SnapshotError::RuntimeSetupFailed => "tmux-thumbs: failed to create private snapshot directory",
      SnapshotError::CaptureFailed => "tmux-thumbs: failed to capture pane; retry",
      SnapshotError::PaneChanged => "tmux-thumbs: pane changed during capture; retry",
      SnapshotError::SourcePaneUnavailable => "tmux-thumbs: source pane unavailable; retry",
      SnapshotError::ViewportUnavailable => "tmux-thumbs: copy-mode viewport unavailable; retry",
    };
    swapper.notify_failure(message);
    return Ok(());
  }
  if !swapper.execute_thumbs() {
    swapper.cleanup_runtime_dir();
    swapper.notify_failure("tmux-thumbs: failed to create picker window");
    return Ok(());
  }
  swapper.swap_panes();
  swapper.resize_pane();
  swapper.wait_thumbs();
  swapper.retrieve_content();
  swapper.execute_command();
  swapper.cleanup_runtime_dir();

  Ok(())
}
