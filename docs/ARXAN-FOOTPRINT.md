# Where Arxan is, and where it is safe to hook

Measured from `darksoulsii-deobf.bin` (build 9527516). Counts, not inferences.

`darksoulsii-deobf.bin` and the shipped `DarkSoulsII.exe` are the same image: mapped to memory
layout, every page of the two is byte-identical (`scripts/ds2-arxan-redirects.py --same-image`).
Any difference between a count taken on one and a count taken on the other is a difference of
method, never of bytes.

## The shape of it

`scripts/ds2-arxan-redirects.py` reproduces every number here, and with `--ghidra` diffs them
against the functions in the Ghidra DS2 program.

```
.pdata records (exception directory)     95434   (all in .text #1)
  of which chained (a fragment, not a start)  32447
  ARXAN-REDIRECTED (e9 rel32 -> .text#2)    286
    primary records (a function start)        226
    chained records (mid-function)             60

Ghidra functions                          88374
  ARXAN-REDIRECTED                          311   (Ds2ArxanStubs.java gives the same)
in both lists                               190
union                                       407   (407 distinct stub entry targets)
```

A redirected entry begins with an unconditional `e9 <rel32>` into `.text` #2 -- VA
`0x141aaf000`-`0x141d43000` -- which is Arxan's own code section, the one carrying the elevated
entropy noted in `PORTING.md`. Every redirect in the union goes to its own distinct stub entry.

### Why the two counts differ, and why neither is the total

The lists overlap in only 190 addresses, so the gap is not a matter of one side finding a few
extra. Each method misses a class of redirect the other sees:

- **Ghidra finds 121 that `.pdata` does not.** 120 of them are functions that no `.pdata` record
  covers at all -- they sit in the gaps between unwind records, and Ghidra found them through a
  call or other reference (`0x14014b8b0`, called from `assignPhantomProperties`, is one). The
  last, `0x14038cea5`, lies inside the primary record that starts at `0x14038c210`.
- **`.pdata` finds 96 that Ghidra does not.** 60 are chained records -- unwind entries for a
  fragment partway through a function, not function starts -- whose first bytes are an Arxan
  `e9`, so Arxan redirects mid-function blocks as well as entries. The other 36 are primary
  records at which Ghidra never created a function; `0x14014bce0`, for example, is reached only
  through data references and a computed jump out of Arxan's own code.

So Arxan's redirects number **at least 407**, not 286 and not 311. Neither method is exhaustive:
a redirected leaf function that nothing references directly and that has no unwind record would
appear in neither list. To ask whether one address is redirected, read its first five bytes
(`Ds2ArxanStubs.java <va>`), rather than looking it up in either list.

dearxan independently finds **48 stubs**. Hundreds of redirects into 48 stubs is consistent:
several entry points chain into a shared check network.

## Arxan took the hot functions

This is the part that matters for choosing a hook site. Ranking every function by static direct
call sites -- `e8 rel32` targets landing exactly on a known `.pdata` function start, 149022
resolved calls -- the top two are **both Arxan-redirected**:

| RVA | call sites | first bytes | verdict |
| --- | ---: | --- | --- |
| `0x00832cb0` | 12401 | `e9 c1 50 34 01` -> `0x141b77d76` | **Arxan. Do not hook.** |
| `0x00c2c9e0` | 4866 | `e9 ba e7 f3 00` -> `0x141b6b19f` | **Arxan. Do not hook.** |
| `0x00832e70` | 2052 | `48 89 5c 24 08 / 57 / 48 83 ec 20` | clean |
| `0x008389e0` | 1287 | `48 89 5c 24 08 / 57 / 48 83 ec 20` | clean |
| `0x00833dc0` | 1006 | `48 83 ec 28` | clean, but only 0x12 bytes long |

That is not a coincidence -- it is Arxan deliberately covering high-value code. Detouring one of
those redirects would mean writing over Arxan's own jump, which is the worst possible way to learn
whether our hooks survive: the experiment would fail for a reason that has nothing to do with
the question.

## The chosen M1 hook site

**RVA `0x00832e70`** -- VA `0x140832e70` at the preferred base.

- 2052 static call sites, so it is genuinely hot and a detour that never fires is a real signal
  rather than an expected one.
- Prologue `48 89 5c 24 08` is a single 5-byte instruction, so MinHook's displaced-instruction
  relocation is the trivial case.
- 0x47 bytes long -- ample room, no jump target inside the first five bytes.
- Not redirected: it is in neither the `.pdata` list nor the Ghidra list, and its first bytes are its own prologue.

Backup: **RVA `0x008389e0`**, same prologue shape, 1287 call sites, 0x4e bytes.

**Resolve it as `module_base + RVA` at runtime.** `DllCharacteristics` is `0x8160`, so
`DYNAMIC_BASE` is set and the loader may relocate the image. Do not hardcode `0x140832e70`.

## How this was derived

`.pdata` gives most function starts for free -- `RUNTIME_FUNCTION[]` at RVA `0x189a000`, 12
bytes each. Bound it by the exception directory's size, not the `.pdata` section's: the section
runs past the table, and that padding parses as six bogus records. About a third of the records
are chained (a fragment of a function, not its start), and leaf functions have none, so the table
is neither only starts nor every start. Counting `e8 rel32` call targets that land exactly on a
record still filters essentially all false positives without disassembling the image. The
redirect counts come from `scripts/ds2-arxan-redirects.py`; the call-site ranking came from a
scratchpad script that was not kept. All of it is reproducible from the image alone, with no
runtime and nothing that can be contaminated.

## What a redirected entry actually looks like

Walked statically with `scripts/ds2-arxan-chain.py`, no game running, nothing contaminatable.
`applySpEffect` (`0x14014bec0`) is the worked example because `ds2-net-effects` needs it:

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
(`0x141aaf000`-`0x141d42fff`), and leaves a five-byte `jmp` behind. The chain runs 16
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

1. **The entry, `0x14014bec0`.** Exactly five bytes of `e9 rel32` -- MinHook's minimum, and
   MinHook relocates `rel32`, so the trampoline holds the re-based `jmp` into the chain and
   preserves original behaviour. Every caller funnels through here. It is also the most direct
   possible confrontation with Arxan, because those five bytes are Arxan's own.
2. **The rejoin point, `entry+0x14`.** In game `.text`, past everything Arxan owns. The cost is
   that the prologue has already run -- `rsp` is down `0x30` and argument one is in `rdi`, not
   `rcx` -- so a detour here is not a normal function entry and must know the frame state.

### Why the M1 probe could not have answered the Arxan question

The same walk on M1's site (`0x140832e70`) terminates at hop 0: the function's
own prologue is at its own entry. It is **not redirected**, so a detour there never touches
Arxan's code at all. That is the mechanical reason both arms of M1 survived and why
`docs/ARXAN-PROBE.md` read the result as "Arxan never threatened this site." It is a null result
about Arxan by construction, and repeating it on another clean site would produce another one.

## What `neuter_arxan` actually does, and what it leaves alone

Read from dearxan's source (`../dearxan`, branch `wip/issue-11-entry-stub-prepatch`, which is the
path dependency `ds2-loader` links), then measured against `DarkSoulsII.exe`.

`neuter_arxan` -> `neuter_arxan_inner` -> `ArxanPatch::build_from_stubs` -> `apply_patch`. That
pipeline emits exactly two kinds of patch and nothing else:

- **`JmpHook { target: si.test_rsp_va, .. }`**, one per analyzed stub. It targets the stub's
  `test rsp, 0xf` -- the instruction dearxan scans for, because normal code has better ways to
  align a stack -- and redirects it to the stub's own `context_pop_va`, so the stub restores
  context and returns without doing its work.
- **`Write { va, bytes }`**, the decrypted contents of each encrypted region, pre-applied because
  the stubs that would have decrypted them are no longer running.

**No entry redirect is ever patched.** The commit named `wip: prepatch Arxan entry stubs before
CRT init` changes *when* patches are applied, not which. So the five-byte `jmp` at a redirected
function entry is identical with dearxan on and dearxan off.

### Measured on DS2

| | |
| --- | --- |
| stubs found | 48 |
| analyzed OK / errored | 48 / 0 |
| stubs declaring encrypted regions | 5 |
| inert stubs (no regions) | 43 |
| encrypted regions declared | 2969 |
| regions dearxan actually applies | **0** |

Every one of the 2969 is eliminated by `apply_relocs_and_resolve_conflicts`, which scores
`entropy` against `base_entropy` and drops a region list whose "decryption" does not look like
plaintext. So **on DS2 `neuter_arxan` applies zero `Write` patches**; its entire effect is 48
`JmpHook`s.

Elimination is dearxan declining to pre-apply a decryption it does not trust. It says nothing
about what a stub does at runtime, so the regions were checked directly, before resolution
(`../dearxan/examples/region-covers.rs`, untracked there):

```
regions_examined=2969 span=0x140001680..0x141cfa783
0x14014bec0: NOT covered by any declared encrypted region   (applySpEffect entry)
0x14014bed4: NOT covered by any declared encrypted region   (its rejoin point)
0x140832e70: NOT covered by any declared encrypted region   (the M1 site)
```

The span brackets all three addresses, so these are genuine gaps rather than a vacuous answer.

### What this settles, and what it leaves

Settled: arm A and arm B of an Arxan probe hook **byte-identical** entries, so the arms are
directly comparable -- this was the open blocker on finding out whether Arxan fights back against a hook on a redirected entry. And no declared decryption
would silently write original bytes back over a detour at any of the three sites, which was the
most likely mechanism for a hook to die without any integrity check being involved.

Left open, and now the *only* thing the runtime experiment has to answer: whether any of the 43
inert stubs reads those bytes and reacts. A stub with no encrypted regions still has a body, and
nothing here shows what it does.
