# Getting a DLL into DARK SOULS II

me3 cannot launch this game — `me3 profile create --game` accepts `darksouls3, sekiro,
eldenring, armoredcore6, nightreign` and nothing else — so the `[[natives]]` mechanism every
er-mods-rs crate assumes does not exist here. Mods load by **proxying one of the game's own
imports**.

## The proxy surface, verified

Exact imported names, read out of `DarkSoulsII.exe`'s import descriptors (build 9527516). These
are the candidates small enough to proxy honestly:

| DLL | Imports | Notes |
| --- | --- | --- |
| **`DINPUT8.dll`** | `DirectInput8Create` | **One named export. The chosen target.** |
| `d3d11.dll` | `D3D11CreateDevice` | Also one — and it lands exactly at device creation, which a future D3D11 overlay wants. Riskier: under Proton this is the Wine d3d11 → DXVK chain, and getting the forward wrong loses rendering, not input. |
| `dxgi.dll` | `CreateDXGIFactory` | One. |
| `WINMM.dll` | `timeBeginPeriod`, `timeEndPeriod`, `timeGetTime`, `timeSetEvent`, `timeKillEvent` | Five, all named. |
| `SHELL32.dll` | `CommandLineToArgvW`, `SHGetFolderPathW` | Two. |
| `XINPUT1_3.dll` | ordinals `#2`, `#3` | **Imported by ordinal, not by name** — a proxy must export matching ordinals, which is fiddlier. |
| `OLEAUT32.dll` | ordinals `#2`, `#6`, `#8` | Same problem. |

`DINPUT8.dll` wins on every axis: a single export, resolved by name, and the least load-bearing
thing in the list. A broken forward costs you controller input, which is obvious and harmless,
rather than the renderer.

## Under Proton

Dropping `dinput8.dll` beside the exe is not enough — Wine prefers its own builtin. The
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
before the executable's entry point** — so the proxy is already in the good position and needs
no suspended-process launcher. This is the reason to proxy an import rather than inject at
runtime: `LoadLibrary` into a live process arrives after the entry stubs have already run, and
per dearxan's README those cannot be undone.

`dearxan::disabler::schedule_after_arxan` is the companion for work that must happen in lockstep
with the entry point rather than before it.

The `disabler` feature pulls `windows-sys`, so it builds only for the Windows target. Declare it
on the loader crate, never on a host-side one.

## What has not been established

Nothing here has been run. The import table is read; the override syntax is Wine's documented
behaviour; the dearxan timing is quoted from its own docs. Whether DS2's 48 Arxan stubs actually
revert a MinHook detour when *left* in place is **untested** — the first crash-logging build is
what answers it.
