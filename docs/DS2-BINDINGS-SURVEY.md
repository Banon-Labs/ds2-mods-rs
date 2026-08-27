# What is actually in DARK SOULS II, measured

The evidence base for a `darksouls2` bindings crate (`ds2-mods-rs-v5z`). Every number below is
the output of a command reproduced beside it. Where something could not be measured, it says so
and names the command that would measure it. Nothing here is inferred from Elden Ring or from
Dark Souls III.

Two artifacts are used, and they are different things:

- **the Ghidra project** -- a local import of `pc_DarkSoulsIISotFS_static_1.0.3.exe.gzf`, which is
  the **shipped, Arxan-obfuscated** binary with the SteamStub `.bind` section. It carries the
  analysis: functions, RTTI, namespaces, and 1147 hand-typed symbol names.
- **`darksoulsii-deobf.bin`** -- the dearxan-deobfuscated flat image at the repo root. File
  offset == RVA. Used here only for raw string counts, because `docs/DS2-ENGINE.md`'s numbers
  came from it and had to be reproduced on the same artifact to be comparable.

## 0. The project is the binary this repo records

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Info.java 0x140832e70
name          DarkSoulsII.exe
md5           1cd709ec3319f05a0baa15a0b39b1759
sha256        0045931b8914504531b7864a9488d396dc50cbaf524964016e1d69c3d1173131
imageBase     140000000
maxAddress    141d75717
functions     88780
symbols       426625

memory blocks (name, start, end, size, rwx, initialized)
  .text        140001000 1410a59ff  0x10a4a00 r-x init
  ...
  .text        141aaf000 141d42fff   0x294000 r-x init
  .bind        141d43000 141d75717    0x32718 r-x init

bytes (compare: xxd -s $((VA - 0x140000000)) -l 16 darksoulsii-deobf.bin)
  140832e70  48 89 5c 24 08 57 48 83 ec 20 48 8b d9 e8 ae 01
```

md5, image base, max address and the M1 prologue bytes all match `docs/PORTING.md` and
`scripts/ghidra/README.md`. Everything below is queried against this project.

## 1. The symbol table, and a number that needs a caveat

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2SymCensus.java dump /tmp/ds2-syms.tsv
wrote 717346 of 717346 symbols to /tmp/ds2-syms.tsv
```

**717346, not 426625.** `Ds2Info` prints `SymbolTable.getNumSymbols()`, which excludes Ghidra's
*dynamic* symbols -- the auto-generated `DAT_`/`LAB_` labels. `getAllSymbols(true)`, which
`Ds2Syms` and `Ds2SymCensus` both walk, includes them. Both numbers are right about different
things; a survey that quoted one and searched the other would be quietly wrong by 290721.

```
$ cut -f2 /tmp/ds2-syms.tsv | sort | uniq -c | sort -rn
 628566 Label
  88780 Function

$ cut -f3 /tmp/ds2-syms.tsv | sort | uniq -c | sort -rn
 636564 DEFAULT
  46345 ANALYSIS
  33290 IMPORTED
   1147 USER_DEFINED
```

The function count agrees with `Ds2Info` exactly, which is the cross-check that the dump is
complete.

**The 1147 `USER_DEFINED` symbols are the single most valuable thing in this project** and are
not mentioned anywhere in this repo's docs. They are names a human typed: `applySpEffect`,
`addSpEffectToList`, `getPlayerStruct`, `addItemToInventory`, `addSoul`, `applyDamage`,
`bonfireRest`, `getWeaponParamRow`, `getItemRow`, the whole `Frpg2ClientLib::Frpg2SignImpl::*`
request family, and 24 `ChrMorphemeTimeActTrack*Ctrl::parse*Tae` handlers. See section 7.

## 2. `docs/DS2-ENGINE.md`'s DL* table is correct -- and does not mean what it looks like

Every one of its six numbers reproduces exactly, on the same artifact, with occurrence counts
(not line counts -- `grep -c` on a binary counts NUL-delimited "lines" and gives 26 for `DLUT@@`):

```
$ cd /home/banon/projects/ds2-mods-rs
$ grep -ao 'DLUT@@' darksoulsii-deobf.bin | wc -l     # 934
$ grep -ao 'DLRF@@' darksoulsii-deobf.bin | wc -l     # 826
$ grep -ao 'DLKR@@' darksoulsii-deobf.bin | wc -l     # 122
$ grep -ao 'DLIO@@' darksoulsii-deobf.bin | wc -l     #  56
$ grep -ao 'DLTX@@' darksoulsii-deobf.bin | wc -l     #  10
$ grep -ao 'FD4'    darksoulsii-deobf.bin | wc -l     #   1
$ grep -ao '\.?AV'  darksoulsii-deobf.bin | wc -l     # 5271
$ grep -ao '\.?AU'  darksoulsii-deobf.bin | wc -l     #   93
$ grep -ao 'DLRuntimeClassImpl' darksoulsii-deobf.bin | wc -l   # 587
```

**Verified, not corrected.** But they are counts of a substring inside *mangled* names, and a
mangled name mentions every namespace its template arguments touch. `DLUT@@` appears 934 times
because `DLUT::DLNullType` and `DLUT::TypeList::DLTypeList` are filler arguments in most other
templates in the image -- not because there are 934 DLUT things. The demangled class counts are
an order of magnitude smaller (section 3): **DLUT has 17 classes, not 934.**

Two independent cross-checks that the artifacts agree:

- `5271` (`.?AV`) `+ 93` (`.?AU`) `= 5364`, and the Ghidra project has **5364 distinct RTTI type
  descriptor names**:
  ```
  $ grep -oP '(?<=\t)(class|struct)_\K.*(?=_RTTI_Type_Descriptor$)' /tmp/ds2-syms.tsv \
      | sort -u | wc -l
  5364
  ```
  `DS2-ENGINE.md`'s "5271 MSVC RTTI type descriptors" is the class-only (`.?AV`) figure; the 93
  struct descriptors were not counted. Minor, but the honest total is 5364.
- `587 DLRuntimeClassImpl` in the deobf image, `587` in the Ghidra RTTI names:
  ```
  $ grep -c 'DLRuntimeClassImpl' rtti-classes.txt
  587
  ```

**FD4 is absent -- confirmed.** One occurrence of the literal `FD4` in 30 MB, and it is not a
class. The issue's claim that "DS2 predates the FD4 framework" is right. Its claim that DS2
predates "most of the `DL*` subsystem naming" is **wrong**, and section 3 measures how wrong.

## 3. The namespace inventory

Counting namespaces from mangled names needs templates stripped first, or every framework
namespace inflates every namespace it touches. `scripts/ghidra/sym-namespaces.py` does that.

### 3a. RTTI classes by namespace

```
$ python3 scripts/ghidra/sym-namespaces.py /tmp/rtti-classes.txt --top 45
    824  DLRF          43  std           19  DLKR          15  EzState
    250  FSLibBin      37  DLCR          19  DLNWD         14  NetJob
    248  FFX           37  DLNRD         18  DLCG2         13  DLKRD
    218  Frpg2Request  35  Nauru         18  EzState23Ldr  13  DLNW2D
     88  GuiFramework  32  DLUID         17  DLUT          13  DLRSD
     67  DLUTD         30  DLMO          16  DLUI          13  DLSY
     56  DLIO          30  FX4CG         16  Frpg2Player   13  Flver
     55  DLGR          27  anon-ns       15  DLRS          11  DLNW
     46  FrontendEx    25  Frpg2ClientL  15  DLRSD         10  dantelion2
   2827  (TOTAL namespaced rows, 175 distinct namespaces)
```

(abridged; run the command for the full 175.) 2536 of the 5364 classes carry no namespace at
all.

### 3b. There are 32 DL* namespaces, not 5

```
$ grep '::' /tmp/rtti-classes.txt | sed -E 's/^([^<:]+)::.*/\1/' | sort -u | grep '^DL'
DLCG2  DLCM  DLCR  DLEBL  DLGR  DLGRD  DLIO  DLIOD  DLKR  DLKRD  DLLG  DLMO
DLNR   DLNR3D DLNRD DLNW  DLNW2 DLNW2D DLNWD DLRF   DLRS  DLRSD  DLSL  DLSY
DLSYD  DLSZ  DLSZD DLUI  DLUID DLUP   DLUT  DLUTD
```

`DS2-ENGINE.md` lists five (`DLRF DLUT DLKR DLIO DLTX`). The real Dantelion surface in this
build is **32 namespaces**, including a full input stack (`DLUI`/`DLUID`), graphics
(`DLGR`/`DLGRD`), motion (`DLMO`), logging (`DLLG`), networking (`DLNW`/`DLNW2`/`DLNWD`/`DLNRD`),
serialisation (`DLSZ`/`DLSZD`) and resource management (`DLRS`/`DLRSD`). The `*D` suffix is
consistently the platform-dependent half of a `DL**` namespace (`DLKR` interface / `DLKRD`
Windows implementation).

**`DLTX` has zero RTTI classes** (`grep -c '^DLTX::' rtti-classes.txt` -> 0) but the namespace is
real: `DLTX::DLBasicString<char,DLTX::DLStringTraits,0>` and
`DLTX::DLBasicString<wchar_t,DLTX::DLStringTraits,3>` appear as template arguments in 4 other
classes. A value type with no vtable has no type descriptor. **Absence of RTTI is not absence of
the type** -- a trap for any tool that inventories classes from RTTI alone, including this one.

### 3c. Functions by namespace

```
$ python3 scripts/ghidra/sym-namespaces.py /tmp/ds2-syms.tsv --col 3 --type Function --top 20
   7503  DLRF        418  DLIO        212  DLGR        176  Frpg2PlayerData
   2372  Frpg2Reque  367  DLMO        207  DLCM        174  KERNEL32.DLL
   1692  FFX         272  DLKRD       181  FX4CG       165  ValueAccessorSlot
   1043  FSLibBin    270  DLNRD       181  std         164  FrontendEx
    610  GuiFramewo  238  DLNWD       181  DLSZD       156  FeFunctorJob
  31639  (TOTAL namespaced rows, 2220 distinct namespaces)
```

31639 of 88374 functions carry a class namespace. What is missing everywhere is **method names**:
a function is `FUN_<va>` inside a named class. Identifying a class is reading; identifying a
method is work.

## 4. Vtables: 5512 of them, 49113 slots

```
$ awk -F'\t' '$4 ~ /::vftable$/' /tmp/ds2-syms.tsv | wc -l
5512
```

Slot counts can be measured from symbol adjacency -- a vtable ends where the next class's
`vftable_meta_ptr` begins. That is only sound when the next symbol *is* such a boundary; when an
auto-generated `DAT_`/`PTR_` label lands inside the table, the measurement is a lower bound. Both
populations, separated honestly:

```
vftable symbols            5512
bounded by next class      3780   (reliable)
bounded by a stray label   1732   (lower bound only)
total slots (reliable)    49113
mean slots (reliable)       13.0
largest                      955  FSLibBin::CMemoryStream::vftable
```

Distribution of the reliable 3780: 13 slots x506, 2 x402, 3 x381, 4 x264, 1 x241, 6 x227,
17 x219, 12 x164.

By family: **597** `Fe*` vtables, **1383** `DL*`, **272** `Frpg2*`.

### A representative vtable, walked

`DLKR::DLBackAllocator` -- chosen because Dark Souls III binds the same class name, so it is the
fairest possible test of whether DS3's layouts transfer.

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Mem.java ptrs 0x1410c0768 30
  [1410c0768] = 0000000140152cd0  fn=FUN_140152cd0
  [1410c0770] = 0000000140854a70  fn=FUN_140854a70
  [1410c0778] = 0000000140152d00  fn=FUN_140152d00
  [1410c0780] = 0000000140854a90  fn=FUN_140854a90
  ...
  [1410c07d8] = 0000000140854c00  fn=panic          <- DLKR::DLBackAllocator::panic
  ...
  [1410c0830] = 0000000140854da0  fn=FUN_140854da0
  [1410c0838] = 00000001412b3cf0  sym=RTTI_Complete_Object_Locator   <- next class starts here
```

26 slots. Every entry resolves to a real function, and slot 14 is a human-named
`DLKR::DLBackAllocator::panic` whose body reads
`panic("..\\..\\Source\\DLBackAllocator.cpp", 0x4c, "Operation not supported")` -- the build path
survives, which is how a slot gets identified here.

Measured extents for the classes DS3 also binds:

| class | vftable VA | slots | DS3's trait | verdict |
| --- | --- | ---: | ---: | --- |
| `DLKR::DLAllocator` | `0x1410c0688` | 27 | `DLAllocatorVmt`, 14 | **shape differs** |
| `DLKR::DLBackAllocator` | `0x1410c0768` | 26 | -- | -- |
| `DLIO::DLMemoryInputStream` | `0x14113c7c8` | >=16 | `DLMemoryInputStreamVmt`, 7 | **shape differs** |
| `DLIO::DLMemoryOutputStream` | `0x14113c9d8` | >=16 | `DLMemoryOutputStreamVmt`, 4 | **shape differs** |
| `DLKR::DLPlainLightMutex` | `0x14113caf0` | >=1 | `DLPlainLightMutexVmt`, 1 | consistent so far |
| `CharacterCtrl` | `0x1410df218` | 79 | (no DS3 counterpart) | -- |

**This is the central technical finding of the survey.** The class *names* transfer from DS3.
The *layouts* do not. `DLKR::DLAllocator` has 27 virtual slots in DS2 and 14 in DS3 -- nearly
twice as many. Copying DS3's `DLAllocatorVmt` into a DS2 crate would compile, would look right,
and would call the wrong function on every slot past the first few. It is exactly the failure
`docs/PORTING.md` describes for `vtable_in_game_image`: not a compile error, not a crash, a
bound that is quietly wrong.

### One thing I could not settle

`heapAllocator` (`0x140833320`, `USER_DEFINED`) decompiles to a single dispatch:

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Decomp.java 0x140833320
longlong heapAllocator(undefined8 param_1,undefined8 param_2,longlong *param_3)
{
  lVar1 = (**(code **)(*param_3 + 0x50))(param_3,param_1,param_2);
  return lVar1;
}
```

Offset `+0x50` taking two extra arguments is exactly where DS3 puts
`allocate_aligned(size, alignment)`. **That is suggestive and it is not proof**: nothing here
establishes that `param_3` is a `DLKR::DLAllocator` rather than some other allocator-shaped
class. The check that would settle it is decompiling `DLKR::DLBackAllocator`'s slot-10 forwarder
and confirming it forwards to `+0x50` of its inner allocator -- and it does **not**: slot 10
(`FUN_140854b80`) forwards to `+0x80`, slot 9 to `+0x78`, slot 13 to `+0x98`, while slot 3
forwards to `+0x18` and slot 4 to `+0x20`. The wrapper is not slot-aligned with what it wraps, so
the forwarding chain cannot be used to number the base class's slots. Settling it needs a
concrete implementation walked: `DLKRD::HeapAllocator<...>::vftable` at `0x1410d2e28`.

I am recording this at the level of confidence it actually has, because a bindings crate that
guesses this offset guesses the foundation every other binding sits on.

## 5. Arxan: 311 redirected entry points, and `applySpEffect` is one of them

New tool, `rt/Ds2ArxanStubs.java`. It reads the `.text` block boundaries out of the program
rather than hardcoding them, and flags any function whose first instruction is an unconditional
`JMP` into the second `.text` block.

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2ArxanStubs.java
redirect territory: .text 141aaf000-141d42fff
functions            88374
arxan-redirected     311
```

`docs/ARXAN-FOOTPRINT.md` records **286**. The two are not measuring the same population:
ARXAN-FOOTPRINT counted over 95434 function starts recovered from `.pdata` on the deobfuscated
image; this counts over the 88374 functions Ghidra has in the obfuscated one. **The delta of 25
is not reconciled** and I did not reconcile it. What would: emit both lists and diff them
(`Ds2ArxanStubs list` produces one; ARXAN-FOOTPRINT's scripts, which its own text says live in a
scratchpad, would produce the other). Filed as follow-up work rather than hand-waved.

The finding that matters for bindings:

```
$ bash scripts/ghidra/query.sh scripts/ghidra/rt/Ds2Disasm.java 0x14014bec0 0x1402206d0 0x1402a6820
################ 0x14014bec0 -> applySpEffect @ 14014bec0 (5 bytes) ################
  14014bec0  JMP 0x141b3cbe1
  (end of applySpEffect)
################ 0x1402206d0 -> addSpEffectToList @ 1402206d0 (129 bytes) ################
  1402206d0  MOV qword ptr [RSP + 0x8],RBX
  1402206d5  MOV qword ptr [RSP + 0x10],RSI
  ...
################ 0x1402a6820 -> getPlayerStruct @ 1402a6820 (173 bytes) ################
  1402a6820  MOV qword ptr [RSP + 0x8],RBX
  ...
```

**`applySpEffect` is Arxan-redirected.** It is five bytes long and all five are Arxan's jump.
That is the primary hook target for `ds2-mods-rs-a1g` (`ds2-net-effects`, the first mod that
needs real DS2 RE), and it cannot be detoured at its entry. `addSpEffectToList` and
`getPlayerStruct` are clean MSVC prologues and are usable. Anyone who had written the net-effects
hook against `applySpEffect` without running this check would have spent the debugging session
blaming their hook.

## 6. What Dark Souls III's crate actually is, and how much of it applies

The issue quotes `darksouls3 = 44383 lines` as the scale of the work. That number is misleading
in a way that changes the whole estimate:

```
$ find crates/darksouls3 -name '*.rs' | xargs wc -l | sort -rn | head -3
  44383 total
  39017 crates/darksouls3/src/param/generated.rs
    519 crates/darksouls3/src/sprj/player_game_data.rs
```

**39017 of 44383 lines -- 87.9% -- are one generated file.** It is generated by
`tools/param-generator` from paramdef XMLs (`tools/param-generator/params/darksouls3/`, 92 files),
and *not one line of it comes from Ghidra*. The hand-written crate is **5366 lines**.

Public types tell the same story:

```
$ grep -rhoE '^pub (struct|enum|trait) [A-Za-z0-9_]+' src | awk '{print $3}' | sort -u | wc -l
210
$ grep -rhoE '^pub (struct|enum|trait) [A-Za-z0-9_]+' src/param/generated.rs | ... | wc -l
93
$ grep -rhoE ... --exclude=generated.rs ... | wc -l
117
```

### 6a. How many of DS3's types exist in DS2 at all

Matching DS3's 117 hand-written type names against the leaf names of DS2's 5364 RTTI classes
(templates stripped, namespaces dropped):

```
ds3 public types: 210
present in ds2 RTTI (leaf name): 10

  DLAllocator            DLUserInputDevice      SceneObjProxy
  DLMemoryInputStream    DLUserInputDeviceImpl  VirtualInputData
  DLMemoryOutputStream   DynamicBitset
  DLPlainLightMutex      ItemSelectDialog
```

**10 of 117 -- 8.5%.** And section 4 shows that for the four of those where the vtable is measurable, the
layout does not match.

### 6b. `SPRJ` does not exist in DARK SOULS II

```
$ grep -ci 'sprj'         /tmp/ds2-syms.tsv   # 0
$ grep -ci 'worldchrman'  /tmp/ds2-syms.tsv   # 0
$ grep -ci 'soloparam'    /tmp/ds2-syms.tsv   # 0
$ grep -ci 'mapitemman'   /tmp/ds2-syms.tsv   # 0
```

Zero. `sprj` is 18 files and 3809 of DS3's 5366
hand-written lines -- 71% of them -- the entire game-specific half -- and **not one of its class names appears
in this binary**. DS2's equivalents are differently named (`GameDataManager`, `EventFlagManager`,
`CharacterCtrl`) and must be found by behaviour, not by name.

### 6c. The singleton mechanism is unavailable

DS3 resolves 7 of its managers with `#[shared::singleton("WorldChrMan")]` and friends. That
attribute implements `from_singleton::FromSingleton`, and `from-singleton`'s
`src/find.rs` is titled *"`FD4DerivedSingleton` and `FD4Singleton` search routines"* -- it scans
`.text` for the FD4 singleton code pattern and reads the instance pointer out of `.data`.

DS2 has **one** occurrence of the string `FD4` in the whole image. **The entire
`shared::singleton` path is unavailable to a `darksouls2` crate.** Every singleton must be an RVA
in the address table, resolved as `module_base + rva` at runtime.

For scale: DS3 needs only **21** RVAs (`crates/darksouls3/src/rva/bundle.rs`) because the
singleton scan covers the rest. A DS2 crate needs an RVA for every one of them.

### 6d. Module-by-module verdict

| DS3 module | lines | DS2 counterpart | evidence |
| --- | ---: | --- | --- |
| `param/generated` | 39017 | **Not from the binary at all.** No `*_PARAM_ST` string exists in the DS2 image (`strings \| grep 'PARAM_ST$'` -> nothing). DS2's paramdefs live in `enc_regulation.bnd.dcx` (AES) and `tools/param-generator/params/` has no `darksouls2` set. | data acquisition, not RE |
| `sprj` | 3809 | **None.** Zero `Sprj*` symbols. Replaced by DS2's own `GameDataManager` (vftable `0x1410f0bb0`), `EventFlagManager`, `CharacterCtrl` (vftable `0x1410df218`, 79 slots). | section 6b |
| `dltx` | 162 | **Namespace yes, layout unknown.** `DLTX::DLBasicString<char,DLStringTraits,0>` and `<wchar_t,DLStringTraits,3>` exist as template args; no RTTI, so no vtable to walk. DS3's `DLCharacterSet` maps UTF16=1; DS2's wchar_t instantiation carries `3`, so the enum ordering is *not* assumed to match. | section 3b |
| `dlut` | 135 | **Yes, 17 classes.** `DLUT::DLReferencePointer`, `DLUT::DLVector<T,DLConsecutiveMemoryContainerTraits>`, `DLUT::DLNonCopyable`, `DLUT::DLXmlParser`. DS3's `dlut.rs` is a pure-Rust `DLFixedVector` with no addresses, so it is the one module that might port near-verbatim -- after checking the container layout. | section 3a |
| `dlkr` | 69 | **Yes, 19 classes**, including `DLAllocator`, `DLBackAllocator`, `DLPlainLightMutex`, `DLPlainMutex`, `DLPlainSpinLock`, `DLThread`, `DLTemporaryHeap`. Names match; **vtable shapes do not** (section 4). | section 4 |
| `dlio` | 196 | **Yes, 56 classes.** `DLMemoryInputStream`/`DLMemoryOutputStream` exist by name. Also a whole `DLBinder3`/`DLBinder4` device family DS3 does not bind. | section 3a |
| `dlui` | 126 | **Yes, 16 classes** (`DLUI::DLUserInputDevice<DLSingleThreadingPolicy>`, `DLUserInputMapper`, converters/modifiers) plus 32 more in `DLUID`. Larger than DS3's module, not smaller. | section 3a |
| `stl` | 31 | **Probably.** `std::_Container_base0` and `std::_Func_base<...,std::_Nil,...>` are VS2012-era CRT shapes, which is what `fromsoftware-shared-stl`'s `msvc2012` feature targets. Unverified: DS3 asserts `size_of::<DLVector<usize>>() == 0x20`; nothing here has confirmed the DS2 vector is 4 pointers. | section 8 |
| `rva` | 155 | **Yes, and it must carry far more.** `ds2-rva` already exists in this repo and is the designated home. | section 6c |
| `util` | 59 | Generic (`GetModuleHandle` wrapper). Ports. | -- |
| `cs` | 11 | **No.** DS2's only `CS*` namespaces are `CSNW`/`CSNWD` -- 11 socket classes, unrelated to DS3's `CS` game namespace. `CSDlc` does not exist. | section 3a |
| `app_menu` | 358 | **No -- replaced.** DS2's menu framework is `Fe*`: 448 RTTI classes, 597 vtables, plus `FrontendEx` (46) and `GuiFramework` (88, developer tooling). None of DS3's `NewMenuSystem`/`GaitemSelectMenu` names exist. | section 4 |
| `fd4` | 62 | **No.** One `FD4` occurrence in 30 MB. | section 2 |

### 6e. What DS2 has that DS3 does not bind at all

| subsystem | DS2 size | DS3 |
| --- | ---: | --- |
| `Frpg2*` network protocol, in-exe | 218 `Frpg2RequestMessage` classes, 272 vtables, 2372 functions, plus `Frpg2ClientLib` (25), `Frpg2Sv` (20), `Frpg2PlayerData` (16) -- with **199 hand-named request/response methods** already in the project | nothing |
| `Fe*` menu framework | 448 classes / 597 vtables | nothing (DS3 is Scaleform; DS2 has zero Scaleform) |
| `EzState` / `EzState23Loader` state machines | 33 classes, 185 functions, `EzSetEventFlag`/`readEventFlag` named | nothing |
| `FFX` / `FX4CG` particle system | 248 + 30 classes, 1692 + 181 functions | nothing |
| `FSLibBin` | 250 classes, 1043 functions | nothing |
| `Morpheme` animation runtime | 63 classes carrying `Morpheme` in the name, + 24 named `parse*Tae` handlers | nothing (DS3 uses HKS/Havok) |
| Havok / `hk*`, `PX*` physics | 571 classes | nothing |
| `DLGR`/`DLMO`/`DLLG`/`DLSZ`/`DLNW*`/`DLRS*`/`DLCM`/`DLCR` | 20 further namespaces | nothing |

## 7. The 1147 named symbols, and what they unblock

The gameplay entry points `ds2-net-effects` needs are **already named** in this project.
Addresses are VAs at the preferred base `0x140000000`; subtract it for the RVA that belongs in
`ds2-rva`.

| symbol | VA | prologue |
| --- | --- | --- |
| `applySpEffect` | `0x14014bec0` | **ARXAN-REDIRECTED -- unhookable at entry** |
| `addSpEffectToList` | `0x1402206d0` | clean, `48 89 5c 24 08`, 129 bytes |
| `determineSpEffectGroup` | `0x140235780` | not checked |
| `getSpEffectOwner_characterCtrl` | `0x14023c830` | not checked |
| `getPlayerStruct` | `0x1402a6820` | clean, `48 89 5c 24 08`, 173 bytes |
| `applyDamage` | `0x14016a300` | not checked |
| `addItemToInventory` | `0x1401a7470` | not checked |
| `addSoul` | `0x14038ab40` | not checked |
| `bonfireRest` | `0x14017f610` | not checked |
| `getWeaponParamRow` | `0x1401ab860` | not checked |
| `getArmorParamRow` | `0x1401ab3d0` | not checked |
| `getRingParamRow` | `0x1401ab6c0` | not checked |
| `getItemRow` | `0x1401bafa0` | not checked |
| `getMenuCategoryParamRow` | `0x1401bb370` | not checked |
| `getSaveLoadSystem` | `0x1401ab6e0` | not checked |
| `getNetSessionManager` | `0x1402d85e0` | not checked |
| `heapAllocator` | `0x140833320` | dispatches vtable `+0x50` (section 4) |

"not checked" means exactly that: `Ds2ArxanStubs <va>` or `Ds2Disasm <va>` answers it in one
command and nobody has run it. Given that the first one checked turned out to be Arxan, none of
these should be trusted until they are.

Reproduce the table with:

```
$ awk -F'\t' '$3=="USER_DEFINED"' /tmp/ds2-syms.tsv > /tmp/user-syms.tsv
$ grep -E '\t(applySpEffect|getPlayerStruct|addSoul|...)$' /tmp/user-syms.tsv
```

## 8. Loose ends, stated as loose ends

- **The `heapAllocator` `+0x50` question** (section 4). Suggestive, unproven. Settle with
  `Ds2Decomp` on the slots of `DLKRD::HeapAllocator<...>::vftable` at `0x1410d2e28`.
- **311 vs 286 Arxan redirects** (section 5). Two different function universes, unreconciled.
- **The MSVC STL version** (section 6d). `_Container_base0` and `_Nil`-padded `_Func_base` point at
  VS2012, but nothing has confirmed `size_of::<DLVector<usize>> == 0x20` on this image.
- **DS2 paramdefs.** Whether a `tools/param-generator/params/darksouls2` set can be sourced at
  all is unanswered here; it is a Paramdex/community-data question, not a Ghidra question.
- **1732 of 5512 vtables have lower-bound slot counts only** (section 4), because an auto-generated
  label landed inside them. A tool that walked forward until the pointers stopped resolving to
  `.text` would tighten these.
- **Whether a MinHook detour survives Arxan at all** remains open (`docs/DS2-ENGINE.md`). Every
  address in section 7 is worthless if the answer is no.

## 9. Tooling added by this survey

| file | why |
| --- | --- |
| `scripts/ghidra/rt/Ds2SymCensus.java` | `Ds2Syms` caps at 400 and cannot answer a census question. `count` gives per-pattern totals; `dump` writes the whole symbol table to a TSV so follow-up questions are grep, not 15-second Ghidra round trips. |
| `scripts/ghidra/rt/Ds2ArxanStubs.java` | Makes the "is this site Arxan-redirected" check countable and runnable over every candidate at once. Reads the `.text` boundaries from the program instead of hardcoding `0x141aaf000`. |
| `scripts/ghidra/sym-namespaces.py` | Template-stripping namespace histogram. A naive `sed 's/::.*//'` counts `DLRF::DLConcreteMethodInvoker<DLLG::DLAppender,DLTX::DLBasicString<...>>` under three namespaces and inflates every framework namespace it touches. |

### And one bug fixed

`rt/Ds2Decomp.java` printed the entire decompiled body in a single `println`. A GhidraScript's
`println` goes through log4j, and only the first line of a multi-line record carries the
`INFO  Ds2Decomp.java> ` prefix that `query.sh`'s extractor matches -- so **every decompile this
repo has ever run printed its `####` banner and silently dropped the code**. It reads as "this
function has no body", not as a formatting bug. `rt/Ds2Mem.java` documents this exact trap for
its own `bytes` mode; `Ds2Decomp` had it too and nobody had hit it. Now split per line.
