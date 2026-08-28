# The save-slot load path, and what a native continue flow would have to do

Everything in the first three sections was read statically from `darksoulsii-deobf.bin` (SOTFS
build 9527516) with `scripts/ds2-rtti.py`, `scripts/ds2-disasm.py`, `scripts/ds2-xrefs.py` and the
Ghidra MCP daemon. **No game was launched to establish any of it**, which is also why the last
section is a list of open questions rather than a design.

## The chain

```
TopMenu 0x47  --row 1, action 2-->  LoadDataList 0x55  --phase 2-->  LoadProfile 0x57  -->  0x6a  -->  StartIngame 0x6b
```

`docs/DS2-TITLE-FLOW.md` establishes the first arrow: the top menu is a fixed vector of six rows,
row 1 is LOAD GAME, and its transition names substate `0x55`. This document establishes the rest.

The substate ids are the game's own, carried at `+0x0c` and matched by `FeStateFlow`'s transition
search. `0x57` was identified through its constructor at `0x1400faa10`, which writes id `0x57`
beside vtable `0x1410bd658`; the vtable's RTTI descriptor names `FeSubStateTitleLoadProfile`.

## `FeSubStateTitleLoadDataList::v3`, the whole decision

`0x1400fba10`. Inert unless the substate's phase is 1 -- its first three instructions are
`mov edx,[rcx+0x10]; dec edx; jne <return>`.

```c
group = FRONTEND_->field_0x98;            // FeGroupTitleDataList
group->vtable[4](group);                  // poll; this is what sets the action below
slot = FRONTEND_->_x564_slotNum;
rec  = savedata + slot*0x1F0;             // null unless 0 <= slot <= 9 and rec[0x1D9] & 1
switch (group[+0x28]) {                   // the confirmed action
  case 1: phase = FRONTEND_->field_0x56c ? 7 : 3;      // back out
  case 2: if (!rec || rec[0x1D9] & 2) return;          // load
          ok = FUN_140af6610(GameManagerImp->field24_0xc0, rec[0x1E8] & 0x3F);
          phase = ok ? 6 : 2;
  case 3: if (!rec || rec[0x1D9] & 2) return; phase = 4;
  case 4: if (!rec || rec[0x1D9] & 2) return; phase = 5;
}
```

The phase it writes is the whole output. `FeSubStateTitleLoadDataList::v5` (`0x1400fb1f0`)
publishes six `FeTransitionEqualValue<int>` objects, all watching that one field:

| phase | destination |
| --- | --- |
| 2 | `0x57` `FeSubStateTitleLoadProfile` |
| 3 | `0x47` `FeSubStateTitleTopMenu` |
| 4 | `0x56` |
| 5 | `0x5f` |
| 6 | `0x5d` |
| 7 | `0x17` `FeSubStateTitleMain` |

## The pointer walk, taken from the instructions rather than from a struct

```text
mov rax,[0x14160de10]            ; FE_TITLE_CONTEXT
mov rdi,[rax+0x98]               ; FeGroupTitleDataList
movsxd rdx,[rax+0x564]           ; the selected slot, SIGNED
mov rax,[0x1416148f0]            ; GAME_MANAGER_IMP
mov rcx,[rax+0xa8]               ; GameDataManager
mov r8,[rcx+0xd8]                ; the ten-slot array
cmp rdx,0xa / jae                ; ten slots; a negative slot means none
imul rax,rax,0x1f0               ; stride
test BYTE PTR [rbx+0x1d9],0x1    ; occupied
mov edx,[rdi+0x28]               ; the group's confirmed action
```

Every offset above is in `ds2-rva`. The stride is corroborated well beyond this function: the
image holds 43 `imul reg,reg,0x1f0` sites, nearly all preceded by the same `cmp reg,0xa; jae`.

### The slot index is not in the save file

`+0x1D9` is **zero in every record** of the `id=4` section of a real `.sl2`, so occupancy is
derived when the per-character entries are loaded, not persisted. Deciding cold whether a
configured slot exists must read entry content -- an occupied slot's `USER_DATA00N` differs from
the pristine default that an unused slot's still holds. See `scripts/ds2-sl2.py`.

Nor does the file record which slot was last used. The nearest thing it has is an accident: the
per-entry AES IVs share four half-words across entries written by one save operation, so the batch
containing the two global entries also names the character whose records were rewritten with them.

## What a continue flow looked like it would do

**This section's design is wrong.** It is kept because the next section is only legible
against it: both fields it proposes to write turned out to be outputs of the list group
rather than inputs to it, and one run of the recorder is what showed that.


Mechanically it is two writes: put the configured slot in
`ds2_rva::FE_TITLE_CONTEXT_SLOT_NUM_OFFSET`, and retarget the top menu's row-1 transition from
`0x55` to `0x57` so the character list is never entered.

**Two things say "looks like" rather than "is", and neither can be settled by reading more
disassembly.**

1. **Skipping `0x55` skips its `enter`.** `FeSubStateTitleLoadDataList::v1` (`0x1400fae80`) runs
   before any of the above, and the update reads the group at `[FE_TITLE_CONTEXT]+0x98` on every
   frame -- something has to have put it there. If `LoadProfile` depends on anything that `enter`
   sets up, a flow that jumps the list arrives with it stale.
2. **The ownership gate must survive.** `FUN_140af6610` takes `rec[0x1E8] & 0x3F` and its result
   selects phase 6 -- a *different destination* -- over phase 2. That is what refuses a character
   the running build cannot legitimately load. Bypassing it turns a clean refusal into a load that
   should not have happened.

`ds2-continue` is the instrument that answers both: it records what the game itself did on the
frame the player confirmed, and `ds2-boot-timeline` records the substate chain around it. Turn
both on for one run:

```bash
python3 scripts/ds2-run.py --boot-timeline --continue-record
```

The line to look for is `ds2-continue: data-list phase=1->2 ... dest=0x57-LoadProfile`, and the
`ds2-boot-timeline` lines bracketing it are the answer to question 1.

## Measured, one run, 2026-08-27

`--boot-timeline --continue-record`, one character loaded by hand. The chain is **longer than the
static read above predicted**, which had it as `0x57 -> 0x6a -> 0x6b`:

```text
0x47 TopMenu  --46.5s-->  0x55 LoadDataList  --1801ms-->  0x57 LoadProfile  --129ms-->
0x61  -->  0x63 PenaltyWarn  -->  0x65  -->  0x54  -->  0x6a  --994ms-->  0x6b StartIngame
```

Four substates sit between `LoadProfile` and `0x6a`. None of them was invented by the observation:
every destination matches what `FeStateTitle`'s factory registers -- `0x63`'s constructor writes id
99 with dest `0x65`, `0x65`'s writes dest `0x54`, `0x6a`'s writes dest `0x6b`. The static table and
the instrument corroborate each other; the traced part was simply short.

The recorder's own lines:

```text
ds2-continue: walk base=0x0000000140000000 phase=1 slot=1 action=0 record=resolved flags=0x0d
ds2-continue: data-list phase=1->2 slot=1 action=0 occupied=true excluded=false own=0x00 dest=0x57-LoadProfile
```

Three things from that:

* **The pointer walk resolves.** The image loads at its preferred base, and `savedata + 1*0x1F0`
  produced a live record.
* **`flags=0x0d`** -- bit 0 set (occupied) and bit 1 clear (not excluded), as the update requires,
  plus bits 2 and 3, whose meaning is not established.
* **`action=0` was a defect in the instrument, not a reading of the game.** Phase `1->2` is
  reachable only from action 2. The action is set by `group->vtable[4](group)`, the original's own
  first call, so a sample taken before the original carries the previous frame's value. The line
  now prints `action=<before>-><after>`; which of the two the game leaves behind after the branch
  is still an open question, and printing both is cheaper than guessing again.

### What this run did NOT settle

It recorded the **normal** path, so it says nothing about the shortcut. Whether `LoadProfile` works
when `LoadDataList`'s `enter` never ran is still open, and now visibly larger than it looked: the
list is resident for 1.8 seconds and six substates follow it, so "set the slot and retarget one
transition" is a claim about a chain, not about a jump. The ownership gate is also still untested
in the only direction that matters -- this character passed it (`own=0x00` took phase 2, not 6),
so the refusal branch has never been exercised.

## What was actually built, and the correction that made it work

Both writes the design above proposes are overwritten before the game reads them. The recorder
printed the proof on the frame the player confirmed a character:

* `_x564_slotNum` -- `slot=0->1`. The group's poll (`group->vtable[4](group)`, the update's own
  first call) republishes the cursor row into that field every frame, so a slot written before the
  original runs is gone before the `switch` reads it.
* `group+0x28` -- `action=0->0`. The poll clears the action every frame as well; it is non-zero
  only for the single frame the player pressed confirm.

So a pre-selection cannot survive a frame and an injected action cannot be seen. What works is to
stop steering the function and **take its branch directly.** `take_load_branch` in
`crates/ds2-continue/src/install.rs` replicates `case 2` in Rust -- resolve the record through the
same pointer walk, test `+0x1D9`, run the ownership gate, close the list, then write the slot and
the phase. The write lands *after* the original has returned, so nothing republishes over it.

**The ownership gate is preserved, not bypassed.** `FUN_140af6610` is two loads and a mask, so the
replication is pure reads -- `owned = (ctx[0x30] | ctx[0x28]) & required; refused = owned != required`
-- and a refused character still takes phase 6 to `0x5d`, exactly where the game would have sent
it. Same inputs, same destination; nothing is forged. That branch has still never been exercised,
because no character on this profile fails it.

Question 1 of the two above is moot rather than answered: the shortcut never skips `0x55`. It
enters the list substate normally, lets `v1` run, and leaves on the first frame the list is
interactive.

### The top menu goes too

`--continue-slot` alone still stops at PRESS ANY BUTTON and the six-row menu. A second detour, on
`FeSubStateTitleTopMenu::v3` (`0x1400ff300`), writes `FE_TOP_MENU_ACTION_LOAD_GAME` into the
substate's own phase field once the menu reports resting -- the same value row 1 would have
produced. Both shortcuts are one-shot (`TOP_MENU_FIRED`, `FIRED`): if the list bounces straight
back -- no save, a refused character -- a re-arming shortcut would ping-pong between the two
screens forever.

## Measured: boot to in-game with no input, 2026-08-27

`--boot-timeline --continue-slot 1`, controller untouched:

```text
ds2-continue: top-menu took action=2 dest=0x55-LoadDataList
0x55 LoadDataList                                              t=4575ms
ds2-continue: preselect slot=1->1 flags=0x0d armed=true
ds2-continue: autoload slot=1 refused=false phase=2 dest=0x57-LoadProfile
0x57 -> 0x61 -> 0x63 -> 0x65 -> 0x54 -> 0x6a -> 0x6b StartIngame  t=5841ms
```

Slot 0 and slot 1 were each confirmed in-game by the player -- the right character, not merely a
character. That is the only check that distinguishes a working autoload from one that loads
whatever the cursor happened to be sitting on.

Against `docs/DS2-BOOT-WORK.md`, where boot to the *top menu* measured 5875.7ms once the substate
floors landed: autocontinue is **in-game** at 5841ms. The character load costs roughly 1.3s and
the two skipped screens more than pay for it.

## Still audible

The title BGM and the frontend's own confirm sounds play through the whole shortcut, which is
conspicuous when nothing on screen appears to be pressing anything. Four static approaches have
been tried and none converged; recording them so the next attempt does not repeat them:

* **The frontend never calls `AppSoundManager` directly.** Every `e8` in `0x000f0000-0x00110000`
  was resolved against `0x002e9000-0x002ea000`: zero hits. Audio reaches the title through the
  scene/sequence layer -- `0x140afda00` (41 call sites) and `0x140afdb80` -- not through named
  sound calls in the substates.
* **FMOD's own mute levers are not in the code.** `getMasterChannelGroup`, `setMute` and
  `setPaused` appear only as name-table strings in the statically linked FMOD; no call site
  references them.
* **`User.Voice.Volume` is the wrong knob** -- it is console chat voice, not game audio.
* **`FeSubStateTitleStartIngame::v1` calls `0x14039a4d0(GameManagerImp, 0.0f, 1)`**, which looks
  exactly like "fade the music out at the handoff" and would be the ideal lever applied earlier.
  It is **unconfirmed**: the argument was traced to `GameManagerImp+0x1160`, but the scan that
  appeared to tie that field to `Sound::SoundManager` matched displacement `0x1160` on any
  register, so the sound-range hits were a different object's field. It may as easily be a screen
  fade. Do not build on it without a second, register-correct read.

`MOFmodSoundManager@DLMO`'s vtable at `0x1411841b8` has been narrowed to 19 float-handling
candidates -- v19, v39, v64, v65, v66, v69, v70, v71, v72, v73, v76, v77, v78, v79, v80, v81,
v101, v117, v120. The next move is a trace, not more reading: hold the title screen open
(`--continue-slot -1`) and break on those slots while the music plays. A *setter* will not fire
during steady playback, so the ones worth catching are the per-frame appliers -- the slots that
fire continuously.

## The caveat that governs every address here

The deobfuscated image is not the byte stream that runs. All four candidate sites were checked
with `scripts/ds2-arxan-chain.py`; `0x1400fae80`, `0x1400fb1f0` and `0x1400ff300` report clean
prologues. `0x1400fba10` reports `UNKNOWN` **only because the script's prologue table does not
carry `40 56` (`rex push rsi`)** -- the entry is ordinary code, not the five-byte `e9` an
Arxan-redirected entry holds. Worth adding to that table.
