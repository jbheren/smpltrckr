#!/usr/bin/env bash
# Compares our renders with libopenmpt's (openmpt123) over the whole corpus, voice by voice.
#
# Each voice is isolated in a copy of the module (examples/isolate.rs): comparing single
# voices keeps the phase between voices, which shifts the energy of a mix, out of the score.
# openmpt123 settings match ours: 48 kHz, linear interpolation, no volume ramping, main song
# only (--subsong 0: without the unreachable parts libopenmpt would otherwise play as
# subsongs).
#
# Usage: scripts/compare-corpus.sh [module…]   (the whole corpus by default)
# Output: one line per module, with its least correlated voice.
set -euo pipefail
export SMPLTRCKR_LANG=en
cd "$(dirname "$0")/.."
cargo build -q --release --bin smpltrckr --examples
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

compare_voice() {
    target/release/examples/isolate "$1" "$2" "$work/v.mod"
    target/release/smpltrckr render "$work/v.mod" -o "$work/ours.wav" > /dev/null
    openmpt123 --batch --quiet --subsong 0 --samplerate 48000 --filter 2 --ramping 0 \
        --stereo 100 --no-float --force -o "$work/ref.wav" "$work/v.mod" > /dev/null 2>&1
    target/release/examples/compare "$work/ours.wav" "$work/ref.wav"
}

[ $# -gt 0 ] || set -- corpus/*
for mod in "$@"; do
    name=$(basename "$mod")
    channels=$(target/release/smpltrckr dump "$mod" 2> /dev/null | sed -n 2p | grep -oE '[0-9]+ voices' | cut -d' ' -f1) || true
    if [ -z "$channels" ]; then
        printf '%-44s unreadable\n' "$name"
        continue
    fi
    worst="" worst_r=2
    for v in $(seq 1 "$channels"); do
        line=$(compare_voice "$mod" "$v")
        r=$(grep -oE 'r=-?[0-9.]+' <<< "$line" | cut -d= -f2)
        if awk "BEGIN{exit !($r < $worst_r)}"; then
            worst_r=$r worst="voice $v: $line"
        fi
    done
    printf '%-44s %s\n' "$name" "$worst"
done
