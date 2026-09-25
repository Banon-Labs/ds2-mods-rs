# DARK SOULS II: Scholar of the First Sin mods

Built for **build 9527516** (Steam appid 335300), x86-64 Windows. A different game build is not
supported and is not detected for you -- every address in these DLLs was read out of that one.

## What each file is

| File | What it does |
| --- | --- |
| `dinput8.dll` | The mod itself. DS2 statically imports `dinput8`, so a copy in the game folder is loaded by the game with no launcher involved. Reads `ds2-mods.toml` from that same folder. |
| `ds2-launcher.exe` | Starts the game suspended and injects the DLLs named by `[launcher] dlls` in `ds2-mods.toml`. Needed **only** for mods that cannot be loaded from inside the running process -- Seamless Co-op is the one this exists for. |
| `ds2_crash_logging.dll` | The crash logger on its own, for loading beside something else. Has not been run inside the game as a standalone DLL. |
| `ds2-launch-linux.sh` | Linux only: starts `ds2-launcher.exe` inside the game's Proton prefix. See [Linux](#linux). |
| `ds2-mods.toml` | Settings, read by both of the above from the folder they sit in. Every key ships set to the default it already has, so unpacking it changes nothing. |
| `SHA256SUMS` | Checksums for the files above. `sha256sum -c SHA256SUMS`. |

Each file is also attested: `gh attestation verify dinput8.dll -R Banon-Labs/ds2-mods-rs` traces a
download back to the commit and the workflow run that built it.

## Installing

Drop `dinput8.dll` into the folder holding `DarkSoulsII.exe` -- on a default Steam install,
`steamapps/common/Dark Souls II Scholar of the First Sin/Game/`. That is the whole install for
everything except Seamless Co-op.

Put `ds2-mods.toml` in that same folder. It ships with every key already set to the default the
DLLs use, so it changes nothing on its own -- it is there so you can see what there is to change,
and so `[launcher] dlls` has an example. Delete it and the DLLs run the same defaults.

## Seamless Co-op is not in this package, and will not be

Install it yourself, then flip one line in `ds2-mods.toml`:

```toml
[seamless]
enabled = true                   # ships as false
dll = "SeamlessCoop/ds2sc.dll"   # where its own launcher puts it -- leave alone for a normal install
```

and have Steam start `ds2-launcher.exe` instead of the game -- see
[Starting the launcher from Steam](#starting-the-launcher-from-steam). This is not only a
licensing line: the mod
refuses to boot on a build it considers out of date, and says so on a dialog whose text exists
only in decrypted process memory. A copy bundled here would go stale on its author's schedule and
fail as a hang nobody downloading this could diagnose. Yours, you update.

`ds2-launcher.exe` refuses the whole launch and starts no process at all when a file in the list
is missing, and names the path it could not find. A session gets every DLL or does not exist.

## Starting the launcher from Steam

Steam starts whatever the game's launch options tell it to (Steam -> DARK SOULS II -> Properties
-> General -> Launch Options), so the launcher goes there and you press Play as usual. Steam is
still the one starting the session, so playtime, the overlay and cloud sync work as they do
without mods. `ds2-launcher.exe` stays running until the game exits, and exits with the game's
own exit code, so Steam sees the session end when the game does.

| You want | Launch options |
| --- | --- |
| Linux, `dinput8.dll` only | `WINEDLLOVERRIDES="dinput8=n,b" %command%` |
| Linux, with the `[launcher]` list (Seamless Co-op) | `WINEDLLOVERRIDES="dinput8=n,b" bash -c 'exec "${@/%DarkSoulsII.exe/ds2-launcher.exe}"' -- %command%` |
| Windows, `dinput8.dll` only | leave it empty |
| Windows, with the `[launcher]` list | `"C:\full\path\to\Game\ds2-launcher.exe" %command%` |

On Linux, `%command%` is Steam's whole Proton chain ending in the path to `DarkSoulsII.exe`. The
`bash -c` line swaps that last path for `ds2-launcher.exe` beside it and leaves the rest of the
chain alone, so Proton runs the launcher in the game's own prefix. On Windows, the launcher
receives the game's path as its first argument; it drops that (it already knows the path) and
passes every argument after it on to the game.

The Windows line has not been tried on Windows.

To go back to plain `dinput8.dll`, switch the launch options back. The launcher with an empty
list is harmless too: it starts the game with nothing injected.

## Linux

The DLLs are PE either way -- the game is a Proton process, so the files and the folder are the
same. `WINEDLLOVERRIDES="dinput8=n,b"` is in every Linux launch option above because Wine has a
`dinput8` of its own and prefers it: without the override, Wine's builtin wins the load and this
package's `dinput8.dll` never runs.

`ds2-launch-linux.sh` starts `ds2-launcher.exe` without Steam starting the game, for when Steam's
launch options are not an option:

```
./ds2-launch-linux.sh            # start the game through the launcher
./ds2-launch-linux.sh --print    # show the command and environment it would use, run nothing
```

It finds the Proton this game's prefix was made by and the container runtime that Proton needs,
the same way Steam does, and starts `ds2-launcher.exe` inside that prefix with the override set.
It refuses when the game is already running, and when the prefix does not exist yet -- start the
game through Steam once first. Steam is not starting that session, so it records no playtime,
gets no overlay and does no cloud sync; the launch options above are the way to keep all three.
