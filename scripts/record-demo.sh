#!/usr/bin/env bash
# Records the README demo: Kaze no Uta playing, voice 3 soloed for a while.
# Writes docs/media/demo-{en,fr,ja}.gif (silent) and .mp4 (with the song's sound).
# Pass languages to record only some of them: scripts/record-demo.sh ja
# Needs asciinema's agg and ffmpeg. Plays through your speakers while recording.
# « Silence, on tourne ! »
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release
bin=./target/release/smpltrckr
song=sessions/2026-10-03-kaze-no-uta/kaze-no-uta.mod
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export SMPLTRCKR_SOCKET=$work/rec.sock
"$bin" render "$song" -o "$work/full.wav" >/dev/null
"$bin" render "$song" --solo 3 -o "$work/solo.wav" >/dev/null
mkdir -p docs/media
langs=${*:-en fr ja}
# Monospace first, then a CJK fallback for the Japanese interface.
fonts="JetBrains Mono,Fira Code,DejaVu Sans Mono,Liberation Mono,Noto Sans CJK JP"
for lang in $langs; do
    case $lang in fr) keyboard=azerty ;; *) keyboard=qwerty ;; esac
    cast=$work/demo-$lang.cast
    SMPLTRCKR_LANG=$lang python3 scripts/record-demo.py "$cast" 112 30 \
        "$bin" edit --keyboard "$keyboard" "$song"
    agg --font-size 14 --text-font-family "$fonts" --idle-time-limit 10 --last-frame-duration 2 "$cast" "docs/media/demo-$lang.gif"
    # Song time of each key: the full mix, then the solo, then the full mix again.
    read -r a b c delay < <(python3 -c "import json; m = json.load(open('$cast.marks.json')); p = m['play']
print(m['solo'] - p, m['unsolo'] - p, m['stop'] - p, int(p * 1000))")
    ffmpeg -loglevel error -y -i "docs/media/demo-$lang.gif" -i "$work/full.wav" -i "$work/solo.wav" \
        -filter_complex "[1]atrim=0:$a,asetpts=PTS-STARTPTS[x];[2]atrim=$a:$b,asetpts=PTS-STARTPTS[y];[1]atrim=$b:$c,asetpts=PTS-STARTPTS[z];[x][y][z]concat=n=3:v=0:a=1,adelay=$delay|$delay,apad[aud];[0:v]fps=30,scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p[v]" \
        -map "[v]" -map "[aud]" -shortest -c:v libx264 -crf 20 -preset slow -movflags +faststart \
        -c:a aac -b:a 160k "docs/media/demo-$lang.mp4"
done
ls -la docs/media
