# Basewake for Android

Turns your Valve Lighthouse V2 base stations on and off from your phone, straight over
Bluetooth. No PC needed.

- Scan finds base stations nearby (their names start with `LHB-`) and remembers them.
- Each station has its own switch; the switch at the top turns every managed station on or
  off at once. "Stop managing" leaves a station out of that.
- Settings: turn off as **Sleep** (fully off) or **Standby** (wakes faster).

## Install

1. Open the latest successful [android workflow run](https://github.com/setsumi-vrc/basewake/actions/workflows/android.yml)
   and download the `basewake-android` artifact.
2. Unzip it and copy `debug/basewake-0.1.0-debug.apk` to your phone.
3. Open it and allow installing from that source when Android asks.

The `release/` APK in the same artifact is not signed and won't install as is.

Needs Android 8.0 or newer with Bluetooth LE.

## Permissions

- **Android 12 and newer:** *Nearby devices* (`BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT`). The scan
  permission is declared `neverForLocation`, so no location access is asked for.
- **Android 11 and older:** *Location*, because those versions require it for any Bluetooth
  scan. Location must also be switched on while scanning. Basewake never uses your location.

If you deny the permission, the app shows a banner with a button to ask again or to open its
settings page.

## Build

Requires JDK 17 and the Android SDK.

```sh
cd android
./gradlew assembleDebug
```

The APK lands in `app/build/outputs/apk/debug/`.
