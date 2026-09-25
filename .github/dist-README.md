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

You need this only for the `[launcher]` list (Seamless Co-op, or anything else you listed there).
For `dinput8.dll` alone, Steam starts the game as usual -- see the table at the end of this
section.

### Windows

1. In Steam, right-click DARK SOULS II: Scholar of the First Sin -> **Manage** -> **Browse local
   files**. Explorer opens the game's install folder. Open `Game` inside it: that is the folder
   holding `DarkSoulsII.exe`, and where `ds2-launcher.exe` goes.
2. Click Explorer's address bar and copy the path. On a default install it is
   `C:\Program Files (x86)\Steam\steamapps\common\Dark Souls II Scholar of the First Sin\Game`.
3. Right-click the game -> **Properties** -> **General** -> **Launch Options**, and enter that
   path followed by `\ds2-launcher.exe`, **in double quotes**, then a space and `%command%`:

   ```
   "C:\Program Files (x86)\Steam\steamapps\common\Dark Souls II Scholar of the First Sin\Game\ds2-launcher.exe" %command%
   ```

   The quotes are required: the path has spaces in it, and without them Steam runs
   `C:\Program` and the game does not start.
4. Press Play.

Steam replaces `%command%` with the path to `DarkSoulsII.exe` and hands it to the launcher as its
first argument. The launcher drops that argument (it already knows where the game is) and passes
any arguments after it on to the game. A console window stays open for as long as the game runs.
That is the launcher waiting for the game, which is how Steam knows the session is still going.
Leave it open.

This has not been tried on Windows yet. It is the launch-option form other Steam wrapper
launchers use, and it is what the launcher's argument handling was written for.

### Linux

Enter this as the launch options (Properties -> General -> Launch Options) and press Play. Nothing
in it needs editing for your install:

```
WINEDLLOVERRIDES="dinput8=n,b" bash -c 'exec "${@/%DarkSoulsII.exe/ds2-launcher.exe}"' -- %command%
```

On Linux, `%command%` is Steam's whole Proton chain ending in the path to `DarkSoulsII.exe`. The
`bash -c` part swaps that last path for `ds2-launcher.exe` in the same folder and leaves the rest
of the chain alone, so Proton runs the launcher in the game's own prefix.

### Every case

| You want | Windows launch options | Linux launch options |
| --- | --- | --- |
| `dinput8.dll` only | leave it empty | `WINEDLLOVERRIDES="dinput8=n,b" %command%` |
| With the `[launcher]` list | `"<Game folder>\ds2-launcher.exe" %command%` | the `bash -c` line above |

To go back to plain `dinput8.dll`, switch the launch options back. Leaving the launcher in place
with an empty list does no harm either: it starts the game with nothing injected.

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
