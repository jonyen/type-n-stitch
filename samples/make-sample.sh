#!/usr/bin/env bash
# Generates samples/sample.mp4: a 20 s synthetic clip for trying the editor and for the
# engine's render test. Speech comes from macOS `say`, with deliberate fillers and long
# pauses so "Remove fillers" and "Tighten pauses" have something to find. The picture is a
# waveform over a flat background, so cuts and freezes are easy to spot in the export.
# The render test in engine/tests/render.rs expects exactly 20 s; the audio is padded to it.
#
# Usage: samples/make-sample.sh [voice]   (default voice: Samantha)
set -euo pipefail

cd "$(dirname "$0")"
voice="${1:-Samantha}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# [[slnc N]] inserts N ms of silence.
say -v "$voice" -r 190 -o "$tmp/speech.aiff" "\
Hi, and welcome to type and stitch. [[slnc 900]] \
Um, this is a short clip for, uh, trying out the editor. [[slnc 300]] \
Delete a word in the transcript, and it gets cut from the video. [[slnc 1500]] \
Remove fillers cuts the ums and, uh, the uhs. [[slnc 300]] \
Tighten pauses shortens long silences, like, um, [[slnc 1600]] that one. \
Thanks for watching."

ffmpeg -hide_banner -loglevel error -y -i "$tmp/speech.aiff" -filter_complex "\
color=c=0x1e1b2e:s=854x480:r=30[bg];\
[0:a]apad=whole_dur=20,atrim=0:20,asplit[a][w];\
[w]showwaves=s=854x200:mode=cline:rate=30:colors=0xa78bfa[wave];\
[bg][wave]overlay=0:140:shortest=1[v]" \
  -map "[v]" -map "[a]" -c:v libx264 -crf 23 -pix_fmt yuv420p -c:a aac -b:a 128k \
  -ar 48000 -t 20 -movflags +faststart sample.mp4

echo "wrote samples/sample.mp4 ($(ffprobe -v error -show_entries format=duration -of csv=p=0 sample.mp4) s)"
