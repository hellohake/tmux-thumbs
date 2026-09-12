#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_NAME="tmux-thumbs-hard-wrap-$$"
SESSION_NAME="hard-wrap"
CAPTURE_FILE="$(mktemp /tmp/tmux-thumbs-hard-wrap-capture.XXXXXX)"
EXPECTED='.ai_doc/records/inbox/life-card-admin-20260911-01a08c45/handoff.md'
EXPECTED_URL='https://host/a'

cleanup() {
  tmux -L "${SOCKET_NAME}" kill-server 2>/dev/null || true
  rm -f -- "${CAPTURE_FILE}"
}
trap cleanup EXIT

fail() {
  printf 'hard-wrap tmux test: %s\n' "$*" >&2
  exit 1
}

wait_until() {
  local description="$1"
  shift
  local attempt
  for attempt in $(seq 1 100); do
    if "$@"; then
      return 0
    fi
    sleep 0.05
  done
  fail "timed out waiting for ${description}"
}

test -x "${ROOT_DIR}/target/release/thumbs" || fail 'release thumbs binary is missing'
test -x "${ROOT_DIR}/target/release/tmux-thumbs" || fail 'release tmux-thumbs binary is missing'

SOURCE_COMMAND="printf '\033[?7l\033[H\033[2J%37s%s\033[2;1H  %s\033[3;1H%s\033[?7h' '' '(.ai_doc/records/inbox/' 'life-card-admin-20260911-01a08c45/handoff.md)。' '（https://host/a），发布于'; exec sleep 120"

tmux -L "${SOCKET_NAME}" -f /dev/null new-session -d -x 60 -y 8 -s "${SESSION_NAME}" "${SOURCE_COMMAND}"
SOURCE_PANE="$(tmux -L "${SOCKET_NAME}" display-message -p -t "${SESSION_NAME}:" '#{pane_id}')"
SOURCE_WINDOW="$(tmux -L "${SOCKET_NAME}" display-message -p -t "${SOURCE_PANE}" '#{window_id}')"
tmux -L "${SOCKET_NAME}" set-option -g @thumbs-alphabet abcd
tmux -L "${SOCKET_NAME}" set-option -g @thumbs-contrast 1

SOURCE_CAPTURE="$(tmux -L "${SOCKET_NAME}" capture-pane -p -t "${SOURCE_PANE}")"
grep -Fq '(.ai_doc/records/inbox/' <<<"${SOURCE_CAPTURE}" || fail 'first physical path fragment is missing'
grep -Fq '  life-card-admin-20260911-01a08c45/handoff.md)。' <<<"${SOURCE_CAPTURE}" || fail 'second physical path fragment is missing'
grep -Fq '（https://host/a），发布于' <<<"${SOURCE_CAPTURE}" || fail 'URL and adjacent Chinese prose are missing'

tmux -L "${SOCKET_NAME}" run-shell -b -t "${SOURCE_PANE}" \
  "${ROOT_DIR}/target/release/tmux-thumbs --dir '${ROOT_DIR}'"

thumbs_is_running() {
  test "$(tmux -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')" != "${SOURCE_PANE}"
}
wait_until 'thumbs pane' thumbs_is_running
THUMBS_PANE="$(tmux -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')"

tmux -L "${SOCKET_NAME}" capture-pane -e -p -t "${THUMBS_PANE}" >"${CAPTURE_FILE}"

python3 - "${CAPTURE_FILE}" <<'PY'
import pathlib
import re
import sys

content = pathlib.Path(sys.argv[1]).read_bytes()
plain = re.sub(rb"\x1b\[[0-9;:]*m", b"", content).decode("utf-8")

if "[a]_doc/records/inbox/" not in plain:
    raise SystemExit("first span is not rendered")
if "life-card-admin-20260911-01a08c45/handoff.md" not in plain:
    raise SystemExit("second span is not rendered")
if "（[b]ps://host/a），发布于" not in plain:
    raise SystemExit("URL does not stop before the CJK closer and Chinese prose")
if plain.count("[a]") != 1:
    raise SystemExit("hard-wrapped path does not have exactly one hint")
if plain.count("[b]") != 1:
    raise SystemExit("URL does not have exactly one hint")
if len(re.findall(rb"\x1b\[(?:34|38;5;4)m", content)) < 2:
    raise SystemExit("both hard-wrapped path spans are not highlighted")
PY

tmux -L "${SOCKET_NAME}" send-keys -t "${THUMBS_PANE}" a

buffer_matches() {
  test "$(tmux -L "${SOCKET_NAME}" show-buffer 2>/dev/null || true)" = "${EXPECTED}"
}
wait_until 'normalized tmux buffer' buffer_matches

source_pane_is_restored() {
  test "$(tmux -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')" = "${SOURCE_PANE}"
}
wait_until 'source pane restoration' source_pane_is_restored

tmux -L "${SOCKET_NAME}" run-shell -b -t "${SOURCE_PANE}" \
  "${ROOT_DIR}/target/release/tmux-thumbs --dir '${ROOT_DIR}'"
wait_until 'second thumbs pane' thumbs_is_running
THUMBS_PANE="$(tmux -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')"
tmux -L "${SOCKET_NAME}" send-keys -t "${THUMBS_PANE}" b

url_buffer_matches() {
  test "$(tmux -L "${SOCKET_NAME}" show-buffer 2>/dev/null || true)" = "${EXPECTED_URL}"
}
wait_until 'URL buffer without adjacent Chinese prose' url_buffer_matches

run_path_case() {
  local session_name="$1"
  local first_line="$2"
  local second_line="$3"
  local expected="$4"
  local source_command source_pane source_window thumbs_pane

  printf -v source_command \
    "bash -c %q" \
    "printf '\033[?7l\033[H\033[2J%s\033[2;1H%s\033[?7h' '${first_line}' '${second_line}'; exec sleep 120"
  tmux -L "${SOCKET_NAME}" new-session -d -x 136 -y 8 -s "${session_name}" "${source_command}"
  source_pane="$(tmux -L "${SOCKET_NAME}" display-message -p -t "${session_name}:" '#{pane_id}')"
  source_window="$(tmux -L "${SOCKET_NAME}" display-message -p -t "${source_pane}" '#{window_id}')"

  tmux -L "${SOCKET_NAME}" run-shell -b -t "${source_pane}" \
    "${ROOT_DIR}/target/release/tmux-thumbs --dir '${ROOT_DIR}'"
  for _ in $(seq 1 100); do
    thumbs_pane="$(tmux -L "${SOCKET_NAME}" list-panes -t "${source_window}" -F '#{pane_id}')"
    if test "${thumbs_pane}" != "${source_pane}"; then
      break
    fi
    sleep 0.05
  done
  test "${thumbs_pane}" != "${source_pane}" || fail "timed out waiting for ${session_name} thumbs pane"

  tmux -L "${SOCKET_NAME}" capture-pane -p -t "${thumbs_pane}" >"${CAPTURE_FILE}"
  test "$(rg -o '\[a\]' "${CAPTURE_FILE}" | wc -l)" -eq 1 || fail "${session_name} does not have one logical hint"
  tmux -L "${SOCKET_NAME}" send-keys -t "${thumbs_pane}" a

  for _ in $(seq 1 100); do
    if test "$(tmux -L "${SOCKET_NAME}" show-buffer 2>/dev/null || true)" = "${expected}"; then
      tmux -L "${SOCKET_NAME}" kill-session -t "${session_name}"
      return 0
    fi
    sleep 0.05
  done
  fail "${session_name} copied an incomplete path"
}

BASE='/data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_card_admin/openspec/changes/'
run_path_case \
  'exact-edge' \
  "      - ${BASE}life-service-card-admin-compatibility" \
  '        /proposal.md：明确使用' \
  "${BASE}life-service-card-admin-compatibility/proposal.md"
run_path_case \
  'word-boundary' \
  "      - ${BASE}life-service-card-admin-" \
  '        compatibility/design.md：命令实际解析版本为权威' \
  "${BASE}life-service-card-admin-compatibility/design.md"
run_path_case \
  'metadata-boundary' \
  ' Directory:            /data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits' \
  ' Permissions:          Full Access' \
  '/data00/home/lihao.hellohake/go/src/code.byted.org/ecom/search_stream/optimize-engine-pre-intent-waits'
run_path_case \
  'glob' \
  'specs/**/*.md 与 grill-spec.md 已复核' \
  '' \
  'specs/**/*.md'

python3 - "${ROOT_DIR}" "${SOCKET_NAME}" <<'PY'
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time

root, socket = sys.argv[1:]
env = {**os.environ, "TMUX": ""}

def tmux(*args):
    return subprocess.run(["tmux", "-L", socket, *args], env=env, check=True, capture_output=True, text=True).stdout

def wait_for(check):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("timed out waiting for isolated terminal")

def run_case(name, text, expected, width, raw=False, marker=None, plain_prefix=None):
    ready = name + "-ready"
    code = (
        "import sys,subprocess,time; sys.stdout.write(" + repr(text) + "); sys.stdout.flush(); "
        "subprocess.run(['tmux','wait-for','-S'," + repr(ready) + "],check=True); time.sleep(30)"
    )
    tmux("new-session", "-d", "-s", name, "-x", str(width), "-y", "10", shlex.join([sys.executable, "-c", code]))
    tmux("wait-for", ready)
    source = tmux("display-message", "-p", "-t", name + ":", "#{pane_id}").strip()
    window = tmux("display-message", "-p", "-t", source, "#{window_id}").strip()
    before = tmux("capture-pane", "-p", "-t", source).splitlines()
    tmux("set-option", "-g", "@thumbs-position", "right")
    tmux("set-option", "-g", "@thumbs-unique", "1")
    tmux("set-option", "-g", "set-clipboard", "off")
    with tempfile.TemporaryDirectory(prefix="thumbs-picked-") as tmp:
        result = Path(tmp) / "selected.txt"
        command = 'printf %s "${THUMB}" > ' + shlex.quote(str(result))
        launcher = shlex.join([root + "/target/release/tmux-thumbs", "--dir", root, "--command", command])
        tmux("run-shell", "-b", "-t", source, launcher)
        def ready_overlay():
            pane = tmux("list-panes", "-t", window, "-F", "#{pane_id}").strip()
            if pane == source:
                return None
            screen = tmux("capture-pane", "-p", "-t", pane)
            return (pane, screen) if "[a]" in screen else None
        pane, overlay = wait_for(ready_overlay)
        after = overlay.splitlines()
        assert overlay.count("[a]") == 1, (name, overlay)
        if marker:
            row = next(i for i, line in enumerate(before) if marker in line)
            assert after[row] == before[row], (name, "unrelated row changed", before, after)
        if plain_prefix:
            assert any(line.startswith(plain_prefix) for line in after), (name, "gutter shifted", overlay)
        if raw:
            styled = tmux("capture-pane", "-p", "-e", "-t", pane)
            assert "\x1b" in styled
            for prefix in ["◆ ", "  │ ", "  └ "]:
                for old, new in zip(before, after):
                    if old.startswith(prefix):
                        assert new.startswith(prefix), (name, old, new)
        tmux("send-keys", "-t", pane, "-l", "a")
        wait_for(lambda: result.exists())
        assert result.read_text() == expected, (name, expected, result.read_text())
        wait_for(lambda: tmux("list-panes", "-t", window, "-F", "#{pane_id}").strip() == source)
    tmux("kill-session", "-t", name)
    print(name + ": ok")

path = "/workspace/" + "abcdefghij" * 5 + "/file.rs"
for width in [40, 60, 80]:
    run_case("soft-wrap-" + str(width), path + "\r\nFOLLOWING_ROW_MUST_STAY\r\n", path, width, marker="FOLLOWING_ROW_MUST_STAY")

run_case("gutter-columns", "◆ /tmp/file.rs\r\nFOLLOWING_ROW_MUST_STAY\r\n", "/tmp/file.rs", 60,
         marker="FOLLOWING_ROW_MUST_STAY", plain_prefix="◆ /tmp/")
relative = "./.ai_doc/records/inbox/design.md"
run_case("cjk-root", "◆ 详细方案已写入" + relative + "，推荐\r\n", relative, 80, plain_prefix="◆ 详细方案已写入./")

prefix = "  │ && stat -c '%a %s %n' /workspace/.ai_doc/records/"
suffix = "  │ inbox/2026-09-12-portable-core-plugin-design.md"
text = "\x1b[?7l\x1b[2J\x1b[H" + prefix + "\x1b[2;1H" + suffix + "\x1b[3;1H  └ finished\x1b[?7h"
for width in [80, 90]:
    run_case("tool-gutter-" + str(width), text, "/workspace/.ai_doc/records/inbox/2026-09-12-portable-core-plugin-design.md", width,
             marker="finished", plain_prefix="  │ && stat -c", raw=True)
PY

printf 'hard-wrap tmux test: ok\n'
