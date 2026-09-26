# The map at `KatanaSfxSystem + 0x2b8` counts spawns that worked, not lookups that failed

Static RE of `DarkSoulsII.exe` (Ghidra, flat image `darksoulsii-deobf.bin`, base `0x140000000`),
2026-09-25. Every claim is tagged **verified in binary** (with the addresses it was read at) or
**inferred**.

## The symptom

The first game run of the Rainbow Stone trail with the self-check on logged, for the sparkle-free
Prism Stone copy registered as effect 25833:

```text
Prism Stone 833: laying the sparkle-free copy, registered as effect 25833
self-check: id 25833: spawned, quality 0, THIS attempt's lookup failed -- the effect is not resident in this map
```

and every later spawn of 25833 said `the id was already recorded as missing before this attempt`.
That reads as "registration went into a table the spawn does not search". It is not. The spawn
found the effect and built it; the diagnostic reads the map backwards.

## What the diagnostic reads

`id_is_missing` in `crates/ds2-invasion-path/src/sfx.rs` walks the `std::map` at `sys + 0x2b8`
(`KATANA_SFX_MISSING_IDS_OFFSET`) and reports the id as missing when it is present. `describe`
calls a `false -> true` change across one attempt "THIS attempt's lookup failed".

The only function that writes that map is `0x140beb400(sys, id)` **verified in binary**: its tree
helpers `0x140bea530` and `0x140beaa00` have no other call sites (their remaining references are
data -- unwind entries -- not calls). Its body **verified in binary** (`0x140beb400..0x140beb4ab`):

- `lower_bound` descent from the head at `[sys + 0x2b8]`;
- found: `inc dword [node + 0x20]` and `mov byte [node + 0x24], 0` (`0x140beb458`, `0x140beb45b`);
- not found: insert `{key = id, +0x20 = 1, +0x24 = 0}` (`0x140beb465..0x140beb4a1`).

So each node is an id and a count.

## When the spawn writes it

`0x140beb670` (the spawn the mod calls) is the only caller of `0x140beb400`, at `0x140beb8a7` and
`0x140beb8bf` **verified in binary**. The path for an id above 9999 (`cmp r13d, 0x2710` at
`0x140beb759`):

```text
0x140beb769  call 0x140bed0a0            ; build into the local block at [rbp-0x41]
0x140beb783  mov  edi, r13d              ; edi = the id itself
0x140beb86c  lea  rcx, [rbp-0x41]
0x140beb870  call 0x140a06580            ; the engine's "is this block empty?" predicate
0x140beb87d  test al, al
0x140beb87f  jz   0x140beb8a2            ; NOT empty -> 0x140beb8a2
0x140beb884  call 0x140127240            ; empty -> build the empty result block, return
...
0x140beb8a2  mov  edx, edi
0x140beb8a7  call 0x140beb400            ; record `id` -- only on the non-empty path
0x140beb8b0  call 0x140a06580            ; second block ([rbp-0x11], the +10000 half)
0x140beb8b7  jnz  0x140beb8c4            ; empty -> skip
0x140beb8bf  call 0x140beb400            ; record the second id
0x140beb8d2  call 0x140beadf0            ; copy both halves into the caller's block
```

That `0x140a06580` returns non-zero for an EMPTY block is **verified in binary** from its callers,
independently of the Arxan-shattered body: `0x140bed0a0` runs the sfxparam, random-rotation and
`FUN_140a06940` configuration only when it returns zero, and `0x140beb670` routes non-zero to
`0x140127240`, the builder of the empty result block. The run agrees: the attempt that inserted
25833 also came back with live node pointers (`spawned`), which the mod reads itself.

**Conclusion, verified in binary:** `0x140beb400` runs only after a spawn produced a non-empty
block. The map is a per-id count of spawns that worked. An id enters it on its first successful
spawn and its count goes up on each later one. A spawn that finds nothing never touches it.

So the logged lines mean:

| logged | actually |
| --- | --- |
| `THIS attempt's lookup failed -- the effect is not resident in this map` | first successful spawn of this id |
| `the id was already recorded as missing before this attempt` | this id has spawned successfully before |
| `the id resolved` (id absent after the attempt) | the spawn produced nothing, or the tree was unreadable as absent |

The 25833 run is therefore a success at every step the engine can report. Whether the stones are
visible on the ground is a separate question the map cannot answer.

## Registration and the spawn use the same table

**Verified in binary:**

- The resource constructor `0x140c100a0` calls `0x140bf1690(manager, obj)`, which parses the id out
  of the name (`0x140c101f0`), calls the no-op system hook, and inserts into `manager + 0x40` through
  `0x140c0fa90`. No id-range test.
- The spawn's core-cache miss goes to the resolver `0x140bef780`
  (`SfxFxAdapterBase`), which calls `0x140bf1500(manager, id)`: `0x140c0fb20(manager + 0x40)`,
  returning `resource + 0x90`. Same table, no id-range test.
- The per-bundle lazy path `0x140bf7960` that `0x140bed0a0` runs first for each bundle only adds
  bundle members; it does not gate the resolver.

That the adapter's manager pointer (`adapter + 0x1d0`) is the same object as `sys + 0x38` is
**inferred**; `register_effect` confirms it at run time by resolving the id through
`0x140bf1500(sys + 0x38 manager)` before it returns `Ok`, and the log line
`registered as effect 25833` is only printed on `Ok`.

## The real negative cache is elsewhere, and `register_effect` already clears it

The placeholder that poisons an unregistered id is the `FXCreatableEffectInvalid` entry in the FX
core's sorted cache, created by `0x140a0aa30` on a resolver miss, and dropped only by system vtable
`+0x40` (`0x140becad0`) (`docs/DS2-SFX-REGISTRY.md`, section 2, on the registry doc's own branch
until it merges; its sentence "The miss also goes into the map at `sys+0x2b8`" is wrong by the
reading above -- the map takes hits, not misses). `register_effect` calls that
invalidate for the id after the constructor and `0x140aff7a0`, and on the hot-swap path after the
re-parse. `stripped_stone` in `gametick.rs` registers the copy before it returns the id to spawn, so
no spawn of 25833 precedes registration in that path **inferred** from the source order. Either way
the invalidate would clear a placeholder left by an earlier spawn. No engine-side change is needed.

## The code change

The engine is fine; the diagnostic is wrong. In `crates/ds2-rva/src/lib.rs`:

- Rename `KATANA_SFX_MISSING_IDS_OFFSET` and its `_LEFT/_PARENT/_RIGHT/_ISNIL/_KEY/_MAX_DEPTH`
  siblings to `KATANA_SFX_SPAWNED_IDS_*`, rewrite the doc to "ids whose spawn produced a non-empty
  block, with a count", and add `KATANA_SFX_SPAWNED_IDS_COUNT_OFFSET: usize = 0x20` (`u32`).
- In the `KATANA_SFX_SPAWN` doc, "the failed-lookup red-black tree at `sys + 0x2b8`" becomes "the
  spawned-ids count tree at `sys + 0x2b8`".

In `crates/ds2-invasion-path/src/sfx.rs`:

- `id_is_missing(system, id) -> Option<bool>` becomes
  `spawn_count(system, id) -> Option<u32>`: the same read-only walk, returning `Some(0)` when the id
  is absent and `Some(count)` read at `node + KATANA_SFX_SPAWNED_IDS_COUNT_OFFSET` when present.
- `Attempt { missing_before, missing_after }` becomes `Attempt { count_before, count_after }`,
  sampled in the same places in `spawn_reporting`.
- `describe` becomes:

```rust
let counted = match (self.count_before, self.count_after) {
    (Some(before), Some(after)) if after == before.wrapping_add(1) => {
        format!("the engine counted this spawn ({after} so far for this id)")
    }
    (Some(before), Some(after)) if after == before => {
        "the engine did not count it -- nothing was built for this id".to_string()
    }
    _ => "spawned-ids tree unreadable".to_string(),
};
```

and a live handle with an uncounted spawn, or an empty handle with a counted one, is logged as a
contradiction rather than folded into either case.

No change to `register_effect`: it already registers into the table the spawn resolves from and
already calls the invalidate afterwards.

## How a run proves it

Same invocation as the first run (`--invasion-path --invasion-path-on --invasion-path-self-check
--invasion-path-markers 833`), with the corrected diagnostic:

- the first 25833 spawn logs `counted this spawn (1 so far)`, and each later one's count is one
  higher than the last -- the prediction of the old reading was the opposite;
- no line reports a contradiction between the handle and the count;
- `Handle::alive` stays true for the placed stones, which is what says they linger.

Visibility on the ground is still a look, not something the map can settle.
