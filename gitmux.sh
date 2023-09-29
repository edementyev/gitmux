#!/usr/bin/env sh

entries=$(~/bin/gitmux "$@")

if [ $? -ne 0 ]; then
    echo "Gitmux returned with error:"
    exit 1
fi

selection=$(echo "$entries" | fzf --preview 'tree -C {}' | sed 's#/$##')

if [ -z "${selection}" ]; then
    exit 0
fi

tmux neww -c "$selection" -n "$(realpath --relative-to="$(realpath "$selection"/../../)" "$selection" | sed -r 's#^(.?.?.?.?).*/#\1/#')"
