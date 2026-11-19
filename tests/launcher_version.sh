#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEST_DIR="$(mktemp -d /tmp/tmux-thumbs-launcher-version.XXXXXX)"
trap 'rm -rf -- "${TEST_DIR}"' EXIT
mkdir -p "${TEST_DIR}/plugin/target/release" "${TEST_DIR}/bin" "${TEST_DIR}/calls"
cp "${ROOT_DIR}/tmux-thumbs.sh" "${TEST_DIR}/plugin/"
export THUMBS_TEST_CALLS="${TEST_DIR}/calls"
export THUMBS_TEST_BINARY_VERSION=0.8.0
export PATH="${TEST_DIR}/bin:${PATH}"

cat >"${TEST_DIR}/plugin/Cargo.toml" <<'TOML'
[package]
name = "thumbs"
version = "0.8.0"

[dependencies]
unicode-width-clusters = { package = "unicode-width", version = "0.2.2" }

[workspace.metadata.example]
version = "9.9.9"
TOML

cat >"${TEST_DIR}/bin/tmux" <<'SH'
#!/usr/bin/env bash
case "${1:-}" in
  split-window) printf '%s\n' "$*" >"${THUMBS_TEST_CALLS}/installer" ;;
  show) exit 1 ;;
  *) exit 0 ;;
esac
SH
cat >"${TEST_DIR}/plugin/target/release/thumbs" <<'SH'
#!/usr/bin/env bash
printf 'thumbs %s\n' "${THUMBS_TEST_BINARY_VERSION}"
SH
cat >"${TEST_DIR}/plugin/target/release/tmux-thumbs" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" >"${THUMBS_TEST_CALLS}/picker"
SH
chmod 700 "${TEST_DIR}/bin/tmux" "${TEST_DIR}/plugin/target/release/"*

bash "${TEST_DIR}/plugin/tmux-thumbs.sh" '%42'
if [ -e "${THUMBS_TEST_CALLS}/installer" ]; then
  printf 'launcher version test: matching package version opened the installer\n' >&2
  exit 1
fi
test -f "${THUMBS_TEST_CALLS}/picker"
rg -Fxq -- '%42' "${THUMBS_TEST_CALLS}/picker"

rm "${THUMBS_TEST_CALLS}/picker"
export THUMBS_TEST_BINARY_VERSION=0.9.0
bash "${TEST_DIR}/plugin/tmux-thumbs.sh" '%42'
test ! -f "${THUMBS_TEST_CALLS}/picker"
rg -Fq 'tmux-thumbs-install.sh update' "${THUMBS_TEST_CALLS}/installer"

rm "${THUMBS_TEST_CALLS}/installer" "${TEST_DIR}/plugin/target/release/thumbs"
bash "${TEST_DIR}/plugin/tmux-thumbs.sh" '%42'
test ! -f "${THUMBS_TEST_CALLS}/picker"
rg -Fq 'tmux-thumbs-install.sh' "${THUMBS_TEST_CALLS}/installer"
if rg -Fq ' update' "${THUMBS_TEST_CALLS}/installer"; then
  printf 'launcher version test: missing binary used the update path\n' >&2
  exit 1
fi

printf 'launcher version test: ok\n'
