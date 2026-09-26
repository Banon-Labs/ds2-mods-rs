# FFX action ids -> runtime classes and param meanings

Static RE of `DarkSoulsII.exe` (flat image `darksoulsii-deobf.bin`, base `0x140000000`), 2026-09-25.
No game run was involved.

Every claim is tagged **[read]** (read out of the binary: a table, a vtable, a source path, a
constant) or **[inf]** (inferred: from names, data values, or shape of the code, not proven).

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
| 1 | `(*[instance+0x50])[index]`: a param of the effect's own root ParamList | **[read]**; "root list" **[inf]** from the preprocessor reading the same pointer |
| 2 | `[[instance+0x98]+0x78][index]` (`0x140a10130`): the argument list handed in by the spawner's Param37 | **[read]** access; "spawner's args" **[inf]** |
| 3 | a 16-byte local slot: inline at `instance+0xa0+16*index` while `[instance+0xc0] <= 2`, else in the heap array at `[instance+0xa0]` | **[read]**; the same storage `0x140a10df0` hands to action 118, so actions 21/54/109/111/112/115/118 write the locals these refs read **[inf]** |
| other | null | **[read]** |

So a child template like 2101 is parameterised by scope-2 references into the `{id, args}` its
parent's Param37 passes; the arg index is the `i16` at `+0xa`.

## 3. id -> class table

Count = number of shipped effects (of 1778 in `sfx9999.ffxbnd.dcx`) whose ResourceSet 4th vector
lists the id. Handler = `table[id]` at `0x1415f9b70` unless noted.

| id | effects | runtime class / behaviour | evidence |
| --- | --- | --- | --- |
| 1 | 1735 | `FXMovementAcceleration<ALLOFF/GRAV/ALLON>` / `...YuraYura` / `...PartialFollow` (chosen by content) | handler `0x140f49df0` -> `0x140f52490` -> ctors `0x140f78740/78950/78800/79ed0/7a1b0` **[read]** |
| 3 | 8 | `FXParticleAppearance_Model` | reg `0x140c0db2a`, compile `0x140f75ca0` (`FXParticleAppearance_Model.cpp`) **[read]** |
| 4 | 41 | spawn N child effects in a cone | handler `0x140f4a140` **[read]** |
| 5 | 1774 | end this instance (push on FXManager list `+0x78`) | `0x140f4a740` -> `0x140a0ae90` **[read]**; "destroy" **[inf]** |
| 7 | 1 | weighted random pick of **one child action** from 8 slots, run through the same table (id < 0x7e) or the handler chain `0x140a0aca0` | stub `mov r9d,8; jmp 0x140f52930`; body sums p0's per-slot weights (`vt+0x58(i)`), draws with the RNG `0x140f56470`, dispatches the chosen slot **[read]** |
| 12 | 2 | `FXElement` state set to 1 if `instance+0x80` is non-null | `0x140f4c2a0` -> `0x140a07180(instance+0xe0, 1, 0)` **[read]** |
| 13 | 0 | `FXElement` state set to 0 | `0x140f4c2c0` -> `0x140a07180(instance+0xe0, 0, 0)` **[read]** |
| 8, 9, 10, 18, 26 | 10,22,3,11,27 | spawn N child effects in a shape (RNG + `0x140a08fb0` in a loop; first param = type-59 effect ref) | handlers `0x140f4a990/4b1a0/4b9b0/4c450/4dd50` **[read]**; shapes **[unknown]** |
| 14 | 1641 | spawn up to 8 child effects (Param37 x8) | `0x140f4c2e0` -> shared body `0x140f51e90` (slot count = preprocessor count, default 7, plus 1) **[read]** |
| 16 | 18 | run up to 8 child actions (Param38 x8) through the same table, else external handler | `0x140f4c390` **[read]** |
| 17 | 1 | no-op | `0x140f4c440` **[read]** |
| 20 | 5 | `FXParticleAppearance_Billboard` (older 24-param layout) | reg `0x140fe186a`, compile `0x141005260` **[read]** |
| 21 | 9 | write a referenced param (p0 via `+0x58`, p1/p2 ints) | `0x140f4ce90` **[read]**; purpose **[inf]** |
| 27 | 140 | `FXClusterAppearance_PointSprite` | reg `0x140fe43f8`, compile `0x14100b800` **[read]** |
| 28 | 1102 | `FXClusterEmitter_Cone` | reg `0x140f7b5b3`, compile `0x140f7b7b0` **[read]** |
| 29 | 183 | `FXClusterEmitter_Square` | reg `0x140f7c353` **[read]** |
| 30 | 303 | `FXClusterEmitter_Circle` | reg `0x140f7d093` **[read]** |
| 31 | 311 | `FXClusterEmitter_Sphere` | reg `0x140f7dfd3` **[read]** |
| 32 | 111 | `FXClusterEmitter_Box` | reg `0x140f7f073` **[read]** |
| 33 | 19 | `FXClusterEmitter_EllipticCone` | reg `0x140f81303` **[read]** |
| 34 | 56 | `FXMovementRotation` | `0x140f4e430` -> ctor `0x140f7a900` **[read]** |
| 35 | 1752 | set local transform: translate + rotate (degrees) | `0x140f4e5f0` **[read]** |
| 36 | 66 | same as 35 plus random jitter on each component | `0x140f4e8d0` **[read]** |
| 40 | 129 | `FXParticleAppearance_Tracer` | reg `0x140fe20c8`, compile `0x141008c80` **[read]** |
| 41 | 2 | end instance if a condition param holds | `0x140f4ee70` **[read]** |
| 43 | 38 | `FXParticleAppearance_Distortion` | reg `0x140fe2f88`, compile `0x14100a510` **[read]** |
| 45 | 1125 | `FXClusterEmitter_FoursidedPyramid` | reg `0x140f81f93`, compile `0x140f82010` **[read]** |
| 46 | 2 | adapter call vtbl+0xb0 with 2 ints | `0x140f4eee0` **[read]** |
| 51 | 17 | `FXElement` state set (`0x140a07180(elem, p0)`) | `0x140f4f0c0` **[read]** |
| 52, 98 | 1, 4 | conditional action / conditional child spawn (`0x140f528d0`) | `0x140f4f0f0`, `0x140f50000` **[read]** |
| 54 | 2 | param-ref arithmetic (types 60/44/71/46) | `0x140f4f190` **[unknown detail]** |
| 55 | 1386 | `FXClusterMovement_Acceleration` | reg `0x140f82693`, compile `0x140f82770` **[read]** |
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
| 80 | 4 | clamp instance time `+0xec` against `+0xf0` when flag 0x20 | `0x140f4f920` **[read]**; meaning **[unknown]** |
| 82 | 24 | `FXClusterAppearance_Line` | reg `0x140fe5da8`, compile `0x141011f70` **[read]** |
| 83 | 17 | acceleration movement, same body as 106 but without the element-state reset and without reading p8 | stub `xor r9d,r9d; jmp 0x140f52150`; that body ends in `0x140f52490` (the class chooser of id 1) **[read]** |
| 84 | 382 | `FXClusterMovement_Yurayura` | reg `0x140f82d63` **[read]** |
| 85 | 0 | spawn up to 8 child effects (same as 14) | stub -> `0x140f51e90`, default count 7 **[read]** |
| 87 | 132 | spawn up to 16 child effects | `0x140f4fe40` -> `0x140f51e90`, default count 15 **[read]** |
| 92 / 95 | 0 / 0 | spawn up to 32 / 64 child effects | stubs -> `0x140f51e90`, default count 31 / 63 **[read]** |
| 93 / 94 / 96 | 0 | pick one of 32 / 16 / 64 child effects at random (same body as 58) | stubs `mov r9d,0x20/0x10/0x40; jmp 0x140f51f40` **[read]** |
| 99 | 3 | end instance on condition | `0x140f50180` **[read]** |
| 105 | 1031 | `FXClusterMovement_PartialFollow` | reg `0x140f838b3`, compile `0x140f83ad0` **[read]** |
| 106 | 91 | acceleration movement (same `0x140f52490` path as 1): resets the element state to 0, then runs the body shared with 83 with flag 1 (reads p8) | `0x140f50540` -> `0x140a07180(elem,0,0)`, `jmp 0x140f52150` with `r9b = 1` **[read]** |
| 107 | 0 | `FXParticleAppearance_RadialBlur` | reg `0x140fe3d38`, compile `0x14100b150` **[read]** |
| 108 | 0 | `FXClusterAppearance_Model` | reg `0x140c0f498`, compile `0x140f76db0` **[read]** |
| 109, 111, 112, 115 | 6,2,5,1 | param-ref writes (types 44/46/87/71) | `0x140f50590/50630/50890/502c0` **[unknown detail]** |
| 113 | 195 | `FXMovementCombined` (acceleration + `FXMovementRotation`) | `0x140f508e0` sets `FXMovementCombined::vftable`, calls `0x140f52490` and `0x140f7a900` **[read]** |
| 117 | 0 | `FXClusterEmitter_EqualDistance` | reg `0x140f80223` **[read]** |
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
| 1 | 11 | curve, scaled by p3 range -> +0x40 (width **[inf]**) |
| 2 | 11 | curve, scaled by p4 -> +0x50 (height **[inf]**); **ignored when p5 != 0** (default curve used) |
| 3, 4 | 7 | scale for p1 / p2 (min/max against a constant, `0x140f5d490`) |
| 5 | 1 | flag bit0 of the block's tail flags word ("square: height follows width" **[inf]**) |
| 6, 7 | 1 | ints -> +0x38, +0x3c |
| 8 | 19 | RGBA colour curve, 4 channels compiled separately (833: 1,.251,.251,1 = the red tint) |
| 9, 10 | 1 | ints -> +0x90, +0x94 |
| 11 | 6 | int curve -> +0xa8 |
| 12 | 40 | +0x34 second texture id |
| 13, 14, 15 | 1 | ints -> tail of block |
| 16 | 11 | curve -> +0x80 |
| 17 | 7 | float -> +0x28 |
| 20 | 79 | int range (kind 3) -> +0x98 (random int, e.g. texture-sheet frame **[inf]**) |
| 21 | 81 | random range -> +0x60 (833: 0..6.28 = random initial roll in radians **[inf]**) |
| 22 + 23 | 11 + 81 | curve x range combined -> +0x70 (roll speed **[inf]**) |
| 24 | 7 | float, only if kind 4 and count > 24 |
| 25, 26 | 11 | curves (count > 25 / > 26) |
| 27 | 7 | float (count > 27) |
| 29 | 1 | flag bit1 (count > 29) |
| 18, 19, 28, 30-40 | | not read by the compile function; presumably read by the generic particle path (p28 is a tick, -1/30 in 833 = the "unset/infinite" sentinel **[inf]**) |

### 28 FXClusterEmitter_Cone **[read]** (compile `0x140f7b7b0`)
Signature `[44,11,11,82,82,82,1,1,82,19]`. p0 (type 44 arg ref) is not read by the compile.
Cone-specific block: p1, p2 (curves) -> +0x28, +0x38; p3 (type 82 = curve x float) -> +0x48; p6
int -> +0x58. Common emitter outputs (written to the caller's struct, i.e. shared FXClusterEmitter
fields): p4, p5, p8 sequences, p9 colour sequence (only if count > 9), p7 int. Names (radius, angle,
spawn rate...) **[unknown]**. Emitters 29-33/45/117 follow the same compile shape; their per-class
indices are in `Ds2DecompAt` output for `0x140f7c3d0 0x140f7d110 0x140f7e050 0x140f7f0f0 0x140f80300
0x140f81380 0x140f82010`.

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
  **[read]**. The templates read those args through scope-2 reference params (section 2.1).
- The draw/update code that consumes each compiled appearance block was not traced, so most
  appearance field names above are **[inf]** or blank.
- Reference params: the class and the scope/index lookup are read (section 2.1); what object sits
  at `instance+0x98` (the scope-2 owner) was not traced to its writer in `0x140a08fb0`.
- Emitter/movement per-class field names (only index -> field is known).
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

Unproven until a run: that a looping curve on action-59 p1/p8 survives the Billboard compile (it is
type-generic, but no shipped 59 does it), and that the FFX point light visibly lights the scene.
