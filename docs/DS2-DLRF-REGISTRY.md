# The DLRF class registry: what it holds, and why it finds no singletons

The question: DS3's bindings find their managers by scanning for the FD4 singleton pattern, and DS2
has no FD4. DS2 does register classes with `DLRF::DLRuntimeClass`. Can that registry be walked at
runtime to find manager instances instead?

**No.** The registry can be enumerated, and name lookup works, but every record in it is class
metadata. Nothing in a record, in the registry, or in the registration path points at an instance,
and the managers a binding needs are not registered at all. Each singleton still needs its own
address, derived from an accessor, exactly as `crates/ds2-rva` does it today.

Everything below was read statically from the Ghidra MCP daemon's `DarkSoulsII.exe` program (port
8766, SotFS build 9527516, image base `0x140000000`). Each claim says whether it was read directly
out of the binary or inferred from how the code uses something.

## Where the registry lives

Two sorted vectors, each reached through a lazily set pointer in `.data`:

| pointer | storage | entry | sorted by | verified |
| --- | --- | --- | --- | --- |
| `0x141668080` | `0x141668108` | `0x20` bytes: class record, `char*` name, `wchar_t*` name, name length | name length | in binary |
| `0x141668090` | `0x141668138` | `0x10` bytes: type token, class record | token address | in binary |

- `0x140844200` and `0x140844250` are the two lazy initialisers. Each writes its pointer only if it
  is still null, pointing it at the static storage above. Verified in binary.
- The storage is a `{begin, end, capacity}` triple at offset 0, with an "initialised" byte at
  `+0x27` (`0x14166812f`, `0x14166815f`) that makes the first user bind the container to the DLRF
  heap (`0x140840130`, `0x1408401b0`). Verified in binary.
- Name lookups: `0x1408405d0` takes a `char*` and `0x1408406f0` a `wchar_t*`. Both binary-search the
  first vector by length, then `memcmp` the names, and return the class record or null.
  `0x14083f980` looks a type token up in the second vector and returns the record. Verified in
  binary.

So a mod can enumerate every registered class at runtime: read `[base + 0x1668080]`, and if it is
non-null walk `(end - begin) / 0x20` entries from `begin`. The pointer is null until the first
registration runs, which is static initialisation, so by the time a mod's hooks run it is populated.
The last part is inferred: the registrars' only callers sit in the `0x14107xxxx` block of small
stubs at the end of `.text` (`0x1410777cb` calls the `GUIFont` registrar, `0x14107680b` the
`GUIPopupMenuNode` one), which is where the compiler places dynamic initialisers. It has not been
observed at runtime.

## How a class is registered

`0x14083f070` is the registration function. It has one data reference and a call from every
registrar, one per `DLRuntimeClassImpl<T>` in the image, which is the same set the RTTI names give.
Verified in binary.

A registrar is a few instructions. `0x140563d30` registers `GuiFramework::GUIFont`:

```
rec = [0x141617840] ?: 0x140566410()      ; get-or-create the class record
rec+0x38 = "GUIFont"
rec+0x40 = L"GUIFont"
0x14083f070(rec)                           ; register
```

`0x14083f070` calls the record's vtable slots `+0x08` (name) and `+0x10` (wide name), inserts one
entry into the name vector, then calls slots `+0x18`, `+0x20`, `+0x28` and `+0x30` and inserts four
`{token, record}` pairs into the token vector, and re-sorts it. Nothing else is written anywhere.
Verified in binary.

## A class record

The record is a static object in `.data`, built on first use by a per-class creator. For `GUIFont`
the creator is `0x140564e60`, which builds the record in place at `0x141617850`: base constructor
`0x14083efd0`, then vtable `0x1410fed68`, then `+0x38` and `+0x40` cleared. `PlayerCtrl`'s is
`0x14037f1c0`, record `0x141614890`, vtable `0x1410e4e58`.

| offset | field | verified |
| --- | --- | --- |
| `+0x00` | `DLRuntimeClassImpl<T>` vtable | in binary |
| `+0x08` | parent class record, null at the root | inferred: every is-a check walks it (below); no writer was found in the decoded code |
| `+0x10` | lazily allocated construction-invoker holder, `0x70` bytes (`0x14083ef10`) | in binary |
| `+0x18` | method vector `{begin, end, capacity}`, `0x20`-byte entries, fed by `0x14083ed60` | in binary |
| `+0x30` | allocator the method vector uses | in binary |
| `+0x38` | `char*` class name | in binary |
| `+0x40` | `wchar_t*` class name | in binary |

The record is `0x48` bytes long: the `PlayerCtrl` record at `0x141614890` is followed at
`0x1416148d8` by other statics. Nothing in the layout has room for an instance.

The `DLRuntimeClassImpl<GUIFont>` vtable at `0x1410fed68`, slot by slot, all verified in binary:

| slot | function | returns |
| --- | --- | --- |
| `+0x00` | `0x140564a10` | scalar deleting destructor (`0x14083f580` frees the method vector) |
| `+0x08` | `0x1405651c0` | `this+0x38`, the name |
| `+0x10` | `0x1405652d0` | `this+0x40`, the wide name |
| `+0x18` .. `+0x30` | `0x140565290`, `0x140565180`, `0x1405651d0`, `0x140565140` | the addresses of four per-class static bytes, `0x14161784a`..`0x14161784e`: type tokens, not data |
| `+0x38` | `0x140565350` | constant false |
| `+0x40` | `0x1405650f0` | destroys an instance handed to it as an argument, through an allocator also handed in |
| `+0x48` | `0x140565230` | `0x38`, the instance size of `GUIFont` |
| `+0x50` | `0x14083f870` | registers a constructor invoker |
| `+0x58` | `0x14083f7c0` | registers a method invoker |

The `+0x40` destroy slot is decompiled by Ghidra under unrelated `EzState` types because the
function body is shared by identical-code folding; the shape (take a handle, call the object's
destructor, free it through the allocator, null the handle) is what is verified. For `PlayerCtrl`
the size slot at `0x1410e4ea0` points at `0x14037f3a0`, which Ghidra has inside a misdecoded
function; the bytes there are `b8 a0 04 00 00 c3`, `mov eax,0x4a0; ret`. Verified in binary.

Every slot returns metadata or acts on an instance the caller supplies. No slot returns an
instance.

## Instance to class works; class to instance does not

The direction the game uses is the other one. A registered class's instance vtable has a
get-runtime-class slot at `+0x18`. For `PlayerCtrl`, whose instance vtable is `0x1410e4bb8` (set by
`0x14037ec60`), slot `+0x18` is `0x14037fbb0`, the lazy getter for the record pointer at
`0x141614878`. Verified in binary.

The game uses that for checked downcasts. `0x14013c1b0` calls the instance's slot `+0x18`, then
walks `record+0x08` until it meets the `PlayerCtrl` record, returning the instance on a match and
null otherwise. `0x14037eb80` and `0x140311490` do the same over a `{record, instance}` pair for
`PlayerCtrl` and `CharacterCtrl`. Verified in binary, and the source of the parent-pointer reading
of `+0x08`.

That is a type test. It needs an instance to start from.

## The managers are not registered

- `GameManagerImp`, whose pointer the repo already reads at `0x1416148f0`: the image has
  `.?AVGameManagerImp@@` and a Steam callback type, and no `DLRuntimeClassImpl<GameManagerImp>`.
  Verified in binary.
- No registered class name ends in `Manager`, `System` or `Imp`. Verified in binary.
- The registered `*Ctrl` classes are `CharacterCtrl`, `PlayerCtrl`, `DemoCharacterCtrl`,
  `PXUserDataCharacterCtrl` and `EnemyGeneratorCtrl`. These are per-character objects with many
  instances, not singletons, and the live `PlayerCtrl` is reached as `GameManagerImp`'s field, not
  through anything DLRF holds. Verified in binary.

The rest of the registered set is resource objects, map and demo components, `Fe*` menu objects and
`GUI*` widgets: types the game constructs by name, which is what a reflection registry with
constructor invokers is for.

`PlayerCtrl`'s record at `0x141614890` sits `0x60` bytes before the `GameManagerImp` pointer at
`0x1416148f0`. That is `.data` layout, not a link: no code reads one through the other.

## What a binding would need

For singletons, nothing from DLRF. Every manager needs its address from an accessor, as
`crates/ds2-rva` records them, and the per-manager work does not shrink.

What DLRF can give a `dl::rf` module, if one is ever wanted:

- A name-to-record lookup, either by calling `0x1408405d0` or by walking the vector behind
  `0x141668080` with the `0x20`-byte entry above. That is two constants and one struct.
- A checked downcast for registered types: read the instance's vtable slot `+0x18`, walk `+0x08`,
  compare with the record found by name. That turns "this pointer should be a `PlayerCtrl`" into
  something a binding can assert at runtime instead of trusting.
- Instance sizes and names for registered types, from vtable slots `+0x48` and `+0x08`, which are
  cheap cross-checks for hand-written struct sizes.

None of that has run. It is read from the binary, and the parent-pointer field is inferred from its
readers because the initialiser stubs that would write it are code Ghidra has decoded at the wrong
instruction boundaries.
