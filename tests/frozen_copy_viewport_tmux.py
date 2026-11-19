#!/usr/bin/env python3
"""The picker must keep copy-mode's frozen rows after background output."""

import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent
BINARY = Path(os.environ.get("TMUX_THUMBS_TEST_BINARY", ROOT / "target/release/tmux-thumbs"))
SOCKET = f"thumbs-frozen-viewport-{os.getpid()}"


def tmux(*args):
    return subprocess.check_output(["tmux", "-L", SOCKET, *args], text=True, timeout=10).rstrip("\n")


def wait_until(description, predicate):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.01)
    raise AssertionError(f"timed out waiting for {description}")


def copy_state(pane):
    return tmux(
        "display-message", "-p", "-t", pane,
        "#{pane_mode}:#{pane_width}:#{pane_height}:#{copy_cursor_x}:#{copy_cursor_y}:"
        "#{scroll_position}:#{rectangle_toggle}:#{selection_active}:#{selection_present}",
    )


def capture_reference(pane):
    # The fixture is stopped here: its base grid still equals the frozen grid.
    height, scroll = map(int, tmux(
        "display-message", "-p", "-t", pane, "#{pane_height}:#{scroll_position}",
    ).split(":"))
    return tmux(
        "capture-pane", "-p", "-N", "-t", pane,
        "-S", str(-scroll), "-E", str(height - scroll - 1),
    ).splitlines()


def assert_same_rows(reference, overlay):
    assert len(overlay) == len(reference), (len(reference), len(overlay))
    for row, (expected, actual) in enumerate(zip(reference, overlay), 1):
        if "/tmp/visible-target.txt" in expected:
            # A hint deliberately overwrites part of this match.
            assert actual.startswith(expected.split("/tmp/")[0]), (row, expected, actual)
            end = expected.index("/tmp/visible-target.txt") + len("/tmp/visible-target.txt")
            assert actual[end:].rstrip() == expected[end:].rstrip(), (row, expected, actual)
        else:
            assert actual.rstrip() == expected.rstrip(), (
                f"visible row {row} moved or changed", expected, actual,
            )


def check_viewport(workspace, mode_keys, update_background, rectangle=False, split=False, zoom=False, cursor_row=4):
    name = f"{mode_keys}-{int(update_background)}-{int(rectangle)}-{int(split)}-{int(zoom)}-{cursor_row}"
    ready = workspace / f"{name}-ready"
    repaint = workspace / f"{name}-repaint"
    updated = workspace / f"{name}-updated"
    code = f"""
import pathlib, time
for i in range(1, 201):
    marker = '/tmp/visible-target.txt '
    print(f'ROW{{i:03d}} 中文内容 ' + marker + 'x' * 140, flush=True)
pathlib.Path({str(ready)!r}).touch()
while not pathlib.Path({str(repaint)!r}).exists():
    time.sleep(0.01)
for i in range(201, 241):
    print(f'NEW{{i:03d}} 后台重绘 ' + 'z' * 100, flush=True)
pathlib.Path({str(updated)!r}).touch()
time.sleep(60)
"""
    pane = tmux(
        "new-window", "-d", "-P", "-F", "#{pane_id}", "-n", name,
        "python3 -c " + shlex.quote(code),
    )
    window = tmux("display-message", "-p", "-t", pane, "#{window_id}")
    tmux("select-window", "-t", window)
    if split:
        tmux("split-window", "-h", "-d", "-t", pane, "sleep 60")
    if zoom:
        tmux("resize-pane", "-t", pane, "-Z")
    wait_until("fixture output", ready.exists)
    wait_until("fixture terminal output", lambda: "ROW200" in tmux("capture-pane", "-p", "-t", pane))
    tmux("set-window-option", "-t", pane, "mode-keys", mode_keys)
    tmux("copy-mode", "-t", pane)
    tmux("send-keys", "-t", pane, "-X", "history-top")
    tmux("send-keys", "-t", pane, "-X", "page-down")
    tmux("send-keys", "-t", pane, "-X", "-N", str(cursor_row), "cursor-down")
    tmux("send-keys", "-t", pane, "-X", "-N", "10", "cursor-right")
    if rectangle:
        tmux("send-keys", "-t", pane, "-X", "rectangle-on")
    reference = capture_reference(pane)
    assert any("中文内容" in row for row in reference)
    assert any("/tmp/visible-target.txt" in row for row in reference), reference
    before = copy_state(pane)
    initial_panes = tmux("list-panes", "-t", window, "-F", "#{pane_id}").splitlines()
    buffers_before = tmux("list-buffers", "-F", "#{buffer_name}:#{buffer_sample}")
    if update_background:
        repaint.touch()
        wait_until("background repaint", updated.exists)
        assert copy_state(pane) == before, "fixture moved the frozen viewport"

    launcher = (
        f"TMPDIR={shlex.quote(str(workspace))} {shlex.quote(str(BINARY))} "
        f"--pane-id {shlex.quote(pane)} --dir {shlex.quote(str(ROOT))} "
        f"> {shlex.quote(str(workspace / 'launcher.log'))} 2>&1"
    )
    tmux("run-shell", "-b", "-t", pane, launcher)

    def picker_pane():
        panes = tmux("list-panes", "-t", window, "-F", "#{pane_id}").splitlines()
        return next((candidate for candidate in panes if candidate not in initial_panes), None)

    try:
        wait_until("picker pane", picker_pane)
    except AssertionError:
        raise AssertionError((workspace / 'launcher.log').read_text())
    picker = picker_pane()
    wait_until("picker render", lambda: "ROW" in tmux("capture-pane", "-p", "-t", picker))
    overlay = tmux("capture-pane", "-p", "-t", picker).splitlines()
    try:
        assert_same_rows(reference, overlay)
        assert tmux("list-buffers", "-F", "#{buffer_name}:#{buffer_sample}") == buffers_before
    finally:
        tmux("send-keys", "-t", picker, "Escape")
        wait_until("source restoration", lambda: picker_pane() is None)
    assert copy_state(pane) == before, ("copy state changed after cancel", before, copy_state(pane))
    wait_until("snapshot cleanup", lambda: not list(workspace.glob("tmux-thumbs-*")))
    tmux("kill-window", "-t", window)


def check_failed_snapshot_preserves_source(workspace, existing_selection):
    pane = tmux(
        "new-window", "-d", "-P", "-F", "#{pane_id}",
        "printf '中文 /tmp/guarded-target.txt\\n'; sleep 60",
    )
    window = tmux("display-message", "-p", "-t", pane, "#{window_id}")
    tmux("select-window", "-t", window)
    wait_until("guard fixture", lambda: "guarded-target" in tmux("capture-pane", "-p", "-t", pane))
    tmux("copy-mode", "-t", pane)
    tmux("send-keys", "-t", pane, "-X", "top-line")
    if existing_selection:
        tmux("send-keys", "-t", pane, "-X", "begin-selection")
        tmux("send-keys", "-t", pane, "-X", "-N", "4", "cursor-right")
    before = copy_state(pane)
    selection_format = "#{selection_start_x}:#{selection_start_y}:#{selection_end_x}:#{selection_end_y}"
    selection_before = tmux("display-message", "-p", "-t", pane, selection_format)
    buffers_before = tmux("list-buffers", "-F", "#{buffer_name}:#{buffer_sample}")
    environment = ""
    if not existing_selection:
        # A pipe job that never marks completion must time out after restoring
        # copy-mode, without opening an empty picker or leaving private files.
        wrapper_dir = workspace / "failed-pipe-bin"
        wrapper_dir.mkdir()
        wrapper = wrapper_dir / "tmux"
        wrapper.write_text(
            "#!/usr/bin/env python3\nimport os, sys\nfrom pathlib import Path\n"
            "args = sys.argv[1:]\n"
            "if args[0] == 'source-file':\n"
            "    script = Path(args[1])\n"
            "    lines = script.read_text().splitlines()\n"
            "    lines = [line.split(' pipe-no-clear ')[0] + ' pipe-no-clear false' "
            "if ' pipe-no-clear ' in line else line for line in lines]\n"
            "    script.write_text('\\n'.join(lines))\n"
            "os.execv(os.environ['THUMBS_REAL_TMUX'], ['tmux', *args])\n"
        )
        wrapper.chmod(0o700)
        environment = (
            f"PATH={shlex.quote(str(wrapper_dir) + ':' + os.environ['PATH'])} "
            f"THUMBS_REAL_TMUX={shlex.quote(shutil.which('tmux'))} "
        )
    launcher = (
        f"{environment}TMPDIR={shlex.quote(str(workspace))} {shlex.quote(str(BINARY))} "
        f"--pane-id {shlex.quote(pane)} --dir {shlex.quote(str(ROOT))}"
    )
    tmux("run-shell", "-t", pane, launcher)
    assert tmux("list-panes", "-t", window, "-F", "#{pane_id}") == pane
    assert "[thumbs]" not in tmux("list-windows", "-F", "#{window_name}").splitlines()
    assert copy_state(pane) == before
    assert tmux("display-message", "-p", "-t", pane, selection_format) == selection_before
    assert tmux("list-buffers", "-F", "#{buffer_name}:#{buffer_sample}") == buffers_before
    assert not list(workspace.glob("tmux-thumbs-*"))
    tmux("kill-window", "-t", window)


def check_blank_top_row_preserves_cursor(workspace):
    pane = tmux(
        "new-window", "-d", "-P", "-F", "#{pane_id}", "-n", "blank-top-row",
        "printf '\\033[H\\033[2J\\033[2;1Habcdefghijklmnopqrstuvwxyz0123456789"
        "\\033[3;1Hmiddle /tmp/cursor-target.txt padding padding'; sleep 60",
    )
    window = tmux("display-message", "-p", "-t", pane, "#{window_id}")
    tmux("resize-window", "-t", pane, "-x", "60", "-y", "8")
    tmux("select-window", "-t", window)
    wait_until("blank row fixture", lambda: "middle" in tmux("capture-pane", "-p", "-t", pane))
    tmux("set-window-option", "-t", pane, "mode-keys", "vi")
    tmux("copy-mode", "-t", pane)
    tmux("send-keys", "-t", pane, "-X", "top-line")
    tmux("send-keys", "-t", pane, "-X", "cursor-down")
    tmux("send-keys", "-t", pane, "-X", "start-of-line")
    tmux("send-keys", "-t", pane, "-X", "-N", "20", "cursor-right")
    tmux("send-keys", "-t", pane, "-X", "cursor-down")
    tmux("send-keys", "-t", pane, "-X", "-N", "15", "cursor-left")
    before = copy_state(pane)
    assert before.split(":")[3:5] == ["5", "2"], before
    launcher = (
        f"TMPDIR={shlex.quote(str(workspace))} {shlex.quote(str(BINARY))} "
        f"--pane-id {shlex.quote(pane)} --dir {shlex.quote(str(ROOT))}"
    )
    tmux("run-shell", "-b", "-t", pane, launcher)
    def picker_pane():
        panes = tmux("list-panes", "-t", window, "-F", "#{pane_id}").splitlines()
        return panes[0] if panes != [pane] else None
    wait_until("blank row picker", picker_pane)
    picker = picker_pane()
    wait_until("blank row render", lambda: "middle" in tmux("capture-pane", "-p", "-t", picker))
    tmux("send-keys", "-t", picker, "Escape")
    wait_until("blank row source restoration", lambda: picker_pane() is None)
    assert copy_state(pane) == before, (before, copy_state(pane))
    wait_until("blank row snapshot cleanup", lambda: not list(workspace.glob("tmux-thumbs-*")))
    tmux("kill-window", "-t", window)


def check_emoji_rows(workspace):
    rows = [
        "x" * 34 + "👩\u200d💻" + "wxyz",
        "/tmp/visible-target.txt",
        "FOLLOWING_MARKER",
        "z" * 38 + "♥\ufe0f",
        "AFTER_VARIATION_SELECTOR",
    ]
    command = "printf %s " + shlex.quote("\r\n".join(rows) + "\r\n") + "; sleep 60"
    pane = tmux("new-window", "-d", "-P", "-F", "#{pane_id}", "-n", "emoji-rows", command)
    window = tmux("display-message", "-p", "-t", pane, "#{window_id}")
    tmux("resize-window", "-t", pane, "-x", "40", "-y", "8")
    tmux("select-window", "-t", window)
    wait_until("emoji fixture", lambda: "AFTER_VARIATION_SELECTOR" in tmux("capture-pane", "-p", "-t", pane))
    tmux("copy-mode", "-t", pane)
    reference = capture_reference(pane)
    launcher = (
        f"TMPDIR={shlex.quote(str(workspace))} {shlex.quote(str(BINARY))} "
        f"--pane-id {shlex.quote(pane)} --dir {shlex.quote(str(ROOT))}"
    )
    tmux("run-shell", "-b", "-t", pane, launcher)
    def picker_pane():
        panes = tmux("list-panes", "-t", window, "-F", "#{pane_id}").splitlines()
        return panes[0] if panes != [pane] else None
    wait_until("emoji picker", picker_pane)
    picker = picker_pane()
    wait_until("emoji render", lambda: "FOLLOWING_MARKER" in tmux("capture-pane", "-p", "-t", picker))
    overlay = tmux("capture-pane", "-p", "-t", picker).splitlines()
    try:
        assert_same_rows(reference, overlay)
    finally:
        tmux("send-keys", "-t", picker, "Escape")
        wait_until("emoji source restoration", lambda: picker_pane() is None)
    wait_until("emoji snapshot cleanup", lambda: not list(workspace.glob("tmux-thumbs-*")))
    tmux("kill-window", "-t", window)


def main():
    with tempfile.TemporaryDirectory(prefix="thumbs-frozen-viewport-") as directory:
        workspace = Path(directory)
        try:
            tmux("-f", "/dev/null", "new-session", "-d", "-x", "120", "-y", "30", "-s", "test")
            tmux("set-option", "-g", "default-size", "120x30")
            tmux("set-option", "-g", "@thumbs-unique", "1")
            tmux("set-option", "-g", "@thumbs-position", "right")
            tmux("set-buffer", "-b", "sentinel", "keep-existing-clipboard")
            for mode_keys, update, rectangle in [
                ("vi", True, False), ("emacs", True, False),
                ("vi", False, True), ("emacs", False, False),
            ]:
                check_viewport(workspace, mode_keys, update, rectangle)
            check_viewport(workspace, "vi", True, split=True)
            check_viewport(workspace, "vi", True, split=True, zoom=True)
            check_viewport(workspace, "vi", True, cursor_row=5)
            check_failed_snapshot_preserves_source(workspace, existing_selection=True)
            check_failed_snapshot_preserves_source(workspace, existing_selection=False)
            check_emoji_rows(workspace)
            check_blank_top_row_preserves_cursor(workspace)
        finally:
            subprocess.run(["tmux", "-L", SOCKET, "kill-server"], stderr=subprocess.DEVNULL)
    print("frozen copy-mode viewport test: ok")


if __name__ == "__main__":
    main()
