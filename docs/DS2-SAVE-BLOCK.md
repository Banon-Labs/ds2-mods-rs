# Refusing the game's own saves

`ds2-save-block` turns off every save DARK SOULS II performs on its own, so the only thing that writes
the container is `ds2-save-file`'s **Save Game to File** row. It installs when that row registers and
not otherwise; there is no key of its own in `ds2-mods.toml`, and removing `save-game-to-file` from
`[menu_row] rows` is how it is turned back off.

Every address and every field offset was read out of `darksoulsii-deobf.bin` (SOTFS build 9527516)
through the Ghidra daemon and `scripts/ds2-arxan-chain.py` before anything was written. What a run
has and has not shown is in [What a run measured](#what-a-run-measured) at the bottom, and it is
less than the reading: the refusal of a named request -- a bonfire, a quit to menu -- has not been
observed yet, because a session nobody is playing never asks for one.

Addresses are VAs, the form the disassembly prints. Subtract `0x140000000` for the RVA `ds2-rva`
records.

## The pipeline, and where the single chokepoint is

```text
bonfire / item pickup / souls / level up / quit to menu / new game cycle   (23 call sites)
  -> SaveLoadSystem::RequestSave  0x1402e7410   sets [sys+0x1a2], maybe [sys+0x1a3], min [sys+0x68]
the 300-second autosave timer                   sets [sys+0x1a2] from inside the update itself
  -> SaveLoadSystem::update       0x1402e6b00   reads [sys+0x1a2] and calls the performer
       -> the performer           0x1402e78c0   one xref in the whole image: the line above
            -> the poll           0x1402e67f0   drives the SLSession that writes the `.sl2`
```

`RequestSave` writes three bytes and returns; it does not save. The update is the only consumer of
those bytes and the only caller of the performer, so it is the one place a save can be stopped before
it exists. The detour goes there, clears the request fields, and then runs the original, which finds
nothing to do.

### Why not hook `RequestSave`

It would miss the autosave, which has no requester at all:

```asm
0x1402e6b99:  movaps xmm0,xmm6                     ; the frame delta, from xmm1
              addss  xmm0,DWORD PTR [rbx+0x64]
              comiss xmm0,DWORD PTR [0x1410d7b40]  ; 300.0
              movss  DWORD PTR [rbx+0x64],xmm0
              jb     <skip>
              ...                                   ; min(kind, 13); [rbx+0x1a2] = 1
```

That block is inside the update, above the consume block, and sets the same flag a bonfire sets.
Twenty-three detours on the requesters would still have let it through every five minutes. Zeroing
`+0x64` on entry each frame holds the accumulator at one frame's delta instead.

### Why not skip the performer

Because the update's bookkeeping happens whether or not the call is made:

```asm
0x1402e6b91:  cmp  BYTE PTR [rbx+0x1a2],al
              jne  <skip>
              ...
              call 0x1402e78c0              ; skipped only when GameManagerImp->PlayerCtrl is null
              mov  BYTE PTR [rbx+0x1a6],1   ; "a save is in flight" -- set either way
              mov  BYTE PTR [rbx+0x1a2],0
              mov  BYTE PTR [rbx+0x1a9],0
              mov  DWORD PTR [rbx+0x68],0xf
```

With `+0x1a6` set and no save actually running, the next frame's poll takes its idle exit --
`0x1402e6a86: mov eax,0x4` -- and the update stores that `4` into `[rbx+0x6c]`. That is a
save-failure status, and the engine's message for one is *"Failed to save game.\nReturning to Title
Menu."* (`ingamesystem.fmg` id 10002). Erasing the request never reaches that path; the four fields
cleared are the same four the game clears itself, so the state is one the engine already produces.

## The five fields

| field | what it is | what the detour does |
| --- | --- | --- |
| `+0x1a2` | a save is wanted | cleared |
| `+0x1a3` | kind 2's second flag, passed to the performer | cleared |
| `+0x1a9` | kind 14's flag, a save deferred by one frame | cleared |
| `+0x68` | the lowest kind asked for | set to `15`, the game's own idle value |
| `+0x64` | seconds toward the 300-second autosave | set to `0.0` every frame |
| `+0x1a6` | a save has been started | read only, to close a permit |

## Nothing waits for the save that no longer happens

The requester that re-asserts every frame (`0x1401bf9b5`, inside the state handler at `0x141bec2b7`)
reads the flag it set:

```asm
0x1401bf9b5:  movzx eax,BYTE PTR [rcx+0x1a5]
              and   al,BYTE PTR [rcx+0x1a4]
              test  BYTE PTR [rcx+0x1a2],al   ; a save is wanted, and saving is allowed
              je    0x1401bf9d4               ; -> straight on, no wait
              mov   edx,0x3
              call  0x1402e7410
0x1401bf9d4:  call  0x1401e6d60               ; unconditional either way
```

A clear flag means it stops asking, and the code after the call is unconditional. The quit path is
the same shape. No path in this system waits on a completion counter, so a save that never starts
does not hang anything.

## The permit

The two rows that ask the game to save on purpose call `ds2_save_block::permit_save()` immediately
before `RequestSave`. A permit is a frame countdown, not a boolean, because `RequestSave` only sets a
flag: the update performs the save some frames later, when `+0x1a4`, `+0x1a5` and the cooldown at
`+0x60` allow it. It is 600 frames -- twice `ds2_save_file::export`'s own 300-frame wait -- and it
closes early on the first frame the save it allowed has actually begun (`+0x1a6` set, both request
flags clear), so one press cannot cover the next bonfire. `policy::decide` is that state machine, and
it is tested on the host.

## What is out of scope

* **Writes that are not the character.** Three other functions build a save request without going
  near the in-game update: `0x1402e7450`, `0x1402e7f10` (reached from the title's character list at
  `0x1400faf80`, `0x1400fbba0`, `0x1400fc3d0` -- creating, deleting or copying a character) and
  `0x1402e7d20`, which is reached only through slot 4 of `FeRegulationSaveJob`'s vtable
  (`0x1410da1c0`). The first two are a player pressing a button on a list; the third is a frontend job
  for regulation data. None of them serialises the live character, which is what `0x1402e78c0` does
  and what this feature stops. Whether that job runs at quit has not been measured.
* **Loads.** Untouched, including `ds2-save-redirect`'s redirection of them and the in-session swap.
* **Progress.** This is the feature, not a side effect: with it on, anything gained since the last
  press of the row is gone when the game closes. `ds2-build-import` applies a build to the live
  character and relies on a later save to persist it; under this feature that save is the row press.

## What a run measured

`python3 scripts/ds2-run.py --all-menu-rows --continue-slot 0`, 2026-09-24, SOTFS build 9527516 under
Proton, DLL `1f12c451`. What the log says, in order:

```text
ds2-menu-row: registered RowId(2) row=save-game-to-file tab=Quit
ds2-save-block: installed at 0x00000001402e6b00 -- this run does not save by itself: ...
ds2-save-redirect: open-redirect ... write=true access=0x40000000 disposition=4 ... DS2SOFS0000.sl2
ds2-save-redirect: open-redirect ... write=true access=0x40000000 disposition=4 ... DS2SOFS0000.sl2
ds2-continue: silence restored by=start-ingame volume=1 source=game fmod_result=0
ds2-save-block: the save tick is live system=0x00007ffff03a7f50 wanted=false deferred=false \
                in-flight=false kind=15 autosave-elapsed=0.000s of 300s
```

Three things are settled by those lines. The row registers, so the gate works. The detour goes on the
address the table records, in the live process, with Arxan live. And the update really is called with
a game in progress -- which is the claim "installed" does not make, and the reason that line exists.

**The two write-opens are the title flow's, not the character's.** They arrive between the shortened
`process-window kind=101` -- the "saving system data" window -- and `start-ingame`, which is *before*
the first call to `SaveLoadSystem::update` in the whole run. That is the out-of-scope writer named
above, arriving exactly where the static reading says it would, and it is why the container's mtime
moves on a load even with this feature on.

### Then the run was quit, and a save was refused

The session was ended from the pause menu's own `Quit Game` row, five minutes in:

```text
ds2-menu-row: quit-to-desktop requested system=0x000000000010fa60 offset=0x13a value=1 requests=1
ds2-save-block: refused a save kind=10 deferred=false total=1 -- nothing was written; ...
ds2-offline: detach refused connect=0 sendto=0 resolve=1 allowed-loopback=0
```

**Kind 10 on the way out, and the container did not move.** Its length and mtime were identical before
and after the whole session -- the 18:12:44 stamp the load left, unchanged through five minutes in the
world and through the exit -- and its MD5 matched byte for byte. That is the feature doing the thing it
exists to do, on the path the user asked about first.

Two smaller findings ride along. The exit asks for kind 10, which is not a kind any of the shipped
callers of `RequestSave` passes explicitly -- worth remembering when reading that function's `min`. And
`quit-to-desktop`, whose own documentation says it quits "with no confirmation and no save", plainly
does reach a save request: what stopped the write here was this feature, not that row.

### What is not measured yet

* **Quit to the title menu**, as opposed to to the desktop. Its request site is read (`0x1401bf9cf`,
  kind 1 or 3, followed by unconditional code) and has not been run.
* **The permit.** No `Save Game to File` press has happened under this feature, so the countdown in
  `policy` has not been exercised against the live `+0x1a4`/`+0x1a5`/`+0x60` gate -- which is the one
  remaining way this feature could be wrong in the direction that costs something: a row that asks for
  a save and gets it erased.
* **The autosave, as an observation.** The 300-second timer was never reached in a run this short, so
  "held at zero" is still a reading of the code plus one first-frame sample rather than a measurement
  across the interval.
