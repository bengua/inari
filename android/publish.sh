#!/usr/bin/env bash
# publish.sh — put a built release APK where the phones fetch it, and say which version it is.
#
#   android/publish.sh [APK] [DIR]
#
# Writes DIR/Inari-<version>.apk, DIR/Inari-latest.apk and DIR/latest.json
# ({versionCode, versionName, url, sha256}): the app reads latest.json to offer its own update,
# and checks the sha256 before installing.
set -euo pipefail
cd "$(dirname "$0")"
APK=${1:-app/build/outputs/apk/release/app-release.apk}
DIR=${2:-${INARI_APK_DIR:-$HOME/.local/share/inari-android/apk}}
code=$(grep -m1 -oE 'versionCode = [0-9]+' app/build.gradle.kts | grep -oE '[0-9]+')
name=$(grep -m1 -oE 'versionName = "[^"]+"' app/build.gradle.kts | cut -d'"' -f2)
[ -s "$APK" ] || { echo "no APK at $APK" >&2; exit 1; }
mkdir -p "$DIR"
cp "$APK" "$DIR/Inari-$name.apk"
cp "$APK" "$DIR/Inari-latest.apk.tmp" && mv "$DIR/Inari-latest.apk.tmp" "$DIR/Inari-latest.apk"
sum=$(sha256sum "$APK" | cut -d' ' -f1)
printf '{"versionCode": %s, "versionName": "%s", "url": "Inari-%s.apk", "sha256": "%s"}\n' \
  "$code" "$name" "$name" "$sum" > "$DIR/latest.json.tmp" && mv "$DIR/latest.json.tmp" "$DIR/latest.json"
echo "published $name ($code) to $DIR"
