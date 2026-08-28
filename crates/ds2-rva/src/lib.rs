//! Every DS2 address this repo knows, and nothing else.
//!
//! # Why this is its own crate, and why it is empty
//!
//! In `../er-mods-rs` the address table lives inside `er-game-base` as `rva.rs`, next to
//! generic process utilities. That works there because the addresses were settled years
//! before the crate was split. Here they are not settled at all: **no reverse engineering has
//! been done yet**, and the assumption that DS2 resembles Elden Ring is exactly the one that
//! must not be allowed to leak into the substrate.
//!
//! So the boundary is enforced by the dependency graph rather than by discipline:
//! `ds2-game-base`, `ds2-hook`, `ds2-hotkey-config` and `ds2-safe-input` do **not** depend on
//! this crate and contain **no** game knowledge. Everything that claims to know something
//! about DARK SOULS II declares it here, where a wrong claim is one file to audit.
//!
//! # The engines are not the same
//!
//! DARK SOULS II shipped in 2014; Elden Ring in 2022. Between them FromSoftware replaced the
//! menu stack, the rendering backend and the object/reflection framework. Concretely, and
//! already verified against this build:
//!
//! - **D3D11, not D3D12.** `DarkSoulsII.exe` imports `d3d11.dll` and `dxgi.dll`.
//! - **No FD4.** The `FD4FileCap` / `DLString<wchar_t>` / DLIO virtual-root layouts that
//!   `er-game-base::filecap` walks postdate this engine.
//! - **No `fromsoftware-rs` bindings.** That workspace has `darksouls3`, `eldenring`,
//!   `nightreign` and `sekiro` members and no `darksouls2`. DS3 is the nearer relative; even
//!   it is a generation away.
//!
//! Do not port an Elden Ring offset into this file. Derive it, from the binary, and say where
//! it came from.
//!
//! # Address convention
//!
//! Every constant here is an **RVA** -- an offset from the image base, not a runtime address.
//! Add [`IMAGE_BASE`] to get the VA the disassembly shows.
//!
//! The authoritative artifact is `darksoulsii-deobf.bin` at the repo root, produced by
//! `../dearxan`'s `deobfuscate` example. It is a flat mapped image: **file offset == RVA**, so
//! a Ghidra address of `0x141234567` is byte `0x1234567` of that file. It is gitignored --
//! it is the copyrighted game binary. Regenerate it with:
//!
//! ```text
//! cargo run --release --manifest-path ../dearxan/Cargo.toml --example deobfuscate \
//!   --no-default-features --features rayon -- <DarkSoulsII.exe> darksoulsii-deobf.bin
//! ```
//!
//! # Recording an address
//!
//! One `pub const`, one doc comment, and the doc comment must say **how the address was
//! established** -- the function it was read out of, the xref count, the string that anchored
//! it. An address with no provenance is a guess wearing a type.

/// Preferred image base of `DarkSoulsII.exe` (`OptionalHeader.ImageBase`).
///
/// `DllCharacteristics` is `0x8160`, so `DYNAMIC_BASE` is set and the loader is free to
/// relocate. **Never assume the live base equals this value** -- resolve it from the loaded
/// module at runtime and add the RVAs below to that. This constant exists to translate
/// between the disassembly's VAs and the RVAs recorded here, nothing more.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

/// `OptionalHeader.SizeOfImage` -- the mapped size, and therefore the length of
/// `darksoulsii-deobf.bin`.
pub const SIZE_OF_IMAGE: u32 = 0x01d7_6000;

/// Steam application id for DARK SOULS II: Scholar of the First Sin.
pub const STEAM_APP_ID: u32 = 335300;

/// The build these addresses were derived from, as reported by `appmanifest_335300.acf`.
///
/// Every constant below is anchored to this build and to nothing else. A Steam update that
/// changes this value invalidates the whole table until each entry is re-derived.
pub const BUILD_ID: u32 = 9_527_516;

// ============================================================================================
// ADDRESSES
//
// One entry, and it is not a feature's address -- it is the subject of an EXPERIMENT. The
// question "does a MinHook detour survive Arxan in this game" gates every hooking feature this
// repo could ever have, and it cannot be answered without patching some specific function. So
// the first address here is the one chosen to be patched, and its doc comment carries the whole
// derivation, because a hook site picked badly makes the experiment fail for the wrong reason.
//
// Everything else is still empty and still honest. The next entries arrive with the first mod
// that needs one.
// ============================================================================================

/// **M1 hook site**: the function the Arxan-survival probe detours. RVA `0x00832e70`.
///
/// # What this address is for
///
/// It is not needed by any feature. It exists so that
/// [`ds2-loader`'s Arxan probe](https://github.com/Banon-Labs/ds2-mods-rs/blob/main/crates/ds2-loader/src/arxan_probe.rs)
/// can patch *something* and watch whether the patch survives. DS2 carries 48 Arxan stubs and
/// 286 Arxan-redirected functions; MinHook works by rewriting a function prologue in `.text`,
/// which is exactly the thing an integrity check looks for. Until a detour has been shown to
/// survive in this game, every plan in this repo that involves hooking is unproven. This
/// constant is the subject of that experiment and nothing else.
///
/// # How it was established
///
/// Measured from `darksoulsii-deobf.bin` (build [`BUILD_ID`]), statically, with no runtime:
///
/// * `.pdata` (`RUNTIME_FUNCTION[]` at RVA `0x189a000`, size `0x117978`, 12 bytes each) gives
///   all **95434** function starts for free. Counting `e8 rel32` call targets that land exactly
///   on one of those starts resolved **149022** direct calls without disassembling 17 MB.
/// * This function is the target of **2052** of them -- rank 3 in the whole binary. A detour
///   here that never fires is a real signal rather than an expected one.
/// * Its prologue is `48 89 5c 24 08` (`mov [rsp+8], rbx`), then `57`, then `48 83 ec 20`. The
///   first instruction is **exactly 5 bytes**, so MinHook relocates one whole instruction into
///   the trampoline and never has to split one -- the trivial case, and the reason this site
///   was preferred over an equally hot one with a 4-byte first instruction.
/// * It is `0x47` bytes long, so the 5-byte `e9 rel32` MinHook writes fits with room to spare,
///   and no branch inside the function targets a byte within those five.
/// * It is **not** one of the 286 Arxan-redirected functions.
///
/// See `docs/ARXAN-PROBE.md` for the experiment this feeds, and `docs/ARXAN-FOOTPRINT.md` for
/// the survey the counts above come from.
///
/// # Resolve it, do not hardcode the VA
///
/// The disassembly shows `0x140832e70`, but [`IMAGE_BASE`] is only the *preferred* base and
/// `DllCharacteristics` is `0x8160` (`DYNAMIC_BASE`). Add this RVA to the base read out of the
/// loaded module at runtime -- `ds2_game_base::mem::game_rva` does exactly that.
pub const ARXAN_PROBE_HOOK_SITE: u32 = 0x0083_2e70;

/// The bytes [`ARXAN_PROBE_HOOK_SITE`] is expected to begin with, before anything patches it.
///
/// The probe reads the live prologue at install time and compares it against this. A mismatch
/// means something else reached this function first -- another mod, an Arxan stub that this
/// build places differently, or the wrong game version -- and the probe declares the run VOID
/// rather than reporting on a patch it did not make cleanly. Five bytes because that is the
/// whole first instruction and the whole of what MinHook overwrites.
pub const ARXAN_PROBE_HOOK_SITE_PROLOGUE: [u8; 5] = [0x48, 0x89, 0x5c, 0x24, 0x08];

/// Length of [`ARXAN_PROBE_HOOK_SITE`], from its `.pdata` `RUNTIME_FUNCTION` entry.
///
/// Recorded because "is there room for a 5-byte jump" is the question a hook site has to answer,
/// and `0x47` answers it without anyone re-reading `.pdata`.
pub const ARXAN_PROBE_HOOK_SITE_LEN: u32 = 0x47;

/// Backup M1 hook site, if [`ARXAN_PROBE_HOOK_SITE`] turns out to be unusable. RVA `0x008389e0`.
///
/// Same derivation, same prologue shape (`48 89 5c 24 08 / 57 / 48 83 ec 20`), **1287** static
/// call sites, `0x4e` bytes long, also not one of the 286. It is the second choice only because
/// it is called less often; on every property that decides whether a hook can be installed it is
/// equivalent.
pub const ARXAN_PROBE_HOOK_SITE_BACKUP: u32 = 0x0083_89e0;

// ============================================================================================
// THE TITLE BOOT SCREENS (`ds2-mods-rs-3rr`)
//
// `FeStateTitle` drives a family of `FeSubState*` objects off a shared 8-slot vtable, whose
// slots are v0 destructor, v1 enter, v2 leave, v3 update, v4 unused, v5/v6 a debug-registration
// pair, v7 a bool query. Four of the eight are empty `ret` stubs in `FeSubStateBase`, so any
// override is that substate's real logic. Full trace: `docs/DS2-TITLE-FLOW.md`.
//
// EACH SUBSTATE KEEPS ITS OWN PHASE COUNTER, AND THE OFFSET IS NOT SHARED. Logo's is at `+0x20`;
// the other two are at `+0x10`. That is measured per class from the field each one's own `v3`
// switches on, not inferred from a common base -- assuming one offset for all three would write
// a 4 into an unrelated member of two of them.
//
// The terminal value IS 4 in all three, and in each case the game itself has a shipped path where
// `v1` writes exactly that and returns having done nothing else. That is what makes forcing it a
// reproduction of the game's own behaviour rather than an invented transition.
// ============================================================================================

/// `FeSubStateTitleLogo::v1` (enter). RVA `0x000fd980`, VA `0x1400fd980`.
///
/// The publisher-logo screen. Its own `v1` sets the phase to [`TITLE_SUBSTATE_PHASE_DONE`] and
/// returns immediately when the scene reference at `+0x18` is null -- the game's shipped "there
/// is no logo to show" path, and the one this mod reproduces.
///
/// Not Arxan-redirected: `scripts/ds2-arxan-chain.py 0x1400fd980` terminates at hop 0 with the
/// function's own prologue (`40 53 48 83 ec 20`, 6 relocatable bytes, comfortably over MinHook's
/// five).
pub const FE_SUBSTATE_TITLE_LOGO_ENTER: u32 = 0x000f_d980;

/// Scene-reference offset within `FeSubStateTitleLogo`. `+0x18`.
///
/// **Nulling this is what stops the logo animating**, and it is the game's own path rather than a
/// new one. `FeSubStateTitleLogo::v1` opens `mov rcx,[rcx+0x18]; test rcx,rcx; je` and the
/// not-taken branch at `0x1400fd9fb` is the whole of the shipped "there is no logo to show" case:
/// write phase 4, return. It plays no sequence and opens nothing.
///
/// The previous version of this crate let the original `enter` run and then wrote phase 4 after
/// it. That advances the flow but the sequence `enter` already started
/// (`0x140afdb80(scene, 0x66, ...)`, or `0x67` on the skip path) keeps playing, which is exactly
/// the logo animation that remained visible.
///
/// # Why nulling it is balanced at both ends
///
/// `FeSubStateTitleLogo::v2` (leave, `0x1400fe830`) is guarded on **this same pointer**:
/// `mov rcx,[rcx+0x18]; test rcx,rcx; je` and it closes only when non-null. So null at `enter` and
/// null at `leave` is precisely the pair the shipped path produces -- neither opens nor closes.
/// Restoring the pointer afterwards would instead produce a close with no matching open, which is
/// the unbalance this avoids.
///
/// **`enter` does not create the scene, it reads one already there**, so nulling the substate's
/// copy does not orphan an allocation this crate made; it declines to start and stop something
/// another object owns.
///
/// This trick does NOT transfer to the other two boot screens, and their `leave` implementations
/// are why: `FeSubStateWarningNoCopy::v2` (`0x1400febb0`) guards on a **global** at
/// `[0x14160de10]+0xf0` rather than on the substate, and `FeSubStateTitleUserPolicy::v2`
/// (`0x1400f96b0`) guards on **phase == 1**. Three classes, three different guards; each one had
/// to be read.
pub const FE_SUBSTATE_TITLE_LOGO_SCENE_OFFSET: usize = 0x18;

/// Phase-counter offset within `FeSubStateTitleLogo`. **`+0x20`, not `+0x10`.**
///
/// Read from `FeSubStateTitleLogo::v3` at `0x1400febf0`, which opens
/// `mov ecx, DWORD PTR [rcx+0x20]` and then a `dec`/`je` chain over phases 1, 2, 3.
pub const FE_SUBSTATE_TITLE_LOGO_PHASE_OFFSET: usize = 0x20;

/// `FeSubStateWarningNoCopy::v1` (enter). RVA `0x000fded0`.
///
/// The unauthorised-copying warning. Its `v1` calls a virtual at `+0x40` on the singleton at
/// `0x1416751f8` and, when that returns nonzero, writes phase 4 and returns -- so this screen
/// already skips itself under some condition the game knows about.
///
/// Not Arxan-redirected (hop 0, prologue `40 53 48 83 ec 20`).
pub const FE_SUBSTATE_WARNING_NO_COPY_ENTER: u32 = 0x000f_ded0;

/// `FeSubStateTitleUserPolicy::v1` (enter). RVA `0x000f9040`.
///
/// The user-policy screen. Its `v1` has two shipped early-outs, both reading persisted flags out
/// of the system-data object reached as `[[0x1416148f0]+0xa8]+0xd8`: `+0x136e` set means phase 3,
/// `+0x136d` set means phase 4. Those are the game's own "already accepted" flags, which is why
/// 4 is the terminal value here too.
///
/// Not Arxan-redirected (hop 0). Its prologue is `48 89 6c 24 20`, exactly five bytes -- MinHook's
/// minimum, met with nothing to spare.
pub const FE_SUBSTATE_TITLE_USER_POLICY_ENTER: u32 = 0x000f_9040;

/// Phase-counter offset within `FeSubStateWarningNoCopy` and `FeSubStateTitleUserPolicy`.
///
/// Read from each one's own `v3` (`0x1400ff360` and `0x1400f96f0`), both of which open
/// `mov edx, DWORD PTR [rcx+0x10]` before their `dec`/`je` chain.
pub const FE_SUBSTATE_PHASE_OFFSET: usize = 0x10;

/// The phase value that means "this substate is finished", for all three boot screens.
///
/// Reached by three independent paths in `FeSubStateTitleLogo` alone: normal completion at the end
/// of phase 3, the player's skip-button path at `0x1400fec6c`, and the null-scene path in `v1`.
/// A player pressing the skip button lands here every time and the flow advances, which is the
/// evidence that something consumes it -- the consumer itself has not been located and does not
/// need to be.
pub const TITLE_SUBSTATE_PHASE_DONE: u32 = 4;

/// **The redirected probe site**: `applySpEffect`, RVA `0x0014bec0`. Deliberately Arxan's.
///
/// [`ARXAN_PROBE_HOOK_SITE`] is a clean function, and that is exactly its limitation: walking it
/// with `scripts/ds2-arxan-chain.py` terminates at hop 0 because its own prologue is at its own
/// entry. Arxan has no presence there, so a detour on it can never provoke Arxan, and both arms
/// of `ds2-mods-rs-z6m` surviving was a null result by construction. This constant exists so the
/// experiment can be run somewhere Arxan actually is.
///
/// # Why this one
///
/// It is the only site found so far that is both genuinely redirected and functionally
/// load-bearing -- `ds2-mods-rs-a1g` needs it -- so one experiment answers both questions.
///
/// # Why hooking it is sound, despite [`ARXAN_REDIRECTED_DO_NOT_HOOK`] below
///
/// That exclusion says a detour over Arxan's redirect would break control flow. It does not,
/// and the reason is in MinHook's `trampoline.c`: for a relative `E9` whose destination lies
/// outside the five bytes being patched, MinHook does not copy the instruction, it emits a
/// `JMP_ABS` (`FF 25` + an absolute 8-byte address) to that destination and marks the trampoline
/// complete. So the trampoline jumps to `0x141b3cbe1`, Arxan's chain runs the stolen prologue,
/// and execution rejoins the original function at entry + `0x14` exactly as it would unhooked.
/// `oldPos` is 5, which meets MinHook's minimum, so the hook installs.
///
/// The two constants below stay excluded for a different reason: they are Arxan's own hot
/// dispatch functions rather than game functions that happen to be redirected.
///
/// # How it was established
///
/// `scripts/ds2-arxan-chain.py 0x14014bec0`, statically, no runtime: five hops -- the entry
/// `jmp`, two obfuscation thunks, then two fragments carrying 16 instructions of genuine stolen
/// prologue -- rejoining game `.text` at entry + `0x14`. Confirmed separately that no Arxan
/// encrypted region covers it (2969 regions examined, span `0x140001680`-`0x141cfa783`), so a
/// stub cannot silently decrypt original bytes back over the detour. See
/// `docs/ARXAN-FOOTPRINT.md`.
pub const ARXAN_PROBE_REDIRECTED_SITE: u32 = 0x0014_bec0;

/// The five bytes [`ARXAN_PROBE_REDIRECTED_SITE`] begins with: Arxan's own redirect.
///
/// `e9 1c 0d 9f 01` is `jmp 0x141b3cbe1`. The displacement is relative and both ends move
/// together, so these bytes are identical whatever base the image loads at -- no relocation
/// applies to them, which is what makes a fixed expectation valid here.
///
/// `neuter_arxan` does not rewrite this. Its patch set is a `JmpHook` at each stub's
/// `test rsp, 0xf` plus `Write`s of decrypted regions, of which DS2 gets zero. So both arms
/// install over byte-identical bytes and the comparison between them is meaningful.
pub const ARXAN_PROBE_REDIRECTED_SITE_PROLOGUE: [u8; 5] = [0xe9, 0x1c, 0x0d, 0x9f, 0x01];

/// **NEVER HOOK THESE.** The two hottest functions in the binary, and both are Arxan's.
///
/// `0x00832cb0` (12401 call sites) begins `e9 c1 50 34 01` -> `0x141b77d76`, and `0x00c2c9e0`
/// (4866 call sites) begins `e9 ba e7 f3 00` -> `0x141b6b19f`. Both jumps land in `.text` #2
/// (VA `0x141aaf000`-`0x141d43000`), Arxan's own section. They are recorded here as a named
/// exclusion rather than left out, because the next person ranking functions by call count will
/// find exactly these two at the top and needs to know why they are skipped.
///
/// Detouring one of them would mean writing over Arxan's own redirect. The experiment would then
/// fail -- or the game would crash -- for a reason that has nothing to do with the question being
/// asked, which is whether Arxan reverts an *ordinary* hook.
pub const ARXAN_REDIRECTED_DO_NOT_HOOK: [u32; 2] = [0x0083_2cb0, 0x00c2_c9e0];

// ============================================================================================
// THE TITLE-FLOW MESSAGE DIALOGS (`ds2-mods-rs-j3b`)
//
// Skipping the three boot screens does not reach the title menu, because the flow then stops on
// message boxes that wait for a button. Six classes draw them, and ALL SIX SHARE ONE `update`:
// `FeSubStateCommonWindowBase::v3`. That is the whole reason this is three constants and not
// eighteen -- there is exactly one place where a dialog decides it is finished.
//
// WHAT THE SHARED UPDATE ACTUALLY DOES, read from `0x140105150`. On the frame a button is
// pressed, its ONLY effect is a store into the result byte at `+0x31`; the dispatch that follows
// reads that byte and nothing else about the press. So writing `+0x31` is not an approximation of
// a button press, it IS the button press, minus the polling. Every remaining constant below is a
// field that store depends on.
//
// The layout was read from the two functions that own it -- `v1` (enter) at `0x140104db0` and
// `v3` (update) at `0x140105150` -- and NOT inferred from the boot screens above. It is a
// different base class with a different layout: these dialogs keep their phase at `+0x30`, where
// `FeSubStateWarningNoCopy` keeps its at `+0x10` and `FeSubStateTitleLogo` keeps its at `+0x20`.
// ============================================================================================

/// `FeSubStateCommonWindowBase::v3` (update). RVA `0x00105150`, VA `0x140105150`.
///
/// The one function every message box in the title flow runs each frame, and the only place any
/// of them transitions. Its phase-1 branch polls for input, stores a result into
/// [`FE_DIALOG_RESULT_OFFSET`], and then dispatches on that byte through vtable slot 8 or 9.
///
/// Shared by all six classes whose `v3` slot holds this address: `FeSubStateCommonWindowBase`,
/// `FeSubStateCommonWindow`, `FeSubStateOfflineModeWindow`, `FeSubStateTitleOnlineCheckFailWarn`,
/// `FeSubStateTitleInformationFailWarn` and `FeSubStateTitleDeleteProfile`. Enumerated by scanning
/// every RTTI-described vtable in the image for that slot value, so the list is exhaustive for
/// this build rather than the ones that happened to be looked for.
///
/// # Why hooking it is sound
///
/// `scripts/ds2-arxan-chain.py 0x140105150` terminates at hop 0: **not** Arxan-redirected, its own
/// prologue is at its own entry. `.pdata` gives it length `0xed`, ample room for a five-byte
/// `e9`. Its first instruction is `48 89 5c 24 08` (`mov [rsp+8], rbx`) -- exactly five bytes, so
/// MinHook relocates one whole instruction and never has to split one. That is the same trivial
/// case as [`ARXAN_PROBE_HOOK_SITE`], which is already proven to install and fire in this game.
pub const FE_DIALOG_UPDATE: u32 = 0x0010_5150;

/// `FeSubStateCommonWindowBase::v1` (enter). RVA `0x00104db0`, VA `0x140104db0`.
///
/// **The one place a title message box is actually created**, and therefore the only place it can
/// be prevented rather than dismissed. It picks one of two show calls off the sign of
/// [`FE_DIALOG_CONFIRM_DEST_OFFSET`] -- `0x1404fe2a0` for a one-button box, `0x1404fe1c0` for a choice
/// -- and ends `mov DWORD PTR [rdi+0x18],0` / `mov WORD PTR [rdi+0x30],1`.
///
/// EVERY dialog class reaches it, including the two that override `v1`:
/// `FeSubStateTitleOnlineCheckFailWarn::v1` formats its message and then `call 0x140104db0` at
/// `0x1400fd471`, and `FeSubStateTitleInformationFailWarn::v1` does the same. So a single detour
/// here covers all six, where a detour on each class's own `v1` would need four.
///
/// # Why skipping it does not leak a window
///
/// `leave` (`0x1401050a0`) closes the window **only when the phase is 1**:
/// `cmp BYTE PTR [rcx+0x30],1` / `jne`, and the not-taken path merely zeroes the phase. So an
/// `enter` that never ran and never opened anything pairs with a `leave` that never closes
/// anything. That conditional is what makes suppression sound here and is exactly what
/// `ds2-intro-skip` did NOT have available -- its `leave` closes unconditionally, which is why
/// that crate lets the original `enter` run and only rewrites the phase afterwards.
///
/// Not Arxan-redirected; `.pdata` gives it RVA `0x104db0`-`0x104df4`, and its first instruction
/// `48 89 5c 24 20` is a whole five bytes.
pub const FE_DIALOG_ENTER: u32 = 0x0010_4db0;

/// Phase byte within a common-window substate. `+0x30`.
///
/// `1` while the box is up and waiting, `2` while it plays its close animation, then `3` or `4`
/// once closed. `v1` (enter) ends with `mov WORD PTR [rdi+0x30], 1` -- a **16-bit** store that
/// sets this byte to 1 and [`FE_DIALOG_RESULT_OFFSET`] to 0 in one instruction, which is what
/// establishes that the two fields are adjacent and in this order.
pub const FE_DIALOG_PHASE_OFFSET: usize = 0x30;

/// Result byte within a common-window substate. `+0x31`.
///
/// `0` undecided, `1` the back/cancel answer, `2` the confirm answer. **This is the byte a button
/// press writes**, and the dispatch at `0x140105200` reads it, calls vtable slot 8 for `1` or slot
/// 9 for `2`, and sets the phase to 2. Writing it is how this mod presses the button.
pub const FE_DIALOG_RESULT_OFFSET: usize = 0x31;

/// **Destination substate id for the CONFIRM edge**, or `-1` when the box has no confirm edge.
/// `+0x12`, a signed WORD.
///
/// This was recorded as an "option count" and that was wrong in a way worth spelling out, because
/// the wrong reading is behaviourally right and therefore does not announce itself. `v5`
/// (`0x140104f30`) publishes the second transition only when this field is non-negative:
///
/// ```text
/// cmp   WORD PTR [rdi+0x12], 0
/// jl    done                         ; negative -> NO confirm edge at all
/// movsx ecx, WORD PTR [rdi+0x12]     ; <- this field
/// mov   [rax+0x08], ecx              ; destination substate
/// mov   BYTE PTR [rax+0x20], 4       ; ...when the phase is 4, the confirm-closed phase
/// ```
///
/// So "negative means a one-button acknowledgement box" is a true *consequence* -- a box with no
/// confirm destination has nothing for a second button to do -- and every sign test the old doc
/// cited is real. But the magnitude is not a count. The constructor at `0x140104c00` seeds it to
/// `-1` (`or eax,-1; mov WORD PTR [rcx+0x12], ax`) and whoever raises a box overwrites it.
///
/// **The practical consequence:** a log line reading `options=42` never meant forty-two options.
/// It meant destination `0x2a`, [`FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW`].
pub const FE_DIALOG_CONFIRM_DEST_OFFSET: usize = 0x12;

/// [`FE_DIALOG_PHASE_OFFSET`] once the box has closed on a [`FE_DIALOG_RESULT_CONFIRM`].
///
/// The pair of [`FE_DIALOG_PHASE_CLOSED_CANCEL`]; `v5` watches for this value to take the
/// [`FE_DIALOG_CONFIRM_DEST_OFFSET`] edge.
pub const FE_DIALOG_PHASE_CLOSED_CONFIRM: u8 = 4;

/// `FeSubStateOfflineModeWindow`'s substate id: `0x2a`.
///
/// **Do not reason about this box from its button labels.** The two-option box the game raises
/// when the server login fails says, in its own text, `Select "OK" to attempt to log in again` and
/// `Select "CANCEL" to start the game in offline mode` -- so the obvious move is to write
/// [`FE_DIALOG_RESULT_CANCEL`]. That is the wrong answer, and it is wrong in the worst possible
/// direction for a mod whose purpose is playing offline.
///
/// Read live out of the object (`kind=0x3e`) on a running game:
///
/// ```text
/// CANCEL_dest = 0x39   FeSubStateTitleGameServerLogin   <- retries the login
/// CONFIRM_dest = 0x2a  FeSubStateOfflineModeWindow      <- plays offline
/// ```
///
/// So the edge that goes offline is the **confirm** edge. Corroborated twice over: the run that
/// produced those numbers had `result=2` in the object and logged
/// `suppressed screen=offline-mode-window kind=42` as the very next line, and eight other boxes in
/// the same substate table carry `0x2a` as their cancel destination.
///
/// Anything answering that box should select its edge by comparing these destination ids against
/// this constant, never by picking a result value from what the buttons are called.
pub const FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW: i16 = 0x2a;

/// [`FE_DIALOG_PHASE_OFFSET`] while the box is up and waiting for a button.
///
/// The only phase in which writing a result does anything: `v3` dispatches from phase 1 and
/// returns immediately in every other phase.
pub const FE_DIALOG_PHASE_WAITING: u8 = 1;

/// [`FE_DIALOG_RESULT_OFFSET`] before anything has been decided.
pub const FE_DIALOG_RESULT_NONE: u8 = 0;

/// The back/cancel answer, and the ONLY answer a one-button box can produce.
pub const FE_DIALOG_RESULT_CANCEL: u8 = 1;

/// The confirm answer, produced when a press lands on a box with real options.
pub const FE_DIALOG_RESULT_CONFIRM: u8 = 2;

/// [`FE_DIALOG_PHASE_OFFSET`] once the box has closed on a [`FE_DIALOG_RESULT_CANCEL`].
///
/// The update computes the closed phase as `sete al` on `result == 2` then `add al, 3`
/// (`0x14010518b`), so a cancel closes to `3` and a confirm to `4`. A one-button box can only ever
/// produce a cancel, which makes `3` **the only terminal phase reachable for one** -- writing it
/// is reproducing the single outcome the game has, not choosing between two.
pub const FE_DIALOG_PHASE_CLOSED_CANCEL: u8 = 3;

/// Vtable slot the dispatch calls for [`FE_DIALOG_RESULT_CANCEL`]. `call [rax+0x40]` at
/// `0x140105212`, and `0x40 / 8 == 8`.
pub const FE_DIALOG_SLOT_ON_CANCEL: usize = 8;

/// Vtable slot the dispatch calls for [`FE_DIALOG_RESULT_CONFIRM`]. `call [rax+0x48]` at
/// `0x140105217`.
pub const FE_DIALOG_SLOT_ON_CONFIRM: usize = 9;

/// `FeSubStateCommonWindowBase`'s own slot-8 handler: `ret 0`, and nothing else. RVA `0x000f89d0`.
///
/// Recorded so a dialog's handlers can be checked to be inert **at runtime**, from its own vtable,
/// before this mod answers it. That check is the difference between "the allowlist below is
/// believed to be safe" and "this build's bytes say answering does nothing but close the box".
pub const FE_DIALOG_INERT_ON_CANCEL: u32 = 0x000f_89d0;

/// `FeSubStateCommonWindowBase`'s own slot-9 handler: `ret 0`. RVA `0x000f89c0`.
pub const FE_DIALOG_INERT_ON_CONFIRM: u32 = 0x000f_89c0;

/// `FeSubStateTitleOnlineCheckFailWarn`'s vtable. RVA `0x010bd7d8`, VA `0x1410bd7d8`.
///
/// The network-check failure box. Its `v1` and its slot-11 message getter both live in
/// `..\..\Source\Frontend\Operator\Title\FeSubStateServerFailWarn.cpp` -- the source path is still
/// in the image at `0x1410bd8b0` -- and its enter formats that path together with error code
/// `0x35b62` into the message it shows. Both its slot-8 and slot-9 handlers are the inert `ret 0`
/// above, so answering it closes it and does nothing else.
pub const FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN: u32 = 0x010b_d7d8;

/// `FeSubStateTitleInformationFailWarn`'s vtable. RVA `0x010bd848`, VA `0x1410bd848`.
///
/// The sibling of [`FE_DIALOG_VTABLE_ONLINE_CHECK_FAIL_WARN`] from the same source file, formatting
/// error code `0x33453`. Its handlers are inert in exactly the same way.
pub const FE_DIALOG_VTABLE_INFORMATION_FAIL_WARN: u32 = 0x010b_d848;

/// `FeSubStateOfflineModeWindow`'s vtable. RVA `0x010bd388`, VA `0x1410bd388`.
///
/// The "playing offline" notice. Uses the shared `v1`, and overrides only the slot-11 message
/// getter (`0x1400f9800`) to pick text `0x67` or `0x65` from category `0x19` depending on the flag
/// at `[0x14160de10]+0x56b`. Handlers inert.
pub const FE_DIALOG_VTABLE_OFFLINE_MODE_WINDOW: u32 = 0x010b_d388;

/// `FeSubStateCommonWindow`'s vtable. RVA `0x010bcff8`, VA `0x1410bcff8`.
///
/// **The dialog that actually appears at boot**, measured: a first run with only the three
/// `FailWarn`/`OfflineMode` vtables allowlisted logged
/// `seen screen=<not-allowlisted> vtable=0x00000001410bcff8` and nothing else, while the three
/// that *were* allowlisted never fired. The generic-sounding name is why it was left out at
/// first, and reading it is what put it back in.
///
/// # Why answering the "generic" window is nonetheless safe here
///
/// Three facts, each read from the image rather than assumed:
///
/// 1. **There is exactly one of these objects in the game.** `scripts/ds2-xrefs.py 0x1410bcff8`
///    finds a single code reference in the whole image, the `lea r13` at `0x1400f75c1`, and it is
///    inside `FeStateTitle::v6` (`0x1400f72e0`) -- the routine that builds the title's substate
///    table, an 88-slot array at `[state+8]` with its count at `[state+0x2c8]`. So this class is
///    not "the generic box used all over the game"; it is one member of the title flow, and no
///    in-game prompt can be an instance of it.
/// 2. **It is a one-button acknowledgement box, by construction.** Its constructor
///    `0x140104c00` executes `or eax,0xffffffff` then `mov WORD PTR [rcx+0x12],ax`, so
///    [`FE_DIALOG_CONFIRM_DEST_OFFSET`] is hardcoded to `-1`. The update's input path can therefore
///    only ever produce [`FE_DIALOG_RESULT_CANCEL`] for it -- there is no second answer to get
///    wrong, and no choice being made on the player's behalf.
/// 3. **Its handlers are inert**, like the others: slots 8 and 9 are the base `ret 0` stubs.
///
/// Its message comes from category `0x19` id `0x1adc0` (`0x1400f7590`), its caption id is `0x20`
/// and its kind field at `+0x0c` is `6`.
pub const FE_DIALOG_VTABLE_COMMON_WINDOW: u32 = 0x010b_cff8;

/// Kind field within a common-window substate. `+0x0c`, set from the constructor's second
/// argument. Logged as a diagnostic; nothing branches on it here.
pub const FE_DIALOG_KIND_OFFSET: usize = 0x0c;

/// **Destination substate id for the CANCEL edge.** `+0x10`, a signed WORD.
///
/// This was recorded as a "caption/message id" and that was wrong. It is a substate id, and `v5`
/// (`0x140104f30`) publishes it as the destination of a transition:
///
/// ```text
/// movsx edx, WORD PTR [rdi+0x10]     ; <- this field
/// mov   [rax+0x18], &this[0x30]      ; watch the PHASE
/// mov   [rax+0x08], edx              ; destination substate
/// mov   BYTE PTR [rax+0x20], 3       ; ...when the phase is 3, the cancel-closed phase
/// ```
///
/// The constructor at `0x140104c00` sets it from its third argument (`mov WORD PTR [rcx+0x10],
/// r8w`). The old reading came from the boot notices, where that argument is `0x20` -- which looks
/// exactly like a caption id and is in fact substate `0x20`, `FeSubStateTitleSteamNetworkCheck`.
///
/// Confirmed against 22 live instances read out of `/proc/<pid>/mem` on a running game: every one
/// holds a plausible substate id, several of them `0x2a`
/// ([`FE_SUBSTATE_ID_OFFLINE_MODE_WINDOW`]), and the one that had fired held `0x39`
/// (`FeSubStateTitleGameServerLogin`).
pub const FE_DIALOG_CANCEL_DEST_OFFSET: usize = 0x10;

/// Elapsed-time accumulator within a common-window substate. `+0x18`, a float.
///
/// `enter` zeroes it (`mov DWORD PTR [rdi+0x18],0` at `0x140104e85`) and the update accumulates the
/// frame delta into it (`addss xmm1,[rcx+0x18]`). Only read while the box is open.
pub const FE_DIALOG_ELAPSED_OFFSET: usize = 0x18;

/// Auto-close timeout within a common-window substate. `+0x14`, a float, `0` meaning none.
///
/// Read by the update at `0x1401051e5`. A dialog with a positive value here closes itself with no
/// press at all, which is what makes "close without a press" a shipped behaviour rather than one
/// this mod invented. Logged as a diagnostic.
pub const FE_DIALOG_TIMEOUT_OFFSET: usize = 0x14;

/// **NOT in the allowlist, and recorded to say why.** `FeSubStateTitleDeleteProfile`'s vtable,
/// RVA `0x010bd6c8`.
///
/// It shares [`FE_DIALOG_UPDATE`] with the three above, so a mod that answered "every common
/// window" would answer this one too. It **overrides slot 8** with a real body at `0x1400fcf30`,
/// which reaches into the save-data object at `[[0x1416148f0]+0xa8]+0xd8`. Whatever that does, it
/// is not nothing, and it is reached only when a player deliberately chooses to delete a profile.
///
/// The runtime handler check against [`FE_DIALOG_INERT_ON_CANCEL`] rejects it on its own, without
/// consulting this constant -- which is the point. This entry exists so the exclusion is written
/// down rather than merely emergent.
pub const FE_DIALOG_VTABLE_DELETE_PROFILE_DO_NOT_ANSWER: u32 = 0x010b_d6c8;

// ============================================================================================
// THE TWO REMAINING STOPS BETWEEN BOOT AND THE MENU (`ds2-mods-rs-j3b`)
//
// Suppressing the notice boxes still does not hand the player a menu. Two things are left, and
// they are different in kind from each other and from the notices:
//
//   * a PRESS ANY BUTTON gate, which waits on input and is the only one of the three that a
//     player must actually act on;
//   * PROCESS WINDOWS -- "please wait" boxes that resolve on their own, but not before a minimum
//     display time has elapsed. These wrap real asynchronous work and MUST NOT be skipped; only
//     the artificial minimum is worth removing.
// ============================================================================================

/// The press-any-button poll. RVA `0x000ff420`, VA `0x1400ff420`.
///
/// **It has exactly one caller in the entire image**: `0x1400fee6b`, inside
/// `FeSubStateTitleMain::v3` (`0x1400fed90`), which is the "PRESS ANY BUTTON" screen's update.
/// Counted by scanning every `e8 rel32` in the image for this target and attributing each hit to
/// its `.pdata` owner. That is what makes detouring it narrow: it is a private helper of one gate,
/// not shared input plumbing, so forcing its result cannot reach anything else in the game.
///
/// It ignores its argument entirely and reads globals -- the singleton at `0x1416751f8`, whose
/// `+0x60` object it passes to `0x140af3f30`, then bit 16 and bit 4 of the returned `+0x10` word,
/// falling back to `[[+0x60]+8]+0x34 & 1`. That is the same "a button was pressed" state word
/// `FeSubStateTitleLogo`'s skip path tests; see `docs/DS2-TITLE-FLOW.md`.
///
/// # Why forcing it true is the right cut
///
/// `FeSubStateTitleMain::v3`'s phase-1 branch runs three things in order: it ticks the title scene,
/// waits for [`FE_TITLE_MAIN_SEQUENCE_GATE`] to report the title sequence is up, and only then
/// consults this poll. Forcing this one true leaves the sequence gate intact -- the title screen
/// still initialises normally -- and then runs the **whole** of the game's own phase-1 body, which
/// is what prepares the top menu. Forcing the substate's terminal phase instead would skip that
/// setup, which is why this is hooked and the phase is not.
///
/// Not Arxan-redirected. `.pdata` gives it RVA `0x0ff420`-`0x0ff465`; its first two instructions
/// are `48 83 ec 28` and `48 8b 49 08`, eight relocatable bytes with no branch targeting them.
pub const FE_TITLE_MAIN_PRESS_ANY_BUTTON: u32 = 0x000f_f420;

/// The gate that must still pass before the press poll is consulted. RVA `0x000f37f0`.
///
/// Returns true when the title scene's currently-playing sequence is `0x67`, via
/// `0x140afdb30(scene, 0x67)` -- which compares the active sequence id against its argument.
/// Recorded so it is clear that [`FE_TITLE_MAIN_PRESS_ANY_BUTTON`] is NOT the only condition on
/// that branch, and that this mod deliberately leaves the other one alone.
pub const FE_TITLE_MAIN_SEQUENCE_GATE: u32 = 0x000f_37f0;

/// `FeSubStateProcessWindowBase::v1` (enter). RVA `0x00104ed0`, VA `0x140104ed0`.
///
/// The "please wait" window shared by six classes -- `FeSubStateProcessWindowBase`,
/// `FeSubStateProcessWindowSimple`, `FeSubStateTitleOnlineCheck`,
/// `FeSubStateTitleGameServerLogin`, `FeSubStateTitleSaveSystemData` and
/// `FeSubStateTitleLoadProfile` -- found by scanning every RTTI vtable for this value in slot 1.
///
/// ```text
/// result = this->vtable[8]();          // STARTS THE ASYNCHRONOUS WORK
/// if (result >= 0) { this->phase = 3; return; }   // nothing to do; no window at all
/// show_process_window(ui, this->caption, 0, 1);
/// this->timer = 0;
/// this->phase = 1;
/// ```
///
/// **THIS ONE MUST NOT BE SUPPRESSED.** Slot 8 starts real work -- a network check, a server
/// login, a system-data save, a profile load -- and the update then waits on slot 10 for it to
/// finish. Skipping the substate would skip the wait, not just the window. That is the whole
/// reason this is treated differently from [`FE_DIALOG_ENTER`], where nothing was pending.
///
/// Not Arxan-redirected; `.pdata` RVA `0x104ed0`-`0x104f24`, prologue `40 53 48 83 ec 20`.
pub const FE_PROCESS_WINDOW_ENTER: u32 = 0x0010_4ed0;

/// Minimum display duration within a process-window substate. `+0x10`, a float, in seconds.
///
/// `FeSubStateProcessWindowBase::v3` (`0x140105270`) phase 1 reads it as
/// `addss xmm1,[rcx+0x14]` / `comiss xmm1,[rcx+0x10]` / `jb` -- so while the timer is BELOW this
/// value the window stays up no matter what, and only once it is reached does the update consult
/// slot 10 to ask whether the work is actually done. Set by the constructor at `0x140104c77`
/// (`movss [rcx+0x10],xmm3`) from its third argument.
///
/// **Zeroing it removes the artificial floor and nothing else.** The slot-10 wait is untouched, so
/// the window still stays up for exactly as long as the operation really takes -- it can no longer
/// linger after the work is finished, and it cannot outrun it either. That is why this is the cut
/// rather than anything that touches the phase.
pub const FE_PROCESS_WINDOW_MIN_DURATION_OFFSET: usize = 0x10;

/// Elapsed timer within a process-window substate. `+0x14`, a float, zeroed by `enter`.
pub const FE_PROCESS_WINDOW_TIMER_OFFSET: usize = 0x14;

/// Phase field within a process-window substate. `+0x20`, a **DWORD** -- not the byte the
/// common-window substates use at `+0x30`. Different base class, different layout.
///
/// `1` while the window is up, `2` while it closes, `3` once finished. `enter` sets it to `3`
/// directly when slot 8 reports there was nothing to do, which is the game's own "no window"
/// path.
pub const FE_PROCESS_WINDOW_PHASE_OFFSET: usize = 0x20;

/// [`FE_PROCESS_WINDOW_PHASE_OFFSET`] while the window is up and the work is outstanding. The
/// only phase in which the minimum duration is read, and therefore the only one in which zeroing
/// it does anything.
pub const FE_PROCESS_WINDOW_PHASE_SHOWING: i32 = 1;

/// Kind field within a process-window substate. `+0x0c`, set by the constructor at `0x140104c87`.
/// Logged as a diagnostic so the boot windows can be told apart in a log.
pub const FE_PROCESS_WINDOW_KIND_OFFSET: usize = 0x0c;

// --------------------------------------------------------------------------------------------
// Suppressing the process window outright, and the title screen's activation animation.
//
// Both go one step further than the two constants above, and both are recorded separately from
// them so either can be switched off on its own.
// --------------------------------------------------------------------------------------------

/// Vtable slot a process window calls from `enter` to START its asynchronous work.
/// `call [rax+0x40]` at `0x140104edc`, and `0x40 / 8 == 8`.
///
/// **This call is the reason a process window cannot simply be suppressed.** It returns a status
/// into [`FE_PROCESS_WINDOW_RESULT_OFFSET`]; a negative value means work is outstanding and the
/// window is shown, a non-negative one means there was nothing to do and the substate goes
/// straight to [`FE_PROCESS_WINDOW_PHASE_DONE`] with no window at all. Any code that hides the
/// window must still make this call and still honour that branch.
pub const FE_PROCESS_WINDOW_SLOT_BEGIN: usize = 8;

/// Where a process window stores the status returned by [`FE_PROCESS_WINDOW_SLOT_BEGIN`]. `+0x24`.
/// Written by `enter` at `0x140104edf` and again by the update at `0x1401052b2`.
pub const FE_PROCESS_WINDOW_RESULT_OFFSET: usize = 0x24;

/// [`FE_PROCESS_WINDOW_PHASE_OFFSET`] once the substate is finished. `3`.
///
/// `enter` writes it directly at `0x140104f17` when there was no work to do -- the game's own
/// no-window path, and the precedent for reaching this phase without ever showing anything.
pub const FE_PROCESS_WINDOW_PHASE_DONE: i32 = 3;

/// `FeSubStateTitleMain::v3` (update). RVA `0x000fed90`, VA `0x1400fed90`.
///
/// The PRESS ANY BUTTON screen's per-frame logic, switching on a phase at
/// [`FE_TITLE_MAIN_PHASE_OFFSET`]:
///
/// | phase | what it does |
/// | --- | --- |
/// | 1 | ticks the scene, waits for [`FE_TITLE_MAIN_SEQUENCE_GATE`], then for a press. On a press it runs the whole top-menu setup and leaves phase 2 (or 3). With no press for long enough it goes to phase **5** -- the attract-mode prologue movie. |
/// | 2 | pure wait on a sequence handle at `+0x38`; sets phase 3 and does nothing else |
/// | 3 | waits for the same sequence to finish, calls `0x140afe8a0` on it, sets phase 4 |
/// | 4 | terminal |
///
/// **Phases 2 and 3 are the activation animation** -- the flourish a player sees after pressing.
/// Phase 1's body is the part that matters, and it has already run by the time either is reached,
/// which is what makes forcing phase 4 from 2 or 3 a skip of the animation rather than of the
/// setup. Compare `ds2-intro-skip`, which forces a terminal phase from `enter`; that is not
/// available here, because from `enter` the setup has not happened yet.
///
/// Not Arxan-redirected. Prologue `48 89 5c 24 18` -- five bytes exactly.
pub const FE_TITLE_MAIN_UPDATE: u32 = 0x000f_ed90;

/// Phase field within `FeSubStateTitleMain`. `+0x10`, a DWORD.
///
/// Read as `mov ecx,[rcx+0x10]` at `0x1400fedb2` before its `dec`/`je` chain. Same offset as the
/// boot screens' phase, and a different one from either window family -- which is why each class's
/// offset in this file is derived from that class's own code and never shared by analogy.
pub const FE_TITLE_MAIN_PHASE_OFFSET: usize = 0x10;

/// The first activation-animation phase. Written by phase 1's tail at `0x1400feee8`.
pub const FE_TITLE_MAIN_PHASE_ANIMATING: i32 = 2;

/// The second activation-animation phase. Phase 1's tail can jump straight here at `0x1400feefa`
/// when the object it just built reports `[+8] == 0`, so BOTH values mean "the setup is done and
/// only the flourish is left".
pub const FE_TITLE_MAIN_PHASE_ANIMATING_LATE: i32 = 3;

/// Terminal phase for `FeSubStateTitleMain`. Written by phase 3 at `0x1400fedf7`.
pub const FE_TITLE_MAIN_PHASE_DONE: i32 = 4;

/// `show_process_window`. RVA `0x004fe760`, VA `0x1404fe760`.
///
/// **The single function that draws a "please wait" box**, and the only place all of them meet.
/// Hooking it is what makes hiding them general, and the alternative was shown to be a losing
/// game: the seven call sites are spread across different vtable slots of different classes, and
/// `FeSubStateTitleInformation` -- the "Retrieving Information" box -- shows its window from its
/// `update` (`v3`, via the continuation chunk at `0x1400ff98e`) rather than from `enter`, so no
/// amount of hooking `enter` reaches it.
///
/// # Its signature, established rather than assumed
///
/// Four register arguments and **no stack arguments**: the body reads nothing above its own frame.
/// It keeps RCX and forwards RDX, R8 and R9 untouched into `0x1405105f0`, which is why a detour
/// must carry all four even though the function appears to use only the first. All seven call
/// sites were checked and set exactly these four -- six do `mov r9b,1; xor r8d,r8d`, and
/// `0x1401088ae` does the mirror `xor r9d,r9d; mov r8b,1`. None writes a fifth at `[rsp+0x20]`.
/// Forwarding them as raw 64-bit registers reproduces even the upper bits the callers leave
/// undefined.
///
/// # Returning zero is the function's own no-op answer
///
/// It opens with `mov rcx,[rbx+0xf0]; test rcx,rcx; jne`, and the not-taken path is
/// `xor eax,eax; ret` -- "there is no window manager, so nothing was shown". **No caller uses the
/// return value**; all seven ignore EAX and immediately write their own phase field. So a detour
/// that returns 0 without drawing is indistinguishable from the shipped path where there was
/// nothing to draw on.
pub const FE_SHOW_PROCESS_WINDOW: u32 = 0x004f_e760;

/// The byte that is nonzero while `FeOperatorTitle` is running. RVA `0x01614804`, VA
/// `0x141614804`.
///
/// Written `1` by `FeOperatorTitle::v2` at `0x1400ef045` and `0` by `FeOperatorTitle::v3` at
/// `0x1400ef123`, which are the operator's setup and teardown. The game reads it itself at
/// `0x140342251` (`cmp BYTE PTR [rip+...],0`), so it is a real state flag rather than a
/// write-only leftover -- `scripts/ds2-xrefs.py` finds no other genuine reference.
///
/// **This is what scopes the process-window hiding to the title flow.** Hiding every process
/// window in the game would take the "Saving..." indicator with it, which is exactly the kind of
/// thing a player is entitled to see. Gating on the game's own "am I in the title flow" flag keeps
/// the change to the boot sequence and leaves gameplay alone, without this mod having to invent a
/// notion of "still booting" or time-box one.
pub const FE_OPERATOR_TITLE_ACTIVE: u32 = 0x0161_4804;

/// Sequence handle within `FeSubStateTitleMain`. `+0x38`.
///
/// Set up by phase 1's press-taken body (`0x1400feec8`, `lea rcx,[rbx+0x38]; call 0x14005a8e0`),
/// which starts the title-text sequence, and then waited on by phases 2 and 3.
pub const FE_TITLE_MAIN_SEQUENCE_HANDLE_OFFSET: usize = 0x38;

/// **NOT a "finish sequence" call, despite how its call sites read.** RVA `0x00afe8a0`.
///
/// `FeSubStateTitleMain::v3` phase 1 calls it on the substate's `+0x18` handle the moment a press
/// is taken, and phase 3 calls it on `+0x38` immediately before writing the terminal phase. Both
/// placements make it look like "stop the animation, we are done" -- and that reading was wrong.
///
/// Its body tail-calls `0x1409d5610`, which compares `[handle]` against the global at `0x14166df98`
/// and, when they differ, builds a record tagged `0x4d4f4d53` ("SMOM") and hands it to
/// `0x1409ebea0`. That is handle validation or telemetry, not playback control.
///
/// Recorded as an exclusion rather than deleted because the mistake is re-derivable: anyone reading
/// phases 1 and 3 will reach the same wrong conclusion from the call sites alone. It was caught by
/// a live run where it returned success and the title text animated in exactly as before.
pub const FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE: u32 = 0x00af_e8a0;

// --------------------------------------------------------------------------------------------
// NAMING THE UI ANIMATION PLAYER (`ds2-mods-rs-j3b`, still open)
//
// The title text animates in and the lever for it is not in the title substate. Both routes into
// the sequence system are thin forwarders that end in a virtual call, and no `Fe*Sequence` class
// exists among the 5269 RTTI names to identify the callee from:
//
//   0x140afdb80(scene, id, ..)  ->  rcx = [scene+0x28]      ; jmp 0x140b50860
//   0x140b50860                 ->  rcx = [rcx+0x30]        ; jmp [[rcx] + 0xc0]
//
// So the class is resolved the other way round: read the live vptr at the end of that chain and
// match it against the RTTI vtable map. That is a measurement, and it replaces the inference that
// produced FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE above.
// --------------------------------------------------------------------------------------------

/// The global holding the title-flow object table. RVA `0x0160de10`, VA `0x14160de10`.
///
/// Already load-bearing elsewhere in the boot flow: `FeSubStateTitleLogo`'s skip path writes its
/// `+0x568`, `FeSubStateTitleInitBranch` writes its `+0x564`, and `FeSubStateOfflineModeWindow`
/// reads its `+0x56b`.
pub const FE_TITLE_GLOBALS: u32 = 0x0160_de10;

/// Offset of the title scene within [`FE_TITLE_GLOBALS`]. `+0x80`.
///
/// `FeSubStateTitleMain::v3` loads it at `0x1400fedc9` (`mov rcx,[rax+0x80]`) and ticks it through
/// a virtual before consulting either of its gates.
pub const FE_TITLE_SCENE_OFFSET: usize = 0x80;

/// First hop of the sequence-player chain: `[scene + 0x28]`, read by `0x140afdb80`.
pub const FE_SEQUENCE_PLAYER_HOP1: usize = 0x28;

/// Second hop: `[hop1 + 0x30]`, read by `0x140b50860`, whose vtable is then dispatched through.
pub const FE_SEQUENCE_PLAYER_HOP2: usize = 0x30;

/// The vtable slot `0x140b50860` dispatches to: `jmp [rax+0xc0]`, and `0xc0 / 8 == 24`.
pub const FE_SEQUENCE_PLAYER_PLAY_SLOT: usize = 24;

/// Put `FeSceneTitle` into its settled state by playing sequence `0x67`. RVA `0x000f3820`.
///
/// `FeSubStateTitleMain::v1` calls `0x1400f3e30` (`0x1400fda54`), which plays sequence **`0x66`**
/// on `[scene+8]` -- the "DARK SOULS II SCHOLAR OF THE FIRST SIN" text animating in -- and nothing
/// in the phase machine stops it. `0x1400f3820` plays **`0x67`** on the same object, the settled
/// state, and it is exactly the sequence [`FE_TITLE_MAIN_SEQUENCE_GATE`] waits to observe before a
/// press is accepted:
///
/// ```text
/// if ([scene+0xf1] != 0) return;
/// rcx = [scene+8];
/// if (!rcx) return;
/// [rcx+0x18]--;
/// play(rcx, 0x67, 0, 0.0f);
/// ```
///
/// # Why this rather than forcing the gate alone
///
/// Forcing [`FE_TITLE_MAIN_SEQUENCE_GATE`] makes the gate report a state the scene is not in: the
/// press is taken early while `0x66` keeps animating underneath. Playing `0x67` puts the scene in
/// the state the gate is waiting for, so the flow reaches an interactive menu **as soon as the data
/// is available rather than pacing itself to an animation** -- which is the behaviour this is kept
/// for, confirmed in-game.
///
/// The four sequence ids used across the Fe scenes are `0x65`, `0x66`, `0x67` and `0x68`, read from
/// the 91 call sites of the play forwarder `0x140afdb80`, with `0x66`/`0x68` the in and out
/// transitions and `0x67` the settled state -- corroborated by `FeSubStateTitleLogo` using the same
/// set.
///
/// **Open:** the title text is still seen animating. Whether that is `0x66` continuing in parallel,
/// `0x67` carrying its own entry animation, or a different object entirely is unresolved; see
/// `docs/DS2-TITLE-FLOW.md`. That is a question about the remaining animation, NOT a reason to drop
/// this call, whose effect on when the menu becomes usable is real.
pub const FE_SCENE_TITLE_PLAY_IDLE: u32 = 0x000f_3820;

// --------------------------------------------------------------------------------------------
// The title top menu. `docs/DS2-TITLE-FLOW.md` carries the trace these came from.
//
// The menu is a fixed vector of six rows, rebuilt from scratch on demand. Nothing is ever
// inserted or removed -- `0x1400f4250` appends the same six descriptors on every path, and the
// only per-row variable is one byte. That byte decides two independent things, and separating
// them is what the constants below exist for.
// --------------------------------------------------------------------------------------------

/// `FeGroupTitleTopMenu`'s enable-and-style pass. RVA `0x000f5000`, VA `0x1400f5000`.
///
/// Called from `FeGroupTitleTopMenu::v25` (`0x1400f4df0`) with the group and the freshly built
/// descriptor list. Read from the disassembly rather than the decompiler, which drops the third
/// argument on one of the two branches:
///
/// ```text
/// for i in 0 .. list[+0x158]:
///     cell = FE_TOP_MENU_CELL_FOR_INDEX(group, i)      # null is skipped
///     desc = list + align + i*FE_TOP_MENU_ROW_STRIDE
///     tmp  = 0x140026790(group+0x100, &scratch, desc)  # RCX/RDX/R8 set BEFORE the branch
///     if desc[+0x34] != 0:
///         tmp[+0x40]->vtable[0](tmp+0x40, 0x67, 0, 0.0)
///         cell[+8] = 3 + (i == group[+0x28])
///     else:
///         cell[+8] = 2
///         tmp[+0x40]->vtable[0](tmp+0x40, 0x7a, 0, 0.0)
/// ```
///
/// **The two effects of "disabled" are separate writes.** `cell[+8] = 2` is what removes the row
/// from cursor navigation; the sequence swap is the entire visual difference. Nothing in the image
/// reads `cell[+8] == 2` to decide how to draw -- the only readers compare against 3 and 4 -- so
/// the appearance of an unavailable row is decided solely by sequence `0x7a`.
///
/// Not Arxan-redirected: `48 8b c4 48 89 58 18` is an ordinary MSVC prologue, seven bytes before
/// the first instruction boundary past five.
pub const FE_TOP_MENU_APPLY_STATES: u32 = 0x000f_5000;

/// `FeSubStateTitleTopMenu::v3` (update). RVA `0x000ff300`, VA `0x1400ff300`.
///
/// Runs every frame while the top menu is the active substate: it ticks the scene through a
/// virtual, copies `[scene+0xe8]` into its own phase, and clears a save-data flag on phase 4. It
/// is the only per-frame function that is specific to this menu, which is what makes it the place
/// to re-assert per-row state without reaching groups this mod has no business touching.
///
/// Takes the frame delta in XMM1 like the rest of the family, even though this member never reads
/// it. A detour that declares only `this` is free to clobber XMM1.
///
/// Not Arxan-redirected; prologue `48 89 5c 24 08`, five bytes exactly.
pub const FE_TOP_MENU_UPDATE: u32 = 0x000f_f300;

/// `FexGroupList<FeGroupGrid>`'s cell-by-index lookup. RVA `0x00108060`, VA `0x140108060`.
///
/// `longlong lookup(group, int index)` -- walks the group's cell list through two of its own
/// virtuals and returns the cell whose `[+0x10]` equals `index`, or null. Called, never patched,
/// so its Arxan status does not arise.
pub const FE_TOP_MENU_CELL_FOR_INDEX: u32 = 0x0010_8060;

/// Offset of `FeGroupTitleTopMenu` within `FeSceneTitle`. `+0xb8`.
///
/// Written by the scene's own builder at `0x1400f4950`: `*(longlong **)(param_1 + 0xb8) = plVar3`,
/// immediately after constructing the group with `0x1400f3250`.
pub const FE_TOP_MENU_GROUP_OFFSET: usize = 0xb8;

/// Stride of one row descriptor in the top-menu list. `0x38`.
pub const FE_TOP_MENU_ROW_STRIDE: usize = 0x38;

/// The row's action id, within a descriptor. `+0x30`, a DWORD, values 1 to 6.
///
/// Read by the activate handler at `0x1400f4a8d` as `[buffer + align + cursor*0x38]`.
pub const FE_TOP_MENU_ROW_ACTION_OFFSET: usize = 0x30;

/// The row's enabled flag, within a descriptor. `+0x34`, one byte.
///
/// Tested at `0x1400f5063`. Its six values, in row order, are the whole of the menu's variability:
/// `1`, has-a-save, online-available, `!online-available`, `1`, `1`.
pub const FE_TOP_MENU_ROW_ENABLED_OFFSET: usize = 0x34;

/// Row count within the descriptor list. `+0x158`.
///
/// Written as a QWORD by the builder and read as a DWORD by the styling pass, which is why it is
/// read as a 32-bit value here.
pub const FE_TOP_MENU_LIST_COUNT_OFFSET: usize = 0x158;

/// Capacity of the descriptor list. `6`, and also the count on every path.
///
/// The builder's own `DLFixedVector` bound: appending a seventh row calls
/// `DLKR::DLBackAllocator::panic` with `"out of memory."`. Used as a sanity bound on a count read
/// out of game memory, so a garbage read cannot drive a loop.
pub const FE_TOP_MENU_ROW_CAPACITY: usize = 6;

/// The cell's state field. `+0x8`, a DWORD, on `FeObjectButtonEx`.
pub const FE_BUTTON_STATE_OFFSET: usize = 0x8;

/// Cell state meaning "unavailable". `2`.
///
/// **This is what removes a row from cursor navigation.** `FeObjectButtonEx::v16` at
/// `0x14004c5c0` is the predicate the navigation search at `0x140107b40` calls on every candidate
/// before accepting it, on all six of its direction branches and again at the shared accept point
/// `0x140107fb0`:
///
/// ```text
/// vtable[3]() == 1 && [rcx+8] == 3
/// ```
///
/// So only state 3 is selectable. The activate handler `0x1400f4a60` does no enable check of its
/// own, which is exactly why this one has to hold.
pub const FE_BUTTON_STATE_UNAVAILABLE: i32 = 2;

/// Cell state meaning "available, not under the cursor". `3`.
pub const FE_BUTTON_STATE_NORMAL: i32 = 3;

/// Cell state meaning "under the cursor". `4`. `FeObjectButtonEx::v1` (`0x14004c5b0`) is
/// `cmp [rcx+8],4; sete al; ret`.
pub const FE_BUTTON_STATE_CURSOR: i32 = 4;

/// Which top-menu rows may be forced visible when the game would hide them. **Row 1 only.**
///
/// The six enable expressions are `1`, has-a-save, online, `!online`, `1`, `1`. Rows 0, 4 and 5 are
/// literal `1` and can never be hidden, so forcing them is meaningless. That leaves three rows the
/// game can delete, and exactly one of them is worth keeping on screen:
///
/// * **row 1, LOAD GAME** -- deleted until a save exists, and it owns its screen slot outright.
///
/// **Rows 2 and 3 -- INFORMATION and GO ONLINE -- are deliberately out, and the reason is not that
/// they look wrong when forced. It is that the pair needs no help.** Their enable bytes are
/// `online` and `!online`, computed from one another at `0x1400f4344`, so their XOR is true at
/// every instant: one of the two is always live, and the slot they share is never empty on its own.
/// The game swaps which one occupies it when `FeSubStateTitleOnlineCheck` sets the flag. Forcing
/// either is not "showing a row the game hides", it is putting a second label in an occupied slot,
/// which is exactly what the all-six run drew on top of itself. The grid coordinates are not the
/// reason -- a run logged them as `0:(0,0) 1:(0,1) 2:(0,2) 3:(0,3) 4:(0,4) 5:(0,5)`, six distinct
/// positions -- so the collision is in the layout, which places the mutually-exclusive pair
/// together.
///
/// A forced row 2 also draws WRONG, and the mechanism is worth recording because it governs any
/// future attempt to restyle a row. A sequence play is vtable slot `0xc0`, and it is not uniform:
/// `FeComponentObject` (`0x1411ddfa8`, `0x140b6a980`) forwards to every child, `FeComponentSprite`
/// (`0x1411de318`, `0x140b6c4f0`) is the only leaf class that acts on it, and `FeComponentTextField`
/// (`0x1411de698`), `FeComponentTextureShape`, `FeComponentMaskShape`, `FeComponentTextureMask` and
/// `FeComponentLinked` all inherit `FeComponentBase`'s, which is `return;` (`0x140b6a970`). **A text
/// field never responds to a sequence play at all** -- it follows an ancestor sprite. Inside a
/// sprite the id is looked up in that sprite's own table (resource at `+0x48`, table at `+0x18` of
/// that, entries at `+0x00`, `u16` count at `+0x08`, `0x10`-byte entries of `{i32 id, u16 start}`)
/// and **a miss falls through to `RET`** -- a silent no-op leaving that sprite where `0x7a` parked
/// it. A row holds more than one sprite, so it can half-move: the plate reaches the faded frame and
/// the caption's sprite never moves. That is the empty plate, and it is a per-SPRITE fact, not a
/// per-row one.
///
/// Which sprites carry a `0x6c` entry is layout data in `GameDataEbl.bdt` and is NOT established.
/// The measurement that looks like it settles this does not: `FeComponentObject`'s bracket getters
/// at slots `0x58`/`0x60` (`0x140b6a4f0`, `0x140b6a490`) tail-jump to the FIRST child only and do
/// not aggregate, so a `0x6c start=89 end=103` read off a row element describes the first sprite on
/// its leftmost spine and nothing else in the tree.
///
/// No game code writes any of these captions: each row's element id occurs exactly ONCE in the
/// whole image, in the row builder (row 2's `17010022` at `0x1400f4462`). The content is authored
/// in the layout and reached by the path the builder assembles.
///
/// **Measured, and the first version got this wrong.** Forcing all six drew two rows on top of each
/// other on screen. The grid coordinates are not the reason -- a run logged them as
/// `0:(0,0) 1:(0,1) 2:(0,2) 3:(0,3) 4:(0,4) 5:(0,5)`, six distinct positions -- so the collision is
/// in the layout resource, which evidently has no designed place for rows the game never shows
/// together.
pub const FE_TOP_MENU_FORCE_SHOWN_ROWS: u32 = 1 << 1;

/// `FrontendEx::SceneObjProxy`'s vtable slot 0: the proxy's own path-to-element resolver.
///
/// **This is the only verified way to get a row's layout element, and guessing cost this project a
/// round of false conclusions.** A probe read `cell+0x40`, fell back to `cell+0x30`, and called the
/// result "the row's element". It is not: all six rows came back with the same pointer, the same
/// sequence table and the same owner, which is one shared object read six times -- and a tree walk
/// from it ran off into unrelated memory past depth 5.
///
/// The real route is the one the game uses. `ComponentPositionProperty::get` (`0x14001e4d0`) does
/// `RCX=[this+8]` (the proxy back-pointer), `CALL [[RCX]]`, then `TEST RAX,RAX` -- so slot 0 takes
/// the proxy and returns the element or null. Inside, `0x140027ce0` matches the path copied to
/// `+0x60` by the binder against the scene at `+0x58` through `0x140afdad0`.
pub const FE_SCENE_OBJ_PROXY_ELEMENT_SLOT: usize = 0;

/// `FeComponentSprite`'s vtable. RVA `0x011de318`, VA `0x1411de318`. From MSVC RTTI.
///
/// The only leaf component class that acts on a sequence play. `FeComponentObject` (`0x011ddfa8`)
/// forwards to every child; `FeComponentBase`'s slot `0xc0` (`0x140b6a970`) is `return;` and
/// `FeComponentTextField`, `FeComponentTextureShape`, `FeComponentMaskShape`,
/// `FeComponentTextureMask` and `FeComponentLinked` all inherit that. So a text field never
/// responds to a play and follows an ancestor sprite instead.
///
/// **"Leaf" is right about sequence plays and wrong about the tree, and the difference crashed a
/// walk.** `FeComponentSprite::findByIdPath` (`0x140b6bec0`) holds children a third way -- not the
/// `+0x38` linked list `FeComponentObject` and `FeComponentScene` use, but the DISPLAY LIST:
///
/// ```asm
/// mov   rbx, [rcx+0x70]          ; the list
/// cmp   dx,  [rcx+0x66]          ; against the live child count
/// cmp   [rbx+0xc], eax           ; this entry's key against the id being looked for
/// mov   rcx, [rbx]               ; the child
/// add   rbx, 0x10                ; next entry
/// ```
///
/// That is the same list [`FLO_DEFINITION_CHILD_COUNT_OFFSET`] bounds and `FUN_140b6bd80` fills,
/// so **the container built from the quit tab's definition is one of these** -- which is why
/// raising that definition's child count grew it. The genuine tree leaves are the classes whose
/// `findByIdPath` is `xor eax,eax; ret` at [`FE_COMPONENT_LEAF_FIND_BY_ID_PATH`].
pub const FE_COMPONENT_SPRITE_VTABLE: u32 = 0x011d_e318;

/// A component's first child. `+0x38`. Siblings chain through [`FE_COMPONENT_SIBLING_OFFSET`].
///
/// Read from `FeComponentObject`'s play at `0x140b6a98f` (`MOV RBX,[RCX+0x38]`).
pub const FE_COMPONENT_CHILD_OFFSET: usize = 0x38;

/// A component's next sibling. `+0x28`. Read at `0x140b6a9c5` (`MOV RBX,[RBX+0x28]`).
pub const FE_COMPONENT_SIBLING_OFFSET: usize = 0x28;

/// A `FeComponentSprite`'s animation resource. `+0x48`. Its sequence table is at `+0x18` of that.
///
/// From `0x140b6c4fa`: `MOV RAX,[RCX+0x48]; MOV RCX,[RAX+0x18]`. A null table means every play on
/// this sprite is a no-op.
pub const FE_SPRITE_RESOURCE_OFFSET: usize = 0x48;

/// The sequence table within a sprite's animation resource. `+0x18`.
pub const FE_SPRITE_TABLE_OFFSET: usize = 0x18;

/// A sequence table's entry array (`+0x00`) and its `u16` entry count (`+0x08`).
///
/// Entries are `0x10` bytes: an `i32` id at `+0` and a `u16` start frame at `+4`. The play scans
/// them linearly and **falls through to `RET` on a miss** (`0x140b6c50c`, `0x140b6c532`), leaving
/// the sprite exactly where it was -- which is how half a row ends up posed and the rest blank.
pub const FE_SPRITE_TABLE_ENTRIES_OFFSET: usize = 0x00;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_COUNT_OFFSET: usize = 0x08;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_ENTRY_STRIDE: usize = 0x10;
/// See [`FE_SPRITE_TABLE_ENTRIES_OFFSET`].
pub const FE_SPRITE_TABLE_ENTRY_START_OFFSET: usize = 0x04;

/// A `FeComponentSprite`'s current playback position, a `f32`. `+0x40`.
///
/// Written by the play at `0x140b6c57b` (`MOVSS [RBX+0x40],XMM6`) as `start(sequence) + offset`.
/// Reading it back is how "did this sprite actually follow the sequence" is answered without
/// looking at the screen.
pub const FE_SPRITE_POSITION_OFFSET: usize = 0x40;

/// Rows 2 and 3, the pair the game guarantees is never shown together.
///
/// `0x1400f4344` computes row 3's enable byte as `row2.enabled == 0`, so their XOR is true at every
/// instant. Adding either to [`FE_TOP_MENU_FORCE_SHOWN_ROWS`] therefore does not reveal a hidden
/// row -- it forces the dead half of a mutually exclusive pair on screen next to the live half.
/// That was done, it drew both INFORMATION and GO ONLINE at once, and
/// `FE_TOP_MENU_PAIR_MUTUALLY_EXCLUSIVE` exists so the next attempt is caught by a log line instead
/// of by someone looking at the screen.
pub const FE_TOP_MENU_PAIR_MUTUALLY_EXCLUSIVE: u32 = (1 << 2) | (1 << 3);

/// `FeObjectButtonEx`'s flags word. `+0x14`, a DWORD.
///
/// Read at `0x14010ad62` (`mov eax,[rbx+0x14]`) in the button's own styling method, and tested for
/// a skip bit at `0x14010ad35` (`test BYTE PTR [rcx+0x14],0x40`) before anything else happens.
pub const FE_BUTTON_FLAGS_OFFSET: usize = 0x14;

/// The flag bit that makes a button draw its alternate look. `0x400`. **DEAD END, DO NOT USE.**
///
/// Setting it on a title-menu row changes nothing, and the reason is one field over. The styling
/// method opens `test BYTE PTR [rcx+0x14],0x40; jne <exit>` at `0x14010ad35` -- **bit 6 is a skip
/// bit**, and a live run logged these buttons' flags as `0x00000040`, so the method returns before
/// it ever reaches the branch this bit selects. `FeObjectButtonEx` draws nothing of its own here;
/// the row's appearance comes entirely from the menu's own pass through the layout element.
///
/// Kept documented rather than deleted so the next attempt does not rediscover it. The mechanics
/// below are accurate; they are simply unreachable for this menu.
///
/// `FeObjectButtonEx`'s styling method branches on `test eax,0x401` at `0x14010ad65`: with neither
/// bit it plays sequence `0x6c`, with either it plays **`0x7e`** instead. **This is the only
/// alternate appearance the button class has**, and it is the nearest thing in the image to a
/// "drawn but not offered" look -- the menu's own styling pass never reaches it, because that pass
/// removes an unavailable row from the screen rather than restyling it.
///
/// Bit `0x400` rather than bit `0x1`, deliberately: both take the same branch, but bit `0x1` also
/// writes `[this+0x18] = 1` at `0x14010ad70`, which a second method (`0x14010af10`) reads to
/// suppress a different sequence. `0x400` changes the appearance and nothing else, which is the
/// smallest change that can produce the effect.
pub const FE_BUTTON_FLAG_ALTERNATE_LOOK: i32 = 0x400;

/// Bind a row descriptor to a `FrontendEx::SceneObjProxy`. RVA `0x00026790`, VA `0x140026790`.
///
/// `proxy* bind(group + 0x100, scratch, descriptor)`. It forwards to `0x140027880`, which installs
/// `SceneObjProxy::vftable`, stores the group at `+0x58` and copies the descriptor's label object
/// to `+0x60`. The menu's styling pass builds one per row per pass into a 144-byte stack scratch
/// and lets it die there.
pub const FE_BIND_SCENE_OBJ_PROXY: u32 = 0x0002_6790;

/// The `ComponentFrameCtrl` embedded in a `SceneObjProxy`. `+0x40`.
///
/// `0x14001e150` initialises the proxy's members in order: `ComponentPositionProperty` at `+0x10`,
/// `ComponentSizeProperty` at `+0x20`, `ComponentFrameCtrl` at `+0x40`. **There is no colour or
/// alpha property among them**, which is why dimming a row is not reachable through this object and
/// has to come from a sequence the layout itself defines.
///
/// Vtable slot 0 of the frame control is the sequence play, called as
/// `play(this, id, 0, 0.0f)` at `0x1400f5087` -- RCX, EDX, R8D, XMM3.
pub const FE_SCENE_OBJ_PROXY_FRAME_CTRL: usize = 0x40;

/// Build the top menu's six row descriptors. RVA `0x000f4250`, VA `0x1400f4250`.
///
/// `list* build(list)` -- fills a caller-supplied 352-byte buffer and returns it. Four call sites,
/// including the cell factory, so calling it again per state change is a rate the game already
/// exceeds on its own.
pub const FE_TOP_MENU_BUILD_ROWS: u32 = 0x000f_4250;

/// Sequence for a row that is available. `0x67`.
///
/// The id the menu's own styling pass plays on an available row at `0x1400f5080`.
pub const FE_TOP_MENU_SEQUENCE_AVAILABLE: i32 = 0x67;

/// Sequence for a row that is drawn but not selectable. `0x6c`. **DO NOT TRUST THIS NAME.**
///
/// A control run -- the game's own title menu, with `show_unavailable` off and none of this mod's
/// writes in it -- showed that this is not a "faded" look and was never needed. The game draws
/// every row, dims one it cannot offer, and swaps INFORMATION and GO ONLINE inside one shared slot.
/// Meanwhile `0x6c` is frames 89..103 of a shared timeline whose next marker is `0x7a` at 104, the
/// removal: it is the segment leading OUT. Playing it and letting it run walks the row off the
/// screen; holding its last frame poses the row invisible. Both were reported as "the row is
/// blank", and both were this constant doing what it actually means.
///
/// The measured markers, all six identical across all six rows:
/// `0x67@1  0x69@9  0x6a@15  0x6b@83  0x6c@89  0x7a@104`.
///
/// If a genuine dimmed look is ever wanted, `0x6b@83` and `0x6a@15` are the unexamined neighbours;
/// which frames render is an alpha curve in `GameDataEbl.bdt` and is not readable from the image.
///
/// Original note, kept because its trial method is sound even though its conclusion was not:
///
/// Sequence for a row that is drawn but not selectable. `0x6c`. **MEASURED ON SCREEN.**
///
/// This one could not be established statically and was not guessed. Sequence ids index a layout
/// resource inside `GameDataEbl.bdt`, so a run played a different candidate on each menu row at
/// once -- `0x6c`, `0x7e`, `0x69`, `0x6b`, `0x70`, with row 0 held at
/// [`FE_TOP_MENU_SEQUENCE_AVAILABLE`] as the control -- and `0x6c` was the one that came back
/// faded. The candidates were the layout's own vocabulary, taken from the sequences
/// `FeObjectButtonEx`'s methods play on their elements, rather than a sweep of the id space.
///
/// Two cheaper routes were tried first and both are dead, recorded so they are not retried:
/// [`FE_BUTTON_FLAG_ALTERNATE_LOOK`] is unreachable on these buttons, and the row's proxy carries
/// position, size and frame-control properties but no colour or alpha.
pub const FE_TOP_MENU_SEQUENCE_FADED: i32 = 0x6c;

/// `FexGroupList<FeGroupGrid>`'s "nothing here is selectable" pass. RVA `0x00106240`.
///
/// Vtable slot 3, and its whole body is a loop writing [`FE_BUTTON_STATE_UNAVAILABLE`] into every
/// cell in the list. Slot 5 (`0x140106290`) is its inverse, resetting `2 -> 3`.
///
/// **This is why the menu is drawn before it can be used**, and it is the reason no separate
/// "is it ready yet" flag has to be invented: the game already publishes that fact, per cell, in
/// the field the navigation predicate reads. A row that cannot be selected -- because its own
/// enable byte was false, or because this pass disabled the whole list while the title scene is
/// still animating -- is exactly a row in state 2.
pub const FE_TOP_MENU_DISABLE_ALL: u32 = 0x0010_6240;

/// The `GameManagerImp` pointer. RVA `0x016148f0`, VA `0x1416148f0`.
///
/// Read at `0x1400f432d` in the top-menu row builder, among many other places.
pub const GAME_MANAGER: u32 = 0x0161_48f0;

/// `SaveLoadSystem` within `GameManagerImp`. `+0xb8`.
///
/// `FeSubStateTitleLoadProfile`'s work starter reads it at `0x1400fc384`
/// (`mov rdi,[rax+0xb8]`) before handing it to the loader.
pub const GAME_MANAGER_SAVE_LOAD_SYSTEM: usize = 0xb8;

/// The `SaveLoadSystem` field that is non-zero while a request is in flight. `+0x8`.
///
/// The pump at `0x1402e6230` opens `mov eax,[rcx+8]; sub eax,2; test eax,0xfffffffd; jne <bail>`,
/// so it only does work when this is `2` or `4`, and every completion path writes `0`. A save or
/// profile load in progress is therefore a non-zero here, read out of the game's own field rather
/// than inferred from a clock.
///
/// **Why this and not the cell states.** The menu is drawn before it can be used, and the obvious
/// candidate for that was `FE_TOP_MENU_DISABLE_ALL` -- but a live run showed the cell states never
/// reading all-unavailable, only the two rows the enable bytes had already ruled out. So whatever
/// holds input during that window is above the cells, and the window measurably coincides with the
/// save arriving: the same run logged the row states flipping from `0b000110` to `0b001000` at the
/// moment the save landed.
pub const SAVE_LOAD_SYSTEM_REQUEST: usize = 0x8;

/// `FeGroupTitleTopMenu::TitleButtonLayout`'s cell factory. RVA `0x000f36b0`, VA `0x1400f36b0`.
///
/// `proxy* build(layout, proxy_out, coords)` -- `coords` is `int[2]`, column then row. It rebuilds
/// the descriptor list, and for an in-range `(0, row)` binds descriptor `row` into the caller's
/// proxy through [`FE_BIND_SCENE_OBJ_PROXY`]; anything out of range gets an empty cell from
/// `0x140027980` instead. The bound proxy is returned in RAX, so a detour has the row's layout
/// element in hand without rebuilding anything.
///
/// **This is the earliest moment a row exists**, which is what makes it the only place able to
/// decide what a row looks like on its very first frame. Everything else -- the styling pass, the
/// substate updates -- runs after the rows are already on screen.
///
/// Not Arxan-redirected; prologue `48 89 5c 24 08`, five bytes exactly.
pub const FE_TOP_MENU_BUILD_CELL: u32 = 0x000f_36b0;

/// A seek offset far past the end of any UI sequence. `1000.0` seconds.
///
/// **The fourth argument to a sequence play is a start offset, not a flag.** Read from
/// `FeObjectButtonEx`'s own styling method, which computes one rather than passing zero:
///
/// ```text
/// play(element, 0x68, 0, (now - start_of_85) * (span_of_68 / span_of_85))
/// ```
///
/// -- it starts sequence `0x68` at the point corresponding to how far `0x85` has already run. So a
/// sequence can be entered part-way, and passing `0.0` everywhere means "always play from the
/// beginning", which for a fade means watching it fade rather than seeing it faded.
///
/// How far into the fade to start it. `14.0`, the fade's own span.
///
/// **The fourth argument to a sequence play is an offset from that sequence's START frame**, read
/// out of `FeComponentSprite::v24` (`0x140b6c4f0`) rather than inferred:
///
/// ```text
/// entry     = table_lookup(this, sequence)      # 0x10 bytes per entry, id at +0, start at +4
/// param_4   = (float)entry.start + param_4      # <- the offset is RELATIVE
/// [this+0x40] = param_4                         # <- and this is the playback position
/// ```
///
/// The position is a plain float at `+0x40` of the sprite; `v9` is `movss xmm0,[rcx+0x40]; ret`.
///
/// With `0x6c` measured at frames 89 to 103, that makes the arithmetic exact, and explains four
/// on-screen results that looked contradictory:
///
/// | passed | lands on | seen |
/// | --- | --- | --- |
/// | `0.0` | 89, the fade's FIRST frame | fades, playing all 14 frames |
/// | `14.0` | 103, its last frame | faded immediately -- what is wanted |
/// | `100.0` | 189 | nothing: past the end of a 104-frame animation |
/// | `103.0` | 192 | nothing |
/// | `1000.0` | 1089 | nothing |
///
/// The earlier reading of this argument as an absolute timeline position is what made `103.0` look
/// like the obvious value; it is off by the sequence's own start every time.
pub const FE_TOP_MENU_SEQUENCE_FADED_SEEK: f32 = 14.0;

// ============================================================================================
// THE TITLE STATE MACHINE ITSELF -- `FeStateFlow`, its resident substate, and the id space.
//
// Everything above this line names a specific substate or a specific screen. These name the
// MACHINE, which is why they are grouped: they are what `ds2-boot-timeline` needs in order to
// instrument every step without knowing any step's name, and what a loading bar would be driven
// from. See `docs/DS2-BOOT-WORK.md` for the trace and the full boot chain.
// ============================================================================================

/// `FeStateFlow::update` -- the dispatcher that drives the resident substate. RVA `0x00104540`.
///
/// # How it was established
///
/// `FeOperatorTitle::v4`'s phase-4 branch calls it at `0x1400ef42b` with `RCX = [operator+0x38]`,
/// and the body is unambiguous about what that object is: it reads the resident substate from
/// `+0x10`, calls `[[resident]+0x18]` (`update`) with the frame delta, and on a transition calls
/// `[[resident]+0x30]` (`v6`, drop transitions), `[[resident]+0x10]` (`leave`), then
/// `[[next]+0x08]` (`enter`) and `[[next]+0x28]` (`v5`, publish transitions). It is not an Arxan
/// redirect -- `scripts/ds2-arxan-chain.py` terminates at hop 0 with the prologue
/// `40 53 48 83 ec 30` (`push rbx; sub rsp,0x30`) at the entry.
///
/// # Its signature is two arguments, and that is read rather than assumed
///
/// `this` in RCX and the frame delta in XMM1. Every other register the body uses it loads from the
/// object first -- `mov rdx,[rbx+0x28]`, `mov r8,[rbx+0x30]` -- so there is no third argument a
/// detour could clobber by using the register as scratch. The float rules out `ds2-hook`'s union,
/// whose shared signature is four integers.
pub const FE_STATE_FLOW_UPDATE: u32 = 0x0010_4540;

/// `FeSubStateBase::v6` -- "drop the transitions I published". RVA `0x001043a0`.
///
/// The flow calls this on the outgoing substate immediately before `leave`, on both of its
/// transition paths (`0x140104584` and `0x1401046a9`). **Checked against all 36 `FeSubState*`
/// vtables: not one overrides slot 6.** That is what makes this single address every departure in
/// the game, and it is the reason `ds2-boot-timeline` can see steps a per-class hook would miss --
/// the failure `ds2-dialog-skip` already hit once with `FeSubStateTitleInformation`, which shows
/// its wait window from `update` rather than `enter`.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the prologue
/// `48 89 6c 24 18 57 41 56` at the entry.
///
/// Its arguments are `(this, transitions, context)`, all integer -- taken from the two call sites,
/// which both set RDX from `[flow+0x28]` and R8 from `[flow+0x30]`.
pub const FE_SUBSTATE_DROP_TRANSITIONS: u32 = 0x0010_43a0;

/// Offset of the resident substate pointer in `FeStateFlow`.
///
/// Read at `0x1401045e3` (`mov rcx,[rbx+0x10]`) before the resident's `update`, and written at
/// `0x1401046bf` (`mov [rbx+0x10],rdi`) when a transition is taken.
pub const FE_STATE_FLOW_RESIDENT_SUBSTATE_OFFSET: usize = 0x10;

/// Offset of the substate list in `FeStateFlow` -- the `TMenuStateBaseList<FeSubStateBase, 0x58>`
/// that `FeStateTitle::v6` fills with 64 substates.
///
/// Read at `0x140104658` (`mov rdx,[rbx+0x20]`), immediately before the loop that searches it for
/// the requested id.
pub const FE_STATE_FLOW_SUBSTATE_LIST_OFFSET: usize = 0x20;

/// Offset of the pending-request id in `FeStateFlow`: the substate an outside caller has asked the
/// flow to move to, or `-1` for none.
///
/// `FeOperatorTitle::v4` writes `0x17` here at `0x1400ef3e4` to return to the title screen. The
/// flow compares it against zero as a **signed** value at `0x140104633` and writes `-1` at
/// `0x140104649` once it has been consumed, which is what fixes the type as `i32`.
pub const FE_STATE_FLOW_PENDING_ID_OFFSET: usize = 0x48;

/// Offset of the entry count in the substate list. Entries themselves start at `+0x08`.
///
/// Read at `0x140104661` (`movsxd r10,[rdx+0x2c8]`) to bound the transition search, and
/// incremented by `FeStateTitle::v6` once per substate it appends.
pub const FE_SUBSTATE_LIST_COUNT_OFFSET: usize = 0x2c8;

/// Offset of a substate's own id.
///
/// **This is the game's id, not a label this repo invented.** `FeStateFlow`'s transition search
/// compares this exact field against the requested id at `0x14010467f`
/// (`cmp [rdi+0xc],esi`), and every substate constructor writes it -- as a literal
/// (`FeSubStateTitleMain` writes `0x17`) or from `EDX` at the call site (the four
/// `FeSubStateTitleLogo` instances get `0x13` through `0x16`).
///
/// It is the same field `FE_DIALOG_KIND_OFFSET` and `FE_PROCESS_WINDOW_KIND_OFFSET` already name
/// at `0x0c` for their own classes, and the `kind=57` / `kind=70` those two log on a real boot are
/// `0x39 FeSubStateTitleGameServerLogin` and the message box built beside
/// `0x44 FeSubStateTitleInformation` -- which is the runtime evidence that this id space is right.
pub const FE_SUBSTATE_ID_OFFSET: usize = 0x0c;

/// Id of `FeSubStateTitleTopMenu` -- the end of the boot chain, and the screen "Continue" is on.
///
/// Written as the literal `0x47` by its constructor at `0x1400fd65d`
/// (`mov QWORD PTR [rcx+0xc],0x47`), and corroborated by `FeSubStateTitleOptionGame`'s transition
/// table, whose phase-4 edge names `0x47` as its destination.
pub const FE_SUBSTATE_ID_TITLE_TOP_MENU: u32 = 0x47;

// ============================================================================================
// THE TWO STATE WORDS BEHIND THE ONE-SECOND FLOORS (`ds2-mods-rs-wxl`).
//
// 0x05 SteamLoadSystemData and 0x44 Information each take ~1.01s, reproducibly to within 2ms
// across runs, which is a clock rather than work. Their own `update` functions contain no
// threshold, and no sleep or wait import is called from anywhere in `SaveLoad2`. So the question
// is which side of the boundary the second is spent on: the substate polling a service that
// finished long ago, or the service genuinely taking a second.
//
// These are the fields that answer it. Both are read once per frame while their substate is
// resident, and only a CHANGE is logged.
// ============================================================================================

/// `GameManagerImp`, the root singleton most engine services hang off. RVA `0x016148f0`.
///
/// Every process-window substate reaches its backend through this: `[+0xb8]` is the storage
/// service, `[+0x22f0]` the network service, `[+0x22e0]` the window system, `[+0xa8]` the object
/// holding the savedata block at its own `+0xd8`.
pub const GAME_MANAGER_IMP: u32 = 0x0161_48f0;

/// Offset of `SaveLoadSystem` in [`GAME_MANAGER_IMP`].
///
/// Read at `0x1400fc3f7` (`mov rbp,[rdx+0xb8]`) in `SaveSystemData`'s enter and at `0x1400fb004`
/// in `SteamLoadSystemData`'s, among others.
pub const SAVE_LOAD_SYSTEM_OFFSET: usize = 0xb8;

/// `SaveLoadSystem`'s request state word.
///
/// **This is the interlock.** Every start entry point refuses while it is non-zero
/// (`0x1402e72c0` and `0x1402e7170` both open `if ([this+0x08] != 0 || [this+0x0c] != 0) return
/// false`), and both pollers gate on it: `0x1402e6230` accepts `{2, 4}`, `0x1402e67f0` tests
/// `bt 0x6a` for `{1, 3, 5, 6}`. If it flips out of "working" long before
/// `0x05 SteamLoadSystemData` advances, the floor is in the substate; if it stays put for the full
/// second, the floor is below, in `SaveLoad2`.
pub const SAVE_LOAD_SYSTEM_STATE_OFFSET: usize = 0x08;

/// The second half of the same interlock, checked alongside [`SAVE_LOAD_SYSTEM_STATE_OFFSET`].
pub const SAVE_LOAD_SYSTEM_SUBSTATE_OFFSET: usize = 0x0c;

/// The title context singleton. RVA `0x0160de10`.
///
/// `[+0x80]` is `FeSceneTitle`, `[+0xa0]` the information job below, `[+0x568]` the skip flag the
/// boot screens share, `[+0x54c]`/`[+0x558]`/`[+0x55c]`/`[+0x560]` result codes the substates
/// publish.
pub const FE_TITLE_CONTEXT: u32 = 0x0160_de10;

/// Offset of the information-download job in [`FE_TITLE_CONTEXT`].
///
/// Read at `0x1400ff787` (`mov rbx,[rax+0xa0]`) in `FeSubStateTitleInformation::v3`'s phase-4
/// branch, which ticks it through `[[job]+0x20]` and then tests the field below.
pub const FE_TITLE_INFORMATION_JOB_OFFSET: usize = 0xa0;

/// The information job's own state, the value `0x44 Information` is waiting on.
///
/// Read at `0x1400ff797` (`mov eax,[rbx+0x18]`) and compared against `5` then `6`; either sends
/// the substate to its terminal phase. Watching it says whether the job finishes early and the
/// substate sits on the result, or the job itself takes the second.
pub const FE_INFORMATION_JOB_STATE_OFFSET: usize = 0x18;

// ============================================================================================
// THE ONE-SECOND FLOORS, LOCATED (`ds2-mods-rs-wxl`). Predicted ~1.86s; MEASURED 875ms.
//
// Lifting both floors moved only `0x05`. `0x44` turned out to be sitting on a download job that
// always fails, not on this timer -- `ds2-mods-rs-umo` -- and the addresses below are what proved
// it. They are correct as read; it was the price on them that was wrong.
//
// Two substates that are NOT `FeSubStateProcessWindowBase` subclasses -- so `ds2-dialog-skip`'s
// min-duration zeroing never reached them -- each hold their own elapsed timer and compare it
// against the SAME float, `0x1410ac698`, which is `1.0f`.
//
// Measured, run 6: `0x05` reaches phase 4 at t=4115.9ms and does not leave it until t=4994.9ms --
// 879ms. `0x44` reaches phase 2 at t=5676.4ms and does not leave until t=6661.4ms -- 985ms. In
// both cases the work they were waiting for had already finished.
//
// DO NOT PATCH THE CONSTANT. `0x1410ac698` has 2042 RIP-relative references from 1548 functions:
// it is MSVC's pooled `1.0f` literal for the whole image, not a tunable belonging to these two.
// The fix is to advance each substate's OWN elapsed field so the game's own comparison passes,
// which is the same shape as the existing min-duration zeroing and leaves both the comparison and
// the transition the game's.
// ============================================================================================

/// `FeSubStateTitleSteamLoadSystemData::v1` -- its `enter`. RVA `0x000faff0`.
///
/// Starts the system-data load through `SaveLoadSystem` (`0x1402e72c0`), shows a process window,
/// and sets phase 1. Measured: the storage work is finished 88ms later; the substate then spends
/// 879ms in phase 4 waiting on the floor below.
pub const FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ENTER: u32 = 0x000f_aff0;

/// Its elapsed timer, a `f32`.
///
/// Zeroed by the constructor at `0x1400fab66`. Accumulated in phases 1 and 2 without ever being
/// compared, and compared in **phase 4** at `0x1400fc13e`:
/// `addss xmm6,[rdi+0x18]; comiss xmm6,[0x1410ac698]; jb return`.
pub const FE_SUBSTATE_STEAM_LOAD_SYSTEM_DATA_ELAPSED_OFFSET: usize = 0x18;

/// `FeSubStateTitleInformation::v1` -- its `enter`. RVA `0x000ff570`.
pub const FE_SUBSTATE_TITLE_INFORMATION_ENTER: u32 = 0x000f_f570;

/// Its elapsed timer, a `f32`, at `+0x5a24` of a `0x5a30`-byte object.
///
/// Compared in **phase 2** at `0x1400ff9b7`, and reset to zero on the way out at `0x1400ff9d7`.
///
/// **Phase 2 waits on two things, and only one of them is the floor**:
/// ```text
/// elapsed += delta
/// if ([r14]->vtable[0x28]()) return;         // the job is still running -- REAL work
/// if (elapsed < 1.0f)        return;         // the floor
/// close the window; phase = 3
/// ```
/// Advancing the timer therefore removes the floor and leaves the job wait entirely intact, which
/// is the whole reason this is safe to do.
pub const FE_SUBSTATE_TITLE_INFORMATION_ELAPSED_OFFSET: usize = 0x5a24;

/// What to write into either elapsed field so the game's own `comiss` passes on the first frame.
///
/// `1.0f` exactly would leave `comiss` at equality, and the branch is `jb` -- below, not
/// below-or-equal -- so equality already passes. A slightly larger value is used anyway so that a
/// frame delta being added before the comparison cannot matter, and so the intent is legible: this
/// is "the minimum display time has elapsed", not "the timer is exactly at the threshold".
pub const FE_SUBSTATE_FLOOR_ELAPSED: f32 = 2.0;

/// The import thunk `DarkSoulsII.exe` calls `KERNEL32!Sleep` through. RVA `0x01aae314`.
///
/// # Why an IAT slot rather than the function
///
/// The engine block -- 3.06s between input initialisation and the title flow -- reproduces to
/// 0.67%, reads nothing from disk after an early burst, and never exceeds half of one core. That
/// combination is the signature of sleeping, not of working, and the binary has a candidate: a
/// `PeekMessageW` / `Sleep(1)` / check-a-flag loop at `0x140fecdd6` that spins until `[rbx+0x11c]`
/// clears. Three seconds of that is about three thousand iterations.
///
/// Counting the game's `Sleep` calls tests it directly, and patching **the IAT slot** rather than
/// `KERNEL32!Sleep` itself is what keeps the test cheap and honest: it is a pointer write in
/// `.idata`, so no code is modified, Arxan's `.text` integrity checks have nothing to see, and only
/// this executable's calls are counted rather than every module in the process.
///
/// `Sleep` is `void Sleep(DWORD)` -- one integer argument, no stack arguments, no return value --
/// which is why this one can be fronted from Rust with an ordinary `extern "system"` function and
/// needs none of the naked-thunk machinery a ten-argument import like `D3D11CreateDevice` would.
///
/// Established by walking the import descriptors: `KERNEL32.dll`'s `FirstThunk` is at RVA
/// `0x1aade6c` and `Sleep` is the entry that lands here. There are **13** call sites in the image;
/// two of them are `Sleep(0)` yield loops and one is the `Sleep(1)` pump above.
pub const SLEEP_IAT_THUNK: u32 = 0x01aa_e314;

/// The frame limiter: "sleep the rest of this frame, or yield if we are already late".
/// RVA `0x00feb910`.
///
/// # What it does, read from its own body
///
/// ```text
/// now       = timeGetTime()
/// elapsed   = (now - [this+0x17c]) * 10000
/// [this+0x174] = moving average of elapsed and the previous frame time
/// [this+0x178] = elapsed
/// remaining = [this+0x170]
/// if (remaining <= 0) tail-call Sleep(0)          ; already late -- yield
/// else                tail-call Sleep(remaining / 1000)
/// ```
///
/// # Why it is worth counting
///
/// The 3.04s engine block asks for 9866 ms of sleep across 3631 `Sleep` calls, and 63% of all
/// `Sleep` calls in the boot pass `0` -- which is this function's late path. If the block is a loop
/// advancing a fixed number of steps per frame, then **calls to this function are frames**, and the
/// count will be the same on every run. That is the measurement that decides whether the engine
/// block can be attacked by pacing at all, and it must exist before anything tries.
///
/// Signature is `void(this)`: RCX in, no other argument read, and both exits are tail-jumps to
/// `Sleep` so nothing is returned. Not an Arxan redirect -- `scripts/ds2-arxan-chain.py` terminates
/// at hop 0 on its own prologue, `40 53 48 83 ec 20` (`push rbx; sub rsp,0x20`), which also gives
/// MinHook six clean bytes to relocate.
pub const FRAME_LIMITER: u32 = 0x00fe_b910;

/// The simpler sibling of [`FRAME_LIMITER`], and **measured never to be called during boot**.
///
/// RVA `0x00feb890`. Same tail -- sleep `[this+0x170]/1000`, or `Sleep(0)` when not positive --
/// but it skips the clock sample and the moving average entirely, writing a fixed `0x4c4b40` into
/// `[this+0x178]` instead. It was instrumented first on the strength of that shared tail and the
/// counter read **zero** across a whole boot, which is what moved the instrument to `0x00feb910`.
/// Recorded so the next person does not spend the same run finding out.
pub const FRAME_LIMITER_RESET: u32 = 0x00fe_b890;

// ---------------------------------------------------------------------------------------------
// THE NETWORK SERVICE AND ITS ONLINE FLAG
//
// One boolean decides whether DARK SOULS II believes it is online, and the whole of `ds2-offline`
// rests on the four facts below. All four were read out of `darksoulsii-deobf.bin`; no game was
// launched to establish any of them.
//
//   net = [[0x1416148f0] + 0x22f0]      GameManagerImp's network service
//   net + 0x3a                          the flag, ONE BYTE
//   0x140513600                         its getter, `movzx eax, byte [rcx+0x3a]; ret`
//   0x140513820                         its setter, `mov byte [rcx+0x3a], dl; ret`
//
// THE FLAG IS BORN ZERO. The service's constructor at `0x140512f30` -- identified by the vtable
// `0x1410d13e8` it installs at `[this]` -- writes `mov BYTE PTR [rbx+0x3a],0` at `0x140512f5a`,
// four instructions in. So "offline" is the state this object is CONSTRUCTED in, and every online
// run is one that left it. That is why the setter can be neutered rather than fought: neutering it
// does not impose a value, it prevents a departure from the game's own initial one.
// ---------------------------------------------------------------------------------------------

/// `NetService::isOnline`. RVA `0x00513600`, VA `0x140513600`.
///
/// The whole function is `movzx eax, BYTE PTR [rcx+0x3a]; ret` -- five bytes, and `docs/
/// DS2-TITLE-FLOW.md` already named it "the game's master online gate" from the other end, while
/// tracing which rows the top menu greys out.
///
/// **34 call sites**, found by scanning the image for `e8` displacements that resolve here
/// (`scripts/ds2-xrefs.py`). Every one of them is immediately followed by `test al,al` and a
/// branch, which is what makes forcing the return value a complete answer for its readers rather
/// than a partial one. Three of the 34 were disassembled to check that the polarity is what it
/// looks like:
///
/// * `FeSubStateTitleOnlineCheck::v8` (`0x1400f98c0`), the substate's own work starter: calls this,
///   and on a zero returns `false` **without starting anything**. So a forced zero does not fake
///   the online check -- it takes the shipped path where the check never runs.
/// * The top-menu builder at `0x1400f433b`: the result becomes `r14b`, which enables row 2 (server
///   information) and disables row 3 (go online), or the reverse.
/// * `0x1400fe739`, inside the same menu code, gating whether a transition is registered at all.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 on the real prologue
/// `0f b6 41 3a c3`, followed by a `90` pad. Five bytes of body and a byte of padding is enough
/// room for the three-byte stub [`NET_IS_ONLINE_STUB`] writes and nothing has to be relocated.
pub const NET_IS_ONLINE: u32 = 0x0051_3600;

/// First byte of [`NET_IS_ONLINE`]'s body, `0f` -- the `movzx` opcode.
///
/// Passed to `ds2_hook::patch_3byte_stub` as its `expected_first` guard. If a future build moves
/// the function, this byte almost certainly differs and the patch aborts instead of landing in the
/// middle of some other function's instruction.
pub const NET_IS_ONLINE_EXPECTED_FIRST: u8 = 0x0f;

/// `xor eax,eax; ret` -- what [`NET_IS_ONLINE`] is replaced with. Reports offline to all 34 readers.
pub const NET_IS_ONLINE_STUB: [u8; 3] = [0x31, 0xc0, 0xc3];

/// `NetService::setOnline`. RVA `0x00513820`, VA `0x140513820`.
///
/// `mov BYTE PTR [rcx+0x3a], dl; ret` -- the exact write-side pair of [`NET_IS_ONLINE`], on the
/// same object at the same offset. Found from the other end rather than by searching for a setter:
/// `NetSvrManager`'s vtable slot `+0x60` (`0x140290040`) opens by calling this with `edx` zeroed,
/// and **`FeSubStateTitleSetOfflineMode::v1` (`0x1400f8f80`) is nothing but a tail-jump into that
/// slot**. So this is the write the game's own "play offline" substate performs.
///
/// Not an Arxan redirect; its own four-byte body plus `ret` sits at its entry, followed by `cc`
/// padding.
pub const NET_SET_ONLINE: u32 = 0x0051_3820;

/// First byte of [`NET_SET_ONLINE`]'s body, `88` -- the `mov r/m8, r8` opcode.
pub const NET_SET_ONLINE_EXPECTED_FIRST: u8 = 0x88;

/// `ret; nop; nop` -- what [`NET_SET_ONLINE`] is replaced with, making the setter inert.
///
/// **Not `xor eax,eax; ret`.** The setter returns nothing, so zeroing `eax` would be a lie about
/// its signature that happens to be harmless; `ret` says what is meant. The two `nop`s exist only
/// because `ds2_hook::patch_3byte_stub` writes three bytes, and they are never executed.
pub const NET_SET_ONLINE_STUB: [u8; 3] = [0xc3, 0x90, 0x90];

/// Byte offset of the online flag inside the network service object. Recorded for diagnostics --
/// nothing in this repo writes it directly.
///
/// Read three ways that agree: the getter's `[rcx+0x3a]`, the setter's `[rcx+0x3a]`, and the
/// constructor's `mov BYTE PTR [rbx+0x3a],0` at `0x140512f5a`.
pub const NET_ONLINE_FLAG_OFFSET: usize = 0x3a;

/// Offset of the network service in [`GAME_MANAGER_IMP`] -- the `this` every call to
/// [`NET_IS_ONLINE`] and [`NET_SET_ONLINE`] is made on.
///
/// The sibling of [`SAVE_LOAD_SYSTEM_OFFSET`], and named the same way. `GAME_MANAGER_IMP`'s own
/// doc comment already recorded `[+0x22f0]` as the network service from the boot-chain trace; this
/// is that offset given a name so the two call sites in `ds2-offline` do not spell it as a
/// literal. Every getter call site disassembled for [`NET_IS_ONLINE`] loads it the same way:
/// `mov rax,[0x1416148f0]; mov rcx,[rax+0x22f0]`.
pub const NET_SERVICE_OFFSET: usize = 0x22f0;

/// The force-offline byte at VA `0x14160de19`, and **it is not the switch it looks like**.
/// RVA `0x0160de19`.
///
/// `ds2-mods-rs-rk4` asked whether setting this removes the network boot chain. It does not.
/// It is read at **exactly one instruction in the whole image**, `0x1400f431f` inside the top-menu
/// builder:
///
/// ```text
/// cmp BYTE PTR [0x14160de19], 0
/// je  ask_the_gate          ; zero -> fall through to call 0x140513600
/// xor r14b, r14b            ; non-zero -> force "not online" and skip the call
/// ```
///
/// So it is a local override of one boolean in one function, and the boot chain -- which calls
/// [`NET_IS_ONLINE`] directly, on its own -- never sees it. Recorded here so the next reader does
/// not have to re-derive that it is a dead end; [`NET_IS_ONLINE`] is the read it was shadowing.
pub const NET_FORCE_OFFLINE_MENU_ONLY: u32 = 0x0160_de19;

// ============================================================================================
// THE SAVE-SLOT LOAD PATH. Which character slot a load actually used, and where the game keeps
// that answer. Read statically from `FeSubStateTitleLoadDataList::v3` at `0x1400fba10`, whose
// whole body is the decision; full trace in `docs/DS2-CONTINUE.md`.
//
// The chain the update walks, taken from its own instructions rather than from a struct:
//
//   mov rax,[0x14160de10]            ; FE_TITLE_CONTEXT
//   mov rdi,[rax+0x98]               ; FeGroupTitleDataList
//   movsxd rdx,[rax+0x564]           ; the selected slot
//   mov rax,[0x1416148f0]            ; GAME_MANAGER_IMP
//   mov rcx,[rax+0xa8]               ; GameDataManager
//   mov r8,[rcx+0xd8]                ; the slot array
//   cmp rdx,0xa / jae                ; ten slots, and a negative slot means none
//   imul rax,rax,0x1f0               ; stride
//   test BYTE PTR [rbx+0x1d9],0x1    ; occupied
//   mov edx,[rdi+0x28]               ; the group's confirmed action
// ============================================================================================

/// `FeSubStateTitleLoadDataList::v3` (update). RVA `0x000fba10`, VA `0x1400fba10`.
///
/// The single site worth instrumenting on the load path, because it is the only place the slot,
/// the action and the outgoing phase are all in scope at once. It runs per frame but does nothing
/// unless the substate's phase is 1, which its first three instructions establish:
/// `mov edx,[rcx+0x10]; dec edx; jne <return>`.
///
/// **Not an Arxan redirect.** `scripts/ds2-arxan-chain.py` reports `UNKNOWN` here only because its
/// prologue table does not carry `40 56` (`rex push rsi`); the entry is ordinary code, not the
/// five-byte `e9` a redirected entry holds.
pub const FE_SUBSTATE_LOAD_DATA_LIST_UPDATE: u32 = 0x000f_ba10;

/// The selected save slot, at `[`[`FE_TITLE_CONTEXT`]`] + 0x564`. Signed; `0..=9` selects, and
/// anything else means "none".
///
/// The Ghidra project names this field `_x564_slotNum` off the same evidence. It is read with
/// `movsxd` and immediately bounds-checked against `0xa`, so a mod writing it must respect both
/// the sign and the bound -- see [`SAVE_SLOT_COUNT`].
pub const FE_TITLE_CONTEXT_SLOT_NUM_OFFSET: usize = 0x564;

/// The `FeGroupTitleDataList` that owns the character list, at `[`[`FE_TITLE_CONTEXT`]`] + 0x98`.
pub const FE_TITLE_CONTEXT_DATA_LIST_GROUP_OFFSET: usize = 0x98;

/// The group's confirmed action, at `group + 0x28`. `1` backs out, `2` loads the selected slot.
///
/// Written by the group's own `vtable[4]`, which the update calls immediately before reading this.
/// `3` and `4` are also handled (they route to `0x56` and `0x5f`) but were not identified.
pub const FE_GROUP_DATA_LIST_ACTION_OFFSET: usize = 0x28;

/// The action value that means "load the selected slot".
pub const FE_DATA_LIST_ACTION_LOAD: i32 = 2;

/// Offset of `GameDataManager` in [`GAME_MANAGER_IMP`].
pub const GAME_DATA_MANAGER_OFFSET: usize = 0xa8;

/// Offset of the ten-slot save array inside `GameDataManager`.
pub const SAVE_SLOT_ARRAY_OFFSET: usize = 0xd8;

/// Stride of one save-slot record. `0x1F0`.
///
/// Confirmed far beyond this one function: the image holds 43 separate `imul reg,reg,0x1f0` sites
/// and nearly all of them are preceded by the same `cmp reg,0xa; jae` bound.
pub const SAVE_SLOT_STRIDE: usize = 0x1f0;

/// How many save slots the game indexes. Ten, enforced at every indexing site.
pub const SAVE_SLOT_COUNT: i32 = 10;

/// Flags byte within a save-slot record. `+0x1D9`.
///
/// **Runtime only.** It is zero in every record of the `id=4` section of a real `.sl2`, so it is
/// derived when the per-character entries are loaded rather than persisted. Reading a save file
/// cold to decide whether a slot exists must use entry content, not this byte.
pub const SAVE_SLOT_FLAGS_OFFSET: usize = 0x1d9;

/// [`SAVE_SLOT_FLAGS_OFFSET`] bit 0: the slot holds a character. The update nulls its record
/// pointer when this is clear.
pub const SAVE_SLOT_FLAG_OCCUPIED: u8 = 0x1;

/// [`SAVE_SLOT_FLAGS_OFFSET`] bit 1: the slot is excluded. Every one of the four action branches
/// returns without acting when this is set.
pub const SAVE_SLOT_FLAG_EXCLUDED: u8 = 0x2;

/// The ownership word within a save-slot record, `+0x1E8`, masked with
/// [`SAVE_SLOT_OWNERSHIP_MASK`].
///
/// On the load action the update passes `record[0x1e8] & 0x3f` to `0x140af6610`, and that result
/// picks phase 6 over phase 2 -- a different destination substate. **A continue flow must not skip
/// this call**: it is what refuses a character the running build cannot legitimately load.
pub const SAVE_SLOT_OWNERSHIP_OFFSET: usize = 0x1e8;

/// Mask applied to [`SAVE_SLOT_OWNERSHIP_OFFSET`] before the check. `0x3f`.
pub const SAVE_SLOT_OWNERSHIP_MASK: u32 = 0x3f;

/// `FeSubStateTitleLoadDataList`'s substate id: `0x55`. The character list.
pub const FE_SUBSTATE_ID_TITLE_LOAD_DATA_LIST: u32 = 0x55;

/// `FeSubStateTitleLoadProfile`'s substate id: `0x57`.
///
/// The destination of the load edge, registered by `FeSubStateTitleLoadDataList::v5`
/// (`0x1400fb1f0`) as the phase-2 transition. Identified through its constructor `0x1400faa10`,
/// which writes id `0x57` alongside vtable `0x1410bd658`.
pub const FE_SUBSTATE_ID_TITLE_LOAD_PROFILE: u32 = 0x57;

/// `FeSubStateTitleStartIngame`'s substate id: `0x6b`. The end of the load chain.
pub const FE_SUBSTATE_ID_TITLE_START_INGAME: u32 = 0x6b;

/// `FeSubStateTitleLoadDataList::v1` (enter). RVA `0x000fae80`, VA `0x1400fae80`.
///
/// Ghidra's project already names it `FUN_1400fae80_continueMenu`. It reads the list group from
/// `[`[`FE_TITLE_CONTEXT`]`]+0x98`, asks `0x1400f0f60` for the occupied slots, and on a non-empty
/// list calls `0x1400f1cb0(group, 1)` and sets its own phase to 1. On an empty list it sets phase
/// 3 and goes straight back to the top menu.
///
/// **The site to write a pre-selected slot at.** `0x1400f1cb0`'s `1` is a mode flag -- it lands in
/// `group+0x2c` and swaps the screen's text ids between load and delete -- so the cursor is not
/// chosen there. It is chosen downstream in `0x1400f1fa0`, and the list code both reads
/// [`FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`] (`0x1400f2021`) and writes it back on a cursor move
/// (`0x1400f220e`, `0x1400f22d7`, `0x1400f238e`). Writing the field before this function runs
/// therefore reaches the list before it is built; writing it after would be overwritten.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` reports a clean prologue at the entry.
pub const FE_SUBSTATE_LOAD_DATA_LIST_ENTER: u32 = 0x000f_ae80;

/// The slot the game itself remembers, at `savedata + 0x1368` -- past the ten records, which end
/// at `0x1360`.
///
/// `FeSubStateTitleSteamLoadSystemData::v2` (`0x1400fb8d0`, substate `0x05`, early boot) copies it
/// into [`FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`] when it is not negative, and `0x1400fdc82` copies it
/// back the other way. So the game has a remembered-slot mechanism of its own.
///
/// **It is not restored from the save file.** The `id=4` section of a real `.sl2` reads zero at
/// the corresponding offset even in a save written immediately after loading slot 1, so whatever
/// seeds this at boot, it is not the file. Recorded so the next reader does not re-derive that.
pub const SAVE_SLOT_CURRENT_INDEX_OFFSET: usize = 0x1368;

/// The call every accepting branch of `FeSubStateTitleLoadDataList::v3` makes before it writes a
/// phase. RVA `0x000f10e0`, VA `0x1400f10e0`, one argument: the list group.
///
/// `cmp qword [rcx+8],0; je ...` then a call through the object at `group+8`. Whatever it settles,
/// all four branches make it first, so a transition driven from outside must make it too.
///
/// Not an Arxan redirect; `ds2-arxan-chain.py` reports `UNKNOWN` only because `48 83 79` is
/// missing from its prologue table, as with `40 56` and `4c 8b 89`.
pub const FE_DATA_LIST_CLOSE: u32 = 0x000f_10e0;

/// Offset of the content/ownership context in [`GAME_MANAGER_IMP`]. A **pointer** field, read as
/// `mov rcx,[rax+0xc0]` at `0x1400fbb32`.
pub const GAME_MANAGER_CONTENT_CTX_OFFSET: usize = 0xc0;

/// Inside the content context, the object holding the owned-content masks. `+0x10`; a null here
/// means the gate passes.
pub const CONTENT_CTX_OWNED_OFFSET: usize = 0x10;

/// The two owned-content masks, OR'd together before the test. `+0x28` and `+0x30`.
///
/// The whole of `0x140af6610`, which the load branch calls and which this repo replicates with
/// pure reads rather than a call:
///
/// ```text
/// owned = (obj[0x30] | obj[0x28]) & required
/// refused = owned != required
/// ```
pub const CONTENT_OWNED_MASK_A: usize = 0x28;

/// The second owned-content mask. See [`CONTENT_OWNED_MASK_A`].
pub const CONTENT_OWNED_MASK_B: usize = 0x30;

/// The phase `FeSubStateTitleLoadDataList` writes to load the selected slot: `2`, which its own
/// transition table routes to [`FE_SUBSTATE_ID_TITLE_LOAD_PROFILE`].
pub const FE_DATA_LIST_PHASE_LOAD: i32 = 2;

/// The phase the character list writes when the player backs out: `3`, routed to
/// [`FE_SUBSTATE_ID_TITLE_TOP_MENU`].
///
/// One of the list's two ways of ending without loading anything, and so one of the two places a
/// shortcut that suppressed something for the duration of the load has to undo it.
pub const FE_DATA_LIST_PHASE_BACK: i32 = 3;

/// The phase written instead when the ownership gate refuses: `6`, routed to `0x5d`.
pub const FE_DATA_LIST_PHASE_REFUSED: i32 = 6;

/// `FeSubStateTitleTopMenu::v3` (update). RVA `0x000ff300`, VA `0x1400ff300`.
///
/// Three statements: poll the top-menu group at `[`[`FE_TITLE_CONTEXT`]`]+0x80`, copy the action
/// the group parked at `group+0xE8` into the substate's own phase, and zero
/// `savedata+0x136e` when that action is 4. **The phase and the action id are the same number**,
/// so the row-1 transition `FeSubStateTitleTopMenu::v5` registers for value
/// [`FE_TOP_MENU_ACTION_LOAD_GAME`] is taken by writing that value into the phase.
///
/// It rewrites the phase from the group on every frame, so a write made here survives exactly one
/// frame -- which is enough, because `FeStateFlow` evaluates transitions immediately after the
/// update returns. Writing the group's field instead would be cleared by the poll, the way the
/// character list's action field was.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_TOP_MENU_UPDATE: u32 = 0x000f_f300;

/// The top menu's row-1 action, LOAD GAME, which routes to
/// [`FE_SUBSTATE_ID_TITLE_LOAD_DATA_LIST`]. `2`.
///
/// Registered unconditionally by the top menu's `v5`, unlike row 3's, so the transition exists
/// whether or not the row is drawn or selectable. Full row table in `docs/DS2-TITLE-FLOW.md`.
pub const FE_TOP_MENU_ACTION_LOAD_GAME: i32 = 2;

/// The top menu's resting phase, meaning "no row activated this frame". `0`.
pub const FE_TOP_MENU_PHASE_RESTING: i32 = 0;

// ---------------------------------------------------------------------------------------------
// Audio. The frontend never calls a named sound function, which is what made this hard to find:
// every earlier sweep looked for `call [rip+N]` into the FMOD IAT and found nothing. MSVC routes
// imports through `jmp [rip+N]` thunks, so the call sites are `call <thunk>` and an IAT-target
// scan misses all of them. Scanning for the thunks first turns up thirteen, and the levers the
// project had concluded were absent are all live.
//
// FMOD is NOT statically linked in this build. `fmodex64.dll` and `fmod_event64.dll` sit beside
// the exe and are imported by name.
// ---------------------------------------------------------------------------------------------

/// The one global holding a `MOFmodSoundManager*` (`DLMO`). RVA `0x0166dfa8`, VA `0x14166dfa8`.
///
/// Read at 17 sites and written at exactly one, `0x1409e5d00`, from the lazy accessor
/// `0x1409e5c90`: it allocates `0xce0` bytes, constructs with `0x1409da780`, and stores the
/// result. `0x1409ddbc0` is the fast path -- `mov rax,[this]; test rax,rax; je <init>; ret`.
///
/// The class is confirmed by RTTI rather than inferred: the vtable at `0x1411841b8` carries a
/// complete-object locator whose type descriptor names `.?AVMOFmodSoundManager@DLMO@@`, and both
/// functions below are slots in it.
pub const SOUND_MANAGER_SINGLETON: u32 = 0x0166_dfa8;

/// `MOFmodSoundManager` -> the master `FMOD::ChannelGroup*`. `0x9f8`.
///
/// **Written by the game's own init and read back by its own update**, which is what makes this
/// an identification rather than a guess:
///
/// * `MOFmodSoundManager::v6` (`0x1409ddbe0`, init) does
///   `lea rdx,[r15+0x9f8]; mov rcx,[r13]; call <System::getMasterChannelGroup>` at `0x1409df157`
///   -- so FMOD itself writes the master group into this field.
/// * `MOFmodSoundManager::v2` (`0x1409e0910`, the command-queue drain) does
///   `movss xmm1,[rdi+0x930]; mov rcx,[rdi+0x9f8]; call <ChannelGroup::setVolume>` at
///   `0x1409e0c8f` -- the image's **only** call to `ChannelGroup::setVolume`.
///
/// Both are methods of the same class on the same vtable, so the two `0x9f8` are the same field.
pub const SOUND_MANAGER_MASTER_GROUP_OFFSET: usize = 0x9f8;

/// `MOFmodSoundManager` -> the master volume the game itself last applied, `f32`. `0x930`.
///
/// The command drain stores its incoming float here (`movss [rdi+0x930],xmm1`) and reloads it
/// three instructions later to hand to `ChannelGroup::setVolume`. So it is not a cached copy of
/// something else -- it is the value the game means the master group to have, which is what makes
/// it the right thing to restore to. Reading it back beats writing `1.0`, which would silently
/// discard whatever the player set in the options menu.
pub const SOUND_MANAGER_MASTER_VOLUME_OFFSET: usize = 0x930;

/// IAT slot for `FMOD::ChannelGroup::setVolume(float)` in `fmodex64.dll`. RVA `0x01aae9b4`.
///
/// An import slot, not code: patching or reading it never touches `.text`, so Arxan's integrity
/// checks have nothing to see. Same property `ds2-offline` relies on for the WS2_32 slots.
///
/// Calling convention is MSVC `__thiscall` on x64: `rcx` is the `ChannelGroup*`, the float goes in
/// `xmm1`, and the return is an `FMOD_RESULT` (`0` == `FMOD_OK`).
pub const FMOD_CHANNEL_GROUP_SET_VOLUME_IAT: u32 = 0x01aa_e9b4;

/// `MOFmodSoundManager::v6`, audio init. RVA `0x009ddbe0`, VA `0x1409ddbe0`.
///
/// The function that *creates* [`SOUND_MANAGER_MASTER_GROUP_OFFSET`]: at `0x1409df157` it does
/// `lea rdx,[r15+0x9f8]` and hands that to `System::getMasterChannelGroup` as the out-parameter.
/// So on return from this function the master group exists and not one sound has played yet,
/// which makes it the earliest moment anything can be silenced.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const SOUND_MANAGER_INIT: u32 = 0x009d_dbe0;

/// `MOFmodSoundManager::v2`, the command-queue drain. RVA `0x009e0910`, VA `0x1409e0910`.
///
/// **The only code in the image that can change a channel group's volume.** Its `setVolume` call
/// at `0x1409e0c96` is the sole `ChannelGroup::setVolume` site, so anything holding the master
/// group at a chosen level only has to out-run this one function and nothing else.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const SOUND_MANAGER_COMMAND_DRAIN: u32 = 0x009e_0910;

/// `MOFmodSoundManager::v0`. RVA `0x009dfef0`, VA `0x1409dfef0`.
///
/// **NOT a per-frame pump, measured.** It contains the image's only `FMOD::EventSystem::update`
/// call (`0x1409e0080`), which FMOD documents as a once-a-frame requirement, and that made it look
/// like the frame pump. A detour on it fired essentially once in a 57-second run, during process
/// teardown -- so a mute re-asserted here is never asserted, and a restore requested here arrives
/// 51 seconds late. Kept as a named constant so the next reader does not repeat the inference.
///
/// Not an Arxan redirect. `scripts/ds2-arxan-chain.py` reported `UNKNOWN` until its prologue table
/// learned `48 8b c4` (`mov rax,rsp`); the entry is ordinary code, not a five-byte `e9` stub.
pub const SOUND_MANAGER_V0_NOT_A_FRAME_PUMP: u32 = 0x009d_fef0;

/// `FeSubStateTitleStartIngame::v1` (enter). RVA `0x000fde30`, VA `0x1400fde30`.
///
/// Slot 1 of vtable `0x1410bdbf8`, which RTTI names `FeSubStateTitleStartIngame`. This is the last
/// substate on the load chain -- the boundary the autocontinue shortcut ends at, and so the point
/// at which anything suppressed for the duration of that shortcut has to be given back.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_START_INGAME_ENTER: u32 = 0x000f_de30;

// ---------------------------------------------------------------------------------------------
// The NOW LOADING screen, as a cover for the title flow.
//
// `FeOperatorNowLoading` is the operator behind `FeSceneNowLoading` / `FeGroupNowLoading` -- the
// full-screen loading page the game shows on every map transition. It is constructed once, during
// GameManagerImp's own init, and then sits idle until something makes it visible. That makes it
// available for the whole title flow, long before anything the title draws.
// ---------------------------------------------------------------------------------------------

/// `GameManagerImp` -> the frontend object that owns the operator table. `0x22e0`.
///
/// Written at `0x1401bcb79` (`mov [rdi+0x22e0],rsi`) in GameManagerImp's init, immediately after
/// `0x140500200` has populated the operator slots on that same object.
///
/// **Verified live** rather than only read: walking
/// `[`[`GAME_MANAGER_IMP`]`] + 0x22e0 + `[`FRONTEND_NOW_LOADING_OPERATOR_OFFSET`] in a running
/// game lands on an object whose vtable is `0x1410fa0c8`, which RTTI names
/// `FeOperatorNowLoading`. Note the container's own head reads as `DLKR::DLBackAllocator`, so it
/// embeds an allocator as its first member; the offset is what matters and the vtable at the end
/// of the walk is what confirms it.
pub const GAME_MANAGER_FRONTEND_ROOT_OFFSET: usize = 0x22e0;

/// That frontend object -> `FeOperatorNowLoading`. `0xc8`.
///
/// Filled by the lazy factory at `0x1405002de` (`mov [rbx+0xc8],rsi`) right after it allocates
/// `0x3c0` bytes and installs vtable `0x1410fa0c8`. The same pointer is mirrored into the
/// operator array at `+0x18`, which is how the frontend iterates operators.
pub const FRONTEND_NOW_LOADING_OPERATOR_OFFSET: usize = 0xc8;

/// That frontend object -> `FeOperatorTitle`. `0xd0`.
///
/// The neighbouring named slot to [`FRONTEND_NOW_LOADING_OPERATOR_OFFSET`], and read live at the
/// top menu: it holds an object whose vtable is `0x1410bc578`, which RTTI names
/// `FeOperatorTitle`. The same two operators are mirrored into the operator array at `+0x18`
/// (NowLoading) and `+0x30` (Title).
pub const FRONTEND_TITLE_OPERATOR_OFFSET: usize = 0xd0;

/// `FeOperatorBase` vtable slot 24 (`+0xc0`) -- show or hide one of an operator's screens.
///
/// `void slot24(this, u32 screen_id, bool show, float fade)`. Windows x64 puts those in `rcx`,
/// `edx`, `r8b` and `xmm3`, which is exactly what the game's own call site loads.
///
/// **This is read from a call site, not inferred from one.** `0x1405116f0` is the game's own
/// switch between the title and the loading screen, and it is a straight swap:
///
/// ```text
/// [param+2] == 1   Title.slot24(0x66, false, 0.0)   NowLoading.slot24(0x65, true,  0.0)
/// [param+2] != 1   Title.slot24(0x65, true,  0.0)   NowLoading.slot24(0x66, false, 0.0)
/// ```
///
/// So the ids are not "the loading screen" and "the title" -- both operators answer to both. The
/// id selects which of that operator's screens, and the operator supplies the content.
///
/// An earlier version of this constant named slot 4 and called it opacity, on the evidence that
/// the operator factory calls slot 4 twice with `0.0f` right after construction. That inference
/// was wrong: setting it to `1.0` at the title changed nothing on screen. Slot 4 is kept out of
/// this file entirely rather than left around to be believed again.
pub const FE_OPERATOR_SET_SCREEN_VTABLE_SLOT: usize = 24;

/// The screen id an operator is asked to SHOW when it takes over. `0x65`.
pub const FE_OPERATOR_SCREEN_ID_SHOW: u32 = 0x65;

/// The screen id an operator is asked to HIDE when it gives way. `0x66`.
pub const FE_OPERATOR_SCREEN_ID_HIDE: u32 = 0x66;

/// `FeOperatorTitle::v2`, the title operator's setup. RVA `0x000ef030`, VA `0x1400ef030`.
///
/// Thirty-two bytes: if `this+0x10` is non-null it runs two calls and writes `1` into
/// [`FE_OPERATOR_TITLE_ACTIVE`]. It is the moment the title frontend becomes live, which makes it
/// the earliest point at which covering the title is both possible and meaningful.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_OPERATOR_TITLE_SETUP: u32 = 0x000e_f030;

/// `FeTitleContext` -> `FeGroupTitleTopMenu`. `0x80`.
///
/// Read by `FeSubStateTitleTopMenu::v3` on its first two instructions --
/// `mov rax,[0x14160de10]; mov rbx,[rax+0x80]` -- and then polled through its own vtable. Not to be
/// confused with [`FE_TOP_MENU_GROUP_OFFSET`], which is where the same group hangs off
/// `FeSceneTitle` rather than off the title context.
pub const FE_TITLE_CONTEXT_TOP_MENU_GROUP_OFFSET: usize = 0x80;

/// `FeGroupBase::close(group)`. RVA `0x000f18b0`, VA `0x1400f18b0`.
///
/// Nineteen instructions, and the whole of the frontend's "make this screen go away":
///
/// ```c
/// scene = group->_0x08;
/// if (scene && group->_0x30) {          // only when the group is open
///     play_sequence(scene, 0x68, 0, 0.0f);
///     scene->_0x18 += 1;
///     group->_0x30 = 0;
/// }
/// ```
///
/// Its mirror is the open at `0x1400f1cb0`, which plays sequence `0x66` and decrements the same
/// counter. `ds2-dialog-skip` already plays `0x67` on `FeSceneTitle` for the settled state, so
/// `0x66`/`0x67`/`0x68` are one family: open, settled, close.
///
/// **This is what the `0x65`/`0x66` at `0x1405116f0` really were** -- sequence ids handed to a
/// scene, not screen ids handed to an operator. Reading them as operator arguments produced a call
/// through vtable slot 24 of an eleven-slot vtable, which read string data and crashed the game.
///
/// Called, never patched, so its Arxan status does not arise -- but it is clean at its entry
/// anyway.
pub const FE_GROUP_CLOSE: u32 = 0x000f_18b0;

/// `FeGroupBase` -> the byte that is `1` while the group is open. `0x30`.
///
/// Set by the open at `0x1400f1cf2` and cleared by the close at `0x1400f18e1`, and tested at the
/// head of both, so each is idempotent on its own. That is what makes closing a group safe to do
/// from a per-frame detour: the second call does nothing.
pub const FE_GROUP_OPEN_FLAG_OFFSET: usize = 0x30;

/// `FeGroupTitleTopMenu::close(group)`. RVA `0x000f3590`, VA `0x1400f3590`.
///
/// **The top menu does not use [`FE_GROUP_CLOSE`].** It has its own pair, and
/// `FeSubStateTitleTopMenu` names both of them by using them:
///
/// * `v1` (enter, `0x1400fde90`): `mov rcx,[0x14160de10]; mov rcx,[rcx+0x80]; call 0x1400f3820`
/// * `v2` (leave, `0x1400feb50`): the same two loads, then `call 0x1400f3590`
///
/// So this is literally what the game calls when the top menu goes away, on the pointer it reads
/// from the same global at the same offset.
///
/// HOW THE WRONG ONE WAS CAUGHT, because it is a cheap trick worth repeating: the first attempt
/// called [`FE_GROUP_CLOSE`] on this group and logged [`FE_GROUP_OPEN_FLAG_OFFSET`] beside it. The
/// data list read `1` -- a clean boolean -- and was hidden. The top menu read `248`, which is not a
/// boolean, and stayed visible. Logging the field a call depends on is what turned "it didn't
/// work" into "that byte is not what I think it is, so this is the wrong class".
pub const FE_GROUP_TITLE_TOP_MENU_CLOSE: u32 = 0x000f_3590;

/// `FeSubStateTitleTopMenu::v1` (enter). RVA `0x000fde90`, VA `0x1400fde90`.
///
/// Loads the top-menu group from `[`[`FE_TITLE_CONTEXT`]`]+0x80` and calls `0x1400f3820`, the
/// group's **open**. Its mirror is `v2` (leave, `0x1400feb50`), which calls
/// [`FE_GROUP_TITLE_TOP_MENU_CLOSE`].
///
/// **DO NOT CLOSE THE GROUP FROM HERE.** Measured: closing it immediately after this `enter`
/// returns took boot-to-top-menu from 4.3s to 29.9s and then killed the process at the top menu,
/// before the character list was ever reached. The substate's update polls this group through its
/// own vtable on every frame, and closing it the same frame it opened leaves that poll working on
/// a group that has been shut. Closing from the update instead is stable across runs.
///
/// Kept as a constant because it names the open/close pair, not because anything hooks it.
///
/// Not an Arxan redirect: clean prologue at the entry.
pub const FE_SUBSTATE_TOP_MENU_ENTER: u32 = 0x000f_de90;

// ============================================================================================
// POSING THE TITLE SCREEN, which is what "hide the menu with the logo" actually needs.
//
// The three constants below replace a whole line of failed reasoning, so the correction is worth
// stating once, at the top, rather than three times below.
//
// `[`FE_TITLE_CONTEXT`]` + 0x80 IS A `FeSceneTitle`. Two crates had it as two different things --
// `ds2-dialog-skip` called it "the title scene" and `ds2-continue` called it "the top menu group"
// -- and both were reading the same eight bytes. It is a `FeSceneTitle`, and its constructor at
// `0x140f3391` settles it without a single inference: that function writes vtable `0x1410bcab0`
// (RTTI `FeSceneTitle`, 17 virtuals) to `[this]`, zeroes `[this+0xb8]` -- which is
// [`FE_TOP_MENU_GROUP_OFFSET`], the top-menu group hanging off the scene, exactly as that constant
// already claimed -- and zeroes a WORD at `[this+0xf0]`, whose second byte is the `+0xf1` that the
// open sets and the close clears.
//
// So ONE object carries the logo, the PRESS ANY BUTTON prompt and the six menu rows, which is why
// substate 0x17 and substate 0x47 both poll it, and why the player experiences them as one screen.
// `0x1400f3820` is that scene's OPEN (not "play sequence 0x67" -- it also builds every row), and
// `0x1400f3590` is its CLOSE.
// ============================================================================================

/// `FeGroupBase::v2(this)` -- pose this object's scene hidden, instantly. RVA `0x00505d40`.
///
/// The whole function, and it is null-checked before it touches anything:
///
/// ```c
/// scene = this->_0x08;
/// if (!scene) return;
/// 0x140afdb70(scene);                                   // a query; its result is discarded
/// play(scene, 0x65, /*pose=*/1, 0.0f);                  // tail call
/// ```
///
/// # Why this and not the close
///
/// [`FE_GROUP_TITLE_TOP_MENU_CLOSE`] plays `0x68` with the pose flag CLEAR, so it *animates* out
/// over the sequence's own span. Measured on screen: with that close called every frame from the
/// top-menu update, the menu stayed visible for the whole of the substate's ~25ms residency --
/// because a fade that needs ~14 frames cannot finish in one or two. **The close was working and
/// the animation was the problem.**
///
/// This is the same play with the flag SET, which is the difference between "start fading" and "be
/// faded". See [`FE_SEQUENCE_PLAY_FLAG_POSE`] for where that flag lands.
///
/// # Why the base and not `FeSceneTitle`'s own override
///
/// `FeSceneTitle::v2` (`0x1400f4190`) is exactly `call 0x140505d40(this)` followed by
/// `inc [this->_0x08 + 0x18]`. That increment is a counter the open decrements and the close
/// increments, and re-asserting a pose every frame through the override would run it up without
/// bound -- and a player who returns to the title screen from in-game would find it never comes
/// back. Calling the base skips the counter, so this leaves **no persistent state behind at all**:
/// nothing to restore, and the game's own next open re-shows the screen by itself.
///
/// # Why it is safe from a per-frame detour, unlike the close
///
/// It does not clear the `+0xf1` open flag and does not run the teardown at `0x1400f41b0`. Those
/// two are what [`FE_SUBSTATE_TOP_MENU_ENTER`] records as having taken boot-to-top-menu from 4.3s
/// to 29.9s and then killed the process: the substate's update polls a group the close had shut.
/// Nothing here shuts anything -- it changes a playback position and nothing else.
///
/// Called, never patched, so its Arxan status does not arise.
pub const FE_SCENE_TITLE_POSE_HIDDEN: u32 = 0x0050_5d40;

/// The sequence [`FE_SCENE_TITLE_POSE_HIDDEN`] plays. `0x65`.
///
/// It completes the family the frontend already had names for -- `0x66` open, `0x67` settled,
/// `0x68` close -- as the state *before* an open, which is what makes it the hidden pose.
///
/// **This is the `0x65` from `0x1405116f0`**, the one whose `(0x65, true, 0.0)` triple was read as
/// operator arguments last session and called through vtable slot 24 of an eleven-slot vtable,
/// which read string data and crashed the game. The arguments were right all along; the receiver
/// was not. Here it arrives on the receiver the game's own code uses.
pub const FE_SCENE_TITLE_SEQUENCE_HIDDEN: i32 = 0x65;

/// The play's third argument, when the caller wants a pose rather than an animation. `1`.
///
/// Read out of `FeComponentSprite`'s slot 24 (`0x140b6c4f0`) rather than guessed, because the flag
/// is inverted on its way to the field it controls:
///
/// ```text
/// xmm6 = (float)entry.start + seek      // the seek is relative; see FE_TOP_MENU_SEQUENCE_FADED_SEEK
/// [this+0x40] = xmm6                    // the playback position
/// test dil,dil                          // dil is THIS flag
/// sete dl                               // dl = (flag == 0)
/// call [vtable+0xb0](this, dl)          // slot 22, "keep playing"
/// ```
///
/// So `0` means play on from here and `1` means hold here. Every animated play in the frontend
/// passes `0`; [`FE_SCENE_TITLE_POSE_HIDDEN`] is the one that passes `1`.
///
/// **This is what makes the seek irrelevant.** Posing the FIRST frame of the hidden sequence needs
/// no offset, so none of the sequence-span arithmetic that
/// [`FE_TOP_MENU_SEQUENCE_FADED_SEEK`] had to establish applies here -- and that arithmetic could
/// not have been done statically anyway, since the spans live in `GameDataEbl.bdt`.
pub const FE_SEQUENCE_PLAY_FLAG_POSE: i32 = 1;

/// `FeSceneTitle::open(scene)` -- what raises the title screen. RVA `0x000f3820`.
///
/// ```text
/// if (!this->_0xf1) {                       // not already open
///     scene = this->_0x08;
///     if (scene) {
///         scene->_0x18 -= 1;
///         play(scene, 0x67, 0, 0.0f);       // the settled pose
///         this->_0xf1 = 1;
///     }
/// }
/// ... ~1000 further bytes: component lookups, row construction, more plays ...
/// ```
///
/// # This is the same address as `FE_SCENE_TITLE_PLAY_IDLE`, which is misnamed
///
/// `ds2-dialog-skip` calls it as "play sequence `0x67` on the title scene". It is the screen's
/// whole open, rows and all, and `[title_skip] title_settle` defaults ON -- so **the mod itself
/// opens the title screen during substate `0x17`**, long before `FeSubStateTitleTopMenu` runs.
/// Tracked as `ds2-mods-rs-ebj`; the duplicate constant is left in place until that lands rather
/// than editing another crate's call site from here.
///
/// # Why this is the site to hook to hide the screen
///
/// MEASURED, from the log's own ordering rather than from reasoning about frames. The open logs at
/// line 49 (`settled screen=title-main`) and a pose driven from `FeSubStateTitleTopMenu`'s update
/// logs at line 60, with the entire network and dialog chain in between. The process windows cover
/// the screen for most of that gap, which is why it reads as a brief flash rather than seconds of
/// menu -- what is seen is the window between the last dialog clearing and substate `0x47`
/// arriving. Hooking the open closes that gap by construction: whatever raises the screen --
/// `title_settle` at `0x17`, or `FeSubStateTitleTopMenu::v1` at `0x47` -- is posed hidden on its
/// way out, so there is no interval in which it is up and un-posed.
///
/// # The one configuration this interacts with
///
/// Posing [`FE_SCENE_TITLE_SEQUENCE_HIDDEN`] leaves the scene's current sequence at `0x65`, and
/// the real gate at `0x1400f37f0` reports settled only for `0x67`. `ds2-dialog-skip` replaces that
/// gate outright when `title_sequence_skip` is on, which is the default, so the wait is already
/// forced and nothing observes the posed sequence. `title_settle` ON with `title_sequence_skip`
/// OFF is the combination that would wait forever at PRESS ANY BUTTON -- with `title_settle` off
/// the open does not happen until `0x47`, by which time the gate is long past.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the clean prologue
/// `48 89 74 24 20` at the entry.
pub const FE_SCENE_TITLE_OPEN: u32 = 0x000f_3820;

/// `SaveLoadSystem`'s save-directory builder -- what produces the folder the `.sl2` lives in.
/// RVA `0x00248db0`.
///
/// ```text
/// FUN_140248db0(std::wstring *out, const wchar_t *subdir)
///     out  = "%APPDATA%\\DarkSoulsII\\"        // via SAVE_APPDATA_ROOT_BUILD
///     out += subdir                            // the Steam ID, as text
///     out += "\\"                              // DAT_1410d04f8, a lone backslash
/// ```
///
/// # Why this site and not the one it calls
///
/// [`SAVE_APPDATA_ROOT_BUILD`] is the wider chokepoint -- it is the only thing in the image that
/// turns `SHGetFolderPathW(CSIDL_APPDATA)` into a DARK SOULS II path -- but it is wider than the
/// job. Its other caller, `FUN_140248d80`, appends `GraphicsConfig_SOFS.xml`, so a detour there
/// moves the graphics config as well as the saves. This function has exactly two callers and both
/// are `SaveLoadSystem` methods (`FUN_1402e6230_saveLoadSetup__` at `0x1402e635c`,
/// `FUN_1402e67f0` at `0x1402e6930`), so hooking it reaches the saves and nothing else.
///
/// # The second argument is the Steam ID, established from the call site
///
/// At `0x1402e6331` the caller makes a virtual call through slot `+0x38` -- the same slot
/// `FUN_140af14e0` uses to fill the cached Steam ID at `DAT_1416681a8` -- converts the result to a
/// string, and hands its character data to this function in `rdx`:
///
/// ```text
/// 0x1402e634a:  cmp    QWORD PTR [rbp-0x19],0x8      // the wstring's capacity field
/// 0x1402e634f:  lea    rdx,[rbp-0x31]                // ... so rdx is the inline buffer,
/// 0x1402e6353:  cmovae rdx,QWORD PTR [rbp-0x31]      // ... or the heap pointer when it spilled
/// 0x1402e6358:  lea    rcx,[rbp-0x1]                 // the out string
/// 0x1402e635c:  call   0x140248db0
/// ```
///
/// That is why the observed layout is `…\DarkSoulsII\<steamid hex>\DS2SOFS0000.sl2` with the
/// graphics config a level above it, and it is why a detour here owns the Steam ID folder too: a
/// redirect can point at a donor save's own folder name instead of renaming it to the running
/// account's.
///
/// **A replacement must end in a backslash.** The caller appends the file name to whatever this
/// leaves behind, and the trailing separator is this function's job, not the caller's.
///
/// Not an Arxan redirect: `scripts/ds2-arxan-chain.py` terminates at hop 0 with the clean prologue
/// `48 89 5c 24 08` at the entry.
pub const SAVE_DIR_BUILD: u32 = 0x0024_8db0;

/// `%APPDATA%\DarkSoulsII\` -- the root every DS2 user path is built on. RVA `0x00248e80`.
///
/// `SHGetFolderPathW(0, 0x1a /* CSIDL_APPDATA */, 0, 0)`, then append the literal
/// `L"\\DarkSoulsII\\"` at `0x1410d04a8`. Recorded because it anchors [`SAVE_DIR_BUILD`] and
/// because it is the site to hook if the graphics config should move as well -- it is deliberately
/// NOT the site this crate hooks. See [`SAVE_DIR_BUILD`] for why.
///
/// Not an Arxan redirect: clean prologue `48 89 5c 24 10` at the entry.
pub const SAVE_APPDATA_ROOT_BUILD: u32 = 0x0024_8e80;

/// `std::wstring::assign(dst, src, len)` -- the game's own assign, in its own CRT. RVA `0x000260b0`.
///
/// Reused rather than reimplemented, and that is the point: the out-parameter of
/// [`SAVE_DIR_BUILD`] is a live MSVC `std::basic_string<wchar_t>` owned by the caller, which may
/// already hold a heap allocation from the game's allocator. Writing its fields by hand would
/// leak that allocation or free it with the wrong allocator; calling the game's assign hands both
/// problems back to the code that owns them. It is the same function
/// `SAVE_APPDATA_ROOT_BUILD` itself calls to seat the `SHGetFolderPathW` result.
///
/// `len` is in `wchar_t`, not bytes, and excludes the terminator.
pub const WSTRING_ASSIGN: u32 = 0x0002_60b0;

/// Byte offset of the length field in the game's `std::wstring`. Length is in `wchar_t`.
///
/// Read out of `FUN_140043050`/`FUN_1400260b0`, which index `_x10_strLen_` at this offset and
/// `_x18_strCapacity_` at [`WSTRING_CAPACITY_OFFSET`], and treat the first sixteen bytes as a
/// union of an inline buffer and a pointer. That is stock MSVC small-string optimisation.
pub const WSTRING_LEN_OFFSET: usize = 0x10;

/// Byte offset of the capacity field in the game's `std::wstring`.
///
/// The discriminant for the small-string union: at or below [`WSTRING_SSO_MAX`] the characters
/// live inline at offset 0, above it offset 0 is a pointer to them. Both string helpers branch on
/// exactly this, and so does the call site documented on [`SAVE_DIR_BUILD`].
pub const WSTRING_CAPACITY_OFFSET: usize = 0x18;

/// Largest capacity that still lives in the inline buffer -- seven `wchar_t` plus a terminator.
///
/// The helpers spell this as `if (7 < capacity) { use the pointer }`, and the disassembled call
/// site as `cmp QWORD PTR [rbp-0x19],0x8` / `cmovae`. Recorded so a reader of a live string does
/// not have to rediscover which side of the comparison is the heap.
pub const WSTRING_SSO_MAX: usize = 7;

// ---------------------------------------------------------------------------------------------
// THE IN-GAME (PAUSE) MENU'S TAB ITEM LISTS
//
// `FeGroupInGameTopSelect` (ctor RVA `0x000a41b0`) owns six `FeGroupInGameGroupSelect` members --
// the six tabs. Each tab's contents are a `DLKR::DLFixedVector` of 8-byte entries built by a
// dedicated function, one per tab, called from that ctor:
//
//   RVA        entries (action, gate)              what the actions open
//   0x000a4990 (0,0)                               FeGroupInGameMenuEquipTop
//   0x000a4db0 (1,0)                               FeGroupInGameMenuInventory2
//   0x000a5620 (2,0) (3,0)                         FeGroupInGameMenuStatusStatus / ...StatusInfo
//   0x000a4fc0 (4,1) (5,2) (6,3)                   FeGroupIngameMessageWrite / ReadHistory / WriteHistory
//   0x000a5900 (7,0) (8,0) (9,4)                   SystemSettingGame / SystemSettingScreen / ReturnTitleCheck
//   0x000a5330 (0xb,0) (0xc,0)                     SystemSettingKeyboard / SystemSettingGraphic
//
// Read out of the six builders and out of the dispatch switch [`FE_INGAME_MENU_DISPATCH`], with
// each destination class confirmed by walking its vtable back to its RTTI type descriptor. None of
// the six builders, the ctor, the dispatch or the per-tab init is an Arxan redirect --
// `scripts/ds2-arxan-chain.py` terminates at hop 0 with a clean prologue on every one of them.
// ---------------------------------------------------------------------------------------------

/// The builder for the tab that carries the quit item. RVA `0x000a5900`.
///
/// `FeGroupInGameTopSelect`'s ctor calls this once, with a stack descriptor in RCX, and it is the
/// ONLY caller -- one candidate RIP-relative reference in the whole image (`0x1400a4382`, inside
/// that ctor), so a detour here reaches this tab and nothing else. It returns its argument in RAX.
///
/// It zeroes the count at [`FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET`] and then pushes three
/// entries -- [`FE_INGAME_MENU_SYSTEM_TAB_ITEMS`]. The entry the player calls "quit" is the third,
/// action [`FE_INGAME_MENU_ACTION_RETURN_TITLE`], which the dispatch resolves to
/// `FeInGameMenuWarehouse + 0x6f10`, the `FeGroupInGameReturnTitleCheck` the warehouse's ctor
/// (`0x1400991e0`) constructs at member `+0xde2`.
///
/// Not an Arxan redirect: clean prologue [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE`] at the
/// entry.
pub const FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS: u32 = 0x000a_5900;

/// The first six bytes of [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`]: `rex push rbx` /
/// `sub rsp,0x50`.
///
/// Recorded so a detour can REFUSE rather than patch when the bytes are not these. That is the
/// only defence against this table being read against a different build: an RVA is just a number
/// and will happily point into the middle of some other function.
pub const FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS_PROLOGUE: [u8; 6] =
    [0x40, 0x53, 0x48, 0x83, 0xec, 0x50];

/// The dispatch: `switch (action)` over every in-game menu item. RVA `0x000a6090`.
///
/// Reached from the tab's confirm handler (`0x1400a6b10`), which reads the entry under the cursor,
/// applies its gate, and passes either the action or `-1`. `-1` selects a different sound id and
/// falls through the switch's `default`, which is how a gated row refuses.
///
/// Cases 0-9, 0xb, 0xc and **0xd** are present. Two shapes: a direct
/// `FeInGameMenuWarehouse` member (0, 2, 4, 5, 6, 9), or a `FexDynamicGroupExecJob` carrying a
/// *kind* that the factory at `0x1400a67c0` turns into a freshly allocated group (3 -> kind 0,
/// 7 -> 2, 8 -> 3, 0xb -> 4, 0xc -> 5, 0xd -> 6). Action `0xa` has no case at all.
///
/// Recorded for provenance and because it is the site to extend for a genuinely new action. Not
/// currently hooked by anything.
///
/// Not an Arxan redirect: clean prologue `48 89 5c 24 18` at the entry.
pub const FE_INGAME_MENU_DISPATCH: u32 = 0x000a_6090;

/// The per-tab init that turns the item list into rows. RVA `0x000a4d20`.
///
/// It binds the grid to the layout ([`FEX_GRID_CONTROL_LAYOUT_BIND`]), then calls
/// `FUN_140021b30(tab, tab->count)`, then the availability pass `0x1400a77c0`, which walks
/// `0..count`, reads entry `i`, and greys the row whose gate refuses.
///
/// **`FUN_140021b30` sets the count the CURSOR is bounded by, not the number of drawable cells.**
/// A run on 2026-08-28 appended a fourth entry to the quit tab and got exactly that: a fourth item
/// the cursor reaches and that responds, with nothing drawn for it. The drawable cells were already
/// fixed by the bind on the line above. This comment used to claim the visible row count came from
/// here; it does not, and `docs/DS2-INGAME-MENU.md` keeps the wrong version beside the right one.
///
/// Not hooked. Not an Arxan redirect: clean prologue `40 53 48 81 ec b0 00 00 00` at the entry.
pub const FE_INGAME_MENU_TAB_INIT: u32 = 0x000a_4d20;

/// `FrontendEx::FexGridControl`'s layout bind -- where a grid's drawable cells come from. RVA
/// `0x000216d0`.
///
/// It takes no extent from anywhere. It DISCOVERS one, by asking the layout for the element at
/// each `(col, row)` and stopping a row at the first one that comes back null:
///
/// ```text
/// for (row = 0; row < 15; row++)
///   for (col = 0; col < 32; col++) {
///       element = (*namer->vtable[0x10])(col, row);
///       if (element == 0) break;                        // this row ends here
///       cell = FUN_14010a060(...);                      // a drawable cell object
///       grid[FEX_GRID_COL_EXTENT_OFFSET] = max(that, col + 1);
///       grid[FEX_GRID_ROW_EXTENT_OFFSET] = max(that, row + 1);
///   }
/// ```
///
/// So the extents are a count of AUTHORED LAYOUT ELEMENTS, not a constant to raise, and the probe
/// stops at the first hole -- authoring cell 4 without cell 3 would find neither. `0x140022160`
/// closes the loop from the other side: resolving a cell whose column equals the column extent
/// takes the `vtable+0x48` one-past-the-end naming branch instead of the ordinary-cell branch.
///
/// Recorded because it is the answer to "why is the appended row invisible", and because it is
/// where anyone extending a menu with new layout data has to look. Not hooked.
pub const FEX_GRID_CONTROL_LAYOUT_BIND: u32 = 0x0002_16d0;

/// Byte offset of a `FexGridControl`'s logical item count -- what the cursor may reach.
///
/// Written by `FUN_140021b30`, read by the grid's `v34` (`0x14001c020`, `return this->+0x38` on the
/// `FexGroupList` base subobject, which lands here on the whole object). This is the field an
/// appended item moves.
pub const FEX_GRID_ITEM_COUNT_OFFSET: usize = 0xc8;

/// Byte offset of a `FexGridControl`'s COLUMN extent -- how many drawable cells the layout gave it.
///
/// Set only by [`FEX_GRID_CONTROL_LAYOUT_BIND`], as a running `max` over the elements it found.
/// `FUN_140021b30` never touches it, which is the whole gap between an item being selectable and
/// an item being visible.
pub const FEX_GRID_COL_EXTENT_OFFSET: usize = 0xd4;

/// Byte offset of the scroll state a `FexGridControl` drives. RVA-relative to the grid.
///
/// `FUN_140021b30` reads it as `grid[0x1c]` and writes the total at `+0x2c`, compares that total
/// against `+0x28`, and takes one of two branches: total at or below `+0x28` plays sequence `0x7a`
/// on the object at `[scroll]`, above it plays `0x70` and computes a thumb size from `+0x44`.
/// That is a scrollbar being hidden or shown, which is what makes "how many cells are VISIBLE"
/// a different number again from the extent and the item count.
pub const FEX_GRID_SCROLL_OFFSET: usize = 0xe0;

/// Within the scroll object: the number of cells on screen at once.
pub const FEX_GRID_SCROLL_VISIBLE_OFFSET: usize = 0x28;

/// Within the scroll object: the total the count above is compared against.
pub const FEX_GRID_SCROLL_TOTAL_OFFSET: usize = 0x2c;

/// Byte offset of a `FexGridControl`'s ROW extent. Set the same way as
/// [`FEX_GRID_COL_EXTENT_OFFSET`].
///
/// `1` means a single-line list, and `0x1400222c0` special-cases it: index `n` maps to
/// `(col = n, row = 0)`. The in-game menu tabs are that shape, so their items run along the column
/// axis and [`FEX_GRID_COL_EXTENT_OFFSET`] is the one that bounds them.
pub const FEX_GRID_ROW_EXTENT_OFFSET: usize = 0xd8;

/// Byte offset of the element count inside a tab's item `DLFixedVector`.
///
/// Every one of the six builders opens with `mov QWORD PTR [rcx+0x30], 0`, and the copy into the
/// live group (`0x1400a3ef0`) reads and writes the same field. In the constructed group the pair
/// lands at `+0xf8` (elements) and `+0x128` (count), which is `0xf8 + 0x30` -- the same struct.
pub const FE_INGAME_MENU_ITEM_VECTOR_COUNT_OFFSET: usize = 0x30;

/// Most entries a tab's item vector can hold.
///
/// Spelled by the builders as `if (5 < newCount) panic("out of memory.")` against
/// `DLFixedVector.inl:0x24c`, and independently by the copy at `0x1400a3ef0`, which panics unless
/// the source count is `< 6`. Both agree: five.
pub const FE_INGAME_MENU_ITEM_VECTOR_CAPACITY: usize = 5;

/// Size of one item entry: a `u32` action id followed by a `u32` gate index.
///
/// The builders write a whole 8-byte slot per push (`*puVar1 = 0x400000009` for the quit entry --
/// action `9`, gate `4`), and the two readers split it: the confirm path takes the action from
/// `[entry]` and the gate from `[entry+4]` (`lea rcx,[rax+4]` at `0x1400a4cce`).
pub const FE_INGAME_MENU_ITEM_STRIDE: usize = 8;

/// The three entries [`FE_INGAME_TOP_SELECT_SYSTEM_TAB_ITEMS`] is expected to leave behind, as
/// `(action, gate)` pairs.
///
/// Held so a detour can CHECK what the original produced before touching it. Without that, a
/// build whose tabs are ordered differently would be silently modified in the wrong place, and
/// the resulting screenshot would be evidence about nothing.
pub const FE_INGAME_MENU_SYSTEM_TAB_ITEMS: [(u32, u32); 3] = [
    (
        FE_INGAME_MENU_ACTION_SETTING_GAME,
        FE_INGAME_MENU_GATE_ALWAYS,
    ),
    (
        FE_INGAME_MENU_ACTION_SETTING_SCREEN,
        FE_INGAME_MENU_GATE_ALWAYS,
    ),
    (
        FE_INGAME_MENU_ACTION_RETURN_TITLE,
        FE_INGAME_MENU_GATE_RETURN_TITLE,
    ),
];

/// Action `7` -- `FeGroupInGameSystemSettingGame`, via dynamic-group kind 2.
pub const FE_INGAME_MENU_ACTION_SETTING_GAME: u32 = 7;

/// Action `8` -- `FeGroupInGameSystemSettingScreen`, via dynamic-group kind 3.
pub const FE_INGAME_MENU_ACTION_SETTING_SCREEN: u32 = 8;

/// Action `9` -- `FeGroupInGameReturnTitleCheck`. **This is the quit item.**
///
/// The dispatch resolves it to `warehouse + 0x6f10`, and the warehouse's ctor builds
/// `FeGroupInGameReturnTitleCheck` (vtable `0x1410b1768`, ctor `0x14006e390`) at exactly that
/// member. It is the confirm dialog that offers to save on the way to the title screen.
pub const FE_INGAME_MENU_ACTION_RETURN_TITLE: u32 = 9;

/// Action `0xd` -- present in the dispatch, listed by **no** tab.
///
/// Its factory branch shares a `case` label with kind 4: `case 4: case 6:` both allocate `0xc68`
/// bytes and call `FUN_1400803b0`, whose vtable writes name `FeGroupInGameSystemSettingKeyboard`.
/// The job carries the kind ONLY to select that branch (`mov edx,[rbx+0x28]` at `0x14002d884`,
/// its one and only read), so executing action `0xd` is byte-for-byte what executing the shipped
/// Key Bindings row already does every time it is pressed.
///
/// That is the entire reason it is the probe's payload: it adds a row without adding a code path.
pub const FE_INGAME_MENU_ACTION_KEY_BINDINGS_UNUSED: u32 = 0x0d;

/// Gate `0` -- no gate. The predicate at `0x1400a4e50` returns `0` (selectable) immediately on it.
pub const FE_INGAME_MENU_GATE_ALWAYS: u32 = 0;

/// Gate `4` -- the one the quit item carries.
///
/// Resolves the session object at `GameManagerImp + 0x22f0` through `FUN_140513270` and asks
/// `FUN_14025f690` about it; a nonzero answer means the row is refused. Neither callee is named in
/// the project yet, so what it actually forbids is NOT recorded here -- only that this is the gate
/// the shipped quit row uses.
pub const FE_INGAME_MENU_GATE_RETURN_TITLE: u32 = 4;

// ---------------------------------------------------------------------------------------------
// QUITTING TO DESKTOP
// ---------------------------------------------------------------------------------------------

/// `FeSubStateTitleShutdown::v1` (enter) -- the game's own quit-to-desktop, in full. RVA
/// `0x000fde20`.
///
/// Three instructions, and there is no fourth:
///
/// ```text
/// mov rax, QWORD PTR [rip+0x15773d1]      ; [FE_SYSTEM_SINGLETON]
/// mov BYTE PTR [rax+0x13a], 1             ; FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET
/// ret
/// ```
///
/// Its `update` (`0x1400ff2e0`) is an empty `ret`, so the substate does not run the shutdown -- it
/// only asks for one. Recorded because it is the whole implementation of "quit to desktop" and it
/// is a byte, not a call.
pub const FE_SUBSTATE_TITLE_SHUTDOWN_ENTER: u32 = 0x000f_de20;

/// The `FeSystem`-ish singleton pointer the title flow reads everything off. RVA `0x016751f8`.
///
/// Already relied on elsewhere in this repo without being named: `FeSubStateTitleLogo`'s skip
/// tests a state word reached through it, and `FeSubStateWarningNoCopy`'s shipped early-out calls
/// one of its virtuals. It holds a POINTER; dereference it before adding an offset.
pub const FE_SYSTEM_SINGLETON: u32 = 0x0167_51f8;

/// Byte offset of the shutdown request inside [`FE_SYSTEM_SINGLETON`]'s target.
///
/// **Setting it to 1 quits the game, and that is the entire mechanism.** Four sites in the image
/// write it (`0x1400f4a9a`, `0x1400fbc00`, `0x1400fde23`, `0x1401c2303`) and exactly two read it
/// (`0x1401bf97e`, `0x1401c0196`) -- both inside `GameManagerImp`'s per-frame master update, the
/// function that also drives `mapManUpdate`, `damageManUpdate`, `bulletManUpdate`, `demoManager`
/// and `saveRequest`.
///
/// Being polled by the main loop is what makes this usable from anywhere: a write takes effect on
/// the next frame, through the game's own shutdown, with no confirmation dialog and no new code
/// path. It is the same byte the title screen's own exit row writes.
///
/// It is NOT a save. The game's quit-to-title flow offers to save because that flow asks; this
/// does not, which is what "without a confirmation" costs.
pub const FE_SYSTEM_SHUTDOWN_REQUEST_OFFSET: usize = 0x13a;

/// The first action id this repo hands out. Slot `n` gets `BASE + n`. Deliberately outside the
/// game's own space.
///
/// The shipped dispatch has cases `0..=9`, `0xb`, `0xc`, `0xd`. Anything else falls to `default`,
/// which plays the ordinary confirm sound and does nothing. That is the correct failure mode for
/// an id whose behaviour lives in a detour: if the detour is ever absent, the row is INERT rather
/// than quietly doing whatever the game does for some id we borrowed.
///
/// The range is `0x1000..` and the ceiling on rows is five per tab, so the ids stay nowhere near
/// anything the game uses no matter how many tabs are eventually measured.
pub const FE_INGAME_MENU_ACTION_BASE: u32 = 0x1000;

/// `FrontendEx::FexGridControl` linear-index -> `(col, row)`. RVA `0x000222c0`.
///
/// `if (grid->rowExtent == 1) { col = index; row = 0; } else { row = index / cols; col = index % cols; }`.
/// Measured at runtime, the in-game menu tabs are one COLUMN by N ROWS -- every tab this repo has
/// looked at reports `col-extent = 1` and `row-extent = itemCount` -- so they take the second
/// branch and an item's cell is `(0, index)`.
pub const FEX_GRID_INDEX_TO_CELL: u32 = 0x0002_22c0;

/// `FrontendEx::FexGridControl` `(col, row)` -> element accessor. RVA `0x00022160`.
///
/// Asks the namer at `[grid + 0xf0]` through one of five vtable slots, picked by whether the cell
/// is on an edge: `+0x30` when `row == -1`, `+0x38` when `row == rowExtent`, `+0x40` when
/// `col == -1`, `+0x48` when `col == colExtent`, and `+0x10` for an ordinary interior cell.
///
/// **The `col == colExtent` branch is the one an appended item takes**, which is why its element
/// resolves to nothing: it is the "one past the end" namer, not the ordinary-cell namer.
pub const FEX_GRID_CELL_TO_ELEMENT: u32 = 0x0002_2160;

// ---------------------------------------------------------------------------------------------
// THE CELL NAMER, AND WHY A FOURTH ROW MIGHT BE FOUR BYTES
// ---------------------------------------------------------------------------------------------

/// The quit tab's cell-namer constructor. RVA `0x000a5b50`.
///
/// Read from the disassembly, because the decompiler drops half of it. It builds a FOUR-component
/// base path and then a SIX-slot array of cell ids of which three are zero:
///
/// ```asm
/// mov  r9d, 0x1eace8
/// lea  r8d, [r9-0x19]                  ; 0x1eaccf
/// mov  edx, 0x1eaba9
/// mov  DWORD PTR [rsp+0x20], 0x1eace6  ; -> path [0x1eaba9, 0x1eaccf, 0x1eace8, 0x1eace6]
/// mov  DWORD PTR [rsp+0xc0], 0x1eacc9
/// mov  DWORD PTR [rsp+0xc4], 0x1eacca
/// mov  QWORD PTR [rsp+0xc8], 0x1eace9  ; and the pad dword behind it
/// mov  QWORD PTR [rsp+0xd0], rbx       ; zero, zero
/// ...  cmp rcx, 6 ; jb                 ; the loop already runs SIX times
/// ```
///
/// Each non-zero id becomes one entry pushed into the namer's list at `+0x18`, and that list is
/// what `FEX_GRID_CELL_TO_ELEMENT` indexes. **So a fourth cell is a four-byte constant here, not a
/// layout edit -- provided an element exists for the id to resolve to.** Whether one does is the
/// question `ds2-menu-row`'s enumerator answers.
///
/// Both cells the tab gained after the fact are out of sequence -- `c9, ca, ... e9` here and
/// `7b, 7c, ... d5` on tab 3 -- which is what appending a row to a shipped tab looks like.
pub const FE_INGAME_MENU_QUIT_TAB_NAMER: u32 = 0x000a_5b50;

/// The quit tab's cell base path, as the four ids the namer builds it from.
pub const FE_QUIT_TAB_BASE_PATH: [u32; 4] = [0x001e_aba9, 0x001e_accf, 0x001e_ace8, 0x001e_ace6];

/// The three cell ids the quit tab ships with, appended to [`FE_QUIT_TAB_BASE_PATH`].
pub const FE_QUIT_TAB_CELL_IDS: [u32; 3] = [0x001e_acc9, 0x001e_acca, 0x001e_ace9];

/// `FrontendEx` scene path builder, four ids. RVA `0x000756a0`.
///
/// `fn(out, id0, id1, id2, id3) -> out`, with the fourth id on the stack at `[rsp+0x20]`.
pub const FE_SCENE_PATH_BUILD4: u32 = 0x0007_56a0;

/// Converts a built path into the form the append below takes. RVA `0x0001f8a0`. `fn(path, out)`.
pub const FE_SCENE_PATH_SEAL: u32 = 0x0001_f8a0;

/// Appends one id to a sealed path, producing an element accessor. RVA `0x0001ed80`.
///
/// `fn(path, out, id) -> out`. The accessor's own vtable slot 0 resolves it: non-zero is the live
/// element, zero means the scene has nothing at that path. That resolve is exactly what the grid's
/// layout bind uses to decide whether a cell exists, so asking it is asking the same question the
/// bind asks.
pub const FE_SCENE_PATH_APPEND: u32 = 0x0001_ed80;

/// The quit tab's cell ids that the LAYOUT authors and the namer never lists.
///
/// `FeSceneInGameMenu`'s element cache (`0x140099f90`) resolves five cell-shaped children under
/// `[0x1eaba9, 0x1eaccf, 0x1eace8, 0x1eace7]`, each followed by its own label element:
///
/// ```text
/// 0x1eacc9 + label 0x1eac46      Game Options
/// 0x1eacca + label 0x1eac47      Screen Settings
/// 0x1eaccd + label 0x1eac4a      <- authored, never listed
/// 0x1eacce + label 0x1eac4b      <- authored, never listed
/// 0x1eace9 + label 0x1eac4c      Quit Game
/// ```
///
/// **THAT PARAGRAPH USED TO SAY THE TAB WAS AUTHORED FOR FIVE ROWS. It was not measured and it is
/// probably false.** The cache ASKS for five ids; `FUN_140afda00` reaches `FUN_140b507d0`, which is
/// a plain lookup returning 0 when nothing is there, and the cache stores the answer without
/// checking it. So five requests is evidence of five requests.
///
/// What was measured afterwards, with controls: a cloned namer entry becomes a real cell, and
/// clones naming `0x1eaccd` or `0x1eacce` do not resolve while an unmodified clone does. The two
/// spares are therefore absent from the container the namer can reach, and most likely absent
/// full stop -- leftovers from a five-row design that did not ship.
///
/// The namer builds its cell paths under `0x1eace6` while the cache resolves under `0x1eace7` --
/// two sibling containers below `0x1eace8`, with the same cell ids in each. The pair is presumably
/// an interaction layer and a drawing layer.
pub const FE_QUIT_TAB_SPARE_CELL_IDS: [u32; 2] = [0x001e_accd, 0x001e_acce];

/// Pushes one built accessor onto a cell namer's list at `namer + 0x18`. RVA `0x000a7b30`.
///
/// `fn(&namer[0x18], accessor)`. This is the call [`FE_INGAME_MENU_QUIT_TAB_NAMER`] makes once per
/// non-zero id in its six-slot array, so appending a fourth entry is the same operation the game
/// performs three times on the way in.
pub const FE_SCENE_NAMER_PUSH: u32 = 0x000a_7b30;

/// Byte offset of the cell list inside a cell namer.
pub const FE_SCENE_NAMER_LIST_OFFSET: usize = 0x18;

/// Most entries a cell namer's list at [`FE_SCENE_NAMER_LIST_OFFSET`] can hold.
///
/// Measured the expensive way: a run that pushed six extra entries onto a list the game had already
/// put three in wrote through a null on the seventh and killed the game at `0x141bee1c4`
/// (`access_kind=1`, `rcx=rdx=rax=0`). Three plus three is fine; three plus four is not.
///
/// Six is also the bound of the id loop in [`FE_INGAME_MENU_QUIT_TAB_NAMER`], so the array and the
/// list it fills are the same size -- which is the sort of agreement worth writing down, because
/// it says the spare slots in that array are genuinely usable rather than accidental padding.
pub const FE_SCENE_NAMER_LIST_CAPACITY: usize = 6;

/// Byte offset of the count inside a cell namer's list, relative to the list itself.
///
/// `FUN_1400a7b30` reads `*(u64*)(list + 0x128)`, refuses above
/// [`FE_SCENE_NAMER_LIST_CAPACITY`], and writes the incremented value back before copying the new
/// element in. Relative to the namer that lands at `+0x140`, which is exactly the field
/// `VLayoutAdapter`'s cell lookup (`0x1400a4b20`) compares the requested row against.
pub const FE_SCENE_NAMER_COUNT_OFFSET: usize = 0x128;

/// Bytes per entry in a cell namer's list. `FUN_1400a7b30` addresses element `n` at
/// `list + (-list & 7) + n * 0x30`, and `0x1400a4b20` reads it back at the same stride.
pub const FE_SCENE_NAMER_ENTRY_STRIDE: usize = 0x30;

/// A cell namer entry, as dumped from two live entries of the quit tab's list.
///
/// ```text
/// e0: a9ab1e00 cfac1e00 e8ac1e00 e6ac1e00 c9ac1e00 <slack> 05000000
/// e1: a9ab1e00 cfac1e00 e8ac1e00 e6ac1e00 caac1e00 <slack> 05000000
///      +0x00    +0x04    +0x08    +0x0c    +0x10            +0x28
/// ```
///
/// Five `u32` ids, then uninitialised slack, then the path LENGTH. The slack genuinely differs
/// between two entries the game built one after the other -- it is stack residue -- which is why a
/// clone must copy an entry rather than be assembled field by field, and why a byte-diff has to
/// ignore everything outside the fields named here.
pub const FE_SCENE_NAMER_ENTRY_CONTAINER_OFFSET: usize = 0x0c;
pub const FE_SCENE_NAMER_ENTRY_ID_OFFSET: usize = 0x10;
pub const FE_SCENE_NAMER_ENTRY_LEN_OFFSET: usize = 0x28;
/// The path length every quit-tab entry carries: root, region, `ace8`, container, cell.
pub const FE_SCENE_NAMER_ENTRY_LEN: u32 = 5;
/// The sibling container the scene's element cache resolves the five-cell set under, where the
/// namer's own entries use [`FE_QUIT_TAB_BASE_PATH`]`[3]`.
pub const FE_QUIT_TAB_CACHE_CONTAINER: u32 = 0x001e_ace7;

// ---------------------------------------------------------------------------------------------
// The frontend layout document (`.flo`), and the container the quit tab's rows hang off.
//
// This is the section that turned the "invisible fourth row" from a file-repacking project into a
// four-pointer edit. The pause menu's rows are records in `menu/02.febnd.dcx`'s
// `l02_01_In-Game.flo`, and the game loads that file IN PLACE -- the header IS the document
// object, and the `u64` file offsets inside it are absolute pointers once the fixup has run. So
// the table that says how many rows a container has can be replaced with a copy that says one
// more, without touching the archive, the DCX, the BND4 or a single byte on disk.
//
// Reproduce every number below with:
//
//     python3 scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02
//     python3 scripts/ds2-flo.py tree /tmp/menu02/l02_01_In-Game.flo --def 0x263
//
// THE FILE IS `/menu/02.febnd.dcx`, NOT `/menu/42.febnd.dcx`. 42 is the OPTIONS screen
// (`l42_01_OptionSetting.flo`) and shares none of these ids; an earlier note in this repo pointed
// at it and cost a wrong search.
// ---------------------------------------------------------------------------------------------

/// `FeLayoutDocument::findDefinition(doc, index)`. RVA `0x00b54740`.
///
/// `fn(&doc, u32 index) -> *definition`. A linear scan of `[[doc]+0x18]` over `[[doc]+0x4c]`
/// entries at stride [`FLO_DEFINITION_STRIDE`], keyed by the `u16` at the definition's `+0x00`;
/// returns null on a miss. Prologue `48 8b 01 44 8b ca 48 85 c0` -- its own, not one of the 286
/// Arxan redirects.
///
/// **Every consumer of a definition goes through here**, which is what makes one detour enough:
/// the builder reads the child count and the child array out of whatever this hands back, and the
/// built container keeps the same pointer at its `+0x48` for the capacity check below.
pub const FLO_FIND_DEFINITION: u32 = 0x00b5_4740;

/// Bytes per definition. `FUN_140b54740`: `add rcx, 0x48`.
pub const FLO_DEFINITION_STRIDE: usize = 0x48;

/// `u16` child count inside a definition. `FUN_140b50f20` walks that many child records --
/// **and `FUN_140b6bd80` uses the same field as the display list's CAPACITY**, refusing to attach
/// a child once `parent+0x66` reaches it. One field, both meanings, so raising it raises both.
pub const FLO_DEFINITION_CHILD_COUNT_OFFSET: usize = 0x02;

/// Pointer to a definition's child record array. A file offset on disk, an absolute pointer once
/// the document is loaded.
pub const FLO_DEFINITION_CHILDREN_OFFSET: usize = 0x08;

/// Bytes per child record. `FUN_140b50f20`: `lVar8 = lVar8 + 0x28`.
pub const FLO_RECORD_STRIDE: usize = 0x28;

/// `u16` definition index a record instantiates -- the argument [`FLO_FIND_DEFINITION`] takes.
pub const FLO_RECORD_DEFINITION_OFFSET: usize = 0x00;

/// Pointer to a record's transform block. Read by `FUN_140b50bc0` as `*(float**)(rec+0x08)`.
pub const FLO_RECORD_TRANSFORM_OFFSET: usize = 0x08;

/// `u16` depth. Passed on only for the leaf kinds; a nested record's copy is inert, and the draw
/// order of siblings follows the order they are attached in.
pub const FLO_RECORD_DEPTH_OFFSET: usize = 0x10;

/// `u16` kind flags, the value `FUN_140b50bc0` switches on: `1` shape, `2` mask, `4` a nested
/// definition, **`8` text**. A FLAG WORD rather than an enum -- the builder masks it with `0xd` and
/// records carrying `0x1004` exist, so the bits above the low nibble mean something unread. Every
/// quit-tab row is plain `4`, and this crate only ever copies the field.
///
/// **THIS SAID "`2` TEXT, `8` TEXTURE" UNTIL 2026-08-28 AND IT WAS WRONG BOTH WAYS.** The check
/// that settles it needs no disassembly: `scripts/ds2-flo.py tree l02_01_In-Game.flo --def 0x22c`
/// walks the caption mark down to the leaf `caption.rs` writes row labels into -- an element whose
/// kind is not in question, because this repo already puts text in it -- and prints `kind=0x8`.
///
/// The derivation, for the version that does need disassembly: each bit selects a table and a
/// builder, the builder calls a constructor, the constructor writes a vtable, and MSVC RTTI names
/// it. `0x1` -> `0x140b6ef80` -> `FeComponentTextureShape` (or `FeComponentTextureMask` when the
/// builder is inside a mask walk, branched at `0x140b50d29`); `0x2` -> `0x140b6d080` ->
/// `FeComponentMaskShape`; `0x8` -> `0x140b6d390` -> `FeComponentTextField`.
///
/// The mistake survived because it never contradicted anything: in `l02_01_In-Game.flo` the mask
/// table's count at `doc+0x4a` is zero, so every `kind & 2` record misses that lookup and falls
/// through to the shape table and draws -- which is exactly what a reader expecting "shape" sees.
pub const FLO_RECORD_KIND_OFFSET: usize = 0x12;

/// `u16` last frame and `u16` first frame. `0xffff` as the last frame means "never ends", which is
/// what every permanent element carries.
pub const FLO_RECORD_LAST_FRAME_OFFSET: usize = 0x14;
pub const FLO_RECORD_FIRST_FRAME_OFFSET: usize = 0x16;

/// `u32` ELEMENT ID -- the field a scene path resolves against.
///
/// `FeComponentObject::findByIdPath` (`0x140b6a130`) is `mov rax,[rcx+0x48]; cmp [rax+0x1c],r9d`:
/// the component's `+0x48` is its record, and `+0x1c` of that record is the id being matched. So
/// the ids in [`FE_QUIT_TAB_CELL_IDS`] are literally these bytes, and a fourth row is a fourth
/// record carrying a fourth id.
pub const FLO_RECORD_ID_OFFSET: usize = 0x1c;

/// Bytes per transform block, from the spacing of the blocks the quit tab's records point at.
pub const FLO_TRANSFORM_SIZE: usize = 0x30;

/// `f32` x and `f32` y inside a transform block.
///
/// Not guessed from position in the struct: `FUN_140b50f20`'s "is this child trivial enough to
/// inline" test reads `pfVar1[0]` and `pfVar1[1]` and requires them to be `0.0`, then `pfVar1[2]`
/// and `pfVar1[3]` and requires them to be `1.0`. Translate-zero and scale-one is an identity
/// test, which fixes all four fields at once.
pub const FLO_TRANSFORM_X_OFFSET: usize = 0x00;
pub const FLO_TRANSFORM_Y_OFFSET: usize = 0x04;

/// The definition index of the container the quit tab's rows are children of.
///
/// It is the definition instantiated by the record whose id is [`FE_QUIT_TAB_BASE_PATH`]`[3]`
/// (`0x1eace6`), which is the last component of every cell path the namer builds. Its seven
/// children are [`FLO_QUIT_TAB_CHILD_IDS`].
pub const FLO_QUIT_TAB_CONTAINER_DEFINITION: u32 = 0x0263;

/// The seven children of [`FLO_QUIT_TAB_CONTAINER_DEFINITION`], in file order, by element id.
///
/// ```text
/// [0] 0x1eac81  def 0x0221  xy (   0, -103   )  the tab's own header
/// [1] 0x1eace9  def 0x0258  xy (-0.10, 103.90)  row 2, Quit Game    <- has a greyed-out variant
/// [2] 0x1eacca  def 0x025d  xy (-3.15,  55.90)  row 1
/// [3] 0x1eacc9  def 0x0262  xy ( 3.95,  10.60)  row 0
/// [4] 0x1eac4c  def 0x022c  xy (60.20, 114.35)  row 2's mark
/// [5] 0x1eac47  def 0x022c  xy (60.20,  65.95)  row 1's mark
/// [6] 0x1eac46  def 0x022c  xy (60.20,  17.55)  row 0's mark
/// ```
///
/// **Seven, and all seven slots are used** -- none of them is flattened away, because
/// `FUN_140b50bc0` only inlines a child whose id is zero and whose transform is the identity, and
/// every one of these has a non-zero id. That is why a fourth row cannot be squeezed into the
/// shipped display list and the child count has to rise.
///
/// This array is the content check `ds2-menu-row` runs before it substitutes anything. A
/// definition index is a number, and index `0x263` on a document this table was not read from is
/// some other container entirely.
pub const FLO_QUIT_TAB_CHILD_IDS: [u32; 7] = [
    0x001e_ac81,
    0x001e_ace9,
    0x001e_acca,
    0x001e_acc9,
    0x001e_ac4c,
    0x001e_ac47,
    0x001e_ac46,
];

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the row whose RECORD a new row is cloned from.
///
/// The record only, not what it points at: its definition index is overwritten with
/// [`FLO_QUIT_ICON_DEFINITION`] straight after the copy. What is inherited is the fields this
/// repo has not decoded -- the `u16` at `+0x02` (`1` here, `0x3b` on the flash records), the kind
/// flags, the frame range -- and all three container rows carry the same values for those, so the
/// choice of 3 is arbitrary and only has to be a plain row rather than a flash.
pub const FLO_QUIT_TAB_ROW_TEMPLATE: usize = 3;

// ---------------------------------------------------------------------------------------------
// The added row's own definition: the Quit Game glyph, tinted, keeping the selection highlight.
//
// A row definition holds TWO children and both of them matter. The first is the icon; the second
// is a shape at `(6.9, -3.45)`, colour `00ffffff`, frames `1..69` -- transparent at rest, which is
// why it reads as decoration in the file and is in fact THE SELECTION HIGHLIGHT. Pointing the
// container's row record straight at the icon (`0x0254`) put the right glyph on screen and took
// the highlight away with it, which a run showed immediately.
//
// So the row gets a copy of row 2's definition with one child swapped, rather than a bare icon.
// ---------------------------------------------------------------------------------------------

/// Row 2's definition -- Quit Game -- the one the added row's own definition is copied from.
pub const FLO_QUIT_ROW_DEFINITION: u32 = 0x0258;

/// Children [`FLO_QUIT_ROW_DEFINITION`] declares, and which is which.
///
/// ```text
/// [0] def 0x0255 id 0x1eacd0 (8.10,  4.55)  the icon, paired with its greyed-out twin
/// [1] def 0x0257 id 0        (6.90, -3.45)  the selection highlight, alpha 0 at rest
/// ```
pub const FLO_QUIT_ROW_CHILDREN: usize = 2;
pub const FLO_QUIT_ROW_ICON: usize = 0;
pub const FLO_QUIT_ROW_HIGHLIGHT: usize = 1;

/// What child [`FLO_QUIT_ROW_ICON`] names in the shipped file, and what replaces it.
///
/// `0x0255` instantiates [`FLO_QUIT_ICON_DEFINITION`] **twice** -- once at `ffffffff` and once at
/// `ff808080` carrying id [`FLO_QUIT_ROW_DISABLED_ID`]. That second one is the greyed-out overlay
/// for a REFUSED quit, and it would never come off:
/// `FeGroupInGameGroupSelect::FUN_1400a77c0` walks the cells and its first act per cell is
/// `cmp DWORD PTR [rcx+0x4], 0` / `je` -- the entry's GATE. Only a gated row reaches the call at
/// `0x1400a78af` that resolves `0x1eacd0` under that cell and sets its visibility from the gate's
/// verdict. This crate's item is [`FE_INGAME_MENU_GATE_ALWAYS`], so the pass skips it, nothing
/// ever hides the twin, and the record's own `ff808080` is what draws.
///
/// `0x1eacd0` is the shared id for that overlay -- `0x0232`, `0x0238`, `0x023f` and `0x0255` all
/// use it, which is the four gated rows in this file: the three message rows (gates 1, 2, 3) and
/// quit (gate 4). It lines up exactly with the builder table in `docs/DS2-INGAME-MENU.md`.
pub const FLO_QUIT_ROW_ICON_GROUP: u32 = 0x0255;
pub const FLO_QUIT_ROW_DISABLED_ID: u32 = 0x001e_acd0;

/// What child [`FLO_QUIT_ROW_HIGHLIGHT`] names, checked and then copied through untouched.
pub const FLO_QUIT_ROW_HIGHLIGHT_DEFINITION: u32 = 0x0257;

/// The definition that IS the Quit Game icon, and nothing else.
///
/// The white copy of row 2's icon, with a child id of `0`, so substituting it for
/// [`FLO_QUIT_ROW_ICON_GROUP`] drops the grey twin and adds no duplicate id to the scene.
pub const FLO_QUIT_ICON_DEFINITION: u32 = 0x0254;

/// The index the added row's definition is filed under. Ours, like
/// [`FLO_ADDED_PANEL_DEFINITION`], and asked for only by our own record.
pub const FLO_ADDED_ROW_DEFINITION: u32 = 0xf258;

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the mark a new row's mark is cloned from.
pub const FLO_QUIT_TAB_MARK_TEMPLATE: usize = 6;

/// Element ids for the rows this repo adds, one per slot, chosen because the file contains none of
/// them.
///
/// **Absent from the whole file, not merely from the quit tab's container.** A row id is what the
/// namer resolves and what the substituted record carries, so an id used anywhere else in the
/// document is an id whose path could resolve to something that already exists. Scanned as raw
/// dwords over all 285088 bytes: of `0x1eacc0..0x1eacdf`, the free ones are `c0`-`c8`, `cc`, `cd`,
/// `ce`, `d3`, `d7` and `df` -- fifteen, against a ceiling of two per tab.
///
/// `0x1eaccd` is first because it is the one already on record: the earlier runtime experiment
/// that named it in the namer got `row-extent 3`, i.e. nothing resolved, which is the same answer
/// the file gives from the other side.
pub const FLO_ADDED_ROW_IDS: [u32; 2] = [0x001e_accd, 0x001e_acce];

/// Element ids for those rows' caption marks, one per slot.
///
/// **These are scoped to the container and only need to be free THERE.** `FeComponentObject`'s
/// `findByIdPath` matches one path component at a time against the record's `+0x1c`, so a label id
/// used under some other container is not a collision -- the path differs before it gets there.
/// Every id in `0x1eac40..0x1eac50` appears somewhere in this document; none of these appears among
/// [`FLO_QUIT_TAB_CHILD_IDS`].
///
/// `0x1eac4a` is first because it is the id the cut fourth row used -- its caption, `0x200f28`, is
/// still in the FMG and still reads "Mouse Settings".
pub const FLO_ADDED_LABEL_IDS: [u32; 2] = [0x001e_ac4a, 0x001e_ac4b];

/// How far apart consecutive added rows and their marks sit.
///
/// Both series are the shipped ones continued. The rows sit at `10.60`, `55.90`, `103.90` and the
/// marks at `17.55`, `65.95`, `114.35`; the last step of each is `48.00` and `48.40`, and those are
/// what a fourth and fifth row continue. **They are not the same number**, which is why the two
/// pitches are separate constants rather than one shared `48`.
pub const FLO_ROW_PITCH: f32 = 48.0;
pub const FLO_MARK_PITCH: f32 = 48.4;

/// Where the added row and its mark go, in the container's own coordinates.
///
/// The three shipped rows sit at y `10.60`, `55.90`, `103.90` and their marks at `17.55`, `65.95`,
/// `114.35`. Both series step by ~48 with +y downwards, so the fourth of each continues it:
/// `103.90 + 48.00` and `114.35 + 48.40`. The x is row 2's and mark 2's, unchanged -- the shipped
/// rows' x values wobble by a few units and there is no pattern in that to continue.
///
/// This is the ROW's position and not the icon's. The record names [`FLO_ADDED_ROW_DEFINITION`],
/// which places its own icon at `(8.10, 4.55)` inside it exactly as row 2 does, so the glyph lands
/// where row 2's would one step down.
pub const FLO_ADDED_ROW_XY: (f32, f32) = (-0.1, 151.9);
pub const FLO_ADDED_MARK_XY: (f32, f32) = (60.2, 162.75);

/// Byte offset of the packed colour inside a transform block, and the tint the added row's icon
/// is drawn with.
///
/// **The offset is read off the loader, not counted off the front of the struct.**
/// `FUN_140b50bc0`'s "is this child trivial enough to inline away" test ends
///
/// ```asm
/// test  DWORD PTR [rax+0x20], 0x10f
/// jne   not_trivial
/// cmp   BYTE PTR [rax+0x1b], 0xff        ; rax is the record's transform block
/// jne   not_trivial
/// ```
///
/// so `+0x1b` is a byte the builder requires to be `0xff` before it will flatten a child away.
/// That is the alpha: the 35 records in `l02_01_In-Game.flo` carrying `00ffffff` are the
/// transparent flash overlays, and every one of them fails that test rather than being inlined.
/// A field the builder refuses to flatten over is a field the draw applies.
///
/// **And the game itself demonstrates the tint on this very icon.** `0x0255` instantiates
/// [`FLO_QUIT_ICON_DEFINITION`] twice, at `ffffffff` and at `ff808080`, and the second one is the
/// greyed-out quit icon. One definition, two colours, two appearances, from this field alone --
/// which is also what proves the colour reaches the shape underneath, since the record carrying
/// `ff808080` is a nested record and the shape below it is `ffffffff`.
///
/// Byte order in memory is **R, G, B, A** -- see [`FLO_ADDED_ROW_TINT`], which had it backwards
/// and cost a run to find out. Only the alpha's position is readable from the file, via the
/// `+0x1b` test above; the file's opaque non-white records are all greys and blacks, so nothing in
/// it distinguishes R-first from B-first.
///
/// Note this does NOT disturb the census in [`FLO_TRANSFORM_FLAGS_OFFSET`]: "RGB is white" there
/// means the low three bytes of the little-endian `u32` are `ff ff ff`, which is the same set of
/// records under either reading.
///
/// **On its own the colour does nothing.** See [`FLO_TRANSFORM_FLAGS_OFFSET`].
pub const FLO_TRANSFORM_COLOUR_OFFSET: usize = 0x18;

/// Byte offset of the flag word beside it, and the two bits that make the colour mean anything.
///
/// **THE COLOUR IS INERT WITHOUT THESE, AND A RUN PROVED IT.** The first version of this wrote
/// `ffff6450` into a transform block copied from a row whose flags were `0`, and the icon came
/// back on screen in its shipped colour with no other symptom -- no refusal in the log, no crash,
/// nothing to read. The field is not a colour, it is a colour PLUS a licence to use it, and
/// copying a block from a row that never wanted one copies the licence's absence.
///
/// Both bits were then settled against all 1045 records in the file, and the split is total:
///
/// ```text
///  flags    n   white  non-white  alpha<ff
///  0x000  928     928          0         0
///  0x001    9       9          0         0
///  0x010   77       0         77        77    alpha only -- RGB is ffffff in all 77
///  0x011   19       0         19        19    likewise
///  0x110    5       0          5         0    RGB changed: ff000000, ff808080
///  0x111    3       0          3         0    RGB changed: 800080ff, cdff80ff, daff73e9
///  0x130    4       0          4         0    RGB changed: ff000000
/// ```
///
/// `0x10` set with a non-white colour: 108 records. `0x10` clear with a non-white colour: **zero**.
/// And every record carrying `0x100` has non-white RGB, while every record without it is
/// `xxffffff` -- varying alpha over white. So `0x10` is "the colour word is live" and `0x100` is
/// "and its RGB is not white".
///
/// An opaque re-skin needs both, which is exactly what `0x0255`'s grey twin carries: flags
/// `0x110`, colour `ff808080`. The one record in the file already doing what this crate wants to
/// do. Bit `0x20` (in `0x130`) and bit `0x1` are left alone -- `0x1` occurs on white records too,
/// so neither is about colour and neither is worth setting on a guess.
pub const FLO_TRANSFORM_FLAGS_OFFSET: usize = 0x20;
pub const FLO_TRANSFORM_COLOUR_LIVE: u32 = 0x0010;
pub const FLO_TRANSFORM_COLOUR_RGB: u32 = 0x0100;

/// The tint, as the four bytes it occupies in memory: **R, G, B, A**.
///
/// **A byte array and not a `u32`, because a `u32` is what got this wrong.** The first version was
/// `0xff_ff_64_50` under a doc comment claiming `0xAARRGGBB` stored B, G, R, A. Written
/// little-endian that lays down `50 64 ff ff`, and the run came back with a BLUE icon -- which is
/// `(0x50, 0x64, 0xff)` read straight through, R first.
///
/// Alpha last was already fixed, by the builder's `cmp BYTE PTR [rax+0x1b], 0xff`. The other three
/// were asserted from nothing, and **the file could not have settled them either way**: its only
/// opaque non-white records are `ff808080` and `ff000000`, greys and blacks, where the order does
/// not show. The three hued records are all translucent and all read plausibly under either
/// convention. So this was never a fact in the file waiting to be read -- it was a coin flip
/// written down as a measurement, and the run is what called it.
///
/// Red, opaque, in the order the bytes are actually laid down. The added row wears the shipped
/// Quit Game glyph, so without this it is the same icon as the row directly above it and the only
/// thing telling them apart is the caption. Red because the row is the one that does not ask.
///
/// **It is a strength and a hue rather than three bytes, because the colour MULTIPLIES.** The
/// game's own greyed-out state is the demonstration: `ff808080` on this very glyph reads as
/// disabled, and a flat mid-grey silhouette would not -- it darkens the artwork, so white is the
/// identity and anything below it composites down. Which means a fraction of a hue is meaningful,
/// and the fraction is the thing worth naming.
pub const FLO_ADDED_ROW_TINT: [u8; 4] = [
    toward_white(FLO_ADDED_ROW_HUE[0], FLO_ADDED_ROW_TINT_STRENGTH),
    toward_white(FLO_ADDED_ROW_HUE[1], FLO_ADDED_ROW_TINT_STRENGTH),
    toward_white(FLO_ADDED_ROW_HUE[2], FLO_ADDED_ROW_TINT_STRENGTH),
    0xff,
];

/// The hue [`FLO_ADDED_ROW_TINT`] is mixed from, at full strength. R, G, B.
pub const FLO_ADDED_ROW_HUE: [u8; 3] = [0xff, 0x64, 0x50];

/// How far [`FLO_ADDED_ROW_TINT`] is pushed from white toward [`FLO_ADDED_ROW_HUE`], out of `255`.
///
/// **This is the one number in this block that is taste and not measurement**, and it is on its
/// own line so it can be turned without touching the hue or the byte order that took a run each to
/// settle. Every value that has actually been on screen, and what it was called:
///
/// ```text
///  255  100% linear  #ff6450   "this is red"           -- a re-skin, a different KIND of row
///   26   10% linear  #fff0ee   "it looks like 0% red"  -- green moved 15 of 255
///   77   30% linear  #ffd1cb   "that looks like 10%"
///  120   47% linear  #ffb7ad   asked for as 33%
/// ```
///
/// **Linear is not perceptual, and the three earlier points say by how much.** A 10% mix showed
/// nothing and a 30% mix read as 10%, which fits a floor of roughly 20% before anything registers
/// and proportional response above it:
///
/// ```text
/// perceived  ~=  (linear - 0.20) / 0.80
/// ```
///
/// That model reproduces all three points (`10% -> 0`, `30% -> 13`, `100% -> 100`) and is what
/// picked `120` for a perceived third rather than another guess at the ramp. It is fitted to three
/// samples of one person's eye on one glyph over one background, so it is a working rule and not a
/// law -- but it beats halving the interval each time.
pub const FLO_ADDED_ROW_TINT_STRENGTH: u8 = 120;

/// Mix one channel `strength/255` of the way from white toward `channel`.
///
/// White is the identity for a multiply, so a partial tint is a partial step away from it.
const fn toward_white(channel: u8, strength: u8) -> u8 {
    (255 - ((255 - channel as u16) * strength as u16) / 255) as u8
}

/// Index of the alpha byte inside [`FLO_ADDED_ROW_TINT`]. It is the byte
/// [`FLO_TRANSFORM_COLOUR_OFFSET`]` + 3` lands on, which is the one the builder reads.
pub const FLO_TINT_ALPHA: usize = 3;

// ---------------------------------------------------------------------------------------------
// Captions: how a pause-menu row gets its text, and where the strings live.
//
// Reproduce the strings with:
//
//     python3 scripts/ds2-ebl.py extract /menu/text/english/ingamemenu.fmg --out /tmp/ds2text
//     python3 scripts/ds2-fmg.py /tmp/ds2text/ingamemenu.fmg --id 0x200f26 --id 0x200f2a
// ---------------------------------------------------------------------------------------------

/// `FeGroupInGameTopSelect::bindCaptions(this)`. RVA `0x000a7130`.
///
/// Ten `(scene path, FMG text id)` pairs, built on the stack at stride `0x38` with the text id at
/// `+0x30`, then a hardcoded ten-iteration loop that for each one does
///
/// ```text
/// FUN_140026790(this + 0x150, accessor, entry)   ; resolve the element
/// FUN_14003d870(command, 7, entry->textId)       ; kind 7 = "text by FMG id"
/// FUN_140029840(accessor + 0x30, command)        ; apply
/// ```
///
/// **The count is a literal in the code and the table is on its stack**, so an eleventh caption
/// cannot be added by substituting data the way the container's child list can. It has to be bound
/// by making the same calls again — which is what `ds2-menu-row`'s caption module does, using the
/// path the original itself just built rather than rebuilding one.
///
/// Three of the ten are the quit tab's, all under [`FE_QUIT_TAB_BASE_PATH`]:
///
/// ```text
/// label 0x1eac46  text 0x200f26  "Game Options"
/// label 0x1eac47  text 0x200f27  "Screen Options"
/// label 0x1eac4c  text 0x200f2a  "Quit Game"
/// ```
///
/// And the two ids the tab never binds are `0x200f28` "Mouse Settings" and `0x200f29` "Keyboard
/// Settings" — the PC port's cut input rows, which is what the two spare cells in
/// [`FE_QUIT_TAB_SPARE_CELL_IDS`] were for. That is the whole story of the missing fourth row,
/// told by the game's own text.
pub const FE_INGAME_TOP_SELECT_CAPTIONS: u32 = 0x000a_7130;

/// Byte offset of the scene holder [`FE_BIND_SCENE_OBJ_PROXY`] is called against, inside
/// `FeGroupInGameTopSelect`. `lea rcx,[rsi+0x150]` in the loop above.
pub const FE_INGAME_TOP_SELECT_SCENE_HOLDER_OFFSET: usize = 0x150;

/// Byte offset, inside the accessor [`FE_BIND_SCENE_OBJ_PROXY`] fills, of the slot the text calls
/// take. `lea rcx,[rbp+0x90]` against an accessor built at `rbp+0x60`.
pub const FE_ELEMENT_ACCESSOR_TEXT_SLOT_OFFSET: usize = 0x30;

/// Bytes the accessor occupies. The original gives it `rbp+0x60 .. rbp+0xf0`.
pub const FE_ELEMENT_ACCESSOR_SIZE: usize = 0x90;

/// `FeElement::setText(accessor + 0x30, string)`. RVA `0x000297d0`.
///
/// **It only READS the string**, which is what makes supplying one cheap. The layout it reads is
/// MSVC's small-string optimisation as `dantelion2` spells it: the capacity at `+0x18` decides
/// where the characters are, inline at `+0x00` when it is `<= 7` and behind the pointer at `+0x00`
/// otherwise. So a caption longer than seven characters is a four-field struct pointing at a
/// `static` in this DLL — no allocator, no constructor, nothing to free.
///
/// It then measures the UTF-16 length itself and calls the element's own `vtable[0x148]`.
pub const FE_ELEMENT_SET_TEXT: u32 = 0x0002_97d0;

/// Byte offset of the capacity field the string layout above is chosen by.
pub const DL_STRING_CAPACITY_OFFSET: usize = 0x18;
/// Capacity at or below which the characters are inline rather than behind the pointer.
pub const DL_STRING_INLINE_CAPACITY: u64 = 7;

/// The quit tab's bottom row, "Quit Game" — the one that returns to the title screen and offers to
/// save on the way. Its caption is retargeted so that the row this repo adds can be the one called
/// "Quit Game", which is what it actually does.
pub const FE_QUIT_TAB_ROW_TITLE_LABEL_ID: u32 = 0x001e_ac4c;

/// The label element id given to the added row.
///
/// `0x1eac4a` rather than something outside the pool: `0x1eac45 + n` is the shared row-label id
/// space the in-game menu screens draw from, and `0x1eac4a` is the one the cut fourth row used
/// (its caption, `0x200f28`, is still in the FMG and still reads "Mouse Settings"). Nothing in the
/// quit tab's container carries it, so nothing is shadowed.
pub const FLO_ADDED_ROW_LABEL_ID: u32 = 0x001e_ac4a;

/// How much taller the quit tab's panel has to be drawn for a fourth row to sit on it.
///
/// **Measured, not chosen.** All three menu tabs share panel definition `0x0221` at scale `(1,1)`;
/// its shape is `57.80 x 341.25` and its own translate puts it at panel-local `y = -59.90 ..
/// 281.35`. The three rows sit at panel-local `y = 113.6, 158.9, 206.9` and a row plate is
/// `48.80` tall, so the bottom row ends at `255.7` and the scroll's own bottom edge is `25.65`
/// below that.
///
/// **That 25.65 is the scroll's bottom MARGIN, and the first version of this constant spent it.**
/// It reached `303.7`, where the fourth row ends, which puts the scroll's bottom curl exactly on
/// the row's bottom edge -- arithmetically "covered" and visibly still short. The target is the
/// fourth row's end PLUS the margin the shipped rows are drawn with:
///
/// ```text
/// (303.7 + 25.65) / 281.35 = 1.1706
/// ```
///
/// Scaling is about the record's own origin, so the top edge moves up by 10.1 as well and the
/// header margin stretches by the same 17%. A scroll can absorb that; a nine-slice would be better
/// and this file does not obviously offer one.
///
/// It scales the panel's CURSOR with it -- `0x1eac81`'s definition carries both the scroll and the
/// highlight. Only the quit tab is affected, because what is scaled is this crate's own copy of
/// that tab's child record rather than the shared definition all three tabs instantiate.
/// **ANSWERED, AND THE ANSWER IS THAT THIS FIELD DOES NOTHING.** `1.0794` and then `1.1706` were both reported
/// as no visible change, and "17% of a 341-unit scroll is invisible" is not credible -- that is 58
/// units, more than a row. So the thing in doubt is no longer the factor, it is whether this field
/// reaches the scroll at all. `2.0` is deliberately unmissable, and because it is applied to ONE
/// axis it separates three outcomes in one look:
///
/// * the scroll is twice as TALL -- this offset is scale-y, and the factor goes back to `1.1706`;
/// * the scroll is twice as WIDE -- this offset is scale-X, and the sibling at `+0x08` is the one;
/// * nothing moves -- the panel record is not what draws the scroll, and the next step is reading
///   the 17 quads in shape `0x0220`'s geometry rather than guessing a fourth time.
///
/// It was the third: nothing moved on either axis. The tree walk then found the reason -- the panel
/// draws through a `FeComponentTextureShape`, which is sized by its own quad
/// ([`FE_TEXTURE_SHAPE_DEST_RECT_OFFSET`]) and never re-derives it from an ancestor's transform.
///
/// So this is `1.0`: the substitution still copies the panel's record like every other, and the
/// scale it writes is the one the game shipped. A number that provably changes nothing has no
/// business sitting in a table of measurements pretending otherwise.
pub const FLO_PANEL_STRETCH_Y: f32 = 1.0;

/// Index into [`FLO_QUIT_TAB_CHILD_IDS`] of the panel the stretch applies to.
pub const FLO_QUIT_TAB_PANEL: usize = 0;

/// `f32` scale-y inside a transform block. `pfVar1[3]` in the builder's identity test.
pub const FLO_TRANSFORM_SCALE_Y_OFFSET: usize = 0x0c;

// ---------------------------------------------------------------------------------------------
// The live component tree, for finding an element the file alone will not identify.
//
// Three stretch factors were tried on the quit tab's panel record and the third one -- scale 2.0,
// deliberately unmissable -- changed nothing on screen while the log proved the write landed. So
// the element being scaled is not the one that draws the banner, and no amount of further
// arithmetic on the `.flo` fixes that. What is needed is the live tree: what is actually built
// under this tab, and which of it is big enough to be the background.
// ---------------------------------------------------------------------------------------------

/// `FeLayoutScene::findByIdPath(scene, ids, count)`. RVA `0x00afdad0`.
///
/// `mov rcx,[rcx+0x28]` then a tail-jump into the search; returns the component or null. This is
/// the lookup every scene path in the frontend bottoms out in, so resolving a path by hand and
/// resolving it the way the grid does are the same operation.
pub const FE_SCENE_FIND_BY_ID_PATH: u32 = 0x00af_dad0;

/// Byte offset, inside a `FrontendEx::SceneObjProxy`, of the scene proxy its resolve reads.
///
/// `SceneObjProxy::resolve` (`0x140027ce0`) opens `mov rcx,[rcx+0x58]; mov rax,[rcx]; call
/// [rax+8]`, so the scene comes from slot 1 of whatever lives here.
pub const FE_SCENE_OBJ_PROXY_SCENE_OFFSET: usize = 0x58;
/// Vtable slot on that object which returns the scene.
pub const FE_SCENE_PROXY_GET_SCENE_SLOT: usize = 0x08;

/// Component tree links, read off `FUN_140b77dc0` and `FeComponentObject::findByIdPath`.
///
/// ```text
/// child   = [parent + 0x38]      first child
/// child   = [child  + 0x28]      next sibling
/// record  = [child  + 0x48]      the `.flo` record, whose +0x1c is the element id
/// ```
pub const FE_COMPONENT_NEXT_SIBLING_OFFSET: usize = 0x28;
pub const FE_COMPONENT_FIRST_CHILD_OFFSET: usize = 0x38;
pub const FE_COMPONENT_RECORD_OFFSET: usize = 0x48;

/// Where a component's own transform starts, which is NOT the same for every class.
///
/// `FeComponentObject`'s constructor (`0x140b69d10`) writes a 4x3 identity at `+0x60`;
/// `FeComponentScene`'s (`0x140b6b730`) writes the same identity at `+0x50`. They are siblings
/// under `FeComponentBase`, not parent and child, which is why the offsets differ -- so a dump
/// covers both ranges and the vtable says which one to read.
pub const FE_COMPONENT_TRANSFORM_DUMP_START: usize = 0x50;
pub const FE_COMPONENT_TRANSFORM_DUMP_END: usize = 0xa0;

/// The ONLY two classes whose `+0x38` is a child list, as vtable RVAs.
///
/// **This cost a crash.** `FUN_140b77dc0` reads `[parent+0x38]` and every component in the tree
/// looked like a parent, so a walk that followed that offset unconditionally descended into a
/// `FeComponentTextureShape`, read some unrelated field as a pointer, and recursed until it died
/// -- `exception_address=DINPUT8.dll+0x769ad` with five identical frames above it.
///
/// The classes that recurse are the ones whose `findByIdPath` (vtable `+0x190`) reaches
/// `FUN_140b77dc0`: `FeComponentObject` matches its own id first and then descends,
/// `FeComponentScene` has no id and descends immediately. Every other `FeComponent*` overrides that
/// slot with something else and is a leaf as far as the tree is concerned.
///
/// Names from `scripts/ds2-rtti-vtables.py 'FeComponent'`.
pub const FE_COMPONENT_OBJECT_VTABLE: u32 = 0x011d_dfa8;
pub const FE_COMPONENT_SCENE_VTABLE: u32 = 0x011d_e158;

/// The display list, its live count, and its stride, inside a `FeComponentSprite`.
pub const FE_COMPONENT_DISPLAY_LIST_OFFSET: usize = 0x70;
pub const FE_COMPONENT_DISPLAY_COUNT_OFFSET: usize = 0x66;
pub const FE_COMPONENT_DISPLAY_ENTRY_STRIDE: usize = 0x10;
/// Offset of the child pointer and of the id key inside one display-list entry.
pub const FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET: usize = 0x00;
pub const FE_COMPONENT_DISPLAY_ENTRY_KEY_OFFSET: usize = 0x0c;

/// Classes whose `findByIdPath` is `xor eax,eax; ret` (`0x140b6d2a0`) -- genuine leaves:
/// `FeComponentLinked`, `FeComponentMaskShape`, `FeComponentTextureMask`,
/// `FeComponentTextureShape`. Following `+0x38` on one of these is what crashed the first walk.
pub const FE_COMPONENT_LEAF_FIND_BY_ID_PATH: u32 = 0x00b6_d2a0;

// ---------------------------------------------------------------------------------------------
// FeComponentTextureShape: the thing that actually draws the quit tab's banner.
//
// Three stretch factors on ancestor transforms did nothing, and this is why: a texture shape is
// sized by its OWN quad, copied into it at build time and never re-derived from a parent.
// ---------------------------------------------------------------------------------------------

/// `FeComponentTextureShape`'s vtable. RVA `0x011dea18`, VA `0x1411dea18`. From MSVC RTTI.
pub const FE_COMPONENT_TEXTURE_SHAPE_VTABLE: u32 = 0x011d_ea18;

/// `FeComponentTextureShape::initFromShape(this, allocator, shapeEntry)`. RVA `0x00b70200`.
///
/// Everything below is read off it. It allocates four parallel arrays, one element per quad, and
/// fills them from the shape table entry's sub-records:
///
/// ```text
/// count = [[this+0x40] + 0x02]                 quads in this shape
/// [this+0x48]  count * 0x30                    per-quad vertex block, seeded from a constant
/// [this+0x50]  count * 0x10                    four floats per quad   <- the RECT
/// [this+0x58]  count * 0x10                    four floats per quad   <- the second RECT
/// [this+0x60]  count * 0x04                    per-quad colour, from sub-record +0x18..+0x1b
/// ```
///
/// For each quad, `sub = [[this+0x40] + 0x08] + i * 0x40` and `geom = [sub + 0x30]`:
///
/// * `geom != 0` -- both rects are filled with the SAME four floats from `geom[0..3]`;
/// * `geom == 0` -- both are filled with `{0, 0, w, h}` taken from `[[sub+0x20] + 0x0e]` and
///   `+0x10`, i.e. the texture's own pixel size.
///
/// The quit tab's banner is shape `0x0220`, `count = 1`, and its single quad reads
/// `(914.20, 1.10, 972.00, 342.35)` -- `57.80 x 341.25`, which is the `341.25` the panel
/// measurements were built on. The sub-record's own translate is `(-914.30, -61.00)`, cancelling
/// the atlas origin, so these are atlas coordinates mapped into layout space.
pub const FE_TEXTURE_SHAPE_INIT: u32 = 0x00b7_0200;

/// The shape table entry a texture shape was built from, and the count field inside it.
pub const FE_TEXTURE_SHAPE_ENTRY_OFFSET: usize = 0x40;
pub const FE_SHAPE_ENTRY_COUNT_OFFSET: usize = 0x02;

/// The two per-quad rect arrays, `0x10` bytes each: DESTINATION and SOURCE.
///
/// The initialiser seeds them identically and so cannot tell them apart. The DRAW can:
/// `FeComponentTextureShape`'s render (`0x140b6f200`, vtable slot 46) ends every quad with
///
/// ```text
/// FUN_140b521c0(ctx, [this+0x50] + i*0x10, colour, [this+0x58] + i*0x10)
/// ```
///
/// and in the branch where a texture is actually bound it **replaces the fourth argument** with a
/// local `{0, 0, texWidth, texHeight}` read from `[tex+0x40]` and `[tex+0x44]`. A rect that can be
/// substituted by the texture's own pixel size is the SOURCE.
///
/// So growing both is what put art on the added row that was mostly transparent -- the destination
/// made room and the source pulled in whatever sits below the banner in the atlas. Growing the
/// destination alone stretches the shipped art to fill instead.
pub const FE_TEXTURE_SHAPE_DEST_RECT_OFFSET: usize = 0x50;
pub const FE_TEXTURE_SHAPE_SOURCE_RECT_OFFSET: usize = 0x58;
pub const FE_TEXTURE_SHAPE_RECT_STRIDE: usize = 0x10;

/// The display-list key the panel's texture shape is filed under.
///
/// Not an element id -- `FUN_140b6bd80` sets a child's key from `FUN_140b6a440(child)`, and a shape
/// with no id of its own lands on `0xffffffff`. Observed on every texture shape in the live tree.
pub const FE_TEXTURE_SHAPE_DISPLAY_KEY: u32 = 0xffff_ffff;

/// How much taller the banner's quad has to be for a fourth row.
///
/// The quad is `341.25` tall and covers three row slots with `25.65` of margin below the last.
/// One more row is one more `48.00` of pitch, so `341.25 + 48.00 = 389.25` keeps the margin
/// exactly. As a rect that is `y1 = 1.10 + 389.25 = 390.35`.
pub const FE_BANNER_QUAD_Y1: f32 = 390.35;
/// What the shipped quad's `y1` reads, checked before anything is written.
pub const FE_BANNER_QUAD_SHIPPED_Y1: f32 = 342.35;

/// The panel definition the quit tab's `0x1eac81` instantiates, and the caret inside it.
///
/// `0x0221` holds exactly two children: the banner shape `0x0220` at `(0, 0)`, and `0x004e` at
/// `(40.85, 244.65)` -- panel-local, which is container y `141.65`, just below the third row's mark
/// at `114.35`. That second one is the scroll caret.
///
/// **It is SHARED.** All three menu tabs' panels instantiate `0x0221`, so moving the caret inside it
/// moves it on every tab. The quit tab therefore gets its own copy of the definition under an index
/// nothing else asks for, reached the same way the container is: our record names it, and the
/// lookup detour answers it.
pub const FLO_PANEL_DEFINITION: u32 = 0x0221;
/// Index of the caret among `0x0221`'s two children.
pub const FLO_PANEL_CARET: usize = 1;
/// Children `0x0221` declares.
pub const FLO_PANEL_CHILDREN: usize = 2;

/// The definition index the quit tab's own panel copy is filed under.
///
/// Outside the file's own range -- `l02_01_In-Game.flo` declares 342 definitions and the highest
/// index seen is `0x0272` -- so a lookup for it can only come from the record this crate wrote.
pub const FLO_ADDED_PANEL_DEFINITION: u32 = 0xf221;

/// Where the caret goes: down by one row pitch, the same `48.00` the added row is spaced at.
///
/// `244.65 + 48.00`. It is the caret's own authored y in panel-local coordinates, so the panel's
/// position does not enter into it.
pub const FLO_CARET_Y: f32 = 292.65;
/// What the shipped caret's y reads, checked before anything is written.
pub const FLO_CARET_SHIPPED_Y: f32 = 244.65;

// =================================================================================================
// THE SOFTWARE KEYBOARD, AND THE ONE DWORD THAT KEEPS IT SAFE TO BORROW
//
// DARK SOULS II already asks Steam for a text field -- `SoftwareKeyboardManagerImpl` wraps
// `ISteamUtils::ShowGamepadTextInput` and the game uses it for character naming. A mod can call the
// same API for its own field, and should, because the alternative is drawing and driving a text
// box by hand. But the game's own dismissal listener does NOT check whether it asked for the
// keyboard, so a session this mod opens is a session the game will react to.
//
// EVERY ADDRESS BELOW WAS BYTE-CHECKED AGAINST `darksoulsii-deobf.bin`, not read out of a report.
// The four that matter are quoted with their bytes in their own doc comments.
// =================================================================================================

/// `SoftwareKeyboard::detail::SoftwareKeyboardManagerImpl`'s singleton pointer. RVA `0x01896a08`.
///
/// **Null until the first soft-keyboard interaction**, and that null is a fact worth checking
/// rather than defending against: the game's dismissal listener is registered by the impl's
/// CONSTRUCTOR, so while this pointer is null there is no listener for `GamepadTextInputDismissed_t`
/// anywhere in the game and a mod's own keyboard session cannot disturb anything.
///
/// Established from the accessor `0x140ff1d20`, which both reads and writes it:
///
/// ```text
/// 0x140ff1d4e: 48 8b 0d b3 4c 8a 00   mov rcx,[rip+0x8a4cb3]   ; -> 0x141896a08
/// 0x140ff1dbe: 48 89 05 43 4c 8a 00   mov [rip+0x8a4c43],rax   ; the store after construction
/// ```
///
/// Both displacements resolve to the same address, from a scan of the accessor's whole body rather
/// than from one hit.
pub const SOFTWARE_KEYBOARD_IMPL_SINGLETON: u32 = 0x0189_6a08;

/// `int32 m_state` inside the impl. `+0x08`.
///
/// The constructor at `0x140ff1e28` opens it at idle -- `c7 43 08 ff ff ff ff`,
/// `mov DWORD PTR [rbx+0x8],0xffffffff` -- and it is the only field any of the three parties
/// (the game, Steam's callback, this mod) needs to agree on.
pub const SOFTWARE_KEYBOARD_IMPL_STATE_OFFSET: usize = 0x08;

/// `m_state` when no session is running. `-1`.
///
/// **The value [`SOFTWARE_KEYBOARD_IMPL_SHOW`] demands before it will open anything**, and the
/// value this mod must restore when its own session ends.
pub const SOFTWARE_KEYBOARD_STATE_IDLE: i32 = -1;
/// `m_state` while a Steam keyboard is up. `0`. Written by `show` only after
/// `ShowGamepadTextInput` returned true.
pub const SOFTWARE_KEYBOARD_STATE_SHOWING: i32 = 0;
/// `m_state` after a dismissal the player did not submit. `2`.
pub const SOFTWARE_KEYBOARD_STATE_CANCELLED: i32 = 2;
/// `m_state` after a dismissal the player submitted. `3`.
pub const SOFTWARE_KEYBOARD_STATE_SUBMITTED: i32 = 3;

/// `SoftwareKeyboardManagerImpl::show`. RVA `0x00ff22e0`. **Recorded for its gate, not to be
/// called.**
///
/// ```text
/// 0x140ff2317: 83 79 08 ff   cmp DWORD PTR [rcx+0x8],0xffffffff
/// 0x140ff231b: 74 07         je  proceed
/// 0x140ff231d: 32 c0         xor al,al          ; refuse, and never reach Steam
/// ```
///
/// Those bytes are the whole reason [`SOFTWARE_KEYBOARD_STATE_IDLE`] has to be restored. `show`
/// refuses unless it reads `-1`, and the only writers of `-1` live inside `getResult`, which the
/// game cannot reach once `show` has started failing -- so a state this mod leaves behind is a
/// state the game never recovers from on its own. It is not a crash. It is the Steam keyboard
/// silently never appearing again for the rest of the process.
pub const SOFTWARE_KEYBOARD_IMPL_SHOW: u32 = 0x00ff_22e0;

/// The game's `GamepadTextInputDismissed_t` listener. RVA `0x00ff2040`. Fourteen bytes, no
/// branches, and it is the entire collision risk:
///
/// ```text
/// 33 c0        xor  eax,eax
/// 38 02        cmp  BYTE PTR [rdx],al     ; the callback's m_bSubmitted
/// 0f 95 c0     setne al
/// 83 c0 02     add  eax,0x2               ; -> 2 or 3
/// 89 41 08     mov  DWORD PTR [rcx+0x8],eax
/// c3           ret
/// ```
///
/// **It reads no text and it cannot fail** -- no allocation, no call, no branch, and the only
/// pointers it touches are its own `this` and the callback payload. So a session this mod opens can
/// neither crash the game nor corrupt it.
///
/// **What it does not do is ask whether the game wanted this dismissal.** There is no pending-request
/// flag and no owner check. It is registered exactly once in the image (`0x140ff1e72`, id `714`) and
/// lives in `steam_api64.dll`'s process-wide table, so it fires for a keyboard THIS MOD opened just
/// as readily -- which is why not registering a listener of our own buys nothing, and why the
/// interlock is on `m_state` instead.
pub const SOFTWARE_KEYBOARD_DISMISSED_HANDLER: u32 = 0x00ff_2040;

/// The `ISteamUtils` version whose `ShowGamepadTextInput` takes a prefill. `"SteamUtils007"`.
///
/// DS2's own `steam_api64.dll` carries only `SteamUtils005` and `SteamClient012`, and at
/// `0x140ff2415` the game loads four arguments and never writes `[rsp+0x28]` -- the
/// `pchExistingText` slot. **That is the GAME's call site, not a limit of the API.**
///
/// Read off Proton Experimental's generated per-version wrappers in
/// `files/lib/wine/x86_64-unix/lsteamclient.so`, which is the closest thing to the Steamworks
/// headers available offline. `005` and `006` marshal four arguments; from `007` on there is a
/// fifth, `mov 0x1d(%rbx),%r9` -- a full 64-bit pointer:
///
/// ```text
/// 005, 006          4 args   slot 0xa0
/// 007, 008, 009, 010  5 args   slot 0xa0     <- prefill arrives here
/// 011                 5 args   slot 0x98     <- and the slot moves
/// ```
///
/// `007` is therefore the SMALLEST upgrade that gains the prefill, and the one that keeps
/// [`STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT`] where the game's own build already proves it to be.
/// `~/.local/share/Steam/linux64/steamclient.so` advertises `SteamUtils001` through `011`, so the
/// client vends it.
///
/// **NEVER HAND THE NEWER POINTER TO THE GAME.** `007` keeps the method at the same slot as `005`,
/// so the game's four-argument call would pass straight through it leaving `r9` holding whatever
/// was there -- a garbage `pchExistingText`. Two interfaces, two owners.
pub const STEAM_UTILS_VERSION_WITH_PREFILL: &str = "SteamUtils007";

/// `ISteamClient012::GetISteamUtils(HSteamPipe, const char *pchVersion)`. Vtable slot `+0x48`.
///
/// From Proton's `ISteamClient_SteamClient012_GetISteamUtils` thunk, which is
/// `mov 0x14(%rbx),%rdx` (the version string), `mov 0x10(%rbx),%esi` (the pipe), `call *0x48(%rax)`.
/// The shim does not validate the version string -- `steamclient64.dll` resolves it -- which is what
/// makes [`STEAM_UTILS_VERSION_WITH_PREFILL`] reachable from a binary that only knows `005`.
pub const STEAM_CLIENT_GET_ISTEAM_UTILS_SLOT: usize = 0x48;

/// `ISteamUtils::IsOverlayEnabled()`. Vtable slot `+0x88`.
///
/// Read out of DS2 itself: [`SOFTWARE_KEYBOARD_IMPL_SHOW`] gates on it before it will ask for a
/// keyboard. **It gates this mod's field too, and it is the one thing about this path that no
/// amount of static reading settles** -- whether the overlay is actually attached is a property of
/// the running Steam client, not of the executable.
pub const STEAM_UTILS_IS_OVERLAY_ENABLED_SLOT: usize = 0x88;

/// `ISteamUtils::ShowGamepadTextInput(...)`. Vtable slot `+0xa0`.
///
/// Observed twice, independently: DS2 calls `[vtbl+0xa0]` at `0x140ff2424` on its `005` pointer, and
/// Proton's `007` wrapper calls `*0xa0(%rax)`. Same slot on both sides of the version bump, which is
/// what makes `007` a drop-in for the game's own layout.
pub const STEAM_UTILS_SHOW_GAMEPAD_TEXT_INPUT_SLOT: usize = 0xa0;

/// `ISteamUtils::GetEnteredGamepadTextLength()`. Vtable slot `+0xa8`. From
/// `SoftwareKeyboardManagerImpl::getResult` (`0x00ff2050`), at `0x140ff20b1`.
pub const STEAM_UTILS_GET_ENTERED_TEXT_LENGTH_SLOT: usize = 0xa8;

/// `ISteamUtils::GetEnteredGamepadTextInput(char *pchText, uint32 cchText)`. Vtable slot `+0xb0`.
/// Same function, at `0x140ff2101`. **The text comes back UTF-8**, which is why the impl keeps a
/// `char*` scratch buffer at `+0x10` rather than a wide string.
pub const STEAM_UTILS_GET_ENTERED_TEXT_SLOT: usize = 0xb0;
