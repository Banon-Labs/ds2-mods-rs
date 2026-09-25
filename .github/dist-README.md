# DARK SOULS II: Scholar of the First Sin mods

Built for **build 9527516** (Steam appid 335300), x86-64 Windows. A different game build is not
supported and is not detected for you -- every address in these DLLs was read out of that one.

## What each file is

| File | What it does |
| --- | --- |
| `dinput8.dll` | The mod itself. DS2 statically imports `dinput8`, so a copy in the game folder is loaded by the game with no launcher involved. Reads `ds2-mods.toml` from that same folder. |
| `ds2-launcher.exe` | Starts the game suspended and injects the DLLs named by `[launcher] dlls` in `ds2-mods.toml`. Needed **only** for mods that cannot be loaded from inside the running process -- Seamless Co-op is the one this exists for. |
| `ds2_crash_logging.dll` | The crash logger on its own, for loading beside something else. Has not been run inside the game as a standalone DLL. |
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

and start the game with `ds2-launcher.exe` instead of through Steam. This is not only a
licensing line: the mod
refuses to boot on a build it considers out of date, and says so on a dialog whose text exists
only in decrypted process memory. A copy bundled here would go stale on its author's schedule and
fail as a hang nobody downloading this could diagnose. Yours, you update.

`ds2-launcher.exe` refuses the whole launch and starts no process at all when a file in the list
is missing, and names the path it could not find. A session gets every DLL or does not exist.

## Linux

The DLLs are PE either way -- the game is a Proton process, so nothing here changes. What does
change is starting `ds2-launcher.exe`, which has to run inside the game's own Proton prefix with
`WINEDLLOVERRIDES=dinput8=n,b` so Wine's builtin `dinput8` does not win the load. Nothing in this
package does that yet; `scripts/ds2-run.py` in the repository is the only thing that resolves the
Proton chain today, and it builds from source.

## What launching through `ds2-launcher.exe` costs

Steam is not starting the game, so that session records no playtime, gets no overlay, and does no
cloud sync. Launching `DarkSoulsII.exe` through Steam as usual keeps all three -- and still loads
`dinput8.dll`, because that one is a static import. The launcher is only for the injected list.
