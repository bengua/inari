# Inari for Android

A thin Android client for an [Inari](../README.md) core: it opens the core's interface in a
WebView behind the phone's fingerprint, keeps the login to the core's gate in the Android
Keystore, receives what other apps share (a photo, a PDF, a link) into the composer, and
updates itself from an APK server you choose. It brings no agent of its own: your cores run
them, on your machines.

## Point it at your core

Where the core is and where updates are published are **build settings**, never in the source.
Create `~/.config/inari-android/build.env` (or set `INARI_BUILD_ENV`) on the machine that builds:

```sh
INARI_DEFAULT_URL=https://your-core.example      # the address the app opens first
INARI_UPDATES_URL=https://your-updates.example   # where `latest.json` and the APKs are published
```

Both are empty by default; the app then asks for the address on first launch.

Optional, so an update keeps what an existing installation already uses:

| Variable | Default | Meaning |
|---|---|---|
| `INARI_APPLICATION_ID` | `app.inari.client` | the Android application id |
| `INARI_PREFS_NAME` | `inari` | name of the saved-settings file |
| `INARI_GATE_KEY_ALIAS` | `inari-gate` | Keystore entry that seals the gate password |
| `INARI_KEY_ALIAS` | `inari` | alias of the release signing key |

## Build

No JDK, Android SDK or Gradle is needed on the machine: `build.sh` runs in a container.

```sh
android/build.sh            # unit tests + debug APK
android/build.sh release    # unit tests + release APK, signed with the key in $INARI_KEYS
```

The release key (`release.p12` and `release.pass` in `$INARI_KEYS`, default
`~/.config/inari-android`) is mounted read-only and never copied. Without it a release build is
left unsigned. `android/publish.sh` writes the APK, `Inari-latest.apk` and `latest.json`
(`versionCode`, `versionName`, `sha256`) to a directory you serve over HTTPS.

## The gate

A core's web interface is normally served behind a reverse proxy that asks for a login. The app
answers that prompt with the stored credentials, sealed in the Keystore and released by the
fingerprint, and never hands them to the page. Only the configured origin is browsed in the
view; any other link opens in the system browser. A core that trusts only loopback (the
default) is meant to be reached through such a proxy over a private network.

## License

MIT, like the rest of Inari.
