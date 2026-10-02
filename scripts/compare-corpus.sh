#!/usr/bin/env bash
# Compare nos rendus à ceux de libopenmpt (openmpt123) sur tout le corpus, voie par voie.
#
# Chaque voie est isolée dans une copie du module (examples/isolate.rs) : comparer des voies
# seules évite que la phase entre voies, qui fait varier l'énergie d'un mixage, fausse la mesure.
# Réglages d'openmpt123 alignés sur les nôtres : 48 kHz, interpolation linéaire, sans lissage
# de volume, morceau principal seulement (--subsong 0 : sans les passages inaccessibles, que
# libopenmpt joue sinon comme des sous-morceaux).
#
# Usage : scripts/compare-corpus.sh [module…]   (tout le corpus par défaut)
# Sortie : une ligne par module, avec la voie la moins bien corrélée.
set -euo pipefail
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
    channels=$(target/release/smpltrckr dump "$mod" 2> /dev/null | sed -n 2p | grep -oE '[0-9]+ voies' | cut -d' ' -f1) || true
    if [ -z "$channels" ]; then
        printf '%-44s non lu\n' "$name"
        continue
    fi
    worst="" worst_r=2
    for v in $(seq 1 "$channels"); do
        line=$(compare_voice "$mod" "$v")
        r=$(grep -oE 'r=-?[0-9.]+' <<< "$line" | cut -d= -f2)
        if awk "BEGIN{exit !($r < $worst_r)}"; then
            worst_r=$r worst="voie $v : $line"
        fi
    done
    printf '%-44s %s\n' "$name" "$worst"
done
