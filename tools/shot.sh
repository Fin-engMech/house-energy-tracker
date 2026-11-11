#!/bin/bash
# Headless screenshot of a scratch copy of the app (never the real data file or port 47615).
# usage: SCRATCH=<dir> tools/shot.sh <port> <out.png> <width,height> [#hash]
#   DATA=<file>     data to use (default: tools/test-data.json, made-up figures; never your real data)
#   WEATHER=<file>  weather JSON to inline (default: $SCRATCH/w.json; get it from /api/weather on a test server)
#   HARNESS=<file>  JS to run after the data loads (tools/harness-*.js print PASS/FAIL over the page)
#   NOSYNC=1        don't patch: test the real start-up (the shot will usually catch "Loading your data…")
set -u
: "${SCRATCH:?set SCRATCH to a scratch folder}"
P=$1; OUT=$2; SIZE=$3; HASH=${4:-}
ROOT=$(cd "$(dirname "$0")/.." && pwd); T=$SCRATCH/t
rm -rf "$T" && mkdir -p "$T/cache" "$T/cfg" "$SCRATCH/ffp"
cp "$ROOT/target/release/energy-tracker" "$ROOT/app.html" "$T/"
cp "${DATA:-$ROOT/tools/test-data.json}" "$T/energy-data.json"
if [ -z "${NOSYNC:-}" ]; then
  python3 "$ROOT/tools/patch-test-copy.py" "$T/app.html" "${WEATHER:-$SCRATCH/w.json}" "$T/energy-data.json" ${HARNESS:+"$HARNESS"} || exit 1
fi
# The XDG folders keep the test server's log, exports and cache out of the real ones.
(cd "$T" && XDG_CACHE_HOME=$T/cache XDG_CONFIG_HOME=$T/cfg setsid ./energy-tracker --no-window --port "$P" --data "$T/energy-data.json" > log 2>&1 &)
sleep 0.6
timeout 90 firefox --headless --no-remote --profile "$SCRATCH/ffp" --window-size="$SIZE" --screenshot "$OUT" "http://127.0.0.1:$P/$HASH" >/dev/null 2>&1
curl -s -m 1 -X POST "localhost:$P/api/quit" >/dev/null
