# Getting a DLL into DARK SOULS II

me3 cannot launch this game -- `me3 profile create --game` accepts `darksouls3, sekiro,
eldenring, armoredcore6, nightreign` and nothing else -- so the `[[natives]]` mechanism every
er-mods-rs crate assumes does not exist here. Mods load by **proxying one of the game's own
imports**.

## The proxy surface, verified

Exact imported names, read out of `DarkSoulsII.exe`'s import descriptors (build 9527516). These
are the candidates small enough to proxy honestly:

| DLL | Imports | Notes |
| --- | --- | --- |
| **`DINPUT8.dll`** | `DirectInput8Create` | **One named export. The chosen target.** |
| `d3d11.dll` | `D3D11CreateDevice` | Also one -- and it lands exactly at device creation, which a future D3D11 overlay wants. Riskier: under Proton this is the Wine d3d11 -> DXVK chain, and getting the forward wrong loses rendering, not input. |
| `dxgi.dll` | `CreateDXGIFactory` | One. |
| `WINMM.dll` | `timeBeginPeriod`, `timeEndPeriod`, `timeGetTime`, `timeSetEvent`, `timeKillEvent` | Five, all named. |
| `SHELL32.dll` | `CommandLineToArgvW`, `SHGetFolderPathW` | Two. |
| `XINPUT1_3.dll` | ordinals `#2`, `#3` | **Imported by ordinal, not by name** -- a proxy must export matching ordinals, which is fiddlier. |
| `OLEAUT32.dll` | ordinals `#2`, `#6`, `#8` | Same problem. |

`DINPUT8.dll` wins on every axis: a single export, resolved by name, and the least load-bearing
thing in the list. A broken forward costs you controller input, which is obvious and harmless,
rather than the renderer.

## Under Proton

Dropping `dinput8.dll` beside the exe is not enough -- Wine prefers its own builtin. The
override must be set:

```
WINEDLLOVERRIDES="dinput8=n,b"
```

`n,b` means "native first, then builtin", so our DLL loads and Wine's real `dinput8` stays
available underneath for the forward.

## Why the proxy is the *right* place for the Arxan patch

`dearxan::disabler::neuter_arxan` is one call and handles the SteamStub 3.1 wrapper on the way
past. Its documentation is explicit about timing:

> When called before the program entry point and Arxan has hooked the MSVC CRT entry sequence,
> Dearxan patches those Arxan entry stubs before invoking `__security_init_cookie`, preventing
> their checks from running.

and warns:

> While best-effort synchronization with the entry point is performed when this function is
> called after it has started executing, it is not perfect and may lead to race conditions. For
> this reason it is **strongly** recommended to use a mod loader that creates the game process
> as suspended.

A statically-imported DLL's `DllMain(DLL_PROCESS_ATTACH)` runs **during import resolution,
before the executable's entry point** -- so the proxy is already in the good position and needs
no suspended-process launcher. This is the reason to proxy an import rather than inject at
runtime: `LoadLibrary` into a live process arrives after the entry stubs have already run, and
per dearxan's README those cannot be undone.

`dearxan::disabler::schedule_after_arxan` is the companion for work that must happen in lockstep
with the entry point rather than before it.

The `disabler` feature pulls `windows-sys`, so it builds only for the Windows target. Declare it
on the loader crate, never on a host-side one.

## The implementation

`crates/ds2-loader` is this document's conclusion, built. `[lib] name = "dinput8"` is what makes
cargo emit `dinput8.dll`; the crate does three things from `DllMain(DLL_PROCESS_ATTACH)` and
nothing else -- `neuter_arxan`, one log line reporting what it said, and a lazy forward of
`DirectInput8Create` to `<system directory>\dinput8.dll`.

`scripts/ds2-run.py` stages it, prints the staged file's SHA-256, launches through Steam with the
override above, and **prints a success block only after reading the DLL's own
`ds2-loader: arxan ...` line out of `<Game>/ds2-loader.log`**. On timeout it says so and exits
non-zero. `--dry-run` stages and launches nothing; `--selftest` exercises the log tailer.

```
bash scripts/check.sh
cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader
```

Two build-side facts worth knowing before they cost you an hour:

* **`dearxan` is a path dependency on `../dearxan`, relative to the workspace root.** That
  resolves in a normal checkout and does *not* resolve in a linked worktree under
  `.claude/worktrees/`, where the workspace root is two levels deeper. A symlink
  `.claude/worktrees/dearxan -> /path/to/dearxan` fixes it for every agent worktree at once, and
  is invisible to git because `.claude/worktrees/` is ignored.
* **`scripts/check.sh` no longer runs `cargo fmt --all`.** `--all` is documented as formatting
  workspace packages *and their local path-based dependencies*, so the moment anything here
  depended on `../dearxan` the gate began failing on that checkout's brace style. It now
  enumerates workspace members from `cargo metadata --no-deps`, which is the same principle as
  the `--no-deps` already on the clippy line.

## What has not been established

**Nothing here has been run.** The import table is read, and `dinput8.dll` builds and has been
checked statically -- it exports exactly `DirectInput8Create` and `DllMain`, imports only
`kernel32.dll`, `api-ms-win-core-synch-l1-2-0.dll` and `ntdll.dll`, and the CRT entry stub really
does call our `DllMain` -- but no DARK SOULS II process has ever loaded it. The override syntax is
Wine's documented behaviour; the dearxan timing is quoted from its own docs.

One thing the launcher cannot fix and flags at runtime: `steam -applaunch` hands the request to an
already-running Steam client over IPC, and the game then inherits *that* client's environment, so
a `WINEDLLOVERRIDES` set by the launcher process is not guaranteed to reach the game. If a run
comes back with no testimony, rule that out first -- quit Steam so the launcher starts it, or set
the per-app launch options to `WINEDLLOVERRIDES="dinput8=n,b" %command%`.

Whether DS2's 48 Arxan stubs actually revert a MinHook detour when *left* in place is **untested**
-- the first crash-logging build is what answers it.

---

# CORRECTION, then RE-CONFIRMATION: me3 really cannot load DS2

Everything above was written on the premise that me3 cannot load DS2. **That premise was
wrong**, and it was wrong because of how it was checked: `me3 profile create --game` accepts
only `darksouls3, sekiro, eldenring, armoredcore6, nightreign`, and that enum was taken as the
whole interface. It is not. `me3 launch` (0.11.0) also offers:

```
-s, --steam-id <STEAM_ID>   Steam APPID of the game to launch
-e, --exe <EXE>             Custom path to the game executable
-n, --native <NATIVES>      Path to DLL file (native DLL mod) [repeatable option]
```

`--game` is a shorthand for known titles, not the only way to name a target. Five DS2 spellings
were tried against `--game` and all five were rejected, so that enum really is closed — but
`--steam-id 335300` and `--exe` bypass it.

me3 also ships the two things this repo was about to build by hand:

```
--disable-arxan [<true|false>]   Neutralize Arxan/GuardIT code protection
--suspend                        Suspend the game until a debugger is attached
```

The second matters more than it looks. dearxan's own documentation says it is **strongly
recommended to use a mod loader that creates the game process as suspended** — me3 does exactly
that.

## What this changes, and what it does not

If me3 works on DS2, then `[[natives]]`-style DLL loading exists here after all, the `dinput8`
proxy is not on the critical path, and far more of er-mods-rs ports with far less change.

**But none of that is established.** A flag existing is not a flag working. me3's mod host may
refuse an unknown appid, and `--no-mem-patch`'s own help says the memory patch applies to "some
supported games (Dark Souls 3, Sekiro and ELDEN RING)" — so parts of me3 clearly are per-game.
Whether `--disable-arxan` is generic or keyed to a known title is unknown.

**Gotcha, already measured:** me3 auto-detects the Flatpak Steam at
`~/.var/app/com.valvesoftware.Steam/.local/share/Steam`, which does not contain DS2. Pass
`--steam-dir /home/banon/.local/share/Steam` or the appid will not resolve.

The `dinput8` proxy stays regardless — it is built, it is gated on its own testimony, and it is
the fallback if me3 turns out to be genuinely per-game. It is no longer the only door.

## Settled: all three entry paths refused

The retraction above was right to reject the *reasoning* — one enum is not the whole interface,
and `--steam-id` / `--exe` genuinely do exist. But tested against this game, all three doors are
shut. me3 0.11.0, DS2 SOTFS appid 335300:

| path | result |
| --- | --- |
| `--game darksouls2` / `darksoulsii` / `ds2` / `sotfs` / `darksouls2sotfs` | rejected by the enum, all five |
| `--steam-id 335300` | `error=unable to determine game from name or app ID` |
| `--exe "<...>/Game/DarkSoulsII.exe"` | `error=unable to determine which game to launch` |

`--steam-id` and `--exe` do not bypass the known-games table — they **resolve against** it. Naming
the executable directly is still not enough to make me3 launch a title it does not know.

So the original conclusion stands, now on evidence that can carry it: **the `dinput8` proxy is
the way in.** Not because me3 has an enum, but because me3 refuses this game by appid and by
executable path as well.

Two things from me3 remain worth knowing even though we cannot use it:

- It ships `--disable-arxan`, which is independent corroboration that neutralizing Arxan is
  standard practice for FromSoftware mod loading and not something this project invented.
- It ships `--suspend`, the suspended-process launch dearxan's docs call strongly recommended.
  Our proxy gets a weaker version of the same guarantee for free, because a statically imported
  DLL's `DllMain` runs before the executable's entry point.
