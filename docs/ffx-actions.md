# FFX action ids -> runtime classes and param meanings

Static RE of `DarkSoulsII.exe` (flat image `darksoulsii-deobf.bin`, base `0x140000000`), 2026-09-25.
Tracking issue: `ds2-mods-rs-tup`. No game run was involved.

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

## 3. id -> class table

Count = number of shipped effects (of 1778 in `sfx9999.ffxbnd.dcx`) whose ResourceSet 4th vector
lists the id. Handler = `table[id]` at `0x1415f9b70` unless noted.

| id | effects | runtime class / behaviour | evidence |
| --- | --- | --- | --- |
| 1 | 1735 | `FXMovementAcceleration<ALLOFF/GRAV/ALLON>` / `...YuraYura` / `...PartialFollow` (chosen by content) | handler `0x140f49df0` -> `0x140f52490` -> ctors `0x140f78740/78950/78800/79ed0/7a1b0` **[read]** |
| 3 | 8 | `FXParticleAppearance_Model` | reg `0x140c0db2a`, compile `0x140f75ca0` (`FXParticleAppearance_Model.cpp`) **[read]** |
| 4 | 41 | spawn N child effects in a cone | handler `0x140f4a140` **[read]** |
| 5 | 1774 | end this instance (push on FXManager list `+0x78`) | `0x140f4a740` -> `0x140a0ae90` **[read]**; "destroy" **[inf]** |
| 7, 12 | 1, 2 | no function at table entry (data/obfuscated) | **[read]** |
| 8, 9, 10, 18, 26 | 10,22,3,11,27 | spawn N child effects in a shape (RNG + `0x140a08fb0` in a loop; first param = type-59 effect ref) | handlers `0x140f4a990/4b1a0/4b9b0/4c450/4dd50` **[read]**; shapes **[unknown]** |
| 14 | 1641 | spawn up to 8 child effects (Param37 x8) | `0x140f4c2e0` **[read]** |
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
| 58 | 6 | pick one of 8 child effects at random (int list weights?) and spawn | `0x140f4f4c0` **[read]**; weights **[inf]** |
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
| 83 | 17 | table entry not decodable as a function | **[unknown]** |
| 84 | 382 | `FXClusterMovement_Yurayura` | reg `0x140f82d63` **[read]** |
| 87 | 132 | spawn up to 16 child effects | `0x140f4fe40` **[read]** |
| 99 | 3 | end instance on condition | `0x140f50180` **[read]** |
| 105 | 1031 | `FXClusterMovement_PartialFollow` | reg `0x140f838b3`, compile `0x140f83ad0` **[read]** |
| 106 | 91 | acceleration movement (same `0x140f52490` path as 1) + element flag | `0x140f50540` **[read]** |
| 107 | 0 | `FXParticleAppearance_RadialBlur` | reg `0x140fe3d38`, compile `0x14100b150` **[read]** |
| 108 | 0 | `FXClusterAppearance_Model` | reg `0x140c0f498`, compile `0x140f76db0` **[read]** |
| 109, 111, 112, 115 | 6,2,5,1 | param-ref writes (types 44/46/87/71) | `0x140f50590/50630/50890/502c0` **[unknown detail]** |
| 113 | 195 | `FXMovementCombined` (acceleration + `FXMovementRotation`) | `0x140f508e0` sets `FXMovementCombined::vftable`, calls `0x140f52490` and `0x140f7a900` **[read]** |
| 117 | 0 | `FXClusterEmitter_EqualDistance` | reg `0x140f80223` **[read]** |
| 118 | 2 | write 3 floats into instance slots `p0, p0+2, p0+4` (`0x140a10df0`) | `0x140f51b10` **[read]** |
| 11000 / 11001 | 16 / 600 | SfxFx game-side actions -> `0x140bef520` / `0x140bef460` | **[read]**; class **[unknown]**, param lists are colour-heavy (lights? **[inf]**) |
| 15000 | 482 | `SfxFxClusterAppearance_Billboard` | reg `0x140bff87a`, create `0x140bfefb0`, compile `0x140c01220` (`AppFFX\Cluster\SfxFxClusterAppearance_Billboard.cpp`) **[read]** |
| 15001 | 1444 | `SfxFxClusterAppearance_MultiTextureBillboard` | reg `0x140c03b6a`, create sets vtable `0x1411ed8d0` **[read]** |
| 15002 | 168 | `SfxFxClusterAppearance_PointSprite` | reg `0x140c0894a`, create `0x140c080b0` **[read]** |
| 18000 | 67 | `SfxFXParticleAppearance_Tracer` | reg `0x140bfbd4a`, create `0x140bfb810` **[read]** |
| 20000-20006 | 1,7,96,0,23,345,6 | Katana draw-entity hosts via `0x140bef5e0` + 7 descriptors | **[read]**; which of `SfxFxDrawEntityHost{Blink,Flicker,PointWind,RadialBlur,SwordTracer,Tracer,...}` per id **[unknown]** |

Ids with table entries but no shipped use: 6, 11, 13, 15, 22-25, 48, 49, 67-69, 81, 85, 86, 88,
92-96, 100-104, 110, 116.

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

- **Child "effects" 2000-2199 (e.g. 2101, 2032, 2023 in 833) are not in any `Game/sfx` bundle**
  (`sfx9999`, `_Append`, `sfxcommon` = one dummy entry). The preprocessor special-cases
  `id - 2000 < 200` (`0x140f52b70`) and builds descriptors for 2020/2023/2024/2031/2032/2034/2101/
  2102 (`0x140f52d60`, reading arg slots 0,1,2,5,6,7,8,9,11,12,13,15). Where their bodies come from
  is the bundle-registration question (other agent). Until then, what Param37 {2101, args} does with
  args 0-8 is only partially known: arg0 tick + arg1 bool feed descriptor type 6 **[read]**.
- The draw/update code that consumes each compiled appearance block was not traced, so most
  appearance field names above are **[inf]** or blank.
- Type-44/59/60/46/71/87 "arg reference" params: resolution path not traced.
- Classes behind 11000/11001 and the per-id Katana hosts 20000-20006.
- Emitter/movement per-class field names (only index -> field is known).
