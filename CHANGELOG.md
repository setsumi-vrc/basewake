# Changelog

All notable changes to Basewake are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- The tray popup's menu rows now light up under the mouse; the highlight was the same colour as
  the popup.

## [0.1.0] - 2026-11-24

First release.

### Added

- Turns Lighthouse V2 base stations on when SteamVR starts and off when it closes, as a SteamVR
  startup app (no launcher or scripts needed). Turning off runs in its own process, so it
  finishes even when SteamVR closes fast.
- Finds base stations by itself (any number) and remembers them.
- All base stations are handled in parallel, with retries and a time limit, and each one's
  result is shown.
- Window with a switch per base station and one for all of them; rename, stop managing or
  remove from each row's menu.
- Settings in the window: turn off as Sleep or Standby, start with SteamVR, look for new base
  stations at start, manage new ones automatically, quit when SteamVR closes.
- Tray icon that shows the state at a glance. Click it to show the window; right-click for a
  quick panel with the switches, *Scan for base stations*, *Open window* and *Quit*.
- Own title bar with links to the GitHub repo, X and the issue tracker.
- Runs on Windows, Linux (with SteamVR) and macOS (window, tray and console commands; SteamVR
  doesn't run there). Linux and macOS are built and tested in CI but not yet on real hardware.
- Panel in the SteamVR dashboard with the same controls.
- `basewakectl` console commands: `on`, `off`, `toggle`, `status`, `scan`, `install`,
  `uninstall`, with real exit codes.
- Carries over settings and base stations from the old *Lighthouse Manager* and replaces its
  SteamVR registration.
