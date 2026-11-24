# Basewake

Wakes your Valve **Lighthouse V2** base stations when SteamVR starts and puts them back to sleep
when it closes. No Python, no scripts, no console windows: one small app with a window, a tray
icon and a panel in the SteamVR dashboard.

- Finds every base station by itself (1, 2, 3 or more): they show up as `LHB-…`.
- Turns them all on or off in parallel, retries, and tells you which one didn't answer.
- Switch each one on or off by hand, rename it, or leave it out.
- Turn off as **Sleep** (fully off) or **Standby** (motor stopped, wakes faster).
- Registers itself as a SteamVR startup app, so it needs no other launcher.

## Install

Windows (from source, needs [Rust](https://rustup.rs)):

```powershell
.\install.ps1             # build and copy to %LOCALAPPDATA%\Programs\Basewake
.\install.ps1 -Register   # same, then start Basewake with SteamVR
```

After registering, **SteamVR → Settings → Startup/Shutdown → Choose Startup Overlay Apps** lists
"Basewake". If you used the old *Lighthouse Manager*, `-Register` replaces it and your settings
and base stations carry over.

## How it runs

| Started as | What happens |
|---|---|
| `basewake` (double-click) | Window + tray icon. Closing the window keeps it in the tray. |
| `basewake --steamvr` (by SteamVR) | Tray only. Turns your base stations on, looks for new ones and adds the Basewake panel to the dashboard. When SteamVR quits it turns them off and (by default) exits. |
| `basewakectl on\|off\|toggle\|status\|scan\|install\|uninstall` | Console commands. If the app is running they go through it, otherwise they run directly. Exit code 0 = all OK, 1 = something failed. |

Only one copy runs; opening it again shows the running one.

## Settings

Everything is set in the window (or the SteamVR panel) and saved to `basewake.toml` in
`%APPDATA%\Basewake`:

- **Turn off as**: Sleep (default) or Standby
- **Start with SteamVR**
- **Find new base stations when SteamVR starts**
- **Manage new base stations automatically**: off = new ones are listed but left alone
- **Quit when SteamVR closes**
- Per base station: a switch to turn it on or off now; the `···` menu (or right-click) to rename,
  stop or start managing it, or remove it. The switch next to the title covers all of them.

Logs go to `%APPDATA%\Basewake\logs` (`BASEWAKE_LOG=debug` for more detail).

## Building

```sh
cargo test
cargo build --release
```

```
src/engine/   Bluetooth, controller (parallel, retries, deadlines), config, logging
src/ui/       the egui panel shared by the window and the SteamVR dashboard
src/desktop/  window and tray
src/vr/       OpenVR bindings, SteamVR registration, session, dashboard renderer
src/ipc.rs    single instance + command forwarding
third_party/  OpenVR SDK 2.12.14 runtime library and license (Valve, BSD-3)
```

## License

[MIT](LICENSE). The OpenVR runtime in `third_party/` is © Valve Corporation, BSD-3-Clause.
