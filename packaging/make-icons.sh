#!/bin/sh
# Regenerate the bitmap icons in assets/ from icon.svg.
# Needs rsvg-convert (librsvg), ImageMagick (magick) and python3.
#   assets/icon-256.png  window icon (RGBA, built into the program)
#   assets/icon.ico      Windows program icon (embedded by build.rs)
#   assets/icon.icns     macOS app bundle icon (used by --install and packaging/macos/make-app.sh)
set -eu
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for s in 16 24 32 48 64 128 256 512 1024; do
  rsvg-convert -w $s -h $s icon.svg -o "$tmp/raw-$s.png"
  # exclude-chunks keeps creation dates (tEXt/tIME) out of the committed files
  magick "$tmp/raw-$s.png" -strip -define png:exclude-chunks=date,time -define png:color-type=6 PNG32:"$tmp/$s.png"
done
cp "$tmp/256.png" assets/icon-256.png
magick "$tmp/16.png" "$tmp/24.png" "$tmp/32.png" "$tmp/48.png" "$tmp/64.png" "$tmp/128.png" "$tmp/256.png" assets/icon.ico
python3 - "$tmp" assets/icon.icns <<'PY'
import struct, sys, os
src, out = sys.argv[1], sys.argv[2]
# PNG-backed icns chunk types and their pixel sizes
types = [("icp4", 16), ("icp5", 32), ("icp6", 64), ("ic07", 128), ("ic08", 256),
         ("ic09", 512), ("ic10", 1024), ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512)]
body = b""
for t, s in types:
    data = open(os.path.join(src, f"{s}.png"), "rb").read()
    body += t.encode() + struct.pack(">I", len(data) + 8) + data
open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)
PY
echo "Wrote assets/icon-256.png, assets/icon.ico, assets/icon.icns"
