#!/usr/bin/env bash
# build.sh — build and test the Inari app in a container: no JDK, no
# Android SDK and no Gradle installed on the machine. The image carries JDK 21,
# platform 35 and build-tools 35.0.0; Gradle comes from the wrapper; the
# dependency cache lives in ~/.cache/inari-gradle. Runs as the calling
# user, so what it writes belongs to you.
#
#   android/build.sh                 unit tests + debug APK
#   android/build.sh release         unit tests + release APK, signed with the key in
#                                    $INARI_KEYS (~/.config/inari-android: release.p12 and
#                                    release.pass), mounted read-only, never copied
#   android/build.sh <gradle tasks>  anything else
set -euo pipefail
cd "$(dirname "$0")"
IMAGE=${INARI_ANDROID_IMAGE:-ghcr.io/cirruslabs/android-sdk:35}
CACHE=${INARI_GRADLE_CACHE:-$HOME/.cache/inari-gradle}
mkdir -p "$CACHE"
TASKS=("$@")
# Where Inari is and where APKs are published: machine settings, never in the repository.
ENV_FILE=${INARI_BUILD_ENV:-$HOME/.config/swarmfox/build.env}
[ -f "$ENV_FILE" ] && . "$ENV_FILE"
export INARI_DEFAULT_URL=${INARI_DEFAULT_URL:-} INARI_UPDATES_URL=${INARI_UPDATES_URL:-}
IDENTITY_ARGS=()
for name in INARI_APPLICATION_ID INARI_PREFS_NAME INARI_GATE_KEY_ALIAS INARI_KEY_ALIAS; do
  if [ -n "${!name:-}" ]; then export "$name"; IDENTITY_ARGS+=(-e "$name"); fi
done
KEYS_ARGS=()
OUT=app/build/outputs/apk/debug/app-debug.apk
if [ "${1:-}" = release ]; then
  KEYS=${INARI_KEYS:-$HOME/.config/swarmfox}
  [ -s "$KEYS/release.p12" ] && [ -s "$KEYS/release.pass" ] || { echo "no release key in $KEYS" >&2; exit 1; }
  KEYS_ARGS=(-v "$KEYS/release.p12:/keys/release.p12:ro" -v "$KEYS/release.pass:/keys/release.pass:ro"
             -e INARI_KEYSTORE=/keys/release.p12 -e INARI_KEYSTORE_PASS_FILE=/keys/release.pass)
  [ -n "$INARI_DEFAULT_URL" ] && [ -n "$INARI_UPDATES_URL" ] ||
    { echo "set INARI_DEFAULT_URL and INARI_UPDATES_URL in $ENV_FILE for a release" >&2; exit 1; }
  TASKS=(testDebugUnitTest assembleRelease)
  OUT=app/build/outputs/apk/release/app-release.apk
fi
[ ${#TASKS[@]} -gt 0 ] || TASKS=(testDebugUnitTest assembleDebug)
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/cache -e GRADLE_USER_HOME=/cache/gradle \
  -e INARI_DEFAULT_URL -e INARI_UPDATES_URL "${IDENTITY_ARGS[@]}" \
  -v "$CACHE:/cache" -v "$PWD:/work" -w /work "${KEYS_ARGS[@]}" "$IMAGE" \
  ./gradlew --no-daemon "${TASKS[@]}"
echo "APK: $(pwd)/$OUT"
