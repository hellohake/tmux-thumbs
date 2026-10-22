# tmux-thumbs Command Alias Argument Design

## Problem

`Prefix+f` displays `tmux-thumbs: source pane unavailable; retry` instead of
opening the picker. The generated binding currently has this parsed shape:

```text
run-shell -b /path/to/tmux-thumbs.sh "#{pane_id}"
```

Although the pane format is present in `list-keys`, it is a second tmux
argument. `run-shell` accepts the shell command as one argument, so it executes
`tmux-thumbs.sh` without the pane ID. The launch script then correctly rejects
the empty source pane.

## Scope

- Preserve the existing `thumbs-pick` command alias and configured prefix key.
- Preserve explicit source-pane capture; do not fall back to `$TMUX_PANE`.
- Change only the alias command grouping needed to pass the pane ID.
- Leave unrelated duplicate historical aliases and picker behavior unchanged.

## Design

Store the script path and expanded pane ID inside one quoted `run-shell`
shell-command argument. The effective binding must have this shape:

```text
run-shell -b "/path/to/tmux-thumbs.sh '#{pane_id}'"
```

At key invocation, tmux expands `#{pane_id}` in the invoking pane context and
passes the resulting value to `tmux-thumbs.sh` as its first argument. The
existing script validation and Rust snapshot flow remain unchanged.

## Verification

Extend the tmux integration test so it rejects a binding where the script path
and pane ID are separate `run-shell` arguments. The test must fail against the
current implementation before the production change, then pass after the alias
stores a single grouped shell command.

After the automated suite passes, reload the plugin in an isolated tmux server
and invoke the real prefix binding. The picker must render hints for the source
pane without displaying the unavailable-pane message.
