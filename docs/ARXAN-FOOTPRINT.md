# Where Arxan is, and where it is safe to hook

Measured from `darksoulsii-deobf.bin` (build 9527516). Counts, not inferences.

## The shape of it

```
.pdata functions                        95434   (all in .text #1)
  ARXAN-REDIRECTED (e9 rel32 -> .text#2)  286
  clean prologue                        95148
distinct Arxan stub entry targets         286
```

Arxan owns **286 functions**, 0.3% of the binary. Each begins with an unconditional
`e9 <rel32>` into `.text` #2 — VA `0x141aaf000`–`0x141d43000` — which is Arxan's own code
section, the one carrying the elevated entropy noted in `PORTING.md`. Each redirect goes to its
own distinct stub entry.

dearxan independently finds **48 stubs**. 286 redirects into 48 stubs is consistent: several
entry points chain into a shared check network.

## Arxan took the hot functions

This is the part that matters for choosing a hook site. Ranking every function by static direct
call sites — `e8 rel32` targets landing exactly on a known `.pdata` function start, 149022
resolved calls — the top two are **both Arxan-redirected**:

| RVA | call sites | first bytes | verdict |
| --- | ---: | --- | --- |
| `0x00832cb0` | 12401 | `e9 c1 50 34 01` → `0x141b77d76` | **Arxan. Do not hook.** |
| `0x00c2c9e0` | 4866 | `e9 ba e7 f3 00` → `0x141b6b19f` | **Arxan. Do not hook.** |
| `0x00832e70` | 2052 | `48 89 5c 24 08 / 57 / 48 83 ec 20` | clean |
| `0x008389e0` | 1287 | `48 89 5c 24 08 / 57 / 48 83 ec 20` | clean |
| `0x00833dc0` | 1006 | `48 83 ec 28` | clean, but only 0x12 bytes long |

That is not a coincidence — it is Arxan deliberately covering high-value code. Detouring one of
those 286 would mean writing over Arxan's own jump, which is the worst possible way to learn
whether our hooks survive: the experiment would fail for a reason that has nothing to do with
the question.

## The chosen M1 hook site

**RVA `0x00832e70`** — VA `0x140832e70` at the preferred base.

- 2052 static call sites, so it is genuinely hot and a detour that never fires is a real signal
  rather than an expected one.
- Prologue `48 89 5c 24 08` is a single 5-byte instruction, so MinHook's displaced-instruction
  relocation is the trivial case.
- 0x47 bytes long — ample room, no jump target inside the first five bytes.
- Not in the 286.

Backup: **RVA `0x008389e0`**, same prologue shape, 1287 call sites, 0x4e bytes.

**Resolve it as `module_base + RVA` at runtime.** `DllCharacteristics` is `0x8160`, so
`DYNAMIC_BASE` is set and the loader may relocate the image. Do not hardcode `0x140832e70`.

## How this was derived

`.pdata` gives every function start for free — `RUNTIME_FUNCTION[]` at RVA `0x189a000`, size
`0x117978`, 12 bytes each. Counting `e8 rel32` call targets that land exactly on one of those
95434 starts filters essentially all false positives without disassembling 17 MB. Scripts are in
the scratchpad; the numbers above are reproducible from the deobfuscated image alone, with no
runtime and nothing that can be contaminated.

## What a redirected entry actually looks like

Walked statically with `scripts/ds2-arxan-chain.py`, no game running, nothing contaminatable.
`applySpEffect` (`0x14014bec0`) is the worked example because `ds2-mods-rs-a1g` needs it:

```
hop 0  0x14014bec0 [game ]  jmp rel            -> 0x141b3cbe1
hop 1  0x141b3cbe1 [arxan]  stack-swap thunk   -> 0x141be7325
hop 2  0x141be7325 [arxan]  stack-swap thunk   -> 0x141cf8f8f
hop 3  0x141cf8f8f [arxan]  fragment (11 insn) -> 0x141b72fc9
hop 4  0x141b72fc9 [arxan]  fragment  (5 insn) -> 0x14014bed4
hop 5  0x14014bed4 [game ]  REJOIN = entry+0x14
```

**Only the entry region is stolen.** Arxan takes the first `0x14` bytes of the function, shatters
them into basic-block fragments dispersed through its own `.text`
(`0x141aaf000`–`0x141d42fff`), and leaves a five-byte `jmp` behind. The chain runs 16
instructions of genuine prologue and then jumps back into the *original* function at
`entry+0x14`, where the entire remaining body sits untouched and in place.

The fragments are real code padded with obfuscation. Hop 3 is `applySpEffect`'s actual prologue:

```asm
mov  QWORD PTR [rsp+0x10],rbx     ; real
push rsp / mov rbx,[rsp] / ...    ; obfuscation: defeats stack-frame analysis
pop  rsp
mov  QWORD PTR [rsp],rdi          ; "push rdi", written obliquely
sub  rsp,0x30                     ; real frame setup
mov  rdi,rcx                      ; real: first argument
jmp  0x141b72fc9                  ; on to the next fragment
```

There is no relocated contiguous copy of the function to detour. There are two hook sites:

1. **The entry, `0x14014bec0`.** Exactly five bytes of `e9 rel32` — MinHook's minimum, and
   MinHook relocates `rel32`, so the trampoline holds the re-based `jmp` into the chain and
   preserves original behaviour. Every caller funnels through here. It is also the most direct
   possible confrontation with Arxan, because those five bytes are Arxan's own.
2. **The rejoin point, `entry+0x14`.** In game `.text`, past everything Arxan owns. The cost is
   that the prologue has already run — `rsp` is down `0x30` and argument one is in `rdi`, not
   `rcx` — so a detour here is not a normal function entry and must know the frame state.

### Why the M1 probe could not have answered the Arxan question

The same walk on M1's site (`0x140832e70`, `ds2-mods-rs-z6m`) terminates at hop 0: the function's
own prologue is at its own entry. It is **not redirected**, so a detour there never touches
Arxan's code at all. That is the mechanical reason both arms of M1 survived and why
`docs/ARXAN-PROBE.md` read the result as "Arxan never threatened this site." It is a null result
about Arxan by construction, and repeating it on another clean site would produce another one.
