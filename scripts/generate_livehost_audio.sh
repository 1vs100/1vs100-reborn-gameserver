#!/bin/sh
# Writes assets/livehost.wma: a 2 minute live host test stream, a steady 440 Hz tone under a
# spoken position ("0 seconds", "10 seconds", ...) every 10 seconds. The title fetches the stream
# anew for each live host segment and plays it from the start.
# Requires ffmpeg (wmav2 encoder) and espeak-ng.
set -eu
cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
duration=120
step=10
: > "$work/list.txt"
at=0
while [ "$at" -lt "$duration" ]; do
    espeak-ng -s 150 -w "$work/say.wav" "$at seconds"
    ffmpeg -hide_banner -loglevel error -y -i "$work/say.wav" \
        -af "aresample=44100,apad=whole_dur=$step,atrim=duration=$step" -ac 1 \
        "$work/at_$at.wav"
    echo "file 'at_$at.wav'" >> "$work/list.txt"
    at=$((at + step))
done
ffmpeg -hide_banner -loglevel error -y -f concat -safe 0 -i "$work/list.txt" "$work/voice.wav"
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=$duration" \
    -i "$work/voice.wav" \
    -filter_complex "[0:a]volume=0.25[tone];[1:a]aresample=44100[voice];[tone][voice]amix=inputs=2:duration=first:normalize=0[out]" \
    -map "[out]" -ac 1 -c:a wmav2 -b:a 32k -packet_size 3200 -map_metadata -1 -fflags +bitexact \
    -f asf assets/livehost.wma
