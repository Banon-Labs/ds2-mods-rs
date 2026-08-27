# Porting `er-mods-rs` to DARK SOULS II

What survives the move from Elden Ring, what is blocked, and what is dead. Every number here
was measured against the two checkouts and the shipped binary, not estimated.

## The three blockers

| # | Blocker | Evidence |
|---|---|---|
| 1 | **me3 cannot load DS2** | `me3 profile create --game` accepts only `darksouls3, sekiro, eldenring, armoredcore6, nightreign`. |
| 2 | **No DS2 bindings exist** | `fromsoftware-rs` members: `shared`, `shared/stl`, `darksouls3`, `eldenring`, `nightreign`, `sekiro`. 21 of 57 er-mods-rs crates depend on `eldenring` + `fromsoftware-shared`. |
| 3 | **Arxan is live** | 48 stubs, all analysed OK by dearxan. Zero encrypted regions (2969 candidates, all eliminated on the entropy test), but the stubs are real anti-debug and integrity code. |

## What the binary actually is

Measured directly from `DarkSoulsII.exe` (build 9527516):

- `ImageBase` `0x140000000`, `SizeOfImage` `0x1d76000`, `DllCharacteristics` `0x8160`
  (`DYNAMIC_BASE` set — never assume the live base).
- SteamStub v3 wrapped: a `.bind` section at `0x141d43000` holds the entry point
  (RVA `0x1d43310`). dearxan unwraps it to recover the real OEP.
- **Not packed.** `.text` sliding 64 KB entropy is 5.23–6.61; it opens
  `40 53 48 83 EC 20 48 8B D9` — `push rbx; sub rsp,0x20; mov rbx,rcx`, a stock MSVC x64
  prologue. The import table is fully intact: 16 descriptors, all named thunks.
- Imports, with thunk counts, because they are the loading surface:
  `KERNEL32` 156, `USER32` 61, `fmod_event64` 48, `fmodex64` 47, `WS2_32` 43,
  `steam_api64` 16, `ole32` 8, `ADVAPI32` 6, `WINMM` 5, `OLEAUT32` 3, `XINPUT1_3` 2,
  `SHELL32` 2, `DINPUT8` 1, `dxgi` 1, `d3d11` 1.

`DINPUT8.dll` at one thunk (`DirectInput8Create`) is the smallest correct proxy surface, which
is why the loader targets it. Under Proton the override must be set —
`WINEDLLOVERRIDES="dinput8=n,b"` — or Wine's builtin wins and nothing loads.

## Tier 0 — substrate

Nothing is testable until this lands.

| Crate | Source | Size | Notes |
| --- | --- | --- | --- |
| `ds2-loader` | new, patterned on `er-ags-stub` | — | `dinput8.dll` proxy; forwards `DirectInput8Create` to the system DLL. Calls the dearxan disabler from `DllMain` before anything else. |
| `ds2-hook` | `er-hook` | 954 lines, **zero deps** | MinHook FFI + cross-DLL union + pluggable log sink. Ports almost verbatim -- see the correction below. |
| `ds2-game-base` | `er-game-base`, partially | 2728 lines in, ~1175 out | See the split below. |
| `ds2-rva` | new | — | The only crate permitted to hold DS2 addresses. Starts empty. |

### Correction: `er-hook` was not purely generic

The first pass through this table claimed the only Elden Ring reference in `er-hook` was a
comment. That was wrong, and the port found it: `PRODUCT_DLL_NAME` and `UNION_REGISTER_EXPORT`
were `b"er_quickload.dll\0"` and `b"er_effects_union_register\0"` -- ER **product identity in
code**, not in a comment. Nothing in this repo exports `er_effects_union_register`, so carrying
them over would have left `register_shared_hook` permanently and silently falling through to the
local union, and renaming them to a DS2 DLL would have invented a fact about a DLL that does not
exist yet. `ds2-hook` takes the export identity as a caller-supplied parameter instead, which
makes this document's claim that the product DLL owns the export literally true rather than
aspirational.

The general lesson, which applies to every remaining row: **"it has no dependencies" is not the
same as "it has no game knowledge."** Read the constants, not just the manifest.

### The `er-game-base` split

| File | Lines | Verdict |
| --- | --- | --- |
| `mem.rs` | 490 | Ports — fault-safe RAM readers, generic. |
| `http.rs` | 339 | Ports — one blocking WinHTTP GET. |
| `log.rs` | 264 | Ports — parameterized append-only file logger. |
| `fnv1a.rs` | 82 | Ports — fingerprints. |
| `rva.rs` | 283 | **Does not port.** ER 1.16.2 singleton table. Replaced by `ds2-rva`, empty. |
| `filecap.rs` | 305 | **Does not port.** `FD4FileCap` / `DLString<wchar_t>` / DLIO virtual roots — FD4 postdates this engine. |
| `pgd.rs`, `profile_summary.rs`, `build_id.rs` | 927 | **Does not port.** ER save and profile layouts. |

Keep er-game-base's tier split: tier A zero-dep, tier B behind a `game-types` feature. Tier B
stays empty here until DS2 bindings exist.

## Tier 1 — portable today, no RE required

`cargo test`-able on Linux with no game attached.

- **`er-hotkey-config`** — 1708 lines (`binding.rs` 221, `keys.rs` 988, `lib.rs` 42,
  `live.rs` 150, `reload.rs` 307). Zero game deps. Straight copy.
- **`er-safe-input`** — 233 lines. Models controller intent (`SafeButton`) rather than raw
  injection. DS2 imports both `DINPUT8` and `XINPUT1_3`, so both backends have a target.
- **`er-crash-logging-core`** — depends only on `er-game-base`. First-chance exceptions to a
  file. **The right first real mod**: it proves the loader, the Arxan neutering and the log
  path while needing zero knowledge of DS2 structures.

## Tier 2 — blocked on reverse engineering

- **A `darksouls2` crate for `fromsoftware-rs`.** Scale, measured: `darksouls3` is 44,383
  lines, `eldenring` 81,093, `shared` 8,434. Model on **DS3, not ER** — DS3 is one generation
  from DS2, ER is three — but expect to keep far less of its module layout
  (`app_menu, cs, dlio, dlkr, dltx, dlui, dlut, fd4, param, rva, sprj, stl, util`), because
  DS2 predates FD4 and most of the DL\* naming. These bindings come from the disassembly by
  hand; there is no reflection metadata to generate them from.
- **`er-net-effects`** — the original feature of er-mods-rs. DS2 has SpEffects, so the feature
  ports; the plumbing does not. Needs the DS2 player structure, the apply-SpEffect function,
  and the SpEffectParam id space. The smallest possible bet that the bindings are right.

## Tier 3 — rewrites, not ports

- **D3D12 → D3D11.** DS2 imports `d3d11.dll` and `dxgi.dll`. `er-d3d12-compositor` (818 lines)
  and everything on it — `er-loading-bar`, `er-loading-portrait` — needs a D3D11 compositor
  written from scratch. `hudhook` has a D3D11 backend, so the hudhook-based crates have a path;
  the bespoke compositor does not.
- **Scaleform.** `er-scaleform-hooks` and `er-gfx` assume the ER menu system is Scaleform GFx.
  **Unverified for DS2** — check before assuming either way. DS2 shows no Scaleform import, but
  it could be statically linked.
- **Boot and menu machinery.** `er-quickload`, `er-save-picker-core`/`-picker`,
  `er-quit-menu-core`/`-menu`, `er-title-flow`, `er-profile-summary-core` all sit on ER's
  `InGameStep` / `CSMenuMan` / Scaleform / D3D12 stack.
- **Multiplayer.** `er-invasion-path` (Havok-AI navmesh), `er-invasion-warp`
  (`CSAutoInvadePoint`), `er-seamless-bugfixes` (Seamless Co-op specific) have no DS2 analogue.
- **Formats.** `soulsformats`, `er-tpf`, `er-flver`, `erpx-rs` target ER format versions. DS2
  params have a different layout and the regulation is `enc_regulation.bnd.dcx` (AES), not
  `regulation.bin`. Partial reuse at best.

## The rule

Do not port an Elden Ring offset, structure layout, or field ordering into this repo on the
strength of it being right in Elden Ring. Derive it from the DS2 binary and record where it
came from. The substrate is shared; the game is not.
