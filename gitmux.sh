#!/usr/bin/env sh

selection=`~/bin/gitmux | fzf --preview 'tree -C {}' | sed 's#/$##'`

if [ -z "${selection}" ]; then
    return
fi

tmux neww -c $selection -n $(realpath --relative-to="$(realpath $selection/../../)" "$selection" | sed -r 's#^(.?.?.?.?).*/#\1/#')
