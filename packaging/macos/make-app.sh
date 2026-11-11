#!/bin/sh
# Build "House Energy Tracker.app" (Apple silicon + Intel) and a .dmg, on a Mac.
# Needs Xcode command line tools and: rustup target add aarch64-apple-darwin x86_64-apple-darwin
#
# Without SIGN_IDENTITY the app is signed ad hoc: fine on your own Mac, but anyone you give it to
# gets a Gatekeeper warning. To ship it, sign with your Developer ID and notarise it:
#   xcrun notarytool store-credentials het-notary      # once: saves your Apple ID login in the keychain
#   SIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)" NOTARY_PROFILE=het-notary packaging/macos/make-app.sh
set -eu
cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="House Energy Tracker"
APP="dist/$NAME.app"
DMG="dist/House-Energy-Tracker-$VERSION.dmg"

for t in aarch64-apple-darwin x86_64-apple-darwin; do cargo build --release --locked --target $t; done
rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/energy-tracker" \
  target/aarch64-apple-darwin/release/energy-tracker target/x86_64-apple-darwin/release/energy-tracker
cp assets/icon.icns "$APP/Contents/Resources/AppIcon.icns"
# The same Info.plist that --install writes, from the program itself (this Mac's own build of it).
case "$(uname -m)" in arm64) host=aarch64 ;; *) host=x86_64 ;; esac
"target/$host-apple-darwin/release/energy-tracker" --info-plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  # The hardened runtime and a secure timestamp are both required for notarisation.
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
else
  codesign --force --sign - "$APP"
fi
codesign --verify --strict --verbose=1 "$APP"

# The disk image shows the app beside a link to Applications, to drag it onto.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$APP" "$stage/$NAME.app"
ln -s /Applications "$stage/Applications"
rm -f "$DMG"
hdiutil create -volname "$NAME" -srcfolder "$stage" -ov -format UDZO "$DMG"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
    xcrun stapler staple "$APP"
    spctl --assess --type open --context context:primary-signature --verbose "$DMG"
  else
    echo "Signed but not notarised: set NOTARY_PROFILE to notarise (see the top of this script)."
  fi
fi
echo "Built $APP and $DMG"
