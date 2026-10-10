# Stable Copy-Mode Snapshot Design

## Problem

`tmux-thumbs` reads a pane's copy-mode geometry and scroll position, creates a
detached `[thumbs]` window whose shell captures the source pane, and immediately
swaps the panes. The detached shell and the parent race: if the swap resizes or
reflows the source pane before `capture-pane` runs, the saved scroll coordinates
refer to a different history layout. Output arriving between the state read and
the capture can invalidate the same coordinates.

The user-visible symptom is a hint overlay for content below the page that was
visible when tmux-thumbs was invoked.

## Required Behavior

- Candidate content must come from a snapshot completed before any temporary
  window can resize or reflow the source pane.
- Physical (`capture-pane -N`) and logically joined (`capture-pane -J`) captures
  must describe the same stable pane state.
- Copy-mode captures must be retried when pane identity, dimensions, scroll
  position, mode state, or history size changes during capture.
- Retries are bounded. If no stable snapshot is available after three attempts,
  tmux-thumbs must fail closed with a visible tmux message and must not open a
  misleading overlay.
- Existing normal-pane, hard-wrap, Unicode, URL, path, clipboard, zoom, and
  concurrent invocation behavior must remain unchanged.

## Design

Represent the source pane metadata as one comparable `PaneState` value. A new
`capture_stable_snapshot` operation performs up to three attempts:

1. Read `PaneState`.
2. Atomically create an invocation-private `0700` runtime directory with
   `create_new` `0600` physical and joined files, then synchronously save both
   captures into those files.
3. Read `PaneState` again.
4. Accept the files only when the state remained compatible; otherwise overwrite
   them on the next attempt.

For copy mode, compatibility includes history size and scroll position. For a
normal live pane, only identity, dimensions, and mode must remain stable because
the default visible capture is already defined at capture time.

Only after the snapshot is accepted may `execute_thumbs` create the detached
window. Its command reads the frozen files, restores the source pane, and signals
completion. The parent retains ownership of the runtime directory until it has
validated `new-window`, waited for picker completion, and retrieved the selected
content; it then removes the entire directory. `swap_panes` therefore cannot
change the data being matched.

## Error Handling

After three unstable attempts, remove the owned runtime directory, display
`tmux-thumbs: pane changed during capture; retry`, and return without creating a
temporary window. Capture failure and picker-window creation failure have their
own messages and also return before swap or wait. A pre-existing runtime path is
never followed or removed. Picker shell arguments use POSIX quoting, including
paths derived from a canonicalized user-provided `TMPDIR`.

## Verification

Add a real-tmux regression that:

- creates wrapped output in a `120x30` source window;
- enters copy mode and records the visible page;
- delays `capture-pane` and forces the detached window to `80x23`, guaranteeing
  the old implementation swaps and reflows before capture;
- asserts the overlay still contains the pre-swap page;
- waits for source-pane restoration and asserts the private runtime root is
  empty after the parent process finishes cleanup.

Run Rust unit tests, the existing hard-wrap integration test, and the new
copy-mode snapshot integration test against a release build.
