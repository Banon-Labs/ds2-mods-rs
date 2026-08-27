# A plan for a `darksouls2` bindings crate

Every claim here is sourced to [`DS2-BINDINGS-SURVEY.md`](DS2-BINDINGS-SURVEY.md), which is the
measurement. This document is the argument built on it. Section references below point into the
survey.

## The one-paragraph answer

Dark Souls III's crate is a good *template* and a useless *source*. Its shape -- a module per
Dantelion namespace, `#[repr(C)]` structs, `vtable_rs` traits, a generated param file, an RVA
bundle -- is the right shape for DS2 and should be copied. Its *contents* transfer at a measured
rate of **zero**: 10 of its 117 hand-written types exist in DS2 by name (section 6a), and of the
four whose vtables are measurable, **all four have a different number of slots** (section 4). Its
entire game-specific half, `sprj` -- 3809 of its 5366 hand-written lines -- has no name-level
counterpart at all (section 6b). And the singleton-discovery mechanism that lets DS3 bind seven
managers with a one-line attribute is FD4-based, and DS2 has one occurrence of the string `FD4`
in 30 MB (section 6c). So: same skeleton, no organs.

## What the issue says, and what needs correcting

`ds2-mods-rs-v5z` says three things about scale and shape. Two are wrong.

| Issue's claim | Verdict |
| --- | --- |
| "crates/darksouls3 = 44,383 lines" as the size of the work | **Misleading.** 39017 of those lines (87.9%) are one machine-generated param file produced from paramdef XMLs, not from Ghidra. The hand-written crate is **5366 lines** (section 6). |
| "DS2 predates the FD4 framework" | **Correct**, and it matters more than the issue realises: it removes `shared::singleton` entirely (section 6c). |
| "DS2 predates ... most of the DL* subsystem naming" | **Wrong.** DS2 has **32 DL\* namespaces** -- more than DS3's crate models, not fewer -- including a full input stack, graphics, motion, logging, networking and serialisation layers (section 3b). |

`docs/DS2-ENGINE.md` and `docs/PORTING.md` had already corrected the third one halfway ("DLRF,
DLUT, DLKR, DLIO and DLTX are all present"). The correction was still an undercount by 27
namespaces.

One number in `docs/DS2-ENGINE.md` needs a footnote rather than a correction: its `DLUT 934 /
DLRF 826 / ...` table is right, but those are substring counts inside mangled names, not class
counts. DLUT has **17** classes (section 2, section 3a).

## Where the crate should live

**Recommendation: `crates/darksouls2` in *this* repo first, not as a `fromsoftware-rs` member.**

The issue frames this as a `fromsoftware-rs` member crate, and eventually it should be. But an
upstream member inherits `fromsoftware-shared`, and two of the things a `fromsoftware-rs` game
crate is expected to use do not work here:

- `shared::singleton` / `from_singleton` -- FD4-only (section 6c).
- `shared::arxan::get_arxan_code_restoration_rvas` -- a `pelite::pattern!` for one specific
  guardIT code-restoration shape. This repo already depends on `dearxan` instead, which is a
  different and more thorough neuter. Nothing has checked whether the shared pattern even matches
  this build, and a crate that silently no-ops its own protection neuter is worse than one that
  has none.

`fromsoftware-shared-stl`, `vtable-rs` and `pelite` are all usable as plain dependencies without
becoming a workspace member. Move it upstream when the layouts are proven and the two gaps above
have an answer -- not before, because upstreaming a crate whose vtables are guesses exports the
guesses.

## Module layout

DS3's layout, with the four places DS2 forces a change marked.

```
crates/darksouls2/src/
  lib.rs
  stl.rs            MSVC2012 containers bound to DS2's allocator
  util.rs           module handle, image bounds
  dl/
    kr.rs           DLKR   19 classes  allocator, mutex/spinlock/rwlock, thread, heaps
    io.rs           DLIO   56 classes  streams, DLBinder3/DLBinder4 file devices
    tx.rs           DLTX    0 classes  DLBasicString<char|wchar_t, DLStringTraits, N>
    ut.rs           DLUT   17 classes  DLReferencePointer, DLVector, DLXmlParser
    ui.rs           DLUI   16 classes  input device / mapper / converters
    rf.rs        (+) DLRF    6 core classes + 587 registrations -- NO DS3 COUNTERPART
  game/          (~) replaces `sprj` entirely -- no shared names
    chr.rs            CharacterCtrl (vftable 0x1410df218, 79 slots), player accessors
    speffect.rs       SpEffect list, apply/determine group
    item.rs           inventory, item lots
    event_flag.rs     EventFlagManager
    game_data.rs      GameDataManager (vftable 0x1410f0bb0)
    save.rs           SaveLoad system
  fe/            (~) replaces `app_menu` -- 448 Fe* classes, 597 vtables
  frpg2/         (+) OPTIONAL, feature-gated. 218 classes, 199 already-named methods.
  param/            generated -- IF a DS2 paramdef set can be sourced (see below)
                 (-) NO `rva` MODULE. NO `fd4` MODULE. NO `cs` MODULE.
```

### The four departures from DS3, and why

**(-) No `rva` module.** DS3 keeps `rva/{bundle,rva_ww,rva_jp}.rs` inside the bindings crate. This
repo has already decided the other way: `crates/ds2-rva` exists, is documented as "the only crate
permitted to hold DS2 addresses", and enforces the boundary through the dependency graph rather
than through discipline. `darksouls2` should depend on `ds2-rva`, not grow a second address table.
DS3's version-detection layer (`Ww1152` vs `Jp11521`) is also premature here -- SotFS PC is one
build, and `ds2-rva::BUILD_ID = 9_527_516` already anchors it.

**(-) No `fd4`, no `cs`.** One `FD4` occurrence in the image (section 2). DS2's only `CS*`
namespaces are `CSNW`/`CSNWD`, 11 socket classes with nothing to do with DS3's `CS` game
namespace; `CSDlc` does not exist (section 6d).

**(+) A `dl::rf` module, which DS3 does not have.** DS2 registers **587 classes** with
`DLRF::DLRuntimeClass`, and the reflection core is small: `DLRuntimeClass`,
`DLAbstractInvokeContext`, `DLMethodInvokeContext`, `DLMethodInvoker<>`,
`DLRuntimeConstructionContext` -- 6 classes and 7503 functions behind them. **This is the biggest
open opportunity in the port** and it is also the biggest thing that could fail: if the registry
can be walked at runtime, name-to-class lookup comes back and partly replaces what FD4 singletons
give DS3. If it only carries type metadata and no instance pointers, it gives nothing for
singleton discovery. *Nothing in this survey establishes which.* See "First investigations".

**(~) `game/` replaces `sprj/`, `fe/` replaces `app_menu/`.** Not renames -- different classes.
Keeping the name `sprj` for a module containing no `Sprj*` class would be a lie in the directory
tree.

## Size estimate

DS3's own per-module line counts are the unit, adjusted by the measured class counts.

| module | DS3 | DS2 estimate | reasoning |
| --- | ---: | ---: | --- |
| `stl.rs` | 31 | 30-60 | Same job. VS2012-era CRT shapes present (section 6d), but `size_of::<DLVector<usize>> == 0x20` is unconfirmed. |
| `util.rs` | 59 | 60 | Generic. |
| `dl/kr.rs` | 69 | 150-250 | Same two types, but `DLAllocator` is **27 slots** here vs 14 in DS3 -- the trait alone is twice as long, and 5 lock types exist rather than 1. |
| `dl/io.rs` | 196 | 200-350 | 56 classes vs DS3's handful; `DLMemoryInputStream` is >=16 slots vs DS3's 7. |
| `dl/tx.rs` | 162 | 150-250 | Same job, but the character-set enum must be re-derived: DS2's `wchar_t` instantiation carries template arg `3` where DS3's `DLCharacterSet` puts UTF16 at `1`. |
| `dl/ut.rs` | 135 | 150-250 | DS3's is pure Rust with no addresses -- the one module that might port near-verbatim, after checking the container layout. |
| `dl/ui.rs` | 126 | 150-300 | 16 `DLUI` + 32 `DLUID` classes; larger than DS3's module. |
| `dl/rf.rs` | -- | 200-400 | New. Only if the registry proves walkable. |
| `game/` | 3809 (`sprj`) | 2000-4000 | Similar line count for **fewer** classes, because every field offset is derived by hand instead of inherited from community knowledge. |
| `fe/` | 358 (`app_menu`) | 300-800 | For a *first useful set*. Binding the whole 448-class framework is not a v1 goal. |
| `frpg2/` | -- | 0 for v1 | 199 methods are already named, so it is cheap later. It is also not on the path to any gameplay mod. |
| `param/generated.rs` | 39017 | **0 or ~35000** | Binary contributes nothing: no `*_PARAM_ST` string exists in the image (section 6d). It is 0 without a paramdef set and ~35000 mechanical lines with one. |

**Hand-written total: roughly 3500-7000 lines.** That brackets DS3's 5366 -- so on *line count*
the issue's instinct to model on DS3 is right.

**On effort it is not close.** DS3's crate was written against a game where the class names match
what the modding community already knew, seven managers resolve through a one-line attribute, and
the address table is 21 entries. For DS2:

- Every field offset is derived from scratch. 10 name matches out of 117, and 0 of 4 checkable
  layouts agreed (section 4, section 6a).
- Every singleton needs its own RVA, derived by decompiling an accessor. There is no scan.
- **Method names do not exist.** 31639 functions carry a class namespace; the method is
  `FUN_<va>` (section 3c). Identifying a class is reading. Identifying a method is work.
- 311 function entry points are Arxan-redirected, and the first one anybody checked --
  `applySpEffect` -- is one of them (section 5).

The realistic read: this is a multi-session job whose *bottleneck is not typing Rust*. Anyone
scheduling it off the 44383 figure will be wrong twice -- the number is 8x too big for the
hand-written surface, and the per-line cost is several times DS3's.

## What DS2 hands you for free, and it is a lot

The survey's most useful practical finding is that the Ghidra project contains **1147
`USER_DEFINED` symbols** -- names a human typed -- and no document in this repo mentions them.
They include the exact entry points the first gameplay mod needs: `applySpEffect`,
`addSpEffectToList`, `determineSpEffectGroup`, `getPlayerStruct`, `addItemToInventory`,
`addSoul`, `applyDamage`, `getWeaponParamRow`, `getItemRow`, `getSaveLoadSystem`,
`getNetSessionManager`, and 199 named `Frpg2*` request/response methods (section 7).

That changes the shape of the first milestone. `ds2-mods-rs-a1g` (`ds2-net-effects`) is described
as needing "the DS2 player structure, the apply-SpEffect function, and the SpEffectParam id
space". Two of those three are already named and addressed. The third is a data question.

**And there is a working recipe for singletons.** `getPlayerStruct` (`0x1402a6820`) reads a plain
global:

```
1402a687b  MOV RAX,qword ptr [0x1416148f0]
1402a6882  MOV RDX,qword ptr [RAX + 0xa8]
```

`0x1416148f0` is inside the uninitialised `.data` block (`0x14160aa00`-`0x1418993ef`) -- a
singleton-shaped static, at RVA `0x16148f0`. That is what a DS2 singleton looks like and how one
is found: decompile an accessor that already has a name, read the global out of it, record the
RVA with provenance in `ds2-rva`. Repeat per manager. It is slower than DS3's scan and it is not
hard.

(Recorded as *observed*, not *established*: nothing yet confirms that global is a player-manager
pointer rather than something `getPlayerStruct` consults on the way. Confirming it is one
`Ds2Xrefs` run on `0x1416148f0`.)

## Ordering

Numbered because the dependencies are real, not because a plan looks tidier numbered.

1. **The Arxan gate.** Does a MinHook detour survive at all? This is already the repo's M1
   question and it is still open (`docs/DS2-ENGINE.md`, "Still unverified"). **Every address
   below is worthless if the answer is no.** Nothing in this plan should start before it.

2. **Arxan-triage the 17 named entry points** (section 7). One command:
   `Ds2ArxanStubs 0x14014bec0 0x1402206d0 ...`. The first one checked was Arxan-redirected, so
   the prior on the rest is not good. Costs minutes; changes which functions the design is built
   around.

3. **Seed `ds2-rva` from the named accessors.** Decompile `getPlayerStruct`, `getSaveLoadSystem`,
   `getNetSessionManager`, `getMapItemPackManager`, `getSpEffectOwner_characterCtrl`; record the
   globals they read. No new crate needed, no layout assumed, and it is the input to everything
   after.

4. **`dl::kr` -- the allocator.** Everything with a container or a string in it needs
   `DLKR::DLAllocator` first. Start by settling the unresolved slot question (section 4): walk
   `DLKRD::HeapAllocator<...>::vftable` at `0x1410d2e28` and identify allocate / allocate-aligned
   / deallocate from the implementations, not from DS3's slot numbers.

5. **`stl` + `dl::tx` + `dl::ut`.** The container and string layer. `DLVector`'s size assertion is
   the first cheap falsifiable test the crate can carry.

6. **`game::chr` + `game::speffect`, minimal.** Player pointer, SpEffect list, one apply path.
   This is the smallest possible bet that the bindings are right, and it is exactly what
   `ds2-mods-rs-a1g` needs. Note that `applySpEffect` itself is unhookable at its entry --
   `addSpEffectToList` (`0x1402206d0`, clean 129-byte prologue) is the usable neighbour.

7. **`dl::rf` investigation.** Can the `DLRF::DLRuntimeClass` registry be enumerated at runtime,
   and does a registration carry an instance pointer? If yes, singleton lookup gets much cheaper
   and step 3 stops being per-manager work. If no, say so and stop. This is worth doing early
   *as an investigation* and late *as code*.

8. **`param`.** Purely a question of whether a DS2 paramdef set exists in a form
   `tools/param-generator` can eat. If it does, ~35000 lines appear for near-zero RE effort and
   the crate suddenly "looks like" DS3's. If it does not, the crate is 5000 lines and that is
   fine.

9. **`fe`, `frpg2`.** Real, large, and not on the path to any currently-filed mod.

## No skeleton, and why

The task allowed a minimal compiling skeleton "if and only if the survey supports it, but only
bindings you can point at specific verified addresses/symbols for."

**The survey does not support one.** Concretely: the simplest possible binding is
`DLKR::DLAllocator`, and after walking its vtable, walking its concrete subclass, and decompiling
four of its slots, section 4 still cannot say which slot allocates. The one piece of evidence
that looked like an answer -- `heapAllocator` dispatching to `+0x50`, exactly where DS3 puts
`allocate_aligned` -- turned out not to survive the check that would have confirmed it. That is
the whole lesson of this survey in one class: the name matches, the shape does not, and the
temptation to fill the gap from DS3 is strongest exactly where it is most wrong.

A `#[repr(C)] pub struct DLAllocator` with DS3's vtable trait would compile today, would look
correct on the page, and would call the wrong function on most slots. `docs/PORTING.md` already
records this failure mode from the `vtable_in_game_image` port: "not a compile error, not a
crash, but a bound that is quietly 60% too large and a validity check that returns `true` for
garbage."

What this branch ships instead is the three tools and one bug fix that make step 4 a short job
rather than a long one (section 9).

## Does the DS3 model hold?

**For the crate's architecture, yes** -- module-per-namespace, `#[repr(C)]` + `vtable_rs`, a
generated param file, a single audited address table, `fromsoftware-shared-stl` for the MSVC
containers. Copy all of it.

**For the crate's content, no, and not partially.** The measured transfer rate is 10 type names
out of 117 and 0 layouts out of 4. Treat every DS3 struct definition as a *hypothesis about DS2
that has already been falsified four times out of four*, and derive it from this binary instead.
That is the rule `docs/PORTING.md` states for Elden Ring; the survey's contribution is showing it
applies to Dark Souls III just as hard, which was not obvious -- DS3 is one generation away, and
one generation was enough.
