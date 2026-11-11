#!/bin/sh
# Type-check the Windows and macOS builds from Linux (no linking, so no SDKs are needed).
# Needs the targets: rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin
# The C parts of ring and objc2 can't be compiled without those systems' headers, so their build
# scripts are skipped (overridden by their `links` names); that's fine for `check`, which never links.
set -eu
cd "$(dirname "$0")/.."
# ring's links name includes its full version (ring_core_0_17_14_), so take it from Cargo.lock.
ring=$(awk '$0 == "name = \"ring\"" { getline; gsub(/[^0-9.]/, ""); gsub(/\./, "_"); print; exit }' Cargo.lock)
[ -n "$ring" ] || { echo "ring isn't in Cargo.lock; update this script" >&2; exit 1; }
# There's no Windows resource compiler here; build.rs would otherwise stop a --release check over the icon.
export HET_ALLOW_NO_ICON=1
for t in x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin; do
  echo "== $t"
  cargo check --release --locked --target "$t" \
    --config "target.$t.ring_core_${ring}_.rustc-cfg=[]" \
    --config "target.$t.objc2_exception_helper_0_1.rustc-cfg=[]"
done
