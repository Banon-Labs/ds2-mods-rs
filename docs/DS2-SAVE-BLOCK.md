# Refusing the game's own saves

`ds2-save-block` turns off every save DARK SOULS II performs on its own, so the only thing that writes
the container is `ds2-save-file`'s **Save Game to File** row. It installs when that row registers and
not otherwise; there is no key of its own in `ds2-mods.toml`, and removing `save-game-to-file` from
`[menu_row] rows` is how it is turned back off.

**Nothing here has been run.** Every address and every field offset is read out of
`darksoulsii-deobf.bin` (SOTFS build 9527516) through the Ghidra daemon and
`scripts/ds2-arxan-chain.py`; every behavioural claim about the game is a claim about its
disassembly. The runtime test is the remaining step.

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
