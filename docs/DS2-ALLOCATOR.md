# `DLKR::DLAllocator` in DS2: the slot table

Which virtual slot of `DLKR::DLAllocator` allocates, settled from the binary by static analysis
alone (Ghidra, the DS2 daemon on port 8766, read-only). Every address is a VA at the preferred
base `0x140000000`.

Each claim is tagged:

* **verified** -- read directly off the binary: the vtable entry, the disassembly, or the
  decompiled body of the function named.
* **inferred** -- a reading of verified facts that the binary does not state outright (a role
  name, a purpose). The evidence it rests on is given next to it.

## Short answer

* `heapAllocator` (`0x140833320`) is `MOV RAX,[R8]; ... JMP [RAX+0x50]` with `RCX=R8`,
  `RDX=size`, `R8=align` (verified). `+0x50` is slot 10, `allocate_aligned(this, size, align)`.
* Slot 9 (`+0x48`) is `allocate(this, size)`, slot 13 (`+0x68`) is `free(this, ptr)`, slots 11
  and 12 are `realloc` and `realloc_aligned`, slot 8 (`+0x40`) is `block_size(this, ptr)`.
  Slots 15 to 19 repeat 9 to 13 for the back end of a two-ended heap.
* The parameter of `heapAllocator` is provably a `DLKR::DLAllocator` at the call sites in
  `FUN_14089cbf0`: the pointer passed is the process-wide system allocator, whose vptr is
  `DLKRD::HeapAllocator<DLKR::DLSystemHeapImpl>::vftable` (`0x1411413c8`), a `DLAllocator`
  subclass. See "Provenance of the `heapAllocator` argument" below.
* `DLKR::DLAllocator::vftable` has 26 function slots, not 27. The 27th qword (`0x1410c0758`) is
  zero in the file image (verified: read from `DarkSoulsII.exe`'s `.rdata`), and
  `HeapAllocator<...DLBiHeap...>::vftable` has the same zero at `0x1410d2ef8`; the next class's
  RTTI locator follows each. `HeapAllocator<DLSystemHeapImpl>::vftable` is followed by
  `0x0000005c00000000`, not a code pointer, and `DLBackAllocator::vftable` by the next locator
  directly (all verified from the file). That the extra qword is not a slot is inferred: none of
  the three concrete classes puts a function there.

## The four vtables walked

| class | vftable | locator (vftable-8) | function slots |
| --- | --- | --- | ---: |
| `DLKR::DLAllocator` (abstract base) | `0x1410c0688` | `0x1412b3bf8` | 26 |
| `DLKR::DLBackAllocator` | `0x1410c0768` | `0x1412b3c70` | 26 |
| `DLKRD::HeapAllocator<DLKR::DLDynamicHeap<WinAssertHeapStrategy<DLKR::DLBiHeapStrategy<DLKR::DLBiHeap, DLKR::DLMultiThreadingPolicy>>>>` | `0x1410d2e28` | `0x1412c6948` | 26 |
| `DLKRD::HeapAllocator<DLKR::DLSystemHeapImpl>` | `0x1411413c8` | `0x1412f9cc0` | 26 |

In the base, slot 0 is `FUN_140152ca0`, slot 2 is `FUN_140152d00` (`return 0xffffffff`), and
every other slot is `_purecall` (`0x140c2e1fc`) (verified).

Subclass relationship (verified): the slot-0 destructors of all three concrete classes
(`FUN_140152cd0`, `FUN_140277a30`, `FUN_14088c300`) store `DLKR::DLAllocator::vftable` back into
the object before an optional `free` -- the MSVC base-destructor epilogue -- and all three share
the base's slot 2, `0x140152d00`, unchanged.

## Slot table

"Bi" is the `DLBiHeap` allocator at `0x1410d2e28`; its slots take `h = *(this+8)`, lock
`h+0x90` through its vtable `+0x10` with `0xffffffff`, call a `DLBiHeap` primitive on `h+0x28`,
and unlock through `+0x20` (verified for slots 9, 13, 15, 19 by disassembly; the rest by
decompilation). "Sys" is the system allocator at `0x1411413c8`.

| slot | off | signature | role | Bi implementation | Sys implementation | tag |
| ---: | --- | --- | --- | --- | --- | --- |
| 0 | `+0x00` | `(this, flags)` | scalar deleting destructor | `0x140277a30` | `0x14088c300` | verified |
| 1 | `+0x08` | `(this) -> u32` | unknown; Bi returns `*(u32*)(h+8)` | `0x140278a50` | `0x14088c770` | body verified, role unknown |
| 2 | `+0x10` | `(this) -> u32` | base returns `0xffffffff`, never overridden here | `0x140152d00` | `0x140152d00` | verified |
| 3 | `+0x18` | `(this, out, ...) -> out` | unknown; writes a 4-byte value through a hidden return pointer (Bi writes `0x73`) | `0x140278900` | `0x14088c700` | body verified, role unknown |
| 4 | `+0x20` | `(this) -> usize` | heap capacity (end minus begin, no lock) | `0x140278c80` -> `0x140856800` | `0x14088c7e0` | inferred from Bi body |
| 5 | `+0x28` | `(this) -> usize` | free bytes (`h+0x28+0x60`, which alloc lowers and free raises) | `0x1402789b0` -> `0x140856810` | `0x14088c710` | inferred from Bi body |
| 6 | `+0x30` | `(this) -> usize` | largest free block (walks the free list, caches the winner) | `0x140278b60` -> `0x140856820` | `0x14088c780` | inferred from Bi body |
| 7 | `+0x38` | `(this) -> usize` | number of live blocks (counts blocks whose `+0x10` is zero, the in-use mark alloc sets and free checks) | `0x1402786d0` -> `0x1408568a0` | `0x14088c630` | inferred from Bi body |
| 8 | `+0x40` | `(this, ptr) -> usize` | usable size of an allocated block | `0x140278810` -> `0x1408568d0` | `0x14088c690` -> `0x140857120` | verified: realloc uses it as the copy length |
| 9 | `+0x48` | `(this, size) -> ptr` | allocate | `0x140277ae0` -> `0x1408568f0` | `0x14088c330` -> `0x140857140` | verified |
| 10 | `+0x50` | `(this, size, align) -> ptr` | allocate aligned | `0x140277c60` -> `0x140277cb0` -> `0x140856910` | `0x14088c3a0` -> `0x140376d80` -> `0x140857490` | verified |
| 11 | `+0x58` | `(this, ptr, size) -> ptr` | reallocate | `0x140279560` -> `0x1402792f0` | `0x14088c880` (thunk to `0x140378de0`) | Bi verified |
| 12 | `+0x60` | `(this, ptr, size, align) -> ptr` | reallocate aligned | `0x140279870` -> `0x1402795e0` | `0x14088c890` (thunk to `0x140379010`) | Bi verified |
| 13 | `+0x68` | `(this, ptr)` | free | `0x1402785f0` -> `0x140856990` | `0x14088c5a0` -> `0x1408572d0` | verified |
| 14 | `+0x70` | -- | unsupported: panics `"Operation unsupported"` | `0x140278660` | `0x14088c610` | verified |
| 15 | `+0x78` | `(this, size) -> ptr` | allocate, back end | `0x140277d20` -> `0x140856940` | `0x14088c3b0` -> `0x140857140` | verified |
| 16 | `+0x80` | `(this, size, align) -> ptr` | allocate aligned, back end | `0x140277df0` -> `0x140856960` | `0x14088c420` -> `0x140376d80` | verified |
| 17 | `+0x88` | `(this, ptr, size) -> ptr` | reallocate, back end (Bi reuses the front-end realloc) | `0x140277ee0` -> `0x1402792f0` | `0x14088c4a0` | Bi verified |
| 18 | `+0x90` | `(this, ptr, size, align) -> ptr` | reallocate aligned, back end | `0x140278030` | `0x14088c4b0` | Bi verified |
| 19 | `+0x98` | `(this, ptr)` | free, back end (same primitive as 13) | `0x140277e70` -> `0x140856990` | `0x14088c430` | verified |
| 20 | `+0xa0` | `(this) -> bool` | unknown; Bi primitive returns `1` | `0x140279990` -> `0x140856b20` | `0x14088c8a0` | body verified, role unknown |
| 21 | `+0xa8` | `(this, ptr) -> bool` | is `ptr` a live block of this heap | `0x1402791c0` -> `0x140856b30` | `0x14088c7f0` | inferred from Bi body |
| 22 | `+0xb0` | `(this, cursor*) -> bool` | step a block-walk cursor (fills address, size, in-use) | `0x140278300` -> `0x140856b80` | `0x14088c530` | inferred from Bi body |
| 23 | `+0xb8` | `(this)` | lock | `0x140279230` | `0x14088c860` | inferred: body is only the `+0x10` lock call |
| 24 | `+0xc0` | `(this)` | unlock | `0x140279a30` | `0x14088c900` | inferred: body is only the `+0x20` unlock call |
| 25 | `+0xc8` | `(this, ptr)` | validate a block (panics `"given memory block does not seem to belong here."`, `"p alignment faild."`) | `0x1402780e0` -> `0x140856be0` | `0x14088c4c0` | inferred from Bi body |

How the allocation slots were told apart (verified):

* `0x1408568f0` and `0x140856910` walk the free list from `h+0x28+0x38` along `[3]` and carve the
  block from the low end; `0x140856940` and `0x140856960` walk from `+0x30` along `[2]` and carve
  from the high end. That is the front/back split, and the reason slots 15 to 19 exist.
* `0x140856910` and `0x140856960` take a third argument and refuse it unless it is a power of two
  (`panic("..\\..\\Source\\DLBiHeap.cpp", 0x208 / 0x262, "Align size must be 2^n")`), raising it
  to at least `0x20`. `0x140857490` (Sys) returns null on a non-power-of-two and falls back to the
  unaligned `0x140857140` below `0x11`. So slot 10 and 16 take `(size, align)`, slot 9 and 15
  take `(size)`.
* `0x140856990` and `0x1408572d0` panic with `"given memory block does not seem to belong
  here."` and `"...freed already."` on a bad pointer: they are free.
* `0x1402792f0` (slot 11): null `ptr` -> allocate; zero `size` -> free; otherwise compare with
  `0x1408568d0(ptr)`, allocate, `memcpy` that many bytes, free the old block. That is realloc, and
  it is what pins slot 8 as the block-size query. `0x1402795e0` (slot 12) is the same with an
  alignment carried into `0x140856910`.
* Slots 9 to 12 and 15 to 18 of Bi call `FUN_140b21520` when the result is null and the request
  was non-zero; in this build that function is an empty `return` (an out-of-memory hook compiled
  out).
* On the system heap there is no back end: slots 9 and 15 reach the same `0x140857140`, and
  slots 10 and 16 the same `0x140376d80`.

## `DLKR::DLBackAllocator` is a front-for-back adapter

Every `DLBackAllocator` forwarder is `MOV RCX,[RCX+8]; MOV RAX,[RCX]; JMP [RAX+n]` (verified).

| its slot | forwards to inner | meaning |
| --- | --- | --- |
| 1, 4-8 | same slot | pass-through |
| 3 | inner `+0x18` with an extra `1` in `R8`, returns the out pointer | pass-through with an argument added |
| 9, 10, 11, 12, 13 | inner `+0x78`, `+0x80`, `+0x88`, `+0x90`, `+0x98` (slots 15-19) | its allocate/free are the inner allocator's back-end allocate/free |
| 14-19 | nothing: `panic("..\\..\\Source\\DLBackAllocator.cpp", 0x4c..0x64, "Operation not supported")` | a back allocator has no back end of its own |
| 20-25 | same slot | pass-through |

So the shift the bindings survey saw (slot 10 forwarding to `+0x80`) is not a numbering mismatch
between two layouts. Both sides use the same 26-slot layout, and the adapter deliberately maps
its front-end family onto the inner allocator's back-end family (the role reading is inferred;
the mapping is verified). Constructors that install `DLBackAllocator::vftable` include
`FUN_1402777b0` (member of `NetSvrMemory` at `+0x40`, its inner-pointer field at `+0x48`
zeroed there) and
`FUN_140834940` (member at `+0x28`, inner pointer at `+0x30` pointing at a
`TemporaryAllocator` at `+0x20`).

## Provenance of the `heapAllocator` argument

Verified chain for the two calls in `FUN_14089cbf0` (`0x14089ccb8`, `0x14089cd19`):

1. Each call is `MOV R8,RAX; MOV EDX,8; LEA ECX,[RDX+0x38]; CALL 0x140833320`, with `RAX` the
   value of the global `0x1416681a0`, loaded or lazily set to `FUN_140838dd0()` just before.
2. `FUN_140838dd0` ends `MOV RCX,RBX; JMP 0x14088be90`, and `0x14088be90` is
   `LEA RAX,[RCX+0x478]; RET`. `RBX` is the global `0x141627ac0`, or `FUN_140839b20()` if unset.
3. The only write to `0x141627ac0` is in `FUN_140839b20` (`0x140839b9c`): the result of
   `FUN_140838fd0` called on the static storage at `0x141667ad0`. `FUN_140838fd0` returns its
   argument.
4. `FUN_140838fd0` first calls `FUN_14088bb40` with the same pointer, which stores
   `0x1411413c8` -- `DLKRD::HeapAllocator<DLKR::DLSystemHeapImpl>::vftable` -- at `+0x478`
   (`MOV [RBX+0x478],RAX` after `LEA RAX,[0x1411413c8]`). Nothing later in `FUN_140838fd0`
   touches `+0x478`.

So at these call sites `heapAllocator`'s third argument is an object whose vptr is `0x1411413c8`,
a `DLKR::DLAllocator` subclass (see "The four vtables walked"). The same global, reached through
`FUN_140af1550` (which lazily fills `0x1416681a0` from `FUN_140838dd0`), is also called directly
at slot 9 with one size argument in `FUN_140833e10` (`(**(code **)(*plVar3 + 0x48))(plVar3,
uVar5)`), which agrees with slot 9 being `allocate(size)`.

What this does not prove: `heapAllocator` has several thousand call sites, and only these were
traced to a vtable. That every caller passes a `DLAllocator` is inferred from the shape: every
call goes through the same `+0x50` with `(size, align)`, and every concrete allocator class
walked here, `DLBackAllocator` included, is a `DLAllocator` subclass with the same 26-slot layout.
`TemporaryAllocator`'s own vtable was not walked.

## Against DS3

DS3's `DLAllocatorVmt` has 14 entries and puts `allocate_aligned` at `+0x50` as well. The DS2
table above agrees with that one offset and differs in length and in everything the back-end
family adds (slots 14 to 19) and after it. A DS2 binding takes its slot numbers from this table,
not from DS3's trait.

## Reproducing

The vtable walks use `getXrefsFrom` on each slot address (each vtable entry is a `DATA` reference
to its target), through `scripts/ghidra/mcp_query.py` against port 8766. Function bodies come
from `getDecompiledCode` and `disassembleFunction` on the addresses in the tables. The zero
qwords were read from `DarkSoulsII.exe` at the file offset of their `.rdata` VA.
