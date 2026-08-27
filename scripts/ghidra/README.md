# `scripts/ghidra` -- static analysis of DARK SOULS II

Headless Ghidra tooling, ported from `er-mods-rs/scripts/ghidra/`. Static RE is the preferred way to
answer a question about this binary: it cannot be contaminated by stray input, needs no game launch,
and produces a fact rather than an observation.

## Setup, once

```bash
bash scripts/ghidra/import-gzf.sh                                   # ~1 min
bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Info.java 0x140832e70
```

The gzf is game-derived and is **not** in the repo -- supply your own via `GZF=`. The default path is
the local copy. `import-gzf.sh` builds a local project under `~/.cache/ds2-ghidra`; everything after
that is `query.sh`.

## The tools

| Script | Answers |
| --- | --- |
| `rt/Ds2Info.java` | What program is open, at what base, with what sections -- and do bytes at a VA match `darksoulsii-deobf.bin`? **Run this first on any new project.** |
| `rt/Ds2Syms.java` | Which symbols contain this keyword? The highest-value script here; see "Why DS2 is easy" below. |
| `rt/Ds2SymAddr.java` | What address is this exact symbol at? Prints every collision with its namespace. |
| `rt/Ds2Decomp.java` | Decompile the function containing this VA. |
| `rt/Ds2Disasm.java` | Disassemble it -- the prologue shape is how a hook site is judged. |
| `rt/Ds2Xrefs.java` | What references this address, and **how many**? The call-site count is how hook sites get picked. |
| `rt/Ds2Mem.java` | Read memory as pointers (vtables), image-relative offsets, bytes, or a string. |
| `rt/Ds2ByteScan.java` | Where does this masked byte pattern occur? `?` wildcards a nibble, for ModRM register fields. |
| `rt/Ds2SymCensus.java` | How MANY symbols match, and give me the whole table. `count` per-pattern totals; `dump` writes every symbol to a TSV so follow-up questions are grep, not another 15s round trip. |
| `rt/Ds2ArxanStubs.java` | Which functions are Arxan-redirected -- the whole census, or a verdict per VA. **Run this on any address before building on it.** |
| `sym-namespaces.py` | Namespace histogram over a `Ds2SymCensus dump`. Not a postScript, and deliberately not in `rt/`. |

## Verified against this repo's existing numbers

`Ds2Info` reports md5 `1cd709ec3319f05a0baa15a0b39b1759`, which is the installed
`DarkSoulsII.exe` byte for byte; image base `0x140000000`; max address `0x141d75717`
(= base + `SizeOfImage 0x1d76000`); and a `.bind` section at `0x141d43000` -- all matching
[`docs/PORTING.md`](../../docs/PORTING.md). 88780 functions, 426625 symbols.

At the M1 hook site the toolkit independently reproduces three recorded facts:

| Fact | Recorded in | Reproduced by |
| --- | --- | --- |
| prologue `48 89 5c 24 08` | `ds2-mods-rs-z6m`, live at runtime | `Ds2Mem bytes 0x140832e70` |
| 0x47 (71) bytes long | `ds2-mods-rs-z6m` | `Ds2Disasm` reports 71 |
| 2052 static call sites | `ds2-mods-rs-z6m` | `Ds2Xrefs` reports `calls=2052` |

So the project's addresses are 1:1 with the RVAs this repo records, and with the live process.

**One caveat that matters.** This project is the **shipped, Arxan-obfuscated** binary -- it has the
SteamStub `.bind` section. `darksoulsii-deobf.bin` is the dearxan-*deobfuscated* flat image. They
agree at every site Arxan did not touch, which is why the M1 site matches exactly. They will
**not** agree at the Arxan-redirected functions. Check before trusting a site: a leading
`JMP rel32` into the second `.text` block is a redirected stub. `rt/Ds2ArxanStubs.java` answers it
for a VA or for the whole image, and reports **311** such entry points over Ghidra's 88374
functions -- where [`docs/ARXAN-FOOTPRINT.md`](../../docs/ARXAN-FOOTPRINT.md) reports 286 over the
95434 function starts recovered from `.pdata`. Two different populations; the delta of 25 is not
reconciled.

## Why DS2 is unusually good at this

`Ds2Syms FeObjectButton` returns `class_FeObjectButton_RTTI_Type_Descriptor` and
`class_DLRF::DLRuntimeClassImpl<class_FeObjectButton,0>_RTTI_Type_Descriptor`. Functions carry class
namespaces (`EventCameraOperator::FUN_1400011b0`), and even build paths survive as strings
(`..\..\Source\DLRuntimeClass.cpp`). 5271 RTTI type descriptors and 587 DLRF-registered runtime
classes mean **identifying a class is reading its name**, not inferring it from vtable shape. What
is missing is method names -- functions are `FUN_<va>` under a named class. See
[`docs/DS2-ENGINE.md`](../../docs/DS2-ENGINE.md).

## Four traps, all of which cost a round trip here

**1. `-scriptPath` compiles the whole directory as one OSGi bundle.** One `.java` that fails to
compile poisons the bundle, and the script you actually asked for fails with `The class could not be
found` -- pointing at entirely the wrong file. That is why `rt/` exists and why **every `.java` in it
must compile**. Do not turn `rt/` into an archive of one-shot investigation scripts; that is the
state `er-mods-rs/scripts/ghidra/` is in, with ~80 historical scripts at its top level and at least
one that no longer compiles.

**2. `osgi.ee=UNKNOWN` is a JDK version problem wearing the same error message.** Ghidra 11.3.1
needs **JDK 21**; the system JDK here is 26. Under 26, bnd cannot map the class-file version to an
OSGi execution environment and emits `Require-Capability: osgi.ee;filter:="(osgi.ee=UNKNOWN)"`,
which Felix can never satisfy -- surfacing as the *same* `class could not be found` as trap 1. Both
scripts pin `JAVA_HOME`. If you have already produced a poisoned bundle, **the cache survives the
fix**: `rm -rf ~/.config/ghidra/ghidra_11.3.1_PUBLIC/osgi/{compiled-bundles,felixcache}` and re-run.

**3. `println` does not go to stdout.** It goes through log4j as
`INFO  Ds2Info.java> imageBase 140000000 (GhidraScript)`. The reflex fix -- piping through
`grep -v '^INFO'` -- deletes the entire output and leaves a run that looks like a silent success.
`query.sh` extracts script lines itself and always passes `ERROR`/exception lines through to stderr.
`DS2_GHIDRA_RAW=1` gives the unfiltered log.

**4. And a multi-line `println` loses everything after line one.** Only the FIRST line of a
log4j record carries the `INFO  <script>> ` prefix that `query.sh` matches, so one `println`
holding embedded newlines prints its first line and silently drops the rest. This is not
hypothetical: `rt/Ds2Decomp.java` shipped that way and **every decompile this repo ever ran
printed its `####` banner and no code**, which reads as "that function has no body". Fixed by
splitting per line. `rt/Ds2Mem.java` documents the same trap for its `bytes` mode. One `println`
per output line, always.

## What was deliberately NOT ported

`er-mods-rs/scripts/ghidra/` has ~130 files. Most are **Elden Ring archaeology**, not tooling: the
`AutoloadGate2..8` series, `TosGateProbe`, `PrivacyPolicyHunt`, `ProfilePortraitDig`, the
`TitleMenu*`/`Row*`/`Save*` families, `GfxDig`, `Y22iGxClusterMap`, `LoadingScreenMap`, and a
Japanese term dictionary (`jp-terms.json`, 118KB). Each encodes a specific ER structure offset or a
specific question already answered. Porting them would import foreign structure layouts into this
repo disguised as tools.

`BytePatternScan.java` is the instructive case, and the reason to read constants rather than
manifests. It looks generic. It is four hardcoded patterns for `[reg+0x1b4]`, an Elden Ring
`ChrLoadState` field. What was worth keeping was the *mechanism* -- masked `findBytes` to tolerate a
varying ModRM register field -- so `rt/Ds2ByteScan.java` keeps the mechanism and takes the pattern
from the caller. Same lesson `docs/PORTING.md` draws from `er-hook`: *"it has no dependencies" is
not the same as "it has no game knowledge."*

Also not ported, because nothing needs them yet:

- **The GhidraMCP daemon stack** (`bootstrap.sh`, `build-ghidramcp.sh`, `mcp-ghidra-daemon.sh`,
  `MCPServeHeadless.java`, `mcp_query.py`). A warm MCP server pays off when a session makes hundreds
  of queries; `query.sh` costs ~15s per run, which is fine until it isn't. Revisit when the Fe* menu
  work makes it hurt.
- **The RF function finder** (`find-functions-rf*.sh`, `FindFunctionStartsRF*.java`). It exists to
  recover function starts a stripped binary lost. This project already has 88780 functions.
- **`autotranslate-jp.py` / `jp-en-dict.json`.** ER-specific message-table translation.
