#!/usr/bin/env bash
# Usage: wait.sh FEEDBACK [SECONDS]
# Waits for the mockup window's "Send to agent" and prints the notes.
#
# The window replaces FEEDBACK in one rename, so it is never read
# half-written. This prints it and moves it aside to FEEDBACK.N, so the next
# wait sees only the next send. With 0 seconds it only checks. Exits 4 when
# nothing arrived in time (default 600 s).
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: wait.sh FEEDBACK [SECONDS]" >&2
    exit 2
fi
file=$1
seconds=${2:-600}
case "$seconds" in
    '' | *[!0-9]*)
        echo "wait: SECONDS must be a whole number" >&2
        exit 2
        ;;
esac

deadline=$((SECONDS + seconds))
until [ -s "$file" ]; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "wait: no feedback after ${seconds}s" >&2
        exit 4
    fi
    sleep 0.2
done

n=1
while [ -e "$file.$n" ]; do
    n=$((n + 1))
done
mv "$file" "$file.$n"
cat "$file.$n"
