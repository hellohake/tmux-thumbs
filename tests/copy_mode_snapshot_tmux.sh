#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOCKET_NAME="tmux-thumbs-copy-snapshot-$$"
SESSION_NAME="copy-snapshot"
TMP_DIR="$(mktemp -d /tmp/tmux-thumbs-copy-snapshot.XXXXXX)"
RUNTIME_TMP_DIR="${TMP_DIR}/runtime space;quote's"
REAL_TMUX="$(command -v tmux)"
VISIBLE_TARGET='/tmp/visible-target.txt'
LIVE_TARGET='/tmp/live-target.txt'
mkdir -m 700 "${RUNTIME_TMP_DIR}"

cleanup() {
  "${REAL_TMUX}" -L "${SOCKET_NAME}" kill-server 2>/dev/null || true
  rm -rf -- "${TMP_DIR}"
}
trap cleanup EXIT

fail() {
  printf 'copy-mode snapshot test: %s\n' "$*" >&2
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
    sleep 0.02
  done
  fail "timed out waiting for ${description}"
}

test -x "${ROOT_DIR}/target/release/thumbs" || fail 'release thumbs binary is missing'
test -x "${ROOT_DIR}/target/release/tmux-thumbs" || fail 'release tmux-thumbs binary is missing'

cat >"${TMP_DIR}/tmux" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  display-message)
    if [ "${TMUX_THUMBS_DROP_SCROLL:-0}" = 1 ] && [[ "$*" == *'#{pane_mode}'* ]]; then
      output="$("${REAL_TMUX}" "$@")"
      IFS=: read -r pane mode height width _scroll history zoom <<<"${output}"
      printf '%s:%s:%s:%s::%s:%s\n' "${pane}" "${mode}" "${height}" "${width}" "${history}" "${zoom}"
      exit 0
    fi
    if [[ "$*" == *'tmux-thumbs: copy-mode viewport unavailable; retry'* ]]; then
      "${REAL_TMUX}" set-buffer -b tmux-thumbs-test-message 'tmux-thumbs: copy-mode viewport unavailable; retry'
      exit 0
    fi
    exec "${REAL_TMUX}" "$@"
    ;;
  capture-pane)
    sleep 0.20
    exec "${REAL_TMUX}" "$@"
    ;;
  new-window)
    pane_id="$("${REAL_TMUX}" "$@")"
    "${REAL_TMUX}" resize-window -t "${pane_id}" -x 80 -y 23
    printf '%s\n' "${pane_id}"
    ;;
  *)
    exec "${REAL_TMUX}" "$@"
    ;;
esac
SH
chmod 700 "${TMP_DIR}/tmux"

python_code=$'import time\nfor i in range(1, 201):\n    marker = "/tmp/visible-target.txt " if i == 15 else "/tmp/drift-target.txt " if i == 19 else "/tmp/live-target.txt " if i == 200 else ""\n    print(f"L{i:03d} " + marker + "x" * 140, flush=True)\ntime.sleep(120)'
printf -v source_command 'python3 -c %q' "${python_code}"

"${REAL_TMUX}" -L "${SOCKET_NAME}" -f /dev/null new-session -d -x 120 -y 30 \
  -s "${SESSION_NAME}" "${source_command}"
SOURCE_PANE="$("${REAL_TMUX}" -L "${SOCKET_NAME}" display-message -p -t "${SESSION_NAME}:" '#{pane_id}')"
SOURCE_WINDOW="$("${REAL_TMUX}" -L "${SOCKET_NAME}" display-message -p -t "${SOURCE_PANE}" '#{window_id}')"

"${REAL_TMUX}" -L "${SOCKET_NAME}" set-option -g @thumbs-position right
"${REAL_TMUX}" -L "${SOCKET_NAME}" set-option -g @thumbs-unique 1
"${REAL_TMUX}" -L "${SOCKET_NAME}" set-option -g @thumbs-regexp-1 '/tmp/visible-target\.txt'
"${REAL_TMUX}" -L "${SOCKET_NAME}" set-environment -g REAL_TMUX "${REAL_TMUX}"
"${REAL_TMUX}" -L "${SOCKET_NAME}" set-environment -g PATH "${TMP_DIR}:${PATH}"

"${REAL_TMUX}" -L "${SOCKET_NAME}" copy-mode -t "${SOURCE_PANE}"
"${REAL_TMUX}" -L "${SOCKET_NAME}" send-keys -t "${SOURCE_PANE}" -X history-top
"${REAL_TMUX}" -L "${SOCKET_NAME}" send-keys -t "${SOURCE_PANE}" -X page-down

scroll_position="$("${REAL_TMUX}" -L "${SOCKET_NAME}" display-message -p -t "${SOURCE_PANE}" '#{scroll_position}')"
pane_height="$("${REAL_TMUX}" -L "${SOCKET_NAME}" display-message -p -t "${SOURCE_PANE}" '#{pane_height}')"
visible_before="$("${REAL_TMUX}" -L "${SOCKET_NAME}" capture-pane -p -N -t "${SOURCE_PANE}" \
  -S "-$scroll_position" -E "$((pane_height - scroll_position - 1))")"
grep -Fq 'L015' <<<"${visible_before}" || fail 'fixture did not position L015 on the visible copy-mode page'
grep -Fq "${VISIBLE_TARGET}" <<<"${visible_before}" || fail 'fixture target is not visible before invoking tmux-thumbs'

launcher="PATH=$(printf '%q' "${TMP_DIR}:${PATH}") REAL_TMUX=$(printf '%q' "${REAL_TMUX}") TMPDIR=$(printf '%q' "${RUNTIME_TMP_DIR}") $(printf '%q' "${ROOT_DIR}/target/release/tmux-thumbs") --pane-id $(printf '%q' "${SOURCE_PANE}") --dir $(printf '%q' "${ROOT_DIR}")"
"${REAL_TMUX}" -L "${SOCKET_NAME}" run-shell -b -t "${SOURCE_PANE}" "${launcher}"

thumbs_is_running() {
  test "$("${REAL_TMUX}" -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')" != "${SOURCE_PANE}"
}
wait_until 'thumbs pane' thumbs_is_running
THUMBS_PANE="$("${REAL_TMUX}" -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')"

overlay_contains_text() {
  "${REAL_TMUX}" -L "${SOCKET_NAME}" capture-pane -p -t "${THUMBS_PANE}" | grep -Fq 'L0'
}
wait_until 'rendered thumbs overlay' overlay_contains_text
overlay="$("${REAL_TMUX}" -L "${SOCKET_NAME}" capture-pane -p -t "${THUMBS_PANE}")"

source_pane_is_restored() {
  test "$("${REAL_TMUX}" -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')" = "${SOURCE_PANE}"
}

runtime_dir_is_empty() {
  test -z "$(find "${RUNTIME_TMP_DIR}" -mindepth 1 -maxdepth 1 -print -quit)"
}

if ! grep -Fq 'L015 ' <<<"${overlay}"; then
  printf '%s\n' "${overlay}" >&2
  "${REAL_TMUX}" -L "${SOCKET_NAME}" send-keys -t "${THUMBS_PANE}" q
  wait_until 'source pane restoration after failed assertion' source_pane_is_restored
  wait_until 'snapshot cleanup after failed assertion' runtime_dir_is_empty
  fail 'overlay drifted away from the pre-swap copy-mode page'
fi
if grep -Fq "${LIVE_TARGET}" <<<"${overlay}"; then
  printf '%s\n' "${overlay}" >&2
  "${REAL_TMUX}" -L "${SOCKET_NAME}" send-keys -t "${THUMBS_PANE}" q
  wait_until 'source pane restoration after live-page assertion' source_pane_is_restored
  wait_until 'snapshot cleanup after live-page assertion' runtime_dir_is_empty
  fail 'overlay captured the live page instead of the visible copy-mode page'
fi

"${REAL_TMUX}" -L "${SOCKET_NAME}" send-keys -t "${THUMBS_PANE}" q
wait_until 'source pane restoration' source_pane_is_restored
wait_until 'snapshot cleanup' runtime_dir_is_empty

invalid_launcher="PATH=$(printf '%q' "${TMP_DIR}:${PATH}") REAL_TMUX=$(printf '%q' "${REAL_TMUX}") TMPDIR=$(printf '%q' "${RUNTIME_TMP_DIR}") TMUX_THUMBS_DROP_SCROLL=1 $(printf '%q' "${ROOT_DIR}/target/release/tmux-thumbs") --pane-id $(printf '%q' "${SOURCE_PANE}") --dir $(printf '%q' "${ROOT_DIR}")"
"${REAL_TMUX}" -L "${SOCKET_NAME}" run-shell -b -t "${SOURCE_PANE}" "${invalid_launcher}"

viewport_message_present() {
  "${REAL_TMUX}" -L "${SOCKET_NAME}" show-buffer -b tmux-thumbs-test-message 2>/dev/null \
    | grep -Fxq 'tmux-thumbs: copy-mode viewport unavailable; retry'
}
wait_until 'copy-mode viewport retry message' viewport_message_present

if "${REAL_TMUX}" -L "${SOCKET_NAME}" list-windows -F '#{window_name}' | grep -Fxq '[thumbs]'; then
  fail 'invalid copy-mode viewport opened a picker window'
fi
test "$("${REAL_TMUX}" -L "${SOCKET_NAME}" list-panes -t "${SOURCE_WINDOW}" -F '#{pane_id}')" = "${SOURCE_PANE}" \
  || fail 'invalid viewport replaced the source pane'
wait_until 'invalid viewport snapshot cleanup' runtime_dir_is_empty

"${REAL_TMUX}" -L "${SOCKET_NAME}" set-option -ag command-alias \
  'thumbs-pick=run-shell -b /tmp/old-tmux-thumbs.sh'
"${REAL_TMUX}" -L "${SOCKET_NAME}" set-option -g @thumbs-key f
"${REAL_TMUX}" -L "${SOCKET_NAME}" bind-key -T prefix f thumbs-pick
"${REAL_TMUX}" -L "${SOCKET_NAME}" run-shell -t "${SOURCE_PANE}" \
  "PATH=$(printf '%q' "${TMP_DIR}:${PATH}") REAL_TMUX=$(printf '%q' "${REAL_TMUX}") bash $(printf '%q' "${ROOT_DIR}/tmux-thumbs.tmux")"
prefix_binding="$(${REAL_TMUX} -L "${SOCKET_NAME}" list-keys -T prefix | grep '^bind-key.* f ' || true)"
alias_state="$(${REAL_TMUX} -L "${SOCKET_NAME}" show-options -s command-alias)"
grep -Fq "${ROOT_DIR}/tmux-thumbs.sh" <<<"${prefix_binding}" \
  || {
    printf '%s\n%s\n' "${alias_state}" "${prefix_binding}" >&2
    fail 'plugin reload did not bind the current tmux-thumbs script'
  }
grep -Fq '#{pane_id}' <<<"${prefix_binding}" \
  || fail 'plugin reload did not bind the invoking pane ID'
if grep -Fq '/tmp/old-tmux-thumbs.sh' <<<"${prefix_binding}"; then
  fail 'plugin reload kept a stale thumbs-pick command alias'
fi

printf 'copy-mode snapshot test: ok\n'
