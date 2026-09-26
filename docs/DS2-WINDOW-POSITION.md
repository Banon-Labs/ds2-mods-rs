# Who moves DARK SOULS II's window, and who writes `App.Window.X`

Read out of `darksoulsii-deobf.bin` (Ghidra on port 8766 for decompilation, `scripts/ds2-xrefs.py`
and `scripts/ds2-disasm.py` over the flat image) with no game launched. Every claim is tagged
**read in binary** (with the address it was read from) or **inferred**. The symptom it answers: the
window appears on DP-1, then leaves every monitor during the first minute of boot, and a clean exit
persists the off-screen X into `userconfig.properties`, so every later launch starts off-screen.

## Short answer

- **No game code can put the window beyond the virtual desktop.** The game's only positioned
  moves are `CreateWindowExW` at the raw `App.Window.X/Y`, a `SetWindowPos` to `(0,0)`, and a
  `SetWindowPos` back to the window's own `GetWindowRect`. Every other `SetWindowPos` carries
  `SWP_NOMOVE`. (read in binary)
- **The game never asks where the monitors are.** `USER32` imports contain no `MoveWindow`,
  `SetWindowPlacement`, `MonitorFromWindow`, `MonitorFromPoint`, `EnumDisplayMonitors`,
  `GetMonitorInfo*` or `ChangeDisplaySettings*`, and none of those names exists anywhere in the
  image as ASCII or UTF-16, so none is resolved by `GetProcAddress` either. `GetSystemMetrics` is
  called twice, with `SM_CXDOUBLECLK`/`SM_CYDOUBLECLK`. (read in binary)
- **The `WM_MOVE` handler persists whatever the window is at, unclamped.** That is the half of the
  bug the game owns: a foreign move becomes the next launch's starting position. (read in binary)
- **This repo's launcher does move the window.** `scripts/ds2-run.py` `settle_on_monitor()` runs
  Hyprland's `hl.dsp.window.move{ monitor = "DP-1", window = "class:^steam_app_335300$" }` right
  after the DLL's first log line -- inside the first minute of boot, which is when the window is
  reported to leave. It is the only window mover found in `crates/` or `scripts/`. That it is the
  mover is **inferred**; one run with the move disabled proves or clears it.

## Every call that positions or sizes the game window

Call addresses are the `ff 15` instruction. `SetWindowPos` flags: `1` NOSIZE, `2` NOMOVE,
`4` NOZORDER, `0x20` FRAMECHANGED, `0x40` SHOWWINDOW.

| call | function | what it does | moves? |
| --- | --- | --- | --- |
| `CreateWindowExW` `0x1402eb858` | WinMain `0x1402eb630` | creates at `App.Window.X/Y` as read, size from `AdjustWindowRectEx` `0x1402eb7f4` | yes, to the config value |
| `ShowWindow` `0x1402eb870` | WinMain | `nCmdShow` | no |
| `SetWindowPos` `0x140ae9689` | draw-system frame `0x140ae9560` | client-size correction, flags `2` | no |
| `ShowWindow` `0x140ae96e4` | draw-system frame | `SW_MINIMIZE` when a fullscreen swap chain has dropped out | no (minimise) |
| `SetWindowPos` `0x140aecf18` | style apply `0x140aece80` | new style + size, flags `0x62` | no |
| `SetWindowPos` `0x140aebbc8` | mode switch `0x140aebab0` | style change, flags `0x63` | no |
| `SetWindowPos` `0x140aebc7f` | mode switch, failure path | back to its own prior `GetWindowRect`, flags `0x60` | only to where it already was |
| `SetWindowPos` `0x140aefc02` | `0x140aefbd0` | topmost toggle, flags `3` | no |
| `SetWindowPos` `0x140af02d2` | per-frame `0x140af0090` | to `(0,0)`, flags `5`, after `WM_SIZE` saw a client under 32 px | yes, to `(0,0)` |
| `ShowWindow` `0x1402eadda` | WndProc `0x1402eabf7` | `SW_MINIMIZE` on a hotkey | no |

The DXGI side (`0x140960e70`: `FindClosestMatchingMode`, `SetFullscreenState`, `ResizeTarget`)
places the window on an output only when entering fullscreen; with `FullScreen` off the windowed
branch is taken. That the flag at draw-system `+0x18` means *windowed* is **inferred** from
`0x140aebab0` setting it to 1 on the path that calls `0x140960e70(..., 1)`.

## Where `App.Window.X/Y` is read and written

- **Read, once, at startup:** `0x140aef500` reads `App.Window.X` at `0x140aef759` and
  `App.Window.Y` through the getter `0x140aee270` (user map at `+0x178`, falling back to the
  default map at `+0x160`, default `0`), and hands them to `CreateWindowExW` unchanged. The window
  is always created windowed first: `KatanaMainApp` vtable slot `+0x110` (`0x1402ee690`) returns
  the windowed flag as 1. No clamp, no monitor query. (read in binary)
- **Written on every `WM_MOVE`:** WndProc `0x1402eabf7` forwards to `0x140af0380`; case `3`
  (`WM_MOVE`) takes `GetWindowRect` unless `IsIconic` or `IsZoomed`, and stores `left`/`top`
  through `0x140aefe40` at `0x140af04bb` and `0x140af04d2`. That is an in-memory map write.
  (read in binary)
- **Flushed to disk at exit:** `0x140aefb30` writes the `+0x178` map to
  `title:/userconfig.properties`, called from shutdown `0x140aedee0` at `0x140aedf8d` after WinMain's
  message loop ends. A crash never reaches it, which is why only clean exits persist the off-screen
  value. (read in binary)

## The minimal durable fix

Two parts, both in `scripts/ds2-run.py`; neither needs a DLL, and no game function needs a hook:

1. **Clamp before launch.** Before `steam -applaunch`, rewrite `App.Window.X/Y` in
   `GAME_DIR/userconfig.properties` when the point is outside `GAME_MONITOR`'s rectangle (from
   `hyprctl monitors -j`, which lists monitors and no windows), putting it back at that monitor's
   origin plus a small margin. That breaks the persistence loop no matter who moved the window.
2. **Do not move a window that is already there.** `settle_on_monitor()` should read the window's
   monitor first -- it already does, after the move -- and dispatch the move only when it differs
   from `GAME_MONITOR`. If the run with the move disabled keeps the window on DP-1, the move is the
   cause and this is the fix; if it does not, the mover is outside this repo (Wine's X11 driver or
   Hyprland placing an Xwayland window across mixed scales) and part 1 still stands.

A detour on `0x140af0380` to drop off-screen `WM_MOVE` writes would also stop the persistence, but
it needs `scripts/ds2-arxan-chain.py` clearance and a DLL, and the launcher clamp does the same job
from outside the process.

Not done here: both changes are to `scripts/ds2-run.py`, which is not pushed without a run behind it
(`DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH`).
