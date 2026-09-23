# The two save-file rows, and the restart that turned out not to be needed

Ported from `../er-mods-rs`'s System>Quit rows. What follows is the static reading behind them, two
design corrections that reading forced, and what is still not done.

The second correction is the larger one and it is worth stating at the top: the load row no longer
ends the process. It returns to the title, points the loads at the container the player picked, and
hands them the game's own character list for it. The section below has the five steps.

**Nothing here has been run.** Every address is read out of `darksoulsii-deobf.bin` (SOTFS build
9527516) with `scripts/ds2-disasm.py` and `scripts/ds2-arxan-chain.py`; every behavioural claim about
the game is a claim about its disassembly. The runtime test is filed, not performed.

Addresses are VAs, the form the disassembly prints. Subtract `0x140000000` for the RVA `ds2-rva`
records.

## What was ported, and what was deliberately not

`er-quit-menu-core` is 28,897 lines across 48 modules. The two rows this document is about correspond
to its `QuitRow::LoadSaveProfiles` ("Load Character from File") and `QuitRow::SaveGameAs`
("Save Game"), and the honest accounting of the port is:

| er-mods-rs | lines | ported? |
| --- | --- | --- |
| `er-save-picker-core::os_dialog` | 728 | **yes**, as `ds2-save-file::dialog` (~250) |
| the two rows' behaviour | -- | **yes**, as `ds2-save-file::{import,export}` |
| `save_picker_menu` + `model` + `overlay` (in-game browser) | 6,911 | no |
| `software_keyboard` | 2,295 | no |
| `scaleform_proxy`, `gfx_swap`, `dim` | 3,676 | no |
| `save_flow` + `save_flow_boxes` + `save_dest_commit_runtime` | 4,046 | no |
| `row_cloner`, `rows`, `row_identity`, `row_staging`, ... | ~4,000 | no -- `ds2-menu-row` already does this |

The unported nine tenths is Elden Ring's menu engine: a Scaleform movie substituted at load, a list
built out of `05_010_ProfileSelect`, a dim cover composited by `er-d3d12-compositor`, and a menu pump
that owns which window is modal. DS2 is D3D11, has no Scaleform, and draws its menus from `.flo`
records -- eight years and one engine generation earlier. Reimplementing that surface for DS2 would be
a project, not a port, and the OS file dialog is the part of ER's own solution that carries no game
coupling at all: it converts strings, calls comdlg32, and dereferences nothing from the module base.

## Save Game to File

### `SaveLoadSystem::RequestSave` does not save

`0x1402e7410`, in full -- 0x29 bytes:

```asm
0x1402e7410:  cmp  edx,0xe                 ; kind 14 sets one flag and returns
              jne  0x1402e741d
              mov  BYTE PTR [rcx+0x1a9],1
              ret
0x1402e741d:  cmp  edx,[rcx+0x68]          ; keep the LOWEST kind asked for
              jge  0x1402e7425
              mov  [rcx+0x68],edx
0x1402e7425:  mov  BYTE PTR [rcx+0x1a2],1  ; "a save is wanted"
              cmp  edx,0x2
              jne  0x1402e7438
              mov  BYTE PTR [rcx+0x1a3],1  ; and kind 2 sets a second flag
0x1402e7438:  repz ret
```

Three byte writes and a `min`. **It returns void** -- a caller that branches on RAX is branching on
whatever the last call left there -- and the save happens later, from `GameManagerImp`'s master
update, exactly like the shutdown byte `ds2-menu-row` writes.

That is the whole reason the row is two phases. At the instant of the press, the bytes on disk are the
LAST save; the one the player asked for does not exist yet.

`scripts/ds2-arxan-chain.py` reports `UNKNOWN` at hop 0 because its prologue table does not carry
`83 fa` (`cmp edx, imm8`). The entry is ordinary code, not the five-byte `e9` stub a redirected entry
keeps in the deobfuscated image, so this is not an Arxan redirect. `ds2-rva` records the three bytes
and the row refuses to call the address if they are not there.

### What the wait watches

| signal | what it says | why it is not enough alone |
| --- | --- | --- |
| `[saveLoadSystem+0x08]`/`+0x0c` | a request is in flight; every completion path zeroes it | says a REQUEST ended, not that the file was rewritten |
| the `.sl2`'s `(length, mtime)` | the file changed | says nothing about whether the writer has finished |

So both: the stamp has to change, and then the interlock has to be idle. The first is the row's
promise (the file), the second is what stops a torn copy.

The interlock is the pair both start paths test -- `0x1402e72c0` and `0x1402e7170` each open
`if ([this+0x08] != 0 || [this+0x0c] != 0) return false`.

### The refusal that protects a save

The destination dialog opens in the save's own folder with the save's own name already filled in, so
pressing Save without typing is the *default* path to naming the live container -- and copying a file
onto itself truncates it. `ds2_save_file_core::dest::is_live_container` compares the two
case-insensitively, because these are Windows paths inside a Wine prefix and a case-sensitive compare
would answer "different file" about the save being played.

## Load Character from File

### It loads in the session that asks, and the five steps are all the game's

The heading here used to read "it cannot load in the session that asks, and the reason is the game's".
The obstacle it described is real and is quoted below; what was wrong was the conclusion that no
ordering escapes it. One does, and `crates/ds2-save-file/src/swap.rs` is it:

| step | what performs it | what it is |
|---|---|---|
| 1 | `ds2_save_redirect::stage` | copy the pick, rebind its Steam ID, write it to `<Game>/ds2-swapped-save/` |
| 2 | `ds2_menu_row::return_to_title` | fire action `9` with its own gate applied: the game's confirm, the game's save, the game's unload |
| 3 | `session_dir::LOAD.arm` | point the load side at the staged copy. The save side is not touched |
| 4 | `SaveLoadSystem` `0x1402e72c0` | ask for a re-read of container entry 7, the section the ten in-memory records come from |
| 5 | `SaveLoadSystem` `0x1402e6230`, every frame | collect that request: this is what clears the interlock and parses entry 7 in |
| 6 | `ds2-continue`'s title gate | take the top menu's own `LOAD GAME` edge, and hand the player the list |

The player then picks a character out of the game's own screen, and on the frame the list takes its
load branch -- phase 2, past the occupancy, exclusion and ownership checks -- `session_dir::SAVE` is
armed as well, so the character that is about to be played saves back into the container it came out
of.

The obstacle, still true, and now only an argument about ordering:

> **DS2 saves on the way out.** The pause menu's Quit Game row is action `9`, which the dispatch
> resolves to `FeGroupInGameReturnTitleCheck` -- the confirm that persists the character on the way to
> the title. A session that re-points the save directory and then quits writes the current character
> into the staged copy, and the LOAD GAME that follows reads back the character the player was trying
> to replace.

That describes a session which re-points **both** sides before leaving. Step 2 happens before step 3,
and step 3 moves only the load side, so the character being left is written by a save side nothing has
touched. The two halves that "both belong to the game" turn out to belong to two different classes.

**Step 4 is the step that is easy to leave out.** `FUN_1400f0f60`, the vector the character list's
`enter` measures, is built from `GameManagerImp->GameDataManager->savedata__` and reads no file at
all. Arming a load redirect therefore changes nothing about what the list SAYS -- the block was filled
at boot. `0x1402e72c0` is what fills it again; it sets container entry 7 for loading, which is the
section those ten `0x1f0`-byte records come from, and its own first act is to test the interlock, so
calling it at a bad moment returns `false` and changes nothing.

**Step 5 is the step that was left out**, and one live run on 2026-09-23 found it. `0x1402e72c0`
only hands the request to the `SLSession` worker and writes the interlock (`_x8 = 4; _xc = 2`).
Nothing on the worker thread ever writes it back: the interlock is cleared, and entry 7 actually
parsed into the records block, by a separate per-frame call, `0x1402e6230`. Its only two callers in
the image are `FeSubStateTitleSteamLoadSystemData::update` (`0x1400fbdb0`) and
`FeSubStateTitleLoadProfile`'s poll (`0x1400fc5b0`) -- both title substates, neither resident while
`0x47 TopMenu` is up, which is exactly where this flow asks. So the run logged `accepted=true` and
then fifteen seconds of nothing:

```text
ds2-save-file: re-read requested system=0x00007ffff03a7f50 accepted=true
ds2-save-file: swap ABANDONED -- the container re-read never finished.
```

The flow now calls `0x1402e6230` itself on every frame it is waiting, which is the shipped call
pattern rather than an approximation of one -- both game callers do the same and hold their phase
while it answers `1`. It returns `1` while the worker is busy, `0` when the records are filled, `4`
if nothing was in flight, and a small error code otherwise.

The old route is still in the tree, reached only when the title flow is not hooked: the row writes
`ds2-load-next-save.txt` beside the executable for `ds2-loader` to consume during
`DLL_PROCESS_ATTACH`, and quits. The loader **deletes it as it reads it**: a handoff that persisted
would leave a player in somebody else's save on every launch until they found a text file to delete.

### The save and the load ask for their directory from two different functions

Which is what makes the in-session version possible, and it is simpler than the plan this section
used to carry. That plan was wrong on both of its premises and is corrected below.

The directory a session uses is supplied by a virtual override, and the two subclasses have their
own. Both are named in the symbols:

| override | reads the directory from | address |
|---|---|---|
| `SaveLoad2::SLSaveSession` | `this[0x1d]`, via `FUN_140a89cf0` | `0x140a8ec40` |
| `SaveLoad2::SLLoadSession` | `this+0xe8`, via `FUN_140a8a180` | `0x140a8f8d0` |

They are otherwise the same three lines -- fetch a wide string, measure it, hand it to the session's
string setter `FUN_140a89050`. So there is no runtime flag to decode: **the class that is asking is
the answer**. Detouring the load-side override alone makes a load read the staged copy while every
save keeps writing the player's own folder, untouched.

What the old plan claimed, and why it was wrong:

* *"`SAVE_DIR_BUILD` is called per request."* It is not. It is reached from exactly one arm of
  `FUN_1402e67f0`'s switch -- session type `0x18`, which builds the directory once during session
  setup and hands it over through `FUN_140a899f0`. A detour there is asked for a folder before
  anything has said whether this session saves or loads.
* *"Split states `{1,3,5,6}` into saves and loads."* Those are the function's outer guard, not a
  discriminator: `cmp eax,6 / ja bail` then `mov ecx,0x6a / bt ecx,eax / jae bail` is an early-out,
  and everything that survives it reaches the same switch.

And it does not need a code patch. The load-side override has no call sites at all -- only two data
references, one of them the vtable slot at `0x1411b64f8`. It is reached exclusively through the
vtable, so arming this is a pointer write into `.rdata` rather than a detour over the function's
prologue: nothing in the instruction stream changes, which takes Arxan out of the question for this
site entirely. `ds2-menu-row` already does the same kind of substitution for the `.flo` child count.

There is one construction path, which bounds what the swap can affect. `SLLoadSession` has two
constructors, `0x140a8f6b0` and `0x140a8f7d0`, and each has exactly one caller; the outer one is
`FUN_140a8a6f0` at `0x140a8a82c`, which builds the session from an `SLLoadContent` passed in by its
caller and reads that content's string through `FUN_140a8a180` -- the same accessor the override
uses at `this+0xe8`. So every load session's directory traces back to the `SLLoadContent` it was
constructed from, and swapping the vtable slot intercepts all of them or none.

Following that chain up answers what the swap catches, and the answer is everything. `FUN_140a8a6f0`
has one caller, `FUN_140a86280` at `0x140a86304`, and that has three, all of them already named from
earlier work in this repo:

* `FUN_1402e6ff0_loadSlotByIndex`
* `FUN_1402e72c0_loadSlot_0_andOtherSetup`
* `FUN_1402e7170_loadSlot22_andOtherSetup`

Every read of a container section goes through the same funnel, so the vtable swap is all-or-nothing
by construction: it cannot be aimed at one load and not another, and the title screen's character
list -- which is itself built from slot reads -- comes along with it. For the picker that is the
wanted behaviour rather than a side effect, since the point is to see the donor container's
characters and choose one. It does mean the swap has to be armed and disarmed around the load rather
than left on, or every subsequent read in the session answers from the staged copy too.

## The row budget, which is the reason there is a list

`FeGroupInGameGroupSelect`'s item vector is a `DLKR::DLFixedVector` of capacity **five**, spelled by
the builders as `if (5 < newCount) panic("out of memory.")` and independently by the copy at
`0x1400a3ef0`, which panics unless the source count is `< 6`. The System tab ships three rows. So
`ds2_menu_row::MAX_ADDED_ROWS` is **two**, and there are now four rows that want a slot:

| row | crate |
| --- | --- |
| `quit-to-desktop` | `ds2-menu-row` |
| `load-build-from-url` | `ds2-build-import` |
| `load-character-from-file` | `ds2-save-file` |
| `save-game-to-file` | `ds2-save-file` |

`[menu_row] rows = [...]` is how a player picks two, and the third is refused at registration with the
numbers in the log rather than by the game's allocator during a menu open. This is the same shape
`er-quit-menu-core::row_config` arrived at, after the same mistake: three DLLs that each armed a
different row set, replaced by a file the player can edit.

**All four at once needs a second tab.** The probe's own five-tab control run measured every tab's
item count:

```
tab  items  builder      what it opens
 0     1    0x1400a4990  Equipment
 1     1    0x1400a4db0  Inventory
 2     2    0x1400a5620  Status, Info
 3     3    0x1400a4fc0  message write / read history / write history
 4     3    0x1400a5900  Game Options, Screen Options, Quit Game   <- the one measured
 5     2    0x1400a5330  Key Bindings, Graphics
```

Tabs 0 and 1 have four free slots each and tab 5 has three. `ds2_menu_row::Tab` has one variant
because the machinery is not quit-specific but the DATA is: a tab needs its cell namer's base path,
its container definition and its shipped child ids read out of `l02_01_In-Game.flo` before a row can
be put on it. `scripts/ds2-flo.py` decodes that file offline, so this is a bounded static job and not
a research question. Filed.

## What to read in the log

| line | what it means |
| --- | --- |
| `ds2-menu-row: config [menu_row] source=Rows rows=[...]` | which key was read and which rows were accepted |
| `... REFUSED-OVER-CEILING=[...]` | more rows were named than the tab holds |
| `... UNKNOWN=[...]` | a name that armed nothing, with every name that would have worked |
| `ds2-save-file: export armed route=... destination=...` | a destination was named; the save has been requested |
| `ds2-save-file: exported bytes=... destination=...` | the copy happened |
| `... THE FLUSH WAS NEVER OBSERVED` | the copy is the last autosave, not the moment of the press |
| `ds2-save-file: export REFUSED reason=destination-is-the-live-save` | the one refusal that protects a save |
| `ds2-save-file: swap staged kind=... steam-id=... into=...` | the pick was copied and rebound; nothing is armed yet |
| `ds2-save-file: swap at the title ...` | the game was left, and the character you left is in your own folder |
| `ds2-save-redirect: load-session armed slot=... original=...` | the loads now answer the staged copy; the saves do not |
| `ds2-save-file: re-read requested ... accepted=true` | the request went to the storage worker; the records are not filled yet |
| `ds2-save-file: swap ready -- the character list now describes ...` | the pump collected it, so the records now come from the staged copy |
| `ds2-save-file: swap ABANDONED -- the container re-read failed status=N ...` | the pump collected an error: the pick is not readable as a container |
| `ds2-continue: title-gate took action=2 dest=0x55-LoadDataList` | the character list is being opened for it |
| `ds2-save-file: swap done slot=N ...` | a character was chosen, and the save side is now armed too |
| `ds2-save-file: swap ABANDONED -- ...` | why it stopped, and whether both sides were put back |
| `ds2-save-file: import in-session unavailable reason=...` | the restart route ran instead, and why |
| `ds2-save-file: import recorded kind=... path=...` | restart route only: the pick is written for the next launch |
| `ds2-save-file: handoff taken path=... -- consumed` | the loader armed it and deleted the file |
| `ds2-save-redirect: save-dir redirected steam-id=... path=...` | the directory the game will actually use |
