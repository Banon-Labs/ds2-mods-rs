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
