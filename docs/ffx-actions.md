# FFX action ids -> runtime classes and param meanings

Static RE of `DarkSoulsII.exe` (flat image `darksoulsii-deobf.bin`, base `0x140000000`), 2026-09-25.
No game run was involved.

Every claim is tagged **[read]** (read out of the binary: a table, a vtable, a source path, a
constant), **[data]** (checked against the shipped `.ffx` files with `scripts/ds2-ffx.py`) or
**[inf]** (inferred: from names, data values, or shape of the code, not proven).

Tools used: Ghidra MCP daemon (read-only), `scripts/ghidra/query.sh` with the new
`scripts/ghidra/rt/Ds2DecompAt.java` (creates a function in memory at a VA analysis never defined,
then decompiles it -- most of the handlers below are table-only targets), `scripts/ds2-disasm.py`,
`scripts/ds2-rtti-vtables.py`, `scripts/ds2-ffx.py` for the data side.

## 1. How an action id reaches code

Three dispatch layers, tried in order by the action handler chain:

| layer | where | covers |
| --- | --- | --- |
| `FFX::FXBasicActionHandler` slot 1 `0x140f51e40` | `if id < 0x7e && table[id]` call `table[id](handler, instance, paramList, 0)`; table at **`0x1415f9b70`** (126 qwords, static data) | ids 1..118 **[read]** |
| `SfxFxActionHandler` slot 1 `0x140bef150` | `switch`: 11000 -> `0x140bef520`, 11001 -> `0x140bef460`, 18000 -> cluster-appearance path | 11000, 11001, 18000 **[read]** |
| `KatanaSfxFxActionHandler` slot 1 `0x1404c94e0` | `switch 20000..20006` -> `0x140bef5e0(..., descriptor)` with 7 distinct descriptors `0x1404c94c0/9490/9470/94d0/94a0/94b0/9480`; default falls through to `SfxFxActionHandler` | 20000..20006 **[read]** |

The 4th handler argument is 0 from this dispatcher and from 16 (`0x140f4c3fc`), but not from
every caller **[read]**: 52 and 98 pass an effect param through `0x140f528d0` (`0x140f528f3`, a
stack argument moved into `r9`), and 54, 56 and 111 pass the address of a stack descriptor
(54 at `0x140f4f375`, 56 at `0x140f4f472`, 111 from its table call at `0x140f5081c`). The shape
spawners (section 4.3) dereference it as a param object whenever the instance is emitting
(`[[instance+0xc8]+0x40] != 0`), so they are meant to run under 52 / 98 only.

Actions that *create an object* do not name the class in the handler. The handler hands the
action's compiled param block to the instance's `FXCGElement` (`instance+0xe0`), which looks the
action id up in one of four `std::map<int, {compile, create, ...}>` registries filled at static-init
time **[read]**:

| registry | registrar | handler that uses it | FXCGElement slot |
| --- | --- | --- | --- |
| particle appearance | `0x140a30d60` (map at `*0x141672f68`) | `0x140f49bf0` (ids 3,20,40,43,59,61,107) | +0x18 -> `FXCGParticleDrawEntity` -> `FXParticle` `0x140f535f0` |
| cluster appearance | `0x140a33220` (a map in the `*0x141672f90` registry) | `0x140f49d30` (ids 27,66,70,71,82,108) | +0x28 / +0x40 -> `FXCGClusterDrawEntity` `0x140a378d0` |
| cluster emitter | `0x140a33370` (map `[*0x141672f90]+0`) | `0x140f49c80` (ids 28-33,45,117) | +0x30 -> `FXCluster+0x28` (`0x140f54210`) |
| cluster movement | `0x140a33470` (map `[*0x141672f90]+8`) | `0x140f49cf0` (ids 55,84,105) | +0x38 -> `FXCluster+0x30` (`0x140f542a0`) |

Each registration passes a *compile* function (reads the positional ParamList into a packed block
whose first dword is the action id, and panics with `"!!invalid parameter size!!"` citing the
class's own `.cpp`) and a *create* function (allocates the class and stores the block pointer).
The `.cpp` path in each compile function is what pins the class name below **[read]**.

`FXBasicActionHandlerPreprocessor` (vtable `0x141196d40`) precomputes at load: ids 14/16/87/92/95
-> number of non-empty child slots (8/7/16/32/64 slots scanned); id 35 -> a cached local transform
**[read]** (`0x140f52c60`). Two 0x78-byte bool tables it exposes: `0x1415f9fc0` = {3,20,24,40,43,59,
60,61,66,71,75,82,107,108} (the appearance/sound set), `0x1415fa040` = {1,6,15,19,34,46,69,83,86,
106,110,113} (the movement set) **[read]**; their exact purpose **[inf]**.

## 2. Param access convention

A compiled ParamList is `{Param** items; i32 count; i32 count2}`; positional param `i` is
`items[i]` = `*(list+0) + 8*i`, so **byte offset / 8 = param index** everywhere below **[read]**.
Param vtable slots: `+0x10` runtime kind, `+0x30` get sequence object, `+0x50` evaluate scalar,
`+0x58` get value object, `+0x60` is-constant **[read]**.

Runtime kind numbers come from the RTTI template arguments (`FXSingleParamReal<int,1>`,
`FXArrayParam<int,2>`, `FXSequenceParam*<int,3>`, `<float,4>`, `<float,5>` array, `<float,6>`
sequence/random, `<vec4,7>` const, `<vec4,8>` array, `<vec4,9>` sequence, `FXPlacement 16/17/18`,
`FXTick 36/37`) **[read]**. File param type 81 ("float pair") evaluates as kind 6 (a random-range
sequence, `FXSequenceParamRandomSingleBase<float,6>`) **[inf, from the kind==6 branches that accept
type-81 slots]**.

### 2.1 Reference params (file types 44, 46, 59, 60, 71, 87)

The param deserializer `0x140f8b860` builds these classes (vtable written by each case, RTTI name
read at vtable-8) **[read]**:

| file type | case fn | class | vtable |
| --- | --- | --- | --- |
| 44 | `0x140f93630` | `FXSingleParamRef<int,1>` | `0x141289688` |
| 46 | `0x140f92f70` | `FXSequenceParamRef<int,3>` | `0x141289768` |
| 59 | `0x140f92be0` | `FXCallParamRef<unsigned int,22>` | `0x141289ea8` |
| 60 | `0x140f92d10` | `FXCallParamRef<unsigned int,23>` | `0x141289f28` |
| 71 | `0x140f94290` | `FXSingleParamRef<FXTick,36>` | `0x14128a2c8` |
| 87 | `0x140f93510` | `FXSequenceParamRef<FXTick,37>` | `0x14128a338` |

None holds a value. Each stores `u16 scope` at `+0x8` and `i16 index` at `+0xa`; every evaluate
slot (e.g. `0x140fbf6c0`, `0x140fbffe0`, `0x140fbf900`) calls `0x140a11100(instance, scope, index)`
to get the target `Param*` and forwards the call to it **[read]**. `0x140a11100`:

| scope | resolves to | reading |
| --- | --- | --- |
| 1 | `(*[instance+0x50])[index]`: the argument list handed in by the spawner's Param37 | **verified in binary**: `0x140a10f70` stores the 3rd argument of `0x140a08fb0` at `+0x50`, and action 79 (`0x140f4f840`) passes Param37's vtable `+0x58` value there |
| 2 | `[[instance+0x98]+0x78][index]`: the effect file's first root ParamList, the same for every instance of that effect id | **[read]**, see below |
| 3 | a 16-byte local slot: inline at `instance+0xa0+16*index` while `[instance+0xc0] <= 2`, else in the heap array at `[instance+0xa0]` | **[read]**; the same storage `0x140a10df0` hands to action 118, so actions 21/54/109/111/112/115/118 write the locals these refs read **[inf]** |
| other | null | **[read]** |

So a child template like 2101 is parameterised by scope-1 references into the args its parent's
Param37 passes; the arg index is the `i16` at `+0xa`. The data agrees: every reference param in
f0002101 is `raw 1 N` (scope 1), for example the appearance action at arg 4, and both of its root
ParamLists are empty. (An earlier version of this table had scopes 1 and 2 the other way round.)

What scope 2 reads **[read]**: `0x140a08fb0` looks the effect id up with `0x140a0a210` (a binary
search over `[manager+0x1c0]`, keyed on the dword at `+0x8`), calls the hit's vtable `+0x18`, and
that result goes to `instance+0x98`. The objects in that list are `FFX::FXObjectType` (vtable
`0x14118c8e8`, ctor `0x140a0faa0`, 0xb0-byte object), whose vtable `+0x18` is `0x140a101b0` =
`return this`; an `FXObjectTypeCollection` (vtable `0x14127f758`) answers the same slot with
`0x140f87560`, which picks one of its members. The `FXObjectType` is filled by the
`FXSerializableEffect` loader `0x140f84110` (reached by the thunk `0x140f86880`, that class's vtable
`+0x18`): it reads the effect id into `+0x8`, then binds three serializers, `0x140f841ff`
`lea r8,[r14+0x78]` -> `0x140f89cb0`, `0x140f8420f` `lea r8,[r14+0x50]` -> `0x140f89cb0`,
`0x140f8421f` `lea r8,[r14+0xa0]` -> `0x140f88b30`, and runs them in that order. So `+0x78` is the
first root ParamList of the `.ffx` (833: the 15-entry list that matches the sfxparam entry), `+0x50`
the second, `+0xa0` the StateMap.

Data **[data]**: across `sfx9999.ffxbnd.dcx` no reference param uses scope 2; the scopes in use are
1 (spawner args), 3 (locals) and 5. Scope 5 falls in the "other -> null" row, so the type-44 and
type-59 refs that carry `raw 5 0` (by far the most common payload for both types) resolve to
nothing. The cluster emitters' p0 is a type-44 slot that their compile functions never read.

Other instance fields `0x140a10f70` writes **verified in binary**: `+0x80` the parent instance (the
spawning instance for action 79), `+0xc8` the object `0x140a0b4b0` built (EmittersStopped reads its
`+0x40`), `+0x58` a mask (`0xdfffffff`, or `0xffffffff` when byte `core+0xa0` is set),
byte `+0x78` = 0 (ParentExists `0x140fd86c0` is `byte [+0x78] == 0`).

## 3. id -> class table

Count = number of shipped effects (of 1778 in `sfx9999.ffxbnd.dcx`) whose ResourceSet 4th vector
lists the id. Handler = `table[id]` at `0x1415f9b70` unless noted.

| id | effects | runtime class / behaviour | evidence |
| --- | --- | --- | --- |
| 1 | 1735 | `FXMovementAcceleration<ALLOFF/GRAV/ALLON>` / `...YuraYura` / `...PartialFollow` (chosen by content) | handler `0x140f49df0` -> `0x140f52490` -> ctors `0x140f78740/78950/78800/79ed0/7a1b0` **[read]** |
| 3 | 8 | `FXParticleAppearance_Model` | reg `0x140c0db2a`, compile `0x140f75ca0` (`FXParticleAppearance_Model.cpp`) **[read]** |
| 4 | 41 | shape spawner: N child effects in a cone (run by 52 / 98, which supply the effect) | handler `0x140f4a140` **[read]**; params section 4.3 |
| 5 | 1774 | end this instance (push on FXManager list `+0x78`) | `0x140f4a740` -> `0x140a0ae90` **[read]**; "destroy" **[inf]** |
| 7 | 1 | weighted random pick of **one child action** from 8 slots, run through the same table (id < 0x7e) or the handler chain `0x140a0aca0` | stub `mov r9d,8; jmp 0x140f52930`; body sums p0's per-slot weights (`vt+0x58(i)`), draws with the RNG `0x140f56470`, dispatches the chosen slot **[read]** |
| 12 | 2 | `FXElement` state set to 1 if `instance+0x80` is non-null | `0x140f4c2a0` -> `0x140a07180(instance+0xe0, 1, 0)` **[read]** |
| 13 | 0 | `FXElement` state set to 0 | `0x140f4c2c0` -> `0x140a07180(instance+0xe0, 0, 0)` **[read]** |
| 8, 9, 10, 18, 26 | 10,22,3,11,27 | shape spawners: 8 circle, 9 rectangle, 10 sphere, 18 box, 26 elliptic cone (RNG + `0x140a08fb0` in a loop; the effect comes from the caller, not from p0) | handlers `0x140f4a990/4b1a0/4b9b0/4c450/4dd50` **[read]**; params section 4.3 |
| 14 | 1641 | spawn up to 8 child effects (Param37 x8) | `0x140f4c2e0` -> shared body `0x140f51e90` (slot count = preprocessor count, default 7, plus 1) **[read]** |
| 16 | 18 | run up to 8 child actions (Param38 x8) through the same table, else external handler | `0x140f4c390` **[read]** |
| 17 | 1 | no-op | `0x140f4c440` **[read]** |
| 20 | 5 | `FXParticleAppearance_Billboard` (older 24-param layout) | reg `0x140fe186a`, compile `0x141005260` **[read]** |
| 21 | 9 | `p0 <- max(p1 + p2, -1)` (integer add into a referenced slot) | `0x140f4ce90` **[read]**; section 4.4 |
| 27 | 140 | `FXClusterAppearance_PointSprite` | reg `0x140fe43f8`, compile `0x14100b800` **[read]** |
| 28 | 1102 | `FXClusterEmitter_Cone` (vtable `0x14127e938`) | reg `0x140f7b5b3`, compile `0x140f7b7b0`, emit `0x140f7b020` **[read]**; params section 4.1 |
| 29 | 183 | `FXClusterEmitter_Square` (vtable `0x14127ea48`) | reg `0x140f7c353`, compile `0x140f7c3d0`, emit `0x140f7baf0` **[read]**; section 4.1 |
| 30 | 303 | `FXClusterEmitter_Circle` (vtable `0x14127ead8`) | reg `0x140f7d093`, compile `0x140f7d110`, emit `0x140f7c8d0` **[read]**; section 4.1 |
| 31 | 311 | `FXClusterEmitter_Sphere` (vtable `0x14127eb68`) | reg `0x140f7dfd3`, compile `0x140f7e050`, emit `0x140f7d530` **[read]**; section 4.1 |
| 32 | 111 | `FXClusterEmitter_Box` (vtable `0x14127ed28`) | reg `0x140f7f073`, compile `0x140f7f0f0`, emit `0x140f7e440` **[read]**; section 4.1 |
| 33 | 19 | `FXClusterEmitter_EllipticCone` (vtable `0x14127ef58`) | reg `0x140f81303`, compile `0x140f81380`, emit `0x140f80880` **[read]**; section 4.1 |
| 34 | 56 | `FXMovementRotation` | `0x140f4e430` -> ctor `0x140f7a900` **[read]** |
| 35 | 1752 | set local transform: translate + rotate (degrees) | `0x140f4e5f0` **[read]** |
| 36 | 66 | same as 35 plus random jitter on each component | `0x140f4e8d0` **[read]** |
| 40 | 129 | `FXParticleAppearance_Tracer` | reg `0x140fe20c8`, compile `0x141008c80` **[read]** |
| 41 | 2 | end instance if a condition param holds | `0x140f4ee70` **[read]** |
| 43 | 38 | `FXParticleAppearance_Distortion` | reg `0x140fe2f88`, compile `0x14100a510` **[read]** |
| 45 | 1125 | `FXClusterEmitter_FoursidedPyramid` (vtable `0x14127f000`) | reg `0x140f81f93`, compile `0x140f82010`, emit `0x140f81740` **[read]**; section 4.1 |
| 46 | 2 | re-anchor the element to the adapter's transform (`FXAdapter` vtable `+0xb0` = `0x140a266c0`) with two bools | `0x140f4eee0` **[read]**; section 4.4 |
| 51 | 17 | `FXElement` state set (`0x140a07180(elem, p0)`) | `0x140f4f0c0` **[read]** |
| 52, 98 | 1, 4 | run a shape spawner (p0) with an effect (p1); 98 then runs p2 on every child the spawn added | `0x140f4f0f0`, `0x140f50000` -> `0x140f528d0` **[read]**; section 4.3 |
| 54 | 2 | run a child action with a `{count limit, rate, step}` descriptor as its 4th argument | `0x140f4f190` **[read]**; section 4.4 |
| 55 | 1386 | `FXClusterMovement_Acceleration` (vtable `0x14127f0f8`) | reg `0x140f82693`, compile `0x140f82770`, update `0x140f82520` **[read]**; params section 4.2 |
| 56 / 114 | 4 / 0 | conditional dispatch of an action id | `0x140f4f3e0` **[read]** |
| 58 | 6 | pick one of 8 child effects at random and spawn it | stub `mov r9d,8; jmp 0x140f51f40`; body draws with `0x140f56470` and spawns via `0x140a08fb0` **[read]**; that p0 holds per-slot weights, as in 7, **[inf]** |
| 59 | 836 | `FXParticleAppearance_Billboard` (41-param layout) | reg `0x140fe189d`, compile `0x141004620` **[read]** |
| 61 | 268 | `FXParticleAppearance_Model` | reg `0x140c0db5d`, compile `0x140f74f90` **[read]** |
| 63 | 17 | set instance time from a tick param (`0x140a11280`) | `0x140f4f4d0` **[read]**; "set time" **[inf]** |
| 66 | 0 | `FXClusterAppearance_QuadLine` | reg `0x140fe63b8` **[read]** |
| 70 | 466 | `FXClusterAppearance_MultiTextureBillboard` | reg `0x140fe5678`, compile `0x14100f1b0` **[read]** |
| 71 | 273 | `FXClusterAppearance_Billboard` | reg `0x140fe4e98`, compile `0x14100c880` **[read]** |
| 75 | 875 | play sound: p0 sound id, p1 float, p2 bool -> adapter vtbl+0x88 | `0x140f4f7a0` **[read]**; p1 = volume? **[inf]** |
| 79 | 1757 | spawn one child effect (Param37) | `0x140f4f840` -> `0x140a08fb0` **[read]** |
| 80 | 4 | no params: when element flag `0x20` is set and `+0xf0 >= 0`, set `+0xec` to `now + [+0xf0]` unless an earlier deadline is already there | `0x140f4f920` **[read]**; section 4.4; what reads `+0xec` **[unknown]** |
| 82 | 24 | `FXClusterAppearance_Line` | reg `0x140fe5da8`, compile `0x141011f70` **[read]** |
| 83 | 17 | acceleration movement, same body as 106 but without the element-state reset and without reading p8 | stub `xor r9d,r9d; jmp 0x140f52150`; that body ends in `0x140f52490` (the class chooser of id 1) **[read]** |
| 84 | 382 | `FXClusterMovement_Yurayura` (vtable `0x14127f1b8`) | reg `0x140f82d63`, compile `0x140f82e70`, update `0x140f82b70` **[read]**; section 4.2 |
| 85 | 0 | spawn up to 8 child effects (same as 14) | stub -> `0x140f51e90`, default count 7 **[read]** |
| 87 | 132 | spawn up to 16 child effects | `0x140f4fe40` -> `0x140f51e90`, default count 15 **[read]** |
| 92 / 95 | 0 / 0 | spawn up to 32 / 64 child effects | stubs -> `0x140f51e90`, default count 31 / 63 **[read]** |
| 93 / 94 / 96 | 0 | pick one of 32 / 16 / 64 child effects at random (same body as 58) | stubs `mov r9d,0x20/0x10/0x40; jmp 0x140f51f40` **[read]** |
| 99 | 3 | end instance on condition | `0x140f50180` **[read]** |
| 105 | 1031 | `FXClusterMovement_PartialFollow` (vtable `0x14127f278`) | reg `0x140f838b3`, compile `0x140f83ad0`, update `0x140f83460` **[read]**; section 4.2 |
| 106 | 91 | acceleration movement (same `0x140f52490` path as 1): resets the element state to 0, then runs the body shared with 83 with flag 1 (reads p8) | `0x140f50540` -> `0x140a07180(elem,0,0)`, `jmp 0x140f52150` with `r9b = 1` **[read]** |
| 107 | 0 | `FXParticleAppearance_RadialBlur` | reg `0x140fe3d38`, compile `0x14100b150` **[read]** |
| 108 | 0 | `FXClusterAppearance_Model` | reg `0x140c0f498`, compile `0x140f76db0` **[read]** |
| 109, 111, 112, 115 | 6,2,5,1 | 109 `p0 <- round(p1 seconds x 1000)`; 111 the sequence-param form of 54; 112 `p0 <- p1`; 115 no-op (`ret`) | `0x140f50590/50630/50890/502c0` **[read]**; section 4.4 |
| 113 | 195 | `FXMovementCombined` (acceleration + `FXMovementRotation`) | `0x140f508e0` sets `FXMovementCombined::vftable`, calls `0x140f52490` and `0x140f7a900` **[read]** |
| 117 | 0 | `FXClusterEmitter_EqualDistance` (vtable `0x14127eec0`) | reg `0x140f80223`, compile `0x140f80300`, emit `0x140f7f6a0` **[read]**; section 4.1 |
| 118 | 2 | write 3 floats into instance slots `p0, p0+2, p0+4` (`0x140a10df0`) | `0x140f51b10` **[read]** |
| 11000 / 11001 | 16 / 600 | `SfxFxDrawEntityHostSpotLight` / `SfxFxDrawEntityHostPointLight` | `0x140bef520` -> ctor `0x140c0be90` (vtable `0x1411edbe8`) / `0x140bef460` -> ctor `0x140c0b6e0` (vtable `0x1411edb88`) **[read]**; params in section 7.3 |
| 15000 | 482 | `SfxFxClusterAppearance_Billboard` | reg `0x140bff87a`, create `0x140bfefb0`, compile `0x140c01220` (`AppFFX\Cluster\SfxFxClusterAppearance_Billboard.cpp`) **[read]** |
| 15001 | 1444 | `SfxFxClusterAppearance_MultiTextureBillboard` | reg `0x140c03b6a`, create sets vtable `0x1411ed8d0` **[read]** |
| 15002 | 168 | `SfxFxClusterAppearance_PointSprite` | reg `0x140c0894a`, create `0x140c080b0` **[read]** |
| 18000 | 67 | `SfxFXParticleAppearance_Tracer` | reg `0x140bfbd4a`, create `0x140bfb810` **[read]** |
| 20000-20006 | 1,7,96,0,23,345,6 | Katana draw-entity hosts via `0x140bef5e0` + 7 descriptors | **[read]**: 20000 SwordTracer, 20001 Flicker, 20002 Blink, 20003 Tracer, 20004 PointWind, 20005 RadialBlur, 20006 DirectionalLight (ctor vtables, section 7.3) |

Ids with table entries but no shipped use: 6, 11, 13, 15, 22-25, 48, 49, 67-69, 81, 85, 86, 88,
92-96, 100-104, 110, 116. Of these, 67 and 68 are a bare `ret 0` (no-op) **[read]**.

Shared bodies behind the thin stubs **[read]**: `0x140f51e90` spawn every non-empty child slot
(14, 85, 87, 92, 95); `0x140f51f40` spawn one child picked at random (58, 93, 94, 96);
`0x140f52930` run one child action picked by weight (7); `0x140f52150` acceleration movement
(83, 106). None of the four is Arxan-redirected (`scripts/ds2-arxan-chain.py`).

## 4. Param meanings (index = position in the action's ParamList)

### 79 spawn child effect **[read]** (`0x140f4f840`)
p0 Param37 {effect id, arg ParamList}; if id != 0 -> `0x140a08fb0(manager, id, args, 0, ..., parent=this instance, ...)`.

### 14 / 87 spawn child effects **[read]**
p0..p7 (14) or p0..p15 (87) Param37, each spawned if id != 0. Slot count trimmed by the preprocessor.

### 35 local transform **[read]** (`0x140f4e5f0`, preprocessor case 0x23)
p0,p1,p2 translation (x is negated: handedness flip); p3,p4,p5 rotation in **degrees** (converted
with 0.017453292; p3 negated). Result written to the instance placement at `instance+0x38` via
adapter vtbl+0xa8. 833: `[0,0.2,0,0,0,0]` = lift 0.2; `[0,0,0,90,0,0]` = tip 90 deg about X.

### 36 jittered local transform **[read]** (`0x140f4e8d0`)
p0-2 translate, p3-5 rotate (deg), p6-8 +/- random translate, p9-11 +/- random rotate (deg).

### 34 FXMovementRotation **[read]** (`0x140f7a900`)
Signature `[11,81,11,81,11,81,1]`. Three (curve, random-scale) pairs: component0 = p4*p5,
component1 = p0*p1, component2 = p2*p3 (stored at +0x18/+0x1c/+0x20); if all three curves are
constant the products are precomputed, otherwise the curves are kept. p6 = bool flag (only read when
count > 6). Units (deg/s vs rad/s) **[unknown]**.

### 1 particle acceleration movement **[read structure, inf semantics]** (`0x140f49df0`)
Signature `[81,11,81,11,1,1,7]`. p0 (range) = scalar speed multiplied onto the emitter's forward
axis -> initial velocity **[inf]**. p1 curve kept only if p2 is kind 6; p2 curve; p3 curve kept only
if p2 is kind 6. p4 int pushed into an id list (a world-accel/force id; ignored if < 0) **[inf]**.
p5 == 1 -> bool at block+0x70; p6 float -> block+0x74. `0x140f52490` then picks ALLOFF / GRAV /
ALLON / YuraYura / PartialFollow from which block fields are zero.

### 75 sound **[read]**
p0 sound id (type 68), p1 float, p2 bool (!= 0).

### 59 FXParticleAppearance_Billboard **[read]** (compile `0x141004620`, block tag 0x3b)
Param index -> compiled field (the draw code that consumes the fields was not traced, so names are
**[inf]** unless stated):

| p | file type | field / use |
| --- | --- | --- |
| 0 | 40 | +0x30 texture id (833: 132, in the ResourceSet texture vector) |
| 1 | 11 | curve, scaled by p3 range -> +0x40: **width** (see below) |
| 2 | 11 | curve, scaled by p4 -> +0x50: **height**; **ignored when p5 != 0** (default curve used) |
| 3, 4 | 7 | scale for p1 / p2 (min/max against a constant, `0x140f5d490`) |
| 5 | 1 | flag bit0 of the block's tail flags word ("square: height follows width" **[inf]**) |
| 6, 7 | 1 | ints -> +0x38, +0x3c |
| 8 | 19 | RGBA colour curve, 4 channels compiled separately (833: 1,.251,.251,1 = the red tint) |
| 9, 10 | 1 | ints -> +0x90, +0x94 |
| 11 | 6 | int curve -> +0xa8 |
| 12 | 40 | +0x34 second texture id |
| 13, 14, 15 | 1 | ints -> `+0x118`, `+0x11c`, `+0x124`; draw use below |
| 16 | 11 | curve -> +0x80 |
| 17 | 7 | float -> +0x28 |
| 20 | 79 | int range (kind 3) -> +0x98 (random int, e.g. texture-sheet frame **[inf]**) |
| 21 | 81 | random range -> +0x60 (833: 0..6.28 = random initial roll in radians **[inf]**) |
| 22 + 23 | 11 + 81 | curve x range combined -> +0x70: **roll speed, radians per second** (see below) |
| 24 | 7 | float -> `+0x128`, only if kind 4 and count > 24, default -1.0 (`0x1410ac6a4`); draw use below |
| 25, 26 | 11 | curves (count > 25 / > 26) |
| 27 | 7 | float -> `+0x120` (count > 27); no reader found in the draw |
| 29 | 1 | flag bit1 of `+0x12c` (count > 29); draw use below |
| 18, 19, 28, 30-40 | | not read by the compile function; presumably read by the generic particle path (p28 is a tick, -1/30 in 833 = the "unset/infinite" sentinel **[inf]**) |

Consumer read **verified in binary**: `FXParticleAppearance_Billboard` (vtable `0x14129c118`) slot
`+0x8` = `0x141004480`, the per-particle bounds update. It evaluates the `+0x70` curve at the
particle's time (`[ctx+0x18]`), multiplies by the frame step `[ctx+0x18] - [ctx+0x14]`, adds it to
the roll angle at `this+0x10`, and wraps it into -pi..pi (constants pi `0x1410ac9fc`, 2 pi
`0x1410aca00`). It then evaluates `+0x40` and `+0x50`, multiplies them by the particle's own scales
at `+0x78` / `+0x7c`, and writes a box of half-size `0.5 * sqrt(w*w + h*h)` around the particle
position. So `+0x40` / `+0x50` are width / height and `+0x70` is roll speed in rad/s. The other
slots are the destructor, a pure-call, `return 0` and `ret`: the textured draw of the block is
not in this class, and the remaining field names stay **inferred**.

Resource bind **[read]**: the registration at `0x140fe1879` passes three functions with id `0x3b`:
`0x140fe18c0` (calls the compile `0x141004620`), `0x140fe18b0` -> `0x140fe13f0` (create: allocates
`FX4CG::FXCGParticleAppearance_Billboard`, vtable `0x141290940`, whose slot `+0x8` is the same
bounds update `0x141004480`) and `0x140fe1960`, which binds the block to graphics resources through
the `FXCGGraphicsResourceManager` interface (vtable `0x141197a68`):

- `+0x30` (p0) and `+0x34` (p12) each go to manager vtable `+0x18` (a texture lookup by id) and
  land in the draw record's first two slots. So p0 is the texture and p12 a second texture.
- With no second texture the shader variant asked of vtable `+0x20` is 2 (4 when the block's dword
  `+0x124` is non-zero), with one it is 3 (5). Which param feeds `+0x124` was not traced.
- `+0x3c` (p7) goes to vtable `+0x40` = `0x140a3bd10` -> `0x140f56410`, a switch on 0..5:
  0 -> 2 (the caller's default), 1 -> 0, 2 -> 1, 3 -> 4, 4 -> 2, 5 -> 3, anything else -> 0. The
  result is stored in the draw record (`+0x18`); vtable `+0x30` (`0x140a3bc70`) is called with the
  same value and returns vtable `+0x38(1)` when that mapping is non-zero, else `+0x38(0)`. That p7
  is the blend mode is **[inf]**. Shipped action-59 p7 values are 0, 4 and 2 **[data]**; p12 is 0 in all of them, so no
  shipped Billboard binds a second texture.

Draw **[read]**: the textured draw is `FX4CG::FXCGParticleAppearance_Billboard` (vtable
`0x141290940`) slot `+0x10` = `0x140fe14e0`, with the block in `r15`. The block offsets above follow
from the compile's stores (`+0x28` p17 and `+0x30` p0 are the anchors, and the draw's `+0x12c` is
the flags word the compile ORs p5 / p29 into). It reads:

- `+0x118` (p13) as the first argument of render-context vtable `+0x68` (`0x140fe166e`).
- `+0x11c` (p14) as an argument of render-context vtable `+0x30` (`0x140fe15fd`) and `+0x98`
  (`0x140fe16a6`).
- `+0x124` (p15): non-zero makes the draw call render-context vtable `+0x60` and `0x140bf2700` on
  the object at `[entity+0xf0]`, and when that returns 0 it uses the class field `this+0x2c`
  instead of `this+0x28` (the two shader variants the resource bind stored).
- `+0x128` (p24): when the class field `this+0x20` is null (it holds the second bound resource,
  which **[inf]** is the p12 texture) and p24 >= 0, the draw calls render-context vtable `+0x98`
  and hands `0x140fe0eb0` a six-entry index list with the arguments 5 / 5; otherwise no index list
  and 4 / 4.
- `+0x12c` bit1 (p29): on the path without an index list, the last argument to `0x140fe0eb0`
  becomes 8 instead of 4.

What the render-context slots do was not traced, so the names stay open. Data **[data]**: p13 and
p14 are -1 and p24 is -1.0 in every shipped action-59 use (so the indexed path never runs), p27 is
0, p15 is 1 in most uses and 0 in the rest, p29 is 0 or 1 in about equal shares.

### 4.1 Cluster emitters (28-33, 45, 117) **[read]**

Every emitter class has the same vtable shape: slot `+0x10` is the emit function
`(this, state, ctx, count)` and slot `+0x18` the shared `0x140f7b590`. The compiled
block is `[this+0x8]`; `state` is the cluster's particle state, whose arrays at `+0xc8/+0xd0/+0xd8`
are position x/y/z, `+0xe0/+0xe8/+0xf0` velocity x/y/z and `+0xc0` the emitter time at emission,
indexed from `[state+0x20]` **[read]**: the movement integrator `0x140f54ff0` adds velocity x dt to
position and subtracts `+0xc0` from the current time to get particle age. Curves are evaluated
through the compiled-curve table `0x14127d290` at the emitter's time `[state+0x1c]` unless marked
per particle.

Conventions shared by all eight **[read]**:

- **Spread angle**, in degrees: converted with `/ 90.0 * pi/2` (`0x1410c9f20`, `0x1410ac6a0`).
  It is the largest tilt of a particle's velocity away from the emission axis.
- **Direction bias** `s`: tilt = `A * r^(1+s)` when `s <= 0`, `A - A * r^(1-s)` when `s > 0`,
  with `A` the spread angle and `r` uniform in 0..1. So 0 spreads the tilt uniformly, positive
  values pull directions toward the axis, negative toward the rim (a consequence of the formula).
  The spin about the axis is uniform in -pi..pi.
- **Speed**: evaluated per particle, then `0x140f7b5f0` rotates `(0, 0, speed)` by the tilt
  (`0x140007b20`) and spin (`0x1403c2580`), applies the emission matrix and writes the third
  matrix row x speed into the velocity arrays. FoursidedPyramid builds its own direction and
  scales the normalised vector by the speed instead.
- **Emission type** 0..3: 0 uses the emitter matrix as is, 1 and 2 multiply it by the matrices at
  `0x141894bd0` / `0x141894c10` (zero in the image, filled at runtime), 3 uses the derived matrix
  from `0x14012ec30`; anything else panics "invalid emission type." (`FXEmitterUtility.inl`).
- **Common tail**: each compile also writes five values into the caller's struct (slots 0..2 type-82
  curves x random, slot 3 a colour curve that defaults to `0x140f5e820` when the list is short,
  slot 4 an int); which params they are is listed per class below. The consumer is the cluster
  appearance **[read]**: the load-time cluster builder `0x140a26900` compiles the emitter
  (`0x140a32910`) into a stack struct, then the movement (`0x140a32af0`), then passes that same
  struct as the 2nd argument of the appearance compile (`0x140a32740` -> `0x140a327a0`, registry
  slot `+0x28`). `FXClusterAppearance_Billboard`'s compile `0x14100c880` compiles slot 0 into its
  block, slot 1 next to it (replaced by the default curve `0x140f5e830` when slot 4 is non-zero),
  slot 3 as four colour channels, and sets bit0 of its flags from slot 4; the PointSprite compile
  `0x14100b800` uses slots 0 and 3 only, and the Sfx Billboard `0x140c01220` slots 0, 1, 3 and 4.
  None of those three reads slot 2. Names **[inf]** from that use: slots 0 and 1 are the
  particle's width and height scales (random-ranged per particle), slot 4 is "height follows
  width" (the same switch as Billboard 59's p5), slot 3 is a per-particle colour, slot 2 a third
  (depth) scale that a flat sprite has no use for. Cone data **[data]**: the three scale slots are
  1.875 +/- 0.2 in the sample, and the colour is white.

p0 of every emitter is a type-44 reference that no compile function reads (section 2.1).

| id / class | block field <- param (meaning) | common tail (slots 0..4) |
| --- | --- | --- |
| 28 Cone | `+0x28` p1 spread angle; `+0x38` p2 direction bias; `+0x48` p3 speed (per particle, `0x140f7b415`); `+0x58` p6 emission type | p4, p5, p8, p9, int p7 |
| 29 Square | `+0x28` p1 width and `+0x38` p12 depth (each x0.5 = half-extent along the matrix's first and second rows; p12 defaults to p1 when the second count is 12 or less); `+0x48` p2 spread angle; `+0x58` p3 direction bias; `+0x68` p10 position concentration `c` (see below); `+0x78` p4 speed; `+0x88` p7 emission type | p5, p6, p9, p11, int p8 |
| 30 Circle | `+0x28` p1 radius; `+0x38` p2 spread angle; `+0x48` p3 direction bias; `+0x58` p4 speed; `+0x68` p9 radial bias, clamped to -1..1; `+0x6c` p11 area flag (default 1); `+0x70` p7 emission type | p5, p6, p10, p12, int p8 |
| 31 Sphere | `+0x28` p1 radius; `+0x38` p2 spread angle; `+0x48` p3 direction bias; `+0x58` p4 speed; `+0x68` p7 volume flag; `+0x6c` p10 orientation flag (default 1); no emission type | p5, p6, p9, p11, int p8 |
| 32 Box | `+0x28/+0x38/+0x48` p1/p2/p3 size per axis (x0.5); `+0x58` p4 spread angle; `+0x68` p5 direction bias; `+0x78` p6 speed; `+0x88` p9 volume flag; `+0x8c` p13 emit type 0..6 (default 0); p11 not read | p7, p8, p12, p14, int p10 |
| 33 EllipticCone | `+0x28` p1 and `+0x38` p2 spread angles (degrees); `+0x48` p3 direction bias; `+0x58` p4 speed; `+0x68` p8 emission type (`0x140f80b3b`) | p5, p6, p7, p10, int p9 |
| 45 FoursidedPyramid | `+0x28` p1 and `+0x38` p2 spread angles (degrees), one per axis (`0x140007b20`, `0x140004c00`); `+0x48` p3 direction bias; `+0x58` p4 speed; `+0x68` p8 emission type (`0x140f81a09`) | p5, p6, p7, p10, int p9 |
| 117 EqualDistance | `+0x58` p4 spread angle; `+0x68` p5 direction bias; `+0x78` p6 speed; `+0x8c` p12 emit type 0..6 as Box (default 0); `+0x90` p13 spacing; p1-p3 (`+0x28/+0x38/+0x48`) and p9 (`+0x88`) are compiled but the emit never reads them | p7, p8, p11, p14, int p10 |

Per-class position rules **[read]**:

- Square: each coordinate is `U(-half, half) * (1 - r^(1 - max(c, 0)))`, with `U` and `r` separate
  random draws, so `c = 1` collapses everything to the centre.
- Circle: radial fraction `q` from p9 by the same bias rule as direction (positive toward the
  centre, negative toward the rim); when p11 is 0, `q = sqrt(q)` (`0x1410ac694` = 0.5 as the
  exponent), which is what makes a uniform fill of the disc area; position =
  `radius * q * (cos a, sin a)` on the matrix's first two rows, `a` uniform in -pi..pi.
- Sphere: a random direction scaled by the radius, times `cbrt(r)` (exponent 0.33333334) when p7 is
  non-zero and times 1 when it is 0 (the shell). p10 = 0 builds the direction from a full random
  rotation (`0x140007ac0` with two random angles), non-zero from a spin about one axis
  (`0x1403c2580`).
- Box: a face is picked with `rand % 6`; the two in-face coordinates are uniform over the face,
  the third is the half-size times 1 (p9 = 0, on the face) or times a uniform 0..1 (p9 non-zero,
  inside). Emit type 0 aims each particle along its face's axis; 1..3 use emission type 1..3 with
  direction mode 6; 4, 5, 6 use emission type 0 with direction modes 4, 5, 6 (fixed axis swaps);
  anything else panics "invalid emit type." (`FXClusterEmitter_Box.cpp`).
- EqualDistance: particle count = `floor(distance moved since the last emit / p13)`, capped by the
  free slots (`[state+0x10] - [state+0x20]`); particles are placed at equal steps along that segment
  from the saved last position (`this+0x10`), which then advances. It emits along the path the
  emitter travelled, not per frame.
- EllipticCone (`0x140f80880`) writes the emission matrix's translation row into every particle's
  position **[read]**, so all particles start at the emitter origin; FoursidedPyramid does the same
  **[inf]**, Cone's position write was not checked. Its direction **[read]**: the tilt is drawn
  with the direction-bias rule against the larger of the two angles (`if p2 < p1` picks p1's
  scaling, else p2's), the spin is uniform in -pi..pi, and `rotX(tilt) * rotZ(spin)` (`0x140007b20`,
  `0x1403c2580`) gives a direction on a circular cone. That direction is then squeezed along the
  narrower axis: with p1 <= p2 it measures the angle in the Y-Z plane (`0x140832700`, pi/2 minus
  `0x140832880`, so an arccosine if that helper is an arcsine **[inf]**) and rotates about X (`0x140007b20`) by
  `sign * (|a| - |a * p1/p2|)`; with p1 > p2 it does the same in the X-Z plane about Y
  (`0x140004c00`) with `p2/p1`. So p1 limits the tilt toward the matrix's Y axis and p2 toward its
  X axis (the axis naming follows the rotation helpers, as for FoursidedPyramid **[inf]**), and the
  result is normalised and scaled by the per-particle speed.

Data **[data]** (`sfx9999.ffxbnd.dcx`): the signatures match these indices, for example Cone
`[44,11,11,82,82,82,1,1,82,19]`, Circle `[44,9,11,11,82,82,82,1,1,7,82,1,19]`, Box
`[44,11,11,11,11,11,82,82,82,1,1,1,82,1,19]`. The speed slot is type 82 everywhere except a few
FoursidedPyramid uses that carry a type-13 curve there; common-tail slot 3 is type 19 or 20. Emission types in use: Cone 0/1, Circle 0/1, Square, Pyramid and EllipticCone 0;
Box emit type 0, 2 or 5. Sphere p7 and p10, Box p9 and Circle p11 are 0 in every shipped use (so
shipped spheres and boxes emit on the surface, and circles fill the area). Circle p9 is 0, -1 or
-0.5. No shipped effect uses 117.

### 4.2 Cluster movements (55, 84, 105) **[read]**

All three hand the particle arrays to the same integrator `0x140f54ff0` (the update passes
position `+0xc8/+0xd0/+0xd8`, velocity `+0xe0/+0xe8/+0xf0`, birth times `+0xc0`, the current and
previous cluster times `[state+0x1c]` / `[state+0x18]`, and the block fields). With `dt` = current -
previous:

- `+0x28` (p0) evaluated at the current time, times the unit vector at `0x1418949b0`, which the
  static initialiser `0x1410a06d0` sets to `(0, -1, 0, 0)` (from `0x1410f5300`): gravity along
  world -Y, turned into the cluster's frame with the inverse of its matrix (`state+0x80`) and added
  to velocity x dt.
- `+0x5c` scales the vector returned by the force manager (global `0x1418949a0`, set by
  `0x140f563b0`) vtable `+0x28` for the id at `[[[state]+0xc8]+0x38]`, added the same way. Wind
  **[inf]**.
- `+0x58` is a force id handed to the same manager's vtable `+0x18`; a non-zero id adds one more
  term (not decoded).
- `+0x38` is evaluated per particle at the particle's age, `+0x48` per particle with the birth time
  as input; `k = v38 * v48 * dt` is added along each particle's own direction of travel, and a
  particle whose speed would go negative (`|v|^2 + k|k| <= 0`) gets velocity 0. So the product is an
  acceleration along the path (negative = drag that stops, never reverses).

| id / class | compile: block field <- param | extra behaviour |
| --- | --- | --- |
| 55 Acceleration (`0x140f82770`) | `+0x28` p0 gravity; `+0x38` p1 path-acceleration curve; `+0x48` p2 its per-particle factor; `+0x58` p3 force id, `max(p3, 0)`; `+0x5c` p4 wind scale | none: `0x140f82520` calls the integrator directly |
| 84 Yurayura (`0x140f82e70`) | p0, p1, p2 as 55; `+0x5c` p3 wind scale; `+0x58` = 0 (no force id); `+0x60` p4 sway angle curve; `+0x70` p5 sway interval | `0x140f82b70`: when the counter `this+0x20` exceeds p5, p4 is evaluated and `0x140f55c30` rotates every velocity by two random angles uniform in -p4..+p4 degrees (constants -pi/180 `0x1410bf2f8`, 2 `0x1410acb14`, pi/180 `0x1410ac9f0`), speed unchanged; the counter is reset to 0. It is incremented by vtable slot `+0x10` = `0x140f82da0` (`inc [this+0x20]` at `0x140f82e2b` / `0x140f82e3e`), the slot that also fills a per-particle random seed array at `[this+0x18]` for the particles just emitted. The cluster update `0x140f54850` calls movement slot `+0x10` once per update, right after the emitter (`0x140f54da8`), so p5 counts cluster updates, not particles or seconds |
| 105 PartialFollow (`0x140f83ad0`) | p0-p5 as 84; `+0x78` p6 follow curve; `+0x88` p7 follow mode (0 unless the second count is over 7) | `0x140f83460` first computes the emitter's motion since the last update (saved at `this+0x30..`): p7 = 0 takes the whole transform change, non-zero only the translation (`0x140132260`). `0x140f557e0` then evaluates p6 at each particle's age, `f`, and sets position to `(1-f) * pos + f * moved(pos)` and velocity likewise with the rotation part: p6 is the fraction of the emitter's motion a particle follows. Then the sway of 84, then the integrator |

Data **[data]**: 55 is `[11,11,81,1,7]` in most uses (p2 is a type-81 random range, so the
per-particle factor is a random pick), p3 is 0 in every use and p4 mostly 0 (else 0.05, 0.1,
0.25). 84 is `[11,11,81,7,11,1]`, p5 is 5, 10, 15 or 30 in most uses. 105 is
`[11,12,81,7,11,1,11,1]`; p7 is 0 or 1, p5 mostly 0.

### 4.3 Shape spawners (4, 8, 9, 10, 18, 26) and the actions that run them (52, 98) **[read]**

The spawners never read their own p0 (a type-59 ref; `raw 5 0`, which resolves to null, in every
shipped use **[data]**). The effect to spawn is their 4th argument `r9` (`0x140f4a9ce` in 8:
`mov rax,[r9]`): its vtable `+0x30` gives the effect param, whose `+0x50` is the effect id, `+0x58`
the arg list and `+0x60` a third value, all handed to `0x140a08fb0`. Each spawner does nothing
unless the instance is emitting (`[[instance+0xc8]+0x40] != 0`). All scalars are evaluated at the
instance time `+0x74`; count is re-evaluated each time the action runs.

52 (`0x140f4f0f0`) and 98 (`0x140f50000`) are the callers that supply it: p0 is the shape action
(a Param38, or a type-60 ref to one), p1 the effect (Param37 or a ref); both must have a non-zero
id. `0x140f528d0` then calls `table[p0 id](handler, instance, p0 list, p1)`. 98 adds p2, an action
run through `0x140a0aca0` on every child that the spawn appended to the instance's child list
(`+0x90`, walked by `+0x88` from the child that was last before the spawn), so it reaches the
fresh children only. Template 2031 **[data]** uses 98 as `p0 = arg 1` (the shape), `p1 = 2125`,
`p2 = arg 0` (action 79 spawning 2101), so every child the shape places spawns 2101 in turn.

Every spawner builds a local transform `{position, rotation}` per child and calls `0x140a08fb0`
with the spawning instance as parent. Shared rules: spread angle in degrees (`/90 * pi/2`) and
direction bias exactly as the cluster emitters (section 4.1); the rotation is `rotX(tilt)` then
`rotZ(spin)`, spin uniform in -pi..pi. Emission type (where present): 0 passes 1 as the 11th
argument of `0x140a08fb0` (`param_11`), 1 and 2 multiply the rotation by -90 / +90 degrees about X
(`0x14016e310` on the unit axis at `0x1418947a0`), 3 leaves it; 1..3 pass 0. In `0x140a08fb0`,
`param_11 == 1` takes the branch that composes the child's rotation with the parent's
(`0x140336f30`), 0 the branch that does not **[read]**; that 0 means world-aligned is **[inf]**.

| id | shape | p1.. meaning (index = file position) |
| --- | --- | --- |
| 4 | cone (`0x140f4a140`) | p1 spread angle, p2 direction bias, p3 count, p4 emission type; position 0 |
| 8 | circle (`0x140f4a990`) | p1 radius, p2 spread angle, p3 direction bias, p4 radial bias (as Circle emitter p9), p5 count, p6 emission type, p7 area flag (default 1; 0 takes `sqrt` of the radial fraction, uniform area); position `radius * q * (cos a, sin a, 0)` |
| 9 | rectangle (`0x140f4b1a0`) | p1 width, p2 spread angle, p3 direction bias, p4 concentration `c`, p5 count, p6 emission type, p7 depth (default = p1); each coordinate is `+/- 0.5 * size * (1 - r^(1-c))` with a random sign |
| 10 | sphere (`0x140f4b9b0`) | p1 radius, p2 spread angle, p3 direction bias, p4 count, p5 volume flag, p6 orientation flag (default 1); no emission type (`param_11` fixed at 1). p6 = 0: a full random rotation (`0x140007ac0`, three angles) and radius x `cbrt(r)` when p5 != 0; p6 != 0: a spin about one axis (`0x1403c2580`) and radius x `r` (linear, not cube root) when p5 != 0. Position is the rotated Z axis times that radius, so the child faces outward, tilted by the spread |
| 18 | box (`0x140f4c450`) | p1, p2, p3 size per axis (x0.5), p4 spread angle, p5 direction bias, p6 count, p7 volume flag, p8 emit type 0..6 (default 0). Face `rand % 6`, in-face coordinates uniform in -1..1, third coordinate the half-size (p7 = 0) or a uniform fraction of it (p7 != 0). Emit type 0: face-axis rotation, `param_11` 1; 1, 2, 3: fixed rotations `0x141894860` / `0x141894840` / `0x141894880`, `param_11` 0; 4, 5, 6: the same three rotations with `param_11` 1 |
| 26 | elliptic cone (`0x140f4dd50`) | p1 angle about X, p2 angle about Y (degrees), p3 direction bias, p4 count, p5 emission type; each angle is `A * u^(1+s)` (s <= 0) or `A - A * u^(1-s)` (s > 0) with `u` uniform in -1..1 (a negative `u` under a non-integer exponent gives NaN, so only `s = 0` is well-behaved; shipped p3 is 0), and the two rotations are multiplied in an order chosen by which angle is nearer 90 degrees; position 0. The cluster counterpart is EllipticCone (33), which squeezes a circular cone instead (section 4.1) |

Data **[data]**: signatures match, for example 8 `[59,9,11,11,9,5,1,1]` (radius 0.5, spread 30,
count 1), 9 `[59,11,11,11,11,5,1,11]`, 10 `[59,11,11,11,5,1,1]`, 18 `[59,11,11,11,11,11,5,1,1]`
(emit type 3 in the sample), 26 `[59,11,11,11,5,1]`, 4 `[59,11,11,5,1]`.

### 4.4 Param-ref writers and the other small actions **[read]**

The `p0 <- value` writers call p0's vtable `+0x58(instance, &value)`; p0 is a type-44 ref, in
shipped data a scope-3 local (section 2.1).

| id | handler | params |
| --- | --- | --- |
| 21 | `0x140f4ce90` | `p0 <- p1 + p2`, floored at -1 (`cmp eax,-1; cmovge`). Shipped: `local0 <- local0 + (-1)`, a countdown that stops at -1 **[data]** |
| 109 | `0x140f50590` | p1 is a tick evaluated at the instance time; `p0 <- (int)(p1 * 1000 +/- 0.5)`, seconds to milliseconds, the unit of the integer tick evaluators (section 5) |
| 112 | `0x140f50890` | `p0 <- p1` (an int evaluated at the instance time) |
| 115 | `0x140f502c0` | `ret 0`: shipped as `[87, 44]` but does nothing |
| 54 | `0x140f4f190` | p0 child action (Param38 or type-60 ref), p1 int `k`, p2 tick `d`, p3 int sequence (its vtable `+0x88` value `n`), p4 tick (evaluated, unused), p5 tick `T`, p6 int `cap`, p7 tick `t0` (optional, default 0). `rate = n / (T - t0)`, or `n * 60` when `T - t0` is not positive; `limit = n * k` if `k > 0`; if `d > 0`, `limit = min(limit, (int)(d * rate + 1) + n)`; if `cap > 0`, `limit = min(limit, cap)`; -1 means unlimited. Runs p0 with `{limit, rate, n}` as its 4th argument. If the instance's effect id (`[[instance+0x98]+8]`, `0x140a10ed0`) is 2034, the descriptor is `{cap, -1.0, -1}` |
| 111 | `0x140f50630` | the same arithmetic with sequence-valued params and no `t0`: p1 `k` and p2 `d` via vtable `+0x58` then `+0x88`, p3 `n` via `+0x88`, p5 `T` via `+0x58` then `+0x90`; no p7. Shipped `[60,46,87,46,71,87,44]` against 54's `[60,44,71,46,71,71,44]` **[data]** |
| 46 | `0x140f4eee0` | p0 int (bool), p1 int (bool, default 0). Calls `FXAdapter` vtable `+0xb0` (`0x140a266c0`) on the element at `instance+0xe0`: it fetches a target transform from the adapter's own (pure-virtual here) slot, takes the element's current placement by its follow mode (`0x140a06f80`: 0 identity, 1 own, 2 parent's), and with p0 != 0 composes the full matrices, with p0 = 0 transforms the position only; it then sets element state 0 (`0x140a07180`) and, when element flag `0x2` is set, passes both flags to `0x140a2fef0` (not decoded). Shipped `p0 = p1 = 1` **[data]** |
| 80 | `0x140f4f920` | reads no params (section 3 row) |

The descriptor that 54 and 111 build is read by a child handler that takes a 4th argument: for ids
past the table it reaches the external handler `0x141894758` as a stack argument, and
`KatanaSfxFxActionHandler` forwards a descriptor to `0x140bef5e0` (section 1). Which draw host
consumes `{limit, rate, n}`, and so the names "count limit / rate / step", are **[inf]**.

### 15000 SfxFxClusterAppearance_Billboard **[read, partial]** (compile `0x140c01220`, tag 15000)
Reads p1-p37 with version gates (p30..p36 only if count > 30..36, defaults -1.0f); also reads a
kind-9 (vec4 sequence) colour from the *extra* arg list at `count+2`, and kind-4 floats at
`count+5/+6/+9/+10`. Field-level meaning not decoded.

## 5. Triggers and effect lifetime

`FXSerializableEvaluatable<int>` -> runtime class, from the deserializer `0x140fcf750` **[read]**.
Each node is `raw <op> <operandType>`; operandType 1 = int operands, 2 = float operands, 3 = leaf.

| op | class | op | class |
| --- | --- | --- | --- |
| 1 | Constant (value follows) | 12 | OperatorLE |
| 2 | Reference to an int param | 13 | OperatorLT |
| 3 | Reference to an FXTick param | 14 | OperatorEQ |
| 4 | CurrentTick | 15 | OperatorNE |
| 5 | TotalTick | 16/17/18/19 | Add / Sub / Mul / Div |
| 8 | OperatorAnd | 20 | OperatorNot |
| 9 | OperatorOr | 21 | ChildExists |
| 10 | OperatorGE | 22 | ParentExists |
| 11 | OperatorGT | 23 | DistanceFromCamera |
| | | 24 | EmittersStopped |

Leaf semantics **[read]**: ChildExists `0x140a10dd0` returns the **number of children** (walks
`+0x90`/`+0x88`); ParentExists `0x140fd86c0` returns `instance+0x78 == 0`; EmittersStopped
`0x140fe0e30` returns `[instance+0xc8]+0x40 == 0`; CurrentTick<int> `0x140fd7c90` returns
`(int)(instance+0x70 * 1000 + 0.5)`, TotalTick<int> the same on `+0x74`.

Effect 833's single trigger on state 0: `Or( Not(ParentExists), EQ(Constant 0, ChildExists) )` ->
fire when the parent is gone **or** the child count is 0 **[read]**. The trigger's `raw 1` is the
target state index **[inf]**; state 1 holds only action 5, which ends the instance. So the prism
stone glow lives exactly as long as its owner and its spawned children do; there is no timer in the
root effect. Per-particle lifetime sits in the child/template params (not decoded).

**Tick unit [read]:** `FXTick` is a float in **seconds**, not frames; integer tick evaluation
converts to **milliseconds** (x1000, +0.5). File values are multiples of 1/30 s (2.667 = 80/30,
-0.0333 = -1/30), i.e. authored at 30 fps and stored as seconds **[inf from data]**.

## 6. Open

- **Child templates 2000-2200 are ordinary members of `sfx9999.ffxbnd.dcx`** (an earlier note here
  said they were in no bundle; that was wrong). `scripts/ds2-ffx.py list` shows f0002020, 2022-2024,
  2031-2035, 2101, 2102, 2105, 2113, 2115 and 2117-2125. The bundle's on-load handler `0x140bf6330`
  parses every `.ffx` member whose name-id is in 2000..2200 **eagerly** at load
  (`iVar - 2000 < 0xc9`); all other ids are parsed lazily on first spawn **[read]**. See
  `docs/DS2-SFX-REGISTRY.md`. The preprocessor also special-cases `id - 2000 < 200`
  (`0x140f52b70`) and builds descriptors for 2020/2023/2024/2031/2032/2034/2101/2102 (`0x140f52d60`,
  reading arg slots 0,1,2,5,6,7,8,9,11,12,13,15); arg0 tick + arg1 bool feed descriptor type 6
  **[read]**. The templates read those args through scope-1 reference params (section 2.1).
- Action 59's draw (`0x140fe14e0`) is read (section 4): p13, p14, p15, p24 and p29 reach
  render-context slots `+0x30/+0x60/+0x68/+0x98` and the vertex setup `0x140fe0eb0`, whose own
  meaning was not traced, so those names stay open; the vertex fill `0x141005910` (colour curve
  p8, the `+0x80/+0x90/+0x98/+0xa8` fields) was not read. The other appearance classes are
  index -> field only.
- Cluster emitters: the common tail is placed by the cluster appearance compiles (section 4.1),
  but what the draw does with those block fields was not followed, so "width / height / colour"
  for the tail slots is inferred from placement; no reader of slot 2 was found.
- Cluster movements: the force-manager terms (`+0x58` id via vtable `+0x18`, `+0x5c` via vtable
  `+0x28`) are named from the call shape only.
- Positional params are now read for every shape spawner and param-ref writer (sections 4.3, 4.4),
  46 and 80. Still open behind them: the meaning of `param_11 = 0` in `0x140a08fb0` (the branch that does
  not compose with the parent's rotation), which draw host consumes the 54 / 111 descriptor, `0x140a2fef0`
  (46) and the reader of element `+0xec` (80).
- `sfxcommon.ffxbnd` was not scanned: its BND4 entries are 28 bytes wide and `scripts/ds2-ffx.py`
  asserts 36, so its action ids are unchecked. `sfx9999_Append.ffxbnd.dcx` uses no id outside
  the table in section 3 **[data]**.
- Field-level meaning of the weight list in 7 and 58 (`vt+0x58(i)` per slot) is read as "weight"
  from how the sum and the draw use it; the file encoding of that list was not checked against
  shipped data.

## 7. Curves over time, flicker, and effect light

Static RE, 2026-09-25. No game run.

### 7.1 File param type -> runtime class **[read]**

The param deserializer switch is `0x140f8b860` (`FXSerializableParam.inl`); each case constructs one
class, identified by the vtable it stores (names from RTTI):

| file type | value | interpolation | after last key |
| --- | --- | --- | --- |
| 3 / 4 | int | none (step) | clamp / **loop** |
| 5 / 6 | int | linear | clamp / **loop** |
| 9 / 10 | float | none (step) | clamp / **loop** |
| 11 / 12 | float | linear | clamp / **loop** |
| 13 / 14 | float | cubic (key = value, in-tangent, out-tangent) | clamp / **loop** |
| 17 / 18 | vec4 (colour) | none | clamp / **loop** |
| 19 / 20 | vec4 | linear | clamp / **loop** |
| 21 / 22 | vec4 | cubic (3 vec4 per key) | clamp / **loop** |
| 25-30 | vec4 kind 15 | none/linear/cubic x none/loop | |
| 33-36 | FXPlacement | none/linear x none/loop | |
| 79 / 81 / 83 / 85 | int / float / vec4 / tick | `FXSequenceParamRandomSingleBase`: a range, value = min + rand*(max-min) | |
| 80 / 82 / 84 / 86 | int / float / vec4 / tick | `FXSequenceParamRandomSequenceBase`: a curve (child Param) x a random factor | |

So the odd/even pairs are `LoopCategoryNone` / `LoopCategoryCyclic`: **13 is a float cubic spline,
not a float3; 21 is a cubic colour.**

### 7.2 How a curve evaluates **[read]**

- Param-object path (`vt+0x50`): linear/none use `0x140a0a2f0` (last key with `t_key <= t`, lerp to
  the next, **last value held after the last key**). Cyclic first maps time with `0x140fc3c10`:
  `t = (round(t*1000) % round(t_last*1000)) / 1000` -- the period is the **last key's time**, and it
  only loops when the curve has **2+ keys**.
- Compiled path (what appearance blocks store): `{u32 mode; ptr -> [n, times[n], values[n]]}`
  (`0x140a078c0`). Float modes run through the table at **`0x14127d290`** (29 entries): mode 4 =
  linear clamp (`0x140f56cd0`), 0x10 = linear cyclic (`0x140f57880`, `fmodf(t, t_last)`), 0x18 =
  constant, 0x1c = zero; mode+1 = the same curve times a random range (the compile of `p1`/`p3` in
  Billboard, `0x140f5da68`, bumps the mode).
- A 2-key cyclic curve is a **sawtooth** (jumps from the last value back to the first). For a smooth
  loop the last key must repeat the first key's value.
- **Random is per particle, not per frame.** The "x random" evaluators (`0x140f56a10`,
  `0x140f56de0`, ...) step a xorshift state passed by pointer, but both callers checked
  (`0x1410044c3` in the Billboard update, `0x140bfc3e0` in `SfxFXParticleAppearance_Tracer`) copy the
  seed from the particle (`+0x70`) into a stack local every frame and never write it back, so the
  random factor is the same every frame for a given particle. There is no per-frame noise param
  type in this set.
- Time base: the Billboard update passes `[ctx+0x18]` (current time) and uses
  `[ctx+0x18]-[ctx+0x14]` as dt for the roll-speed field. That this is particle age and not effect age
  is **[inf]**.

Shipped examples (`scripts/ds2-ffx.py tree --id N`):

- **181**: one child 2101 whose appearance slot is action **11001** (a point light), orange
  `rgba 1,.627,.251,.251`, radius 10, **flicker p6=5 p7=10 p8=0.8**. Light only, 2375 bytes.
- **28** (and 27): 11001 light, radius 0->25 over 1 s, flicker 5/10/0.9, plus
  `SfxFxClusterAppearance_MultiTextureBillboard` (15001) p49 = **type 20** (looping colour),
  keys `0:(5,3,2,1) 0.5:(10,3,1,2) 1.0:(5,2,1,1)` -- a 1 s HDR orange pulse.
- **1391**: 15000 p10/p11 = **type 12** (looping float), 2 keys over 0.133 s (0.4->2.0, 0.8->1.0):
  a fast sawtooth on size.

Across the bundle: 11001 appears 433 times in 412 effects; 387 have the flicker on. Most common
settings are p6/p7/p8 = 5/10/0.95 (252 uses), 5/10/0.9, 5/10/0.8, 10/30/0.5, 1/4/0.5. No shipped
action-59 Billboard uses a looping curve on size or colour (only p11 type 6 and p32 type 18).

### 7.3 Actions 11000 / 11001 are real lights **[read]**

`0x140bef460` (11001) builds `SfxFxDrawEntityHostPointLight` (ctor `0x140c0b6e0`, vtable
`0x1411edb88`); `0x140bef520` (11000) builds `SfxFxDrawEntityHostSpotLight` (ctor `0x140c0be90`,
vtable `0x1411edbe8`). Per-frame update of the point light: `0x140c0ba70`.

11001 params (14 in most shipped uses):

| p | field | use |
| --- | --- | --- |
| 0 | +0x40 | vec4 curve, x p10 x flicker -> light colour A (diffuse **[inf]**) |
| 1 | +0x48 | vec4 curve, x p11 x flicker -> light colour B (specular **[inf]**) |
| 2 | +0x60 | float curve = **radius**; light is registered only while radius > 0 (vt+0xa0 on the adapter), "large" flag when > 44.0 |
| 3 | +0x68 | float, not read by the update **[unknown]** |
| 4 | +0x6c | bool -> light byte |
| 5 | +0x70 | float -> light float (default 1.0) |
| 6, 7 | +0x74, +0x78 | **flicker period min / max, in 1/30 s ticks** |
| 8 | +0x7c | **flicker floor** (brightness dips to this; default 1.0) |
| 9 | +0x90 | int, not read by the update **[unknown]** |
| 10, 11 | +0x50, +0x58 | vec4 curves multiplied onto p0 / p1 (shipped alpha 1..7 = intensity **[inf]**) |
| 12, 13 | +0xac, +0xad | bools -> light bytes |

Flicker **[read]**: enabled unless p6 == p7 == 0 or p8 == 1.0. Each period is a random integer in
[p6, p7] ticks (xorshift seeded 123456789); brightness is a triangle wave 1.0 -> p8 -> 1.0 over the
period, multiplied onto both colours. Curve time = instance time minus the light's start time.

The same `2101` child template carries either appearance: arg slot 4 is the appearance action
(833: `59` billboard; 832 and 181: `11001` light).

Lights go through the deferred renderer (`GXDeferredLightRenderer::_RenderLights<GXDrawPointLight>`
in RTTI) and so light the map and characters **[inf: the adapter vt+0x90/+0xa0 hand-off was not
traced to the renderer]**.

Katana actions 20000-20006 **[read, ctor vtables]**: 20000 SwordTracer, 20001 Flicker, 20002 Blink,
20003 Tracer, 20004 PointWind, 20005 RadialBlur, 20006 DirectionalLight. Shipped uses in sfx9999:
20002 x67, 20004 x20, 20005 x186, 20006 x8, 20000 x1, **20001 x0**. `SfxFxDrawEntityFlicker` reads
p12..p24 (ints, floats, kind-7 vec4); what it does was not decoded.

### 7.4 KatanaSfxSystem+0x120 is environment-light *receiving*, not emitting **[read]**

`sys+0x120` is a 0x70 object built by `0x140bfa760(obj, allocator, sys)`; `obj+8` = the system.
When an sfxparam entry has `+0x2c > 0`, `0x140bfaee0` takes a record from pool `0x140bfa070` (and
from `0x140bfa0f0` too when byte `+0x1a` is set), then `0x140c0b560` sets `root+0x119 = 2` and
allocates four float4 at `root+0xf0..0x108`. Every frame `0x140bfb1d0` -> `0x140bfb2b0` calls
`KatanaSfxSystem` vt+0xe0 = `0x1404c50f0`, which samples **the map's baked light data at the
effect's position** (`MapManager` -> area `+0x188` -> `0x140401800`, three shorts x 1/1024 = RGB).
The sample is cross-faded over 0.5 s when it changes, then
`root+0xf0 = max(1 - w*rgb, 0) + sample * w*rgb` with `rgb = entry+0x20..0x28` and
`w = entry+0x2c`. Result: the effect is **darkened/tinted by the area's lighting**. It casts
nothing. 833 has `+0x2c = 0`: fully self-lit.

sfxparam entry (0x40 bytes), from `0x140bed0a0`, `0x140bfaee0`, `0x140bfb140`, `0x140bfb2b0`:

| off | type | meaning |
| --- | --- | --- |
| +0x00 | i32 | effect id (blob sorted by id; lookup `0x140bf9dc0` = binary search per blob, **first blob that has the id wins**) **[read]** |
| +0x04 | f32 | equals root ParamList p0; no consumer found **[unknown]** |
| +0x08 | i32 | start-time skip, ticks of 0.033 s (= root p3); 833: 30 = starts 1 s in **[read]** |
| +0x0c | i32 | random extra start ticks, 0..n **[read]** |
| +0x12 | u8 | with +0x14: non-zero -> `KatanaSfxWindControl` via `sys+0x308` (`0x140bf9190` -> sys vt+0xd8 `0x1404c4e00`, ctor `0x1404c58e0`) **[read]** |
| +0x13 | u8 | set in 1968/2325 entries; no consumer in the spawn **[unknown]** |
| +0x14 | i32 | see +0x12 (15 in 11 entries) **[read]** |
| +0x1a | u8 | also take the second pool record (the 3 x float4 at root+0xf8..0x108; directional light terms **[inf]**) **[read gate]** |
| +0x1c | i32 | passed to the `KatanaSfxWindControl` ctor **[read]**; meaning **[unknown]** |
| +0x20..+0x28 | f32 x3 | env-light tint colour (= root p7..p9) **[read]** |
| +0x2c | f32 | env-light weight; > 0 turns the sampling on (= root p10) **[read]** |
| +0x30..+0x3c | f32 x4 | rgb x a -> `root+0xd0` constant colour multiplier (= root p11..p14) **[read]** |

The root ParamList of the effect matches its sfxparam entry (p0, p3, p7..p14) in 695 of 958
effects; the entry, not the root list, is what the spawn reads **[read]**.

`0x140becc90(sys, blob, size)` accepts any buffer with magic `"sfxp"` and version 5, copies it with
the list's allocator, and appends it to the tail of `[sys+0x118]+8`. A DLL can call it with its own
blob. Because lookup takes the first match in list order, an appended blob cannot override an id the
shipped `sfxParameter.sfxparam` already has; a new id (25001) is fine. No sfxparam entry is needed
for an effect to spawn or to carry an 11001 light.

### 7.5 Light shafts, bloom, lens flare **[read]**

No FFX appearance or Sfx host renders shafts or flares. The engine has them as global post
effects: `ToneMap_LightShaftBlur.fpo` / `ToneMap_LightShaftComposite.fpo` (loaded by
`0x140b92900`), `ToneMap_Bloom*`, `GXLensFlareFilter` and `LensFlare_FlashLight*` shaders. "Light
Shaft", "Bloom" and "Player Light Shadow" are graphics-config labels (`0x1410d0420` area, beside
`GraphicsConfig_SOFS.xml`). What drives the shaft source (sun or light list) was not traced. The
cheap stand-in is bloom: colour values above 1 (27/28 use 5..10) make the billboard bloom.

### 7.6 What a mod needs for a flickering, light-casting stone

1. **Light:** the effect needs an 11001 point light. With no resizing: `ds2-ffx.py patch --id 181
   --new-id 25002` with leaf edits (colour at `0x04f4`/`0x052e`, radius `0x0568`, flicker
   `0x05d8`=p6, `0x05f4`=p7, `0x0610`=p8), register it like 25001, and spawn 25001 and 25002 at the
   same position. For a candle: p6 3, p7 9, p8 0.6-0.8, radius 3-6.
2. **Glow flicker:** the billboard p1/p2 (size, type 11) or p8 (colour, type 19) must become the
   looping type (12 / 20) with 3+ keys whose last key repeats the first, e.g. alpha
   `0:1.0 0.13:0.8 0.2:0.95 0.33:0.75 0.47:1.0`. That grows the ParamList, and `ds2-ffx.py patch`
   only rewrites leaves in place: the patcher needs an insert-keyframes operation that fixes every
   enclosing object length. Random types (81/82/84) do not flicker a single particle; they are fixed
   per particle.
3. **sfxparam:** not needed for either. Only add an entry (via `0x140becc90`) to use start-time skip,
   env-light tinting (+0x2c), the constant colour multiplier (+0x30), or wind.

A looping colour on action-59 p8 does survive the Billboard compile: the type-20 class's per-channel
compile `0x140fb8ae0` emits mode `0x10` (looped) with 2+ keys **[read]**. Still unproven until a
run: that the FFX point light visibly lights the scene. The exact byte changes for the stone's
derived copy (181's light child spliced over the sparkle child, p8 as a type-20 curve, and the
enclosing lengths to fix) are in `docs/DS2-STONE-LIGHT-FLICKER.md`.
