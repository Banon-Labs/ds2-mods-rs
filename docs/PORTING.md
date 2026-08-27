# Porting `er-mods-rs` to DARK SOULS II

What survives the move from Elden Ring, what is blocked, and what is dead. Every number here
was measured against the two checkouts and the shipped binary, not estimated.

## The three blockers

| # | Blocker | Evidence |
|---|---|---|
| 1 | **me3 cannot load DS2** (re-confirmed) | All three entry paths refused: the `--game` enum rejects five DS2 spellings; `--steam-id 335300` gives `unable to determine game from name or app ID`; `--exe <DarkSoulsII.exe>` gives `unable to determine which game to launch`. `--steam-id`/`--exe` resolve against the known-games table rather than bypassing it. See [`LOADING.md`](LOADING.md). |
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
| `pgd.rs`, `profile_summary.rs` | 247 | **Does not port.** ER save and profile layouts. |
| `build_id.rs` | 680 | **Does not port** — though not for the reason first recorded here. It is a *build watermark* (git sha via `build.rs`, PE timestamp, module enumeration), not a save layout. Its first half is genuinely generic; its second half carries ER release-tag and roster specifics, and the whole thing needs a `build.rs`. Revisit if a DS2 watermark is ever wanted. |

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
  path while needing zero knowledge of DS2 structures. **Ported**, as
  `ds2-crash-logging-core` + `ds2-crash-logging` (`ds2_crash_logging.dll`). Its second module
  did not come with it -- see below.

## Tier 2 — blocked on reverse engineering

- **A `darksouls2` crate for `fromsoftware-rs`.** Scale, measured: `darksouls3` is 44,383
  lines, `eldenring` 81,093, `shared` 8,434. Model on **DS3, not ER** — DS3 is one generation
  from DS2, ER is three. **Correction:** this section originally said DS2 predates DLRF and that
  there is no reflection metadata to generate bindings from. That is wrong — the image carries
  **587 DLRF-registered runtime classes** and 5271 RTTI type descriptors. FD4 is genuinely
  absent (one incidental occurrence), but `DLRF`, `DLUT`, `DLKR`, `DLIO` and `DLTX` are all
  present, so DS3's `dlio/dlkr/dltx/dlut` modules have counterparts here and only `fd4` clearly
  does not. See [`DS2-ENGINE.md`](DS2-ENGINE.md).
- **`er-net-effects`** — the original feature of er-mods-rs. DS2 has SpEffects, so the feature
  ports; the plumbing does not. Needs the DS2 player structure, the apply-SpEffect function,
  and the SpEffectParam id space. The smallest possible bet that the bindings are right.

## Tier 3 — rewrites, not ports

- **D3D12 → D3D11.** DS2 imports `d3d11.dll` and `dxgi.dll`. `er-d3d12-compositor` (818 lines)
  and everything on it — `er-loading-bar`, `er-loading-portrait` — needs a D3D11 compositor
  written from scratch. `hudhook` has a D3D11 backend, so the hudhook-based crates have a path;
  the bespoke compositor does not.
- **Scaleform.** `er-scaleform-hooks` and `er-gfx` assume the ER menu system is Scaleform GFx.
  **Now verified: DS2 does not use Scaleform at all** — zero occurrences in the image. Its UI is
  the `Fe*` front-end framework. Both crates are inapplicable, and no ER menu technique
  transfers. See [`DS2-ENGINE.md`](DS2-ENGINE.md).
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

## Constants carry game knowledge too

Two of the three wave-0 ports found Elden Ring facts hiding in code that this document had
called generic. Both were found by reading the constants, not the manifest:

- **`er-hook`** — `PRODUCT_DLL_NAME` / `UNION_REGISTER_EXPORT` were `b"er_quickload.dll\0"` and
  `b"er_effects_union_register\0"`. A crate with an empty `[dependencies]` still named an
  Elden Ring product DLL in its code. Now a caller-supplied parameter.
- **`er-game-base::mem::vtable_in_game_image`** — bounded pointers against a hardcoded
  `0x3000000` image span. That is an **Elden Ring measurement**. DS2's `SizeOfImage` is
  `0x01d7_6000` — about 19 MB smaller — so the function ported unchanged would have accepted
  pointers well past the end of the DS2 image and reported them as in-bounds. It is replaced by
  `ptr_in_module(ptr, base, size_of_image)` plus `module_image_size()`, which reads the real
  `SizeOfImage` out of the loaded PE headers. Derived, not assumed.

The second one is the shape of failure to watch for through the whole port: not a compile error,
not a crash, but a bound that is quietly 60% too large and a validity check that returns `true`
for garbage. Nothing about it looks wrong on the page.

### Wave 2, `er-crash-logging-core`: an entire module was the game knowledge

The crash logger itself ported almost verbatim -- everything in it is Win32 and PE-format
mechanics, and a fault is symbolized against the loader's own module table (the PEB
`InMemoryOrderModuleList`), which is correct for whatever build is running and needs no address
table at all. `ds2-rva` is untouched and stays empty.

Its sibling module `hang.rs` (1,585 lines, half the crate) **does not port**, and the manifest
gives no hint of it: the crate's only dependency is `er-game-base`. Read the constants and the
Elden Ring is everywhere:

- `GAME_FRAME_COUNTER_RVA = 0x3d8_567c` -- the dword `MainUpdate` increments once per frame, on
  `eldenring.exe` 1.16.2 specifically.
- `GAME_MODULE_NAME = "eldenring.exe"` -- checked before that RVA is read at all.
- `CS::LoadingScreenData` field offsets, "verified against live values on 1.16.2" -- a class from
  a framework that postdates this engine.

All three of its detectors hang off those. Strip them and what is left is a thread-suspension
harness with nothing to watch. The DS2 equivalent starts in the disassembly, with a per-frame
counter nobody has found yet, and the address it produced would belong in `ds2-rva` rather than
in a crash logger. Porting the harness first would have shipped a watchdog whose only possible
report is that it is disarmed.

Three smaller constants that were quietly wrong rather than obviously so:

- `SELF_DLL_SIZE_FALLBACK = 0x0400_0000` -- 64 MB, printed as the DLL's own span whenever the PE
  headers could not be read. No mod DLL is 64 MB; that is slack, not a measurement. Now reported
  as `0x0`, which means unknown and is true.
- `context_rbp` was written into every record as a hardcoded `0x0` -- the snapshot never read
  `CONTEXT+0xa0`. A field that always lies is worse than an absent one; it is read now.
- `build=` came from `er-game-base::build_id::GIT_DESCRIPTION`, and `build_id.rs` did not port.
  The line is now composed in `install` from what can be read at runtime -- the loaded DLL's own
  COFF `TimeDateStamp`, where the loader mapped it, and the crate version -- and published
  through `ds2-game-base::log::set_identity_line`. It is called a *watermark*, not a timestamp:
  the MSVC linker writes a content hash into that field for reproducible builds.

**Validated without the game**, which was the point of choosing this crate first: the
`load_and_crash` example loads `ds2_crash_logging.dll` into a throwaway process under Wine and
raises `0xc0000005`. All five artifacts appeared, addresses resolved as
`kernelbase.dll+0xd967`, and the rich minidump tier succeeded. That says nothing yet about
DARK SOULS II, where the loader, the Arxan stubs and the already-installed filters are all
still ahead.
