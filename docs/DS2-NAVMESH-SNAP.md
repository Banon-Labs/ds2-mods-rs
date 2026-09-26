# Navmesh snap: world position to navigation-graph id

How DARK SOULS II maps a world position onto a node of its navigation graph, which is the step
`ds2-invasion-path` needs before it can ask the route planner for a path between two players.

Image base `0x140000000`; every address below is a VA at that base and the RVA is the VA minus it.
Each claim is marked **verified** (read off the disassembly in `DarkSoulsII.exe`, Ghidra, build
9527516) or **inferred** (a reading of that evidence that the binary alone does not settle).

## Summary

The snap is a synchronous function that returns the id in `eax`. It is not an asynchronous job,
and it needs no engine context. `crates/ds2-invasion-path/src/navquery.rs` (`snap_reporting`)
already calls it, and a live run on `4f8d0a5` logged it working:

```
self-check: snap start=0x00000001 (sweep slot 0, 0.0 m off) goal=0x0002007e (sweep slot 0, 0.0 m off)
  | world holds 1 graph(s); map index 0 gave key 0x00ffffff and the sweep agreed | guard OK
READY -- 2 segment(s) decoded to 26 point(s), 66.1 m of path
tick: first walkable route
```

The question of which function snaps a position is answered, statically and by that run. How to
read a route's node attributes is answered statically in
[Route segment ids are edge ids](#route-segment-ids-are-edge-ids); it still needs a run.

## The chain

| VA | RVA | Signature | What it does | Status |
|---|---|---|---|---|
| `0x140bab1f0` | `0x00bab1f0` | `u32 (u32 map_index)` | `(index & 0x3f) << 24 \| 0xffffff`: the map key | verified |
| `0x14039a9f0` | `0x0039a9f0` | `NvNaviGraphWorld* (GameManagerImp*)` | `[[gm+0xBC0]+0x10]`, or 0 when `gm+0xBC0` is null; does not null-check `gm` | verified |
| `0x140badb90` | `0x00badb90` | `graph_data* (NvNaviGraphWorld*, u32 key)` | linear scan of the inline array at `world+0x28`, `[world+0x68]` entries, for `[[entry+0x28]+0x1c] == key & 0x3f000000 \| 0xffffff`; 0 when that map is not resident | verified |
| `0x140babf90` | `0x00babf90` | `u32 (graph_data*, const f32x4* pos, f32 radius /*xmm2*/, u32 filter /*r9d*/, f32* out_dist /*[rsp+0x20]*/)` | the snap: calls `0x140bac070` on each sub-graph `[[data+0x28]+0x40+i*8]`, `i < [[data+0x28]+8]`, keeps the nearest; `0xffffffff` when none is in range | verified |
| `0x140bac070` | `0x00bac070` | `u32 (NvNaviGraph*, pos, radius, filter, f32* inout_dist)` | per sub-graph: AABB reject, then per triangle node tests `attrs[+0x48][i]` against the filter and measures distance; returns `0x140bab230(sub->[0], i)` | verified |
| `0x140bab230` | `0x00bab230` | `u32 (u32 subgraph_key, u16 index)` | packs the id (below) | verified |

`out_dist` may be NULL (`0x140babf90` tests it at `0x140bac05b`). When it is not, it receives the
value `0x140bac070` wrote, and `0x140babf90` takes `sqrtf` of that value (`0x140bac01f`) to
narrow the radius for the next sub-graph, so the value is a squared distance. Verified.

The id layout, from `0x140bab230` (verified):

```
id = ((key >> 24 & 0x3f) << 7 | (key >> 17 & 0x7f)) << 17 | (index & 0x7fff)
```

So a snap-produced id has a map index in bits 24..29, a sub-graph number in bits 17..23, the
triangle index in bits 0..14, and bits 15 and 16 always clear. `id | 0x1ffff` is the key
`0x140bb2620` hashes to find the `NvNaviGraph` (sub-graph) the id belongs to. The live run's
`start=0x00000001` and `goal=0x0002007e` are sub-graphs 0 and 1 of map index 0.

The filter's low three bits are a minimum node class: `0x140bac070` admits a node only when
`(filter & 7) < (attrs & 7)`, and it rejects nodes with attribute bit 8 unless filter bit 3 is set,
and type-`0x40` nodes unless filter bit 4 or 5 is set. Verified.

## Engine callers

Every direct caller of `0x140babf90` (xrefs, verified):

| Caller | Key source | Position source | Radius / filter | Notes |
|---|---|---|---|---|
| `0x14037be30` | `MapManager+0x170` (`[[0x1416148f0]+0x38]+0x170`) | owner `vtable[0x148]` | 20.0 (`0x1410ad5ec`) / `0x28` | the `NaviGraphLocationComponent` snap; returns -1 unless the owner is-a `CharacterCtrl` (type check against `[0x1416145d8]`, which `0x140311bf0` fills with `DLRuntimeClassImpl<CharacterCtrl>`) |
| `0x14042c9a0` | `0x1403ba380(entity)`, else byte `[[ctrl+8]+0x38]+0x110` | argument `rdx` | arguments | `ChrAiNavimeshCtrl`'s snap; callers pass 10.0 (`0x1410ad5e8`) and `0x40` |
| `0x1403dadd0` | `0x1403ba380` of the player's map entity | argument `rdx` | 10.0 / `0x40` | the `MapManager` player tracker; snaps only when the id hint in `r8d` is -1 |
| `0x1401602b0` | `MapManager+0x170` | owner `vtable[0x210]` | 20.0 / `0x28` | same shape as `0x14037be30`; the decompiler shows the result unused, and what this caller is for is not established |
| `0x140bbb7f0` | task fields | task fields | task fields | `NvNaviPolyNearestSearchTask`'s body: the asynchronous task is only a wrapper around the same synchronous function |

`MapManager+0x170` is written every frame by `MapManager`'s update `0x1403be060` at `0x1403be213`
from `0x1403ba380` of the player's map entity, or -1 when there is none (verified). So the two key
sources are the same number, and -1 has to be tested before the key is built: `0x140bab1f0(-1)` is
`0x3fffffff`, a valid-looking key for a map that does not exist.

### The per-character cache

Every character already carries its current node id, so a route between two players does not
strictly need a snap at all.

- `0x14037bc00` is `[comp+0x38] = 0x14037be30(comp)`. Verified.
- Its only direct caller is `0x140312dd0` (the function Ghidra calls `assignPhantomProperties`,
  a `CharacterCtrl` method) at `0x140313765`, with `comp = [CharacterCtrl+0x3b0]`. Just before, at
  `0x14031373c`, it calls `0x14048ab90(comp, -1)`, which writes `[comp+0x38]` and creates the
  component's navigator at `[comp+0x30]` through `0x140bae6c0(GameManagerImp+0xBC0)`. Verified.
- `0x1401ca7b0(PlayerCtrl)` walks the character's component list for the one whose runtime class
  is `[0x141616238]`, which `0x14048ac50` fills with `DLRuntimeClassImpl<NaviGraphLocationComponent>`.
  `MapManager`'s update reads `[that+0x38]` (`0x1403be17a`) and hands it to `0x1403dadd0` as the
  hint. Verified.
- So `[CharacterCtrl+0x3b0]` is the `NaviGraphLocationComponent` and `+0x38` is that character's
  cached node id. Inferred: the offsets and the class-cluster addresses agree, but no single
  instruction ties `+0x3b0` to the runtime class.
- How often `+0x38` is refreshed is not established. `0x140312dd0`'s only direct caller is
  `0x14037f3d0`, which is reached through a vtable, not a direct call, so whether it runs every frame
  is unknown. Inferred only.

## Arxan check

First bytes read from `darksoulsii-deobf.bin`. None of the entry points above starts with an
`e9` into the second `.text` (`0x141aaf000`..`0x141d43000`):

| VA | First bytes |
|---|---|
| `0x14037be30` | `48 89 5c 24 18 57` |
| `0x14042c9a0` | `48 89 5c 24 08 48 89 74` |
| `0x1403dadd0` | `48 89 5c 24 10 48 89 6c` |
| `0x140babf90` | `48 8b c4 56 48 83 ec 60` |
| `0x140bab1f0` | `83 e1 3f c1 e1 18` |
| `0x140badb90` | `4c 63 51 68` |
| `0x14039a9f0` | `48 8b 81 c0 0b 00 00` |
| `0x140bac070` | `40 55 53 57 41 54` |
| `0x14037bc00` | `40 53 48 83 ec 20` |

One callee is redirected: `0x140312ad0` is `jmp 0x141c5a440`. `0x14042c9a0` (at `0x14042c9c8`) and
`0x1403dadd0` (at `0x1403dae3f`) both call it to resolve a character's map entity. Calling those two
wrappers is fine, since Arxan's stub rejoins, but `0x140312ad0` must not be hooked, and a caller
that only has a position should use the `0x140badb90` + `0x140babf90` pair, as `navquery.rs` does,
rather than `0x14042c9a0`.

## Frida confirmation

Nothing below has been run. It reads, and does not call, so it cannot disturb the game thread.

1. `Interceptor.attach(base + 0xbabf90)`. On enter, save `rcx`, the three floats at `rdx`, `r9d`,
   `[rsp+0x28]` (`out_dist`; the fifth argument sits above the return address) and the return
   address. On leave, record `eax`. Frida's x64 context does not give a dependable `xmm2`, so do
   not read the radius there: read the constants `[0x1410ad5ec]` (expect 20.0) and `[0x1410ad5e8]`
   (expect 10.0) once, and tie each call to one by return address. Expect filter `0x28` from
   `0x14037bf06`, and `0x40` from `0x14042ca5e` and `0x1403daecc`. Expect bits 15 and 16 of every
   result other than `0xffffffff` to be clear.
2. For the returned id, check `id >> 24 & 0x3f` equals `[[[0x1416148f0]+0x38]+0x170]` whenever that
   field is not `0xffffffff`.
3. Read `[[player CharacterCtrl]+0x3b0]+0x38` once per frame and compare it with the latest
   result from return address `0x14037bf06` for the same owner. They should be equal. The number
   of frames between changes in that field while the player walks answers the open refresh-rate
   question above.
4. Read `[[[[[0x1416148f0]+0x38]+8]+0x18]+8]` (the `MapManager` tracker's current id, written by
   `0x1403dadd0` at `0x1403daedf`) and confirm it equals the player's `+0x38` value.
5. For a second character (the other player), repeat step 3 on its `CharacterCtrl`.
   `0x14037be30` keys off the local player's map, not that character's, so its id should always
   carry the local player's map index in bits 24..29.

## Route segment ids are edge ids

The self-check's route audit reported `Widest agent this route admits: NONE` and
`node 0x000281cd attrs 0x00370036 type 0x30 capacity 6 -- IMPASSABLE even to the smallest agent`.
The cause is that the ids a finished route stores are not triangle ids. They are EDGE ids, and bit
15 is the tag that says so. The audit indexed the per-triangle attribute table with an edge index.

### Two packers, one bit apart

`0x140bab200` is `0x140bab230` plus one instruction, `bts eax, 0xf` at `0x140bab21c` (bytes
`0f ba e8 0f`). Verified in binary:

```
0x140bab230 (triangle id): ((key>>24 & 0x3f) << 7 | (key>>17 & 0x7f)) << 17 | (index & 0x7fff)
0x140bab200 (edge id):     the same, | 0x8000
```

So a packed navi id is:

| Bits | Meaning |
|---|---|
| 0..14 | index: a triangle index when bit 15 is clear, an edge index when it is set |
| 15 | `1` = edge id (`0x140bab200`), `0` = triangle id (`0x140bab230`) |
| 16 | clear in both packers |
| 17..23 | sub-graph number |
| 24..29 | map index |

`id | 0x1ffff` still names the owning `NvNaviGraph` for both kinds, because bit 15 is inside the
mask; `0x140bb3f00` looks up a graph from a triangle id that way and then packs an edge id with
that graph's own key word `[graph+0]`. Verified. `0x000281cd` is map 0, sub-graph 1, edge `0x1cd`.

### Where route segment ids come from

`0x140bb4ac0` writes each `0x60`-byte segment. Verified in binary:

- The last segment's `+0x08` is `[planner+0x84]` and the first segment's `+0x0c` is
  `[planner+0x88]`. `0x140bb4310` fills those from `0x140bb3f00(planner, [planner+0x34], goal
  centroid)` and `0x140bb3f00(planner, [planner+0x38], start centroid)`. `0x140bb3f00` takes the
  given triangle's three edge references (the `u16`s at `+0x6`, `+0x8`, `+0xa` of its `0xc`-byte
  triangle record, each shifted right by one), picks the edge whose midpoint is nearest the given
  point, and returns `0x140bab200(graph key, edge)`. So the two end ids are edge ids of the start
  and goal triangles.
- Every other `+0x08`/`+0x0c` comes out of the gate layer's shared table
  `[[gate_graph+0x28]+0x30]` (`u32` entries), at `i16 [link+0x14]` or `i16 [link+0x16]` plus a slot,
  where `link` is a `0x24`-byte record from `[[gate_graph+0x28]+0x20]` and the half is picked by
  comparing `[link+0x18]` with the graph key `| 0x1ffff`. `0x140bb9d50` is that read wrapped
  around `0x140bb9de0`'s slot choice. Verified.
- Every consumer of those table values masks them with `0x7fff` and indexes the graph's EDGE array
  at `+0x58`, never its triangle array: `0x140bb4ac0` (portal midpoint), `0x140bb9de0` and
  `0x140bba040`. Verified. That the table values also carry bit 15 is inferred from the one live
  value, `0x000281cd`, and from the two end ids being built by `0x140bab200`.

### NvNaviGraph layout this needs

Each row verified in binary, from the functions named.

| Offset | Type | What | Read by |
|---|---|---|---|
| `+0x00` | `u32` | graph key; `\| 0x1ffff` is what `0x140bb2620` hashes | `0x140bb3f00`, `0x140bac070` |
| `+0x2c` | `i16` | triangle count | `0x1404018d0`, `0x140bac070`, `0x140bb3f00` |
| `+0x2e` | `i16` | edge count | `0x140bb3f00`, `0x140bb4ac0`, `0x140bba040` |
| `+0x40` | `f32[3]*` | vertices, stride `0xc` | all of the above |
| `+0x48` | `u32*` | per-TRIANGLE attribute word | `0x140bac070`, `0x14042ee40`, `0x140bba040` |
| `+0x50` | record `*` | triangles, stride `0xc`: `u16` vertices at `+0,+2,+4`, `u16` edge refs `(edge << 1 \| bit)` at `+6,+8,+a` | `0x1404018d0`, `0x140bac070`, `0x140bb3f00` |
| `+0x58` | record `*` | edges, stride `0x10` | `0x140bb4ac0`, `0x140bb9de0`, `0x140bba040` |

Edge record (stride `0x10`), from `0x140bba040`, verified:

| Offset | Type | What |
|---|---|---|
| `+0x0`, `+0x2` | `i16` | the edge's two vertex indices |
| `+0x4`, `+0x6` | `i16` | the other two edges of the triangle on side 0 |
| `+0x8`, `+0xa` | `i16` | the other two edges of the triangle on side 1 |
| `+0xc`, `+0xe` | `i16` | the triangle on side 0 and side 1; `0x7fff` = none (a boundary edge) |

`0x140bba040` reads an edge's attribute word as `[graph+0x48][tri * 4]` with
`side = (u16 [edge+0xc] == 0x7fff)` and `tri = u16 [edge+0xc + side*2]`, falling back to
`[graph+0x48][0]` when `tri >= 0x7fff`. Verified.

### The decode for the audit

For one route id:

1. `graph = 0x140bb2620([world+0x88], id | 0x1ffff)`; 0 means unreadable. (Unchanged.)
2. If `id & 0x8000 == 0`: `tri = id & 0x7fff`; require `tri < i16 [graph+0x2c]`; the word is
   `u32 [[graph+0x48] + tri*4]`.
3. If `id & 0x8000 != 0`: `edge = id & 0x7fff`; require `edge < i16 [graph+0x2e]`;
   `rec = [graph+0x58] + edge*0x10`; for each of `i16 [rec+0xc]` and `i16 [rec+0xe]` that is
   `>= 0`, `!= 0x7fff` and `< i16 [graph+0x2c]`, the word is `u32 [[graph+0x48] + tri*4]`. A portal
   inside one graph has two triangles, and the route crosses both; a boundary edge has one.

Inferred, not proven: `0x00370036` at `attrs[0x1cd]` is two small `u16` vertex or edge indices
because edge `0x1cd` is past the end of that sub-graph's triangle count and the read ran off the
attribute array. The bound check in steps 2 and 3 makes that case an unreadable node instead of a
verdict.

### Code change for crates/ds2-invasion-path

Not made here. What it should be:

- `crates/ds2-rva/src/lib.rs`: add `NAVI_ID_EDGE_FLAG: u32 = 0x8000`,
  `NV_NAVI_GRAPH_TRIANGLE_COUNT_OFFSET = 0x2c`, `NV_NAVI_GRAPH_EDGE_COUNT_OFFSET = 0x2e`,
  `NV_NAVI_GRAPH_EDGES_OFFSET = 0x58`, `NV_NAVI_EDGE_STRIDE = 0x10`,
  `NV_NAVI_EDGE_TRIANGLES_OFFSET = 0xc`, and correct `NV_NAVI_GRAPH_NODE_ATTRS_OFFSET`'s doc to
  say it is indexed by TRIANGLE index, which a route id is not.
- `crates/ds2-invasion-path/src/navquery.rs`, `node_attributes(table, id)`: keep the graph lookup,
  then branch on `id & NAVI_ID_EDGE_FLAG` as in steps 2 and 3, and return the triangle(s) and their
  words (for example `Vec<(u16 tri, u32 attrs)>`) instead of one `u32`. Bound every index by the
  graph's own `i16` counts before reading.
- `audit()`: push one `AuditedNode` per triangle word, carrying the triangle index next to the
  route id, and count an id whose bound check fails as `unreadable`. `widest` stays as it is.
- `crates/ds2-invasion-path/src/gametick.rs`, `describe_audit`: print the triangle index beside
  each id (`node 0x000281cd edge 0x1cd -> tri N attrs ...`) so a wrong decode is visible in the
  log.

### What a run should show

For a route the character can walk (the self-check's start `0x00000001` to goal `0x0002007e`
with `READY`), inferred from the decode:

- every route id printed has `id & 0x8000` set, and its edge index resolves to one or two
  triangle indices below the graph's triangle count;
- `unreadable` absent or zero;
- attribute words with small type and capacity fields, like the two already logged on normal
  ground (`0x00000007`, `0x00000047`), and no word that splits into two small `u16`s;
- `Widest agent this route admits: size class N of 6` with some `N`, not `NONE`.

A route of all edge ids that still says `NONE` after this change means the edge record layout
above is wrong for that graph, not that the ground is impassable.
