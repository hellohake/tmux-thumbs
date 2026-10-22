#!/usr/bin/env bash

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

DEFAULT_THUMBS_KEY=space

THUMBS_KEY="$(tmux show-option -gqv @thumbs-key)"
THUMBS_KEY=${THUMBS_KEY:-$DEFAULT_THUMBS_KEY}

THUMBS_COMMAND="thumbs-pick=run-shell -b \"${CURRENT_DIR}/tmux-thumbs.sh '#{pane_id}'\""
THUMBS_ALIAS_INDEX="$(
  tmux show-options -s command-alias 2>/dev/null \
    | sed -nE 's/^command-alias\[([0-9]+)\] "?thumbs-pick=.*/\1/p' \
    | head -n 1
)"

if [ -n "${THUMBS_ALIAS_INDEX}" ]; then
  tmux set-option -s "command-alias[${THUMBS_ALIAS_INDEX}]" "${THUMBS_COMMAND}"
else
  tmux set-option -as command-alias "${THUMBS_COMMAND}"
fi
tmux bind-key "${THUMBS_KEY}" thumbs-pick
