# SFX effect registry: how an `.ffx` becomes a spawnable id, and how a mod adds one

Static RE of `DarkSoulsII.exe` (flat image `darksoulsii-deobf.bin`, base `0x140000000`), 2026-09-25.
Tags as in `docs/ffx-actions.md`: **[read]** = read out of the binary, **[inf]** = inferred.
Where a claim was also exercised in the running game (Frida, `scripts/frida/stone.js` stage
`custom`), it says **[run]**. The mod-side implementation is `register_effect` in
`crates/ds2-invasion-path/src/sfx.rs`; its RVAs live in `crates/ds2-rva` (`SFX_EFFECT_*`,
`KATANA_SFX_SYSTEM_*`).

Every function named below was walked with `scripts/ds2-arxan-chain.py`: none is an Arxan redirect
(none begins with `e9` into `.text` #2 at `0x141aaf000`+). The ones the script reports as
`UNKNOWN` (`0x140becad0`, `0x140a36e50`, `0x140a11100`, `0x140becc90`, `0x140c12e10`) begin with a
plain `mov`/`movzx`/`lea`, i.e. their own code **[read]**.

## 1. The objects

`KatanaSfxSystem` (vtable `0x1410f6448`, RTTI `.?AVKatanaSfxSystem@@`; `SfxSystem` base, ctor
`0x140beae80`) **[read]**:

| off | what |
| --- | --- |
| `+0x08` | allocator (`DLAllocator*`) |
| `+0x10` | FX manager (`SfxFxManagerBase`, vtable `0x1411ed9e8`); its `+0x8` is the FX core |
| `+0x20` | `u8` passed as the last argument of every effect parse; non-zero also skips the spawn lookup |
| `+0x30` | `SfxResourceManager`: name table (`0x140b025b0` add, `0x140b02640` remove) |
| `+0x38` | `SfxFxResourceManager` (ctor `0x140bf13a0`): three id hash tables, `+0x40` effects, `+0x60` textures, `+0x20` models **[inf]** |
| `+0x48..+0x58` | `vector<SfxBinderResourceObject*>` resident bundles (loaded at boot) |
| `+0x68..+0x78` | `vector<SfxBinderResourceObject*>` per-area bundles |
| `+0x118` | list of `.sfxparam` blobs (`docs/ffx-actions.md` 7.4) |
| `+0x2b8` | `std::map` of ids that failed to resolve (`KATANA_SFX_MISSING_IDS_OFFSET`) |

Vtable slots that matter here **[read]**:

| slot | thunk | target | does |
| --- | --- | --- | --- |
| `+0x20` | | `0x140becd70` | releases the resident bundles |
| `+0x38` | `0x1404c5630` | `0x140becac0` | `ret 0`: the "id registered" hook does nothing |
| `+0x40` | `0x1404c5640` | `0x140becad0` | invalidate(id): `[sys+0x10]` -> `0x140a36e50` -> `0x140a0b3c0(core = [mgr+0x8], id)` |

Effect hash tables (`SfxFxResourceManager+0x40`, `+0x60`): bucket = `(id * 0x89) % buckets`, insert
at the bucket head `0x140c0fa90`, lookup takes the first match `0x140c0fb20`, remove the first
match `0x140c0fb60` **[read]**.

Per effect, two objects **[read]**:

- `SfxEffectData`, 0x20 bytes, vtable `0x1411ee680`, ctor `0x140c12e10(obj, allocator)`, refcount
  at `+0x8`. `0x140c12ee0(data, bytes, len, u8 flag)` parses a `DLsE` blob through
  `DLMemoryInputStream` and the FFX serializer `0x140f78620`, and stores the resulting
  `FXCreatableEffect*` at `+0x18`; it returns 1 on success and frees any previous creatable first.
- `SfxEffectResourceObject`, 0x98 bytes, vtable `0x1411ee088`, ctor
  `0x140c100a0(obj, sys, const wchar_t* name, SfxEffectData*)`. It stores the data at `+0x90`
  (add-ref), sets the resource name, and calls `0x140bf1690(sys+0x38 manager, obj)`, which takes
  `id = _wtoi(name + 1)` (`0x140c101f0` -> `0x140c2e1f0`), calls system vtable `+0x38` (the no-op)
  and inserts `obj` under `id` into the manager's `+0x40` table. **Registration happens in the
  constructor.** The name must be the basename `f%07d.ffx`: the id is parsed from the character
  after the `f`.

## 2. How spawning finds an id

`0x140beb670` (spawn) -> `0x140bed0a0(sys, out, id, pos, ...)` **[read]**:

1. For every bundle in `sys+0x48` and `sys+0x68` with `binder+0x110 != 0` (it held `.ffx` members):
   `0x140bf7960(binder, id)` materialises the member lazily (section 3).
2. Unless `sys+0x20` is set: FX manager vtable `+0x38` = `0x140a369c0` -> `+0x88` = `0x140a37220` ->
   `0x140a08fb0(core, id, ...)`.
3. `0x140a08fb0` binary-searches the core's sorted `FXCreatableEffect*` cache at `core+0x1c0`
   (`0x140a0a210`, id at `+0x8`). On a miss, `0x140a0aa30(core, id)` asks the resolver
   `[core+0x198]` vtable `+0x8` = `SfxFxAdapterBase` `0x140bef780`, which calls
   `0x140bf1500(SfxFxResourceManager, id)`: hash lookup in `+0x40`, returns
   `resource+0x90` (the `SfxEffectData`); its `+0x18` is the creatable, and it goes into the cache.
4. **Negative cache.** If the resolver reports not-found, `0x140a0aa30` inserts an
   `FFX::FXCreatableEffectInvalid` placeholder (0x10 bytes, id at `+0x8`) into the cache. Nothing on
   the registration path clears it (the only hook it calls is the `ret` at vtable `+0x38`); only
   vtable `+0x40` (`0x140becad0` -> `0x140a0b3c0`) removes the cached entry, and if that id has live
   instances, `0x140a09d50` despawns them. A miss does not touch the map at `sys+0x2b8`: that map
   counts live roots per id and drives the timed purge in section 6.

## 3. How the shipped bundles register ids

A bundle is an `SfxBinderResourceObject` (0x1e70 bytes, vtable `0x1411ed458`, ctor `0x140bf5eb0`).
When its file read completes, `0x140af6e70` calls its vtable `+0x80` = `0x140bf6330(binder, bytes,
size)`, which walks the BND members by extension **[read]**:

| member | handling |
| --- | --- |
| `.ffx` | sets `binder+0x110 = 1`. Name-id in 2000..2200: parsed and registered **eagerly**, right here. Every other id: left for the lazy path |
| `.tpf` (name-id < 900020) | each texture -> `SfxTextureResourceObject` (`0x140c104c0`) -> manager `+0x60` by id |
| `.flv` | model resource (`0x140c0fc30`) |
| `.vpo` / `.fpo` | shader resources (`0x140c109d0`) |
| `.sfxparam` | `0x140becc90(sys, data, size)`: appended to `sys+0x118` |

If `binder+0x110` is set, the whole bundle is then copied into a `Memory` at `binder+0x118` so the
lazy path can reopen it **[read]**.

Lazy path `0x140bf7960(binder, id)` **[read]**:

- skips ids already in `binder+0xf0`; for id < 30000 a "tried" bitset at `binder+0x120` and a
  "found" bitset at `binder+0xfc8` stop repeat scans. Ids >= 30000 rescan the bundle on every spawn.
- builds the member name `N:\FRPG2\data\Sfx\Output\Effect\f%07d.ffx` (UTF-16 at `0x1411ed600`;
  `N:\FRPG2_64\...` at `0x1411ed660` for ids above 9999) and looks it up by name (`0x140af7910` ->
  `0x140912290`: BND4 hash table if present, else case-insensitive `_wcsicmp`). The effect id is the
  member **name**, not the BND4 entry id.
- then the same calls as section 4: `SfxEffectData` ctor, parse, `SfxEffectResourceObject` ctor
  with the basename (`0x14083ab80`), refcounts, `0x140aff7a0`, add to the `sys+0x30` name table,
  keep it in `binder+0xd0`, push the id to `binder+0xf0`.

Per-area bundles: map load `0x1403d1fa0` calls `0x1403d12b0(sys, 5000 + ((mapId >> 16) & 0xff),
lq)`, which loads `sfx%04d.ffxbnd.dcx`, `sfx%04dres.ffxbnd` and the `_Append` variants through
`0x140bec9b0(sys, path, is_effect_bundle)`; map unload `0x1403d1400` -> `0x1403d1e50` ->
`0x140bece70(sys, path)` releases them **[read]**. On release, `SfxEffectResourceObject` vtable
`+0x10` = `0x140c10300` -> `0x140bf1780` calls system vtable `+0x40` (invalidate, despawns live
instances) and removes the id from the manager **[read]**.

## 4. Registering a new id from a buffer

Everything the lazy path does, minus the bundle. No binder, no file I/O, no VFS.

```
a    = *(sys + 0x08)                                   // allocator
d    = 0x140833320(0x20, 8, a)                         // allocator->vt[0x50](a, size, align)
0x140c12e10(d, a)                                      // SfxEffectData ctor
0x140833650(d + 0x8)                                   // add-ref
ok   = 0x140c12ee0(d, ffx, len, *(u8*)(sys + 0x20))    // parse; must return 1
r    = 0x140833320(0x98, 8, a)
0x140c100a0(r, sys, L"f0025000.ffx", d)                // registers 25000 in (sys+0x38)+0x40
*(i32*)(r + 0x10) += 1; *(i32*)(r + 0x14) += 1         // hold both counts for the process
0x140aff7a0(r)                                         // resource start (vt+0x40 is a no-op ret)
(*(void**)(*(u64*)sys + 0x40))(sys, 25000)             // invalidate: drop any cached placeholder
0x140beb670(sys, ctrl, 25000, &{pos, dir}, 0, 0xff, 0) // spawn
```

Status: the calls and their order are **[read]**; the sequence works in the running game **[run]**
(an edited copy of f0000833 registered as 25000 spawned a live effect; a glow-only variant
registered as 25001 was watched in game and looked as intended).

Rules that follow from the code:

- **The id inside the `DLsE` must equal the registered id** (the Effect root body `+4`). With a
  mismatch the spawn comes back empty **[run]**; the registry key comes from the name, the
  creatable carries its own id **[inf]** for why.
- **Register before the first spawn, or invalidate after.** A spawn of an unregistered id
  poisons the core cache with `FXCreatableEffectInvalid` (section 2, step 4) **[read]**.
- **Hot swap:** `0x140c12ee0` on the existing `SfxEffectData` (from `0x140bf1500`) re-parses in place
  and frees the old creatable; follow it with the invalidate **[read]**, **[run]**.
- **Id range:** above 9999 skips the remaster probe (`id + 20000` / `+ 10000`,
  `KATANA_SFX_BASE_ID_MAX`) and below 30000 keeps the per-binder "tried" bitset in force, so every
  spawn does not rescan every loaded bundle. 25000..29999 must not collide with a
  `f%07d.ffx` member of `sfx9999_Append` or a per-area `_Append` bundle **[inf]** (not checked for
  every area bundle).
- **Lifetime:** nothing but the mod references the resource, so area changes and both timed
  purges miss it. Section 6 has the purges and why neither can reach a buffer-registered effect.
- **Textures** are global: any texture id already registered by a loaded `.tpf` (the resident
  `sfx9999res.ffxbnd` loads at boot) resolves through manager `+0x60` (`0x140bf15b0`) **[read]**.
  A new texture needs a `SfxTextureResourceObject` (`0x140c104c0`), which this path does not build.
- **sfxparam** entries are optional (`docs/ffx-actions.md` 7.4); `0x140becc90` appends a blob, and
  an existing id in the shipped blob wins over an appended one **[read]**.
- **Thread:** spawn and the lazy parse run on the caller's thread, which for the mod is the game
  thread **[read]** for the call chain. The async file completion `0x140af6e70` also runs on the
  game thread, from the per-frame engine update (section 7); the buffer path does not use it.

A bundle-level alternative exists: construct a binder (`0x140bf5eb0`) and call its vtable `+0x80`
with a whole `.ffxbnd` in memory. It then has to be pushed into `sys+0x68` (vector growth
`0x140bed840`) and its state fields faked (`+0x34 = 2`, `+0x78`/`+0x79 = 1`, `+0xc5 = 1`,
`0x140aff5f0`) **[read]**; more surface than the buffer path for one effect.

## 5. Loading from a path instead

`0x140bec9b0(sys, const wchar_t* vfs_path, u8 is_effect_bundle)` allocates a binder, registers it in
`sys+0x30`, pushes it into `sys+0x68` and starts the async read (`0x140aff7a0` ->
FileResourceObject vtable `+0x40` = `0x140af6f60` -> `0x140b0c1c0` -> `0x140b5aff0`) **[read]**.
`0x140bec5a0(sys, path)` returns 0 while another `sys+0x68` binder is still loading, and
`0x140bece70(sys, path)` unloads by exact name **[read]**.

VFS: mount aliases live in a table set by `0x140859590(alias, target)` (define/replace) and
`0x140859600(alias, target)` (add a secondary target); `KatanaMainApp` `0x1402ef410` sets them at
boot, and its dev branch maps `title` to an absolute drive path **[read]**. Whether retail
`DLFileDevice` accepts a drive-letter target (`0x14089bd00`) was not read. The buffer path in
section 4 does not need any of this.

## 6. What can free a registered effect

Static RE, 2026-09-25. Claims here are tagged **verified in binary** (with the address) or
**inferred**.

Resource object fields used below (`ResourceObject` base) **verified in binary**: `+0x10` a use count
(`i32`), `+0x14` a holder count (`i32`), `+0x28` the `GetTickCount` of the last release,
byte `+0x78` loaded, byte `+0x7f` "may be purged". `0x140aff440` is release: it decrements `+0x14`
and destroys the object at zero; with `+0x10 == 0` and `+0x14 == 1` it hands the object back to its
manager for removal.

There are two timed purges. Neither can reach a buffer-registered effect.

### 6.1 Per-id purge of bundle members (`KatanaSfxSystem` update)

The map at `sys+0x2b8` holds one node per id: key `+0x1c`, live count `+0x20` (`i32`), age `+0x24`
(`i8`) **verified in binary**.

- `0x140beb400` (called only by the spawn `0x140beb670`, on a non-empty result) adds 1 to the count
  and sets the age to 0.
- `0x140beb3a0(sys, id)` subtracts 1, never below 0. Its only caller is the `SfxFxObjectRoot`
  destructor `0x140c0b290`, with the id at `root+0x34`. So the count is the number of live roots of
  that id, not a running total of spawns.
- The system update `0x140be9e00` adds the frame time to `sys+0x2d0`. Once it passes 5.0 s (float
  at `0x1410ad5e4`), or when byte `sys+0x304` is set, it walks the map (`0x140bea2f7..0x140bea4a4`)
  and resets the accumulator. A node with count > 0 is skipped. A node with count <= 0 and age 0
  gets its age raised to 1. A node with count <= 0 and age >= 1, or any count <= 0 node when
  `sys+0x304` is set, is purged: for every binder in `sys+0x48` and `sys+0x68` with
  `binder+0x110` set, `0x140bf8260(binder, id)` runs, and the node is erased.
- `0x140bf8260` looks only in that binder's lazily materialised list (`binder+0xd0`) for a
  resource whose name parses to `id`. On a match it removes it from the `sys+0x30` name table
  (`0x140b02640`), drops it from `binder+0xd0` and `binder+0xf0`, and releases it. The release that
  frees it reaches `0x140c10300` -> `0x140bf1780`: system vtable `+0x40` (invalidate, despawns live
  instances) and removal from `SfxFxResourceManager+0x40`.

Net effect: a shipped effect that was spawned through `0x140beb670` is unloaded 5 to 10 s after
its last root is destroyed. The next spawn re-materialises it from the bundle copy at
`binder+0x118` through `0x140bf7960`. That the "found" bitset test `0x140bf8d20` lets that re-parse
run is **inferred**; the call is verified, its body was not read.

Why this cannot free the mod's effect **verified in binary**: `register_effect` never puts the
resource into any binder's `+0xd0` list, and `0x140bf8260` searches nothing else. The id's node is
still counted, aged and erased, but the purge finds nothing to release. Child ids that the effect
spawns from inside the FX core (`0x140a08fb0`, for example 2101 under 833) never go through
`0x140beb400`, so they are not in this map at all.

### 6.2 Resource-manager purge (`LiveResourceManager`)

`SfxResourceManager` (`sys+0x30`, ctor `0x140bf2d20`, base ctor `0x140b01c50`) is a
`LiveResourceManager`: 0x6d name-hash buckets at `+0x20`, and an embedded
`ResourceMemoryWatchDog` at `+0xc0` (pointer to it at `+0xc8`) **verified in binary**.

`0x140b02750` asks the watchdog (`[+0xc8]` vtable `+0x8`) for a pressure level and then runs
`0x140b027f0(mgr, bucket, age_ms)` over one bucket (level 0), three buckets (level 1), or all 0x6d
(level 2), with the age limits at `+0xd4` / `+0xd8` / `+0xdc`. `0x140b027f0` takes an object only
if `+0x10 == 0`, byte `+0x7f` is set, `GetTickCount() - [+0x28]` exceeds the limit, and its vtable
`+0x50` agrees; then it unloads it, or removes and releases it when `+0x14 == 1`
**verified in binary**. `KatanaSfxSystem` calls `0x140b02750` on `sys+0x30` every update
(`0x140be9e42`, `0x140be9d70`).

For this manager the watchdog is the base `ResourceMemoryWatchDog` (vtable `0x1410d9340`, written
by `0x140b01c50`; the Sfx ctor does not replace it), whose `+0x8` is `0x1402de900` = `return 0`.
So only level 0 ever runs: one bucket per update, round-robin **verified in binary**. What the
limit at `+0xd4` is for this manager was not read.

Why this cannot free the mod's effect **verified in binary**: `register_effect` does not add the
resource to `sys+0x30` (it does not call `0x140b025b0`), and it holds `+0x10` at 1, which fails the
first test anyway.

### 6.3 What else removes an effect

- `SfxFxResourceManager+0x40` loses an entry only through `0x140c0fb60`, whose only effect-table
  caller is `0x140bf1780` (the resource's own release path) **verified in binary**
  (`0x140bf17c0` and `0x140bf1800` remove from `+0x20` and `+0x60`). `0x140c0fb60` removes the first
  entry with that id, so a shipped bundle member with the same id as a registered effect would, on
  its own release, remove the registered entry. Pick ids no bundle ships.
- The FX core cache (`core+0x1c0`, with a per-entry use count vector at `core+0x1e0`) loses an entry
  only through `0x140a0b3c0`, called from `0x140a0aa30` (re-resolve) and the invalidate at system
  vtable `+0x40` **verified in binary**. No timer, and nothing reads the use count to evict.

### 6.4 For the Prism Stone trail

The trail's derived effect lives until the process exits. Nothing unloads it: the mod-held count
at `+0x10` and the missing name-table and binder-list entries keep both purges away, and area
changes only release binder members. The only ways to lose it are the mod releasing it, or a
bundle member with the same id being released (6.3). The shipped children it spawns (2101, 2032)
are eager members of the resident `sfx9999` bundle and are not spawned through `0x140beb670`, so
the per-id purge does not touch them either **inferred** (from the call paths above; not run).

## 7. Which thread runs the async file completion

**Verified in binary:**

- `0x140af6f60` (FileResourceObject start) builds a `TFileReadOperationCb<FileResourceObject>`
  (vtable `0x1411d5d18`) that stores the object at `+0x78` and `0x140af6e70` at `+0x88`, and queues
  it with `0x140b0c1c0` -> `0x140b5aff0`.
- The callback runs from the operation's vtable `+0x50` = `0x140af6f30`. Both ends of an operation
  jump there: `0x140b4c650` (vtable `+0x20`, data ready) and `0x140b4c610` (vtable `+0x30`, failed).
- Those slots are called by `0x140b5b510`, which drains one finished-request queue;
  `0x140b5b300(fileMgr)` runs it over all four queues.
- `0x140b5b300` is called from:
  - `0x140b0bd10`, called by the engine update `0x140af0090` at `0x140af0113`, once per frame,
    right after the input update. That is the game thread (`docs/DS2-FRAME-COUNTER.md`).
  - `0x140b0c3f0` (the wait loop `0x140b4c740` and `GameManagerImp` `0x1401c1ab0`) and
    `0x140b0c400` (blocking loads: title prologue `0x1400fdcd0`, `0x1401c10b0`, `0x140b31xxx`).
    These run on whichever thread is waiting.
  - `0x140b5ac10`, shutdown drain.

So a bundle's on-load handler `0x140bf6330`, and its unlocked writes to the resource tables and
`sys+0x118`, run on the game thread during the frame, or on the thread of a blocking load. Whether
any blocking load runs off the game thread was not checked **inferred** from the caller names only.
The DLRS worker behind the queue does the I/O; that it never calls the operation's vtable itself is
**inferred** (the only dispatcher found is `0x140b5b510`, but vtable calls cannot be xref'd).
