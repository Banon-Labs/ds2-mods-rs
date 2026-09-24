# The infusion mark, the item icon, and the requirement the game already tests

Everything here was read statically from `darksoulsii-deobf.bin` (SOTFS build 9527516) with the
Ghidra MCP daemon on port 8766, `scripts/ds2-disasm.py`, `scripts/ds2-arxan-chain.py`,
`scripts/ds2-ebl.py` and `scripts/ds2-flo.py`. **No game was launched to establish any of it, and
nothing built on it has been in front of a running game.** Addresses are VAs, the form the
disassembly prints; subtract `0x140000000` for the RVA `ds2-rva` records.

## The question, and the answer that contradicts it

The question was "where does an infused weapon's unique icon come from". It does not have one.

**The item icon is the item's own id and nothing else.** `FUN_140035e10` builds the icon key as a
`{kind, id}` pair; the id comes from `FUN_14003bd90`, which reads the `u32` at inventory-entry
`+0x18` and passes it through `FUN_14003c8c0` -- a swap between two specific ids, not an infusion
offset. The kind is `6`, which falls to the default arm of the path builder `FUN_14048c3e0`:

```c
default:
  FUN_140043050(local_90, L"IC_%010d.tpf", 0xc);
  lVar8 = 2;                       // prefix table entry 2
...
local_b0[2] = L"icon:/tex/Icon/";
FUN_140003690_format_wstr(param_1, path, local_b0[lVar8], param_3);
```

So the path is `icon:/tex/Icon/IC_<10 digits>.tpf`, one texture per item id, with no infusion
component anywhere in the derivation. `FUN_14002fa50` (`FeIconTexManager`, named by its own panic
string `"..\\..\\Source\\Frontend\\FeIconTexManager.cpp"` / `"pIcon == NULL!"`) then turns that
name into a `FeIconProxy`, and `FUN_1400bc850` binds it onto the cell's icon element.

## The infusion mark is nine sibling elements, and the bind switches one on

`FUN_1400bc850` at `0x1400bc850` is the item-cell bind -- one call per visible row per refresh,
from `FUN_1400bc2b0`, `ItemSelectDialog`'s list rebuild. Its last loop, in full:

```asm
0x1400bca70  lea   rdx,[rsp+0x20]
0x1400bca75  mov   rcx,rsi                 ; the FeItemData
0x1400bca78  call  0x140034e70             ; the infusion nibble
0x1400bca7d  mov   QWORD PTR [rbp+0x98],1  ; the id path's count
0x1400bca88  movzx r15d,BYTE PTR [rax]     ; r15 = the nibble
0x1400bca9a  movzx eax,bl                  ; bl = i, 0..15
0x1400bca9d  add   eax,0x5f5c3e0           ; the element id
0x1400bcaa2  mov   DWORD PTR [rdi],eax
0x1400bcaa4  lea   rcx,[r14+0x2d0]         ; the infusion container's accessor
0x1400bcaaf  lea   r8,[rbp+0x70]           ; the path
0x1400bcaaf  lea   rdx,[rbp-0x20]          ; out
0x1400bcab3  call  0x140027c80             ; resolve
0x1400bcab8  cmp   bl,r15b
0x1400bcabf  sete  dl
0x1400bcac2  call  0x14001e270             ; setVisible(i == infusion)
0x1400bcac9  cmp   bl,0x10
0x1400bcacc  jb    0x1400bca70
```

Sixteen ids driven, **nine authored**. The mechanism is a visibility toggle over pre-authored
sibling elements: not an icon-id offset, not an atlas source-rect swap, not a param-table column.

`FUN_140034e70` is the whole of the model side:

```asm
0x140034e79  movzx EDX,word ptr [RCX + 0x2]   ; the handle off the FeItemData
0x140034e7d  cmp   DX,-0x1                    ; 0xffff means no item
0x140034e9f  call  0x1401abfb0                ; ItemInventory2's entry lookup
0x140034ea9  cmp   byte ptr [RAX + 0x1e],0x1  ; the item TYPE
0x140034ead  jbe   0x140034ebc
0x140034eaf  xor   AL,AL                      ; type > 1 -> no infusion
0x140034ebc  movzx EAX,byte ptr [RAX + 0x26]
0x140034ec0  and   AL,0xf                     ; the low nibble IS the infusion
```

So: entry `+0x1e` is the item type, entry `+0x26`'s low nibble is the infusion, and only types
`0` and `1` can carry one. The name for it in the binary is `EquipmentCustomAttribute` -- the
blacksmith's infusion menu is `FeGroupNpcMenuChangeAttribute`, and `ItemInventory2`'s setter shows
up in the RTTI string
`.?AV?$FeFunctorJob@U?$JOB_MEMBER_FUNCTOR_ARG2@VItemInventory2@@FVEquipmentCustomAttribute@@X@@@@`.

## The elements, in the file

```
$ python3 scripts/ds2-ebl.py extract /menu/02.febnd.dcx --out /tmp/menu02
$ python3 scripts/ds2-flo.py tree /tmp/menu02/l02_02_Inventory.flo --def 0x7a
def 0x007a children=8                                    the item cell
  [0] def=0x005d id=0x05f5c3e0 xy=(0, 0)                 the icon group
  [1] def=0x0051 id=0x05f5c800 xy=(2.05, 0.70)           a highlight, 00ffffff at rest
  [2] def=0x0070 id=0x05f5c3e2 xy=(51.40, 48.15)         THE INFUSION CONTAINER
  [3] def=0x0072 id=0x001ed2b9 xy=(-6.70, -8.95)
  [4..6] def=0x0079 id=0                                 highlights, frames 63..81
  [7] def=0x0077 id=0x05f5c3e1 xy=(9.45, 73.35)          the durability gauge

$ python3 scripts/ds2-flo.py tree /tmp/menu02/l02_02_Inventory.flo --def 0x70
def 0x0070 children=9
  [0] def=0x005f id=0x5f5c3e9   [3] def=0x0065 id=0x5f5c3e6   [6] def=0x006b id=0x5f5c3e3
  [1] def=0x0061 id=0x5f5c3e8   [4] def=0x0067 id=0x5f5c3e5   [7] def=0x006d id=0x5f5c3e2
  [2] def=0x0063 id=0x5f5c3e7   [5] def=0x0069 id=0x5f5c3e4   [8] def=0x006f id=0x5f5c3e1
```

Nine children, ids `0x5f5c3e9` down to `0x5f5c3e1`, all at `(0, 0)`, each a `kind = 0x4` nested
definition holding exactly one `kind = 0x1` shape. **Slot `0` is not authored**, which is why an
uninfused weapon shows nothing, and neither are slots `10..15`.

The same container is authored three times over, which is why `ds2-item-warn` fingerprints it by
its nine ids rather than by an index:

| document | container | item cell |
| --- | --- | --- |
| `l02_02_Inventory.flo` | `0x0070` | `0x007a`, container at `(51.40, 48.15)` |
| `l02_03_equipment.flo` | `0x006b` | `0x0075`, container at `(51.40, 48.15)` |
| `l02_01_In-Game.flo` | `0x0121` | `0x0128` at `(53.90, 55.25)`, `0x012c` at `(51.40, 52.80)` |

### The cell's other elements, for the record

`FUN_1400b7680` builds the cell view's eight accessors, and its whole body is eight id-path chains:

| offset | path | what the bind writes |
| --- | --- | --- |
| `+0x090` | `0x5f5c3e0 / 0x5f5c3e0` | the icon texture, and a sequence (`0x70` / `0x7a`) |
| `+0x120` | `0x5f5c3e0 / 0x5f5c3e1` | visible from entry `+0x1f` bit 1 |
| `+0x1b0` | `0x5f5c3e0 / 0x5f5c3e2` | visible from `FUN_140039140` |
| `+0x240` | `0x5f5c3e1` | the durability gauge, value and visibility |
| `+0x2d0` | `0x5f5c3e2` | the infusion container |
| `+0x360` | `../ 0x5f5c420 / 0x5f5b9f2` | the item name |
| `+0x3f0` | `../ 0x5f5c421 / 0x5f5c5ad` | the item count, red (`0x98`) over the cap |
| `+0x480` | `0x5f5c3e0 / 0x5f5c3e3` | visible from `FeItemData` byte `+4` |

## The requirement test, which the game performs on every stat row it draws

`FUN_1400bcde0` decides which sequence a stat row in the item detail pane plays. Disassembled:

```asm
0x1400bcdfa  mov   rax,[0x1416148f0]       ; GameManagerImp
0x1400bce07  mov   rcx,[rax+0x22e0]        ; the frontend root
0x1400bce14  call  0x1404ffb20             ; mov rax,[rcx+0x138]; ret -- the player's stat table
0x1400bce1f  call  0x14003d750             ; &DAT_14155def0 + key*12
0x1400bce24  movsx rcx,WORD PTR [rax+0x4]  ; the player-stat index for this column
0x1400bce2b  js    0x1400bce5c             ; negative -> not a requirement row
0x1400bce2d  movss xmm0,[rdi+0x4]          ; the item's required value
0x1400bce32  lea   rdx,[rcx+rcx*2]         ; idx*3
0x1400bce39  movd  xmm1,[rsi+rdx*8]        ; statTable[idx * 0x18], an i32
0x1400bce3e  cvtdq2ps xmm1,xmm1
0x1400bce41  comiss xmm0,xmm1
0x1400bce44  jbe   0x1400bcf0e             ; required <= have -> met
0x1400bce4a  mov   DWORD PTR [rbx],0x98    ; UNMET -> sequence 0x98, element 0x5f5c5b7
```

`0x98` is the same style the bind uses for an over-cap item count, and `0x67` is the normal one.
That `0x98` is the RED one is inference from where it is used, not a measurement -- see "What is
not measured".

### The key table, dumped

`DAT_14155def0`, `0x60` entries of 12 bytes, bounded by `FUN_14003d750`'s own
`if (0 <= key && key < 0x60)`. Exactly ten carry a non-negative `i16` at `+0x04`:

```
key  stat  key  stat  key  stat
0x11   8   0x33   8   0x42  10
0x12   9   0x34   9   0x43  11
0x13  10   0x35  10
0x14  11   0x36  11
```

Corroborated from the other side by the detail pane's own row tables, which `FUN_1400bbaa0` walks:
the weapon pane (`0x1415640c0`, 19 rows) opens `0x33 0x34 0x35 0x36`, the armour pane
(`0x141564090`, 10 rows) opens `0x11 0x12 0x13 0x14`, and the ring pane (`0x141564078`, 5 rows)
contains `0x42 0x43`. Two instruments, one answer.

### Where the required value lives

`FUN_1400312e0(row, key)` is the column accessor -- a `switch (key - 1)` over `0x5e` cases, every
arm a plain load returning in `RAX`:

```c
case 0x33: return *(u16*)(row + 0x70);   // required Strength
case 0x34: return *(u16*)(row + 0x72);   // required Dexterity
case 0x35: return *(u16*)(row + 0x74);   // required Intelligence
case 0x36: return *(u16*)(row + 0x76);   // required Faith
case 0x11: return *(u16*)(row + 0x3c);   // armour, same four
case 0x12: return *(u16*)(row + 0x3e);
case 0x13: return *(u16*)(row + 0x40);
case 0x14: return *(u16*)(row + 0x42);
```

`row` is what `FUN_140035070(allocator, out, descriptor)` leaves in `out[0]`, and the descriptor is
what `FUN_14003c2d0(item, out)` builds. All three are plain-integer functions with no allocation
and no out-parameter destructor, which is what makes the chain safe to call from a detour --
unlike `FUN_140037560`, the route the detail pane itself takes, which constructs a `DLString` the
caller has to free.

## The game has two requirement checks and they disagree

There is no shared predicate. The presentation check above and the mechanics check below are
separate implementations over the same four columns.

`FUN_14034d3c0` at `0x14034d3c0` (RVA `0x0034d3c0`, prologue
`48 8b c4 55 53 56 57 48 8b ec 48 83 ec 68 41 8b f0`, clean by `ds2-arxan-chain.py`) is the
mechanics one:

```c
float lack(WeaponParamRow *row, ChrStatus *stat, int grip) {
    req = *(u16*)(row + 0x18);                    // 0x14034d443
    if ((unsigned)(grip - 2) <= 1) req >>= 1;     // 0x14034d44c  shr cx,1
    d0 = 1.0f - (float)str / (float)(req ? req : 1);
    // +0x1a / +0x1c / +0x1e against DEX / INT / FTH
    return clamp(max(0,d0) + max(0,d1) + max(0,d2) + max(0,d3), 0.0f, 1.0f);
}
```

Requirements met is `== 0.0f`. It feeds `FUN_14034cc80` (`0x0034cc80`), which turns the ratio into
a damage multiplier out of `PlayerLackOfStatsParam`. Two narrower siblings exist --
`FUN_14034ce60` (`0x0034ce60`, Strength and Dexterity) and `FUN_14034cfe0` (`0x0034cfe0`,
Intelligence and Faith) -- and armour has its own pair at `0x0034d150` and `0x0034ccf0` reading
`ArmorParam + 0x2c/0x2e/0x30/0x32`.

Its stat source is `FUN_14038d510` at `0x14038d510`, nine bytes in full:

```asm
48 63 c2            movsxd rax,edx
0f b7 44 41 16      movzx  eax,word ptr [rcx+rax*2+0x16]
c3                  ret
```

`chrStatus + 0x00 + i*2` is the base stat block and `+0x16 + i*2` the effective one; indices
`4..7` are Strength, Dexterity, Intelligence, Faith. The effective block is
`clamp(base + modifiers, 1, 99)` -- `FUN_1401ffcc0` (`0x001ffcc0`), called from `FUN_14038d570`
at `0x14038d61c` -- so rings and spEffects are provably in it, which is exactly what could not be
proved about the frontend's table.

### Two-handing, which is not a Strength multiplier

DS2 does not scale Strength. `FUN_14034d3c0` halves the weapon's Strength requirement with an
integer `shr cx,1` when the grip state is `2` or `3`. The `1.5x` that does exist belongs to power
stance: `FUN_140350170` (`0x00350170`) requires
`effStr >= min(99, (short)trunc(max(reqStrL, reqStrR) * 1.5f))` and the same for Dexterity, with
`1.5f` at `0x1410bd0a8` (`mulss xmm0,[0x1410bd0a8]` at `0x14035025c` and `0x14035028b`), for grip
states `4`, `5` and `6`. Grip states `5` and `6` do not get the halving.

**Neither reaches the presentation check**, which takes no grip argument at all. So the detail
pane's requirement numbers -- and the badge -- ignore both.

### The cached answer, and why it is the wrong shape here

`FUN_14034a980` (`0x0034a980`) caches the mechanics ratio per weapon record:

| offset in record | content |
| --- | --- |
| `+0x38` | `float` deficiency over all four stats; `0.0` means every requirement is met |
| `+0x3c` | the same over Strength and Dexterity only |
| `+0x40` | the same over Intelligence and Faith only |

Records live at `equipObj + 0x50 + n * 0x48` for `n = 0..7` -- six equipped slots plus the two
held weapons -- with armour's equivalent at `equipObj + 0x290 + i * 0x30 + 0x28`. It is a boolean
answer already computed, and it covers only equipped items; a badge drawn on inventory rows is
mostly drawn on items that are in none of those eight records. Hence the per-cell recomputation.

### The second corroboration of the requirement column offsets

The frontend record's `+0x70`/`+0x72`/`+0x74`/`+0x76` -- what `FUN_1400312e0` returns for keys
`0x33`..`0x36` -- are copied raw out of `WeaponParam`, with no transform in between:

```asm
140201ec0  0f b7 4f 18   movzx ecx,[rdi+0x18]   ->  140201ec6  66 89 4b 70   mov [rbx+0x70],cx
140201eca  0f b7 4f 1a   movzx ecx,[rdi+0x1a]   ->  140201ece  66 89 4b 72   mov [rbx+0x72],cx
140201ed2  0f b7 4f 1c   movzx ecx,[rdi+0x1c]   ->  140201ed6  66 89 4b 74   mov [rbx+0x74],cx
140201eda  0f b7 4f 1e   movzx ecx,[rdi+0x1e]   ->  140201ede  66 89 4b 76   mov [rbx+0x76],cx
```

`WeaponParam.param` is 473 rows at stride `0xbc`, reached by `getWeaponParamRow` (`0x001ab860`)
off `CharacterManager + 0x420`. Data-side sanity over the 373 rows with id at or above 1,000,000:
`+0x18` has 68 zeros and a maximum of 70; `+0x1c` has 330 zeros; every staff has Strength and
Dexterity at zero with a high `+0x1c`, and every chime has a high `+0x1e` with `+0x1c` at zero.

## What ships: `crates/ds2-item-warn`

Off unless `[item_warn] enabled = true`. Two hooks, both clean prologues by
`scripts/ds2-arxan-chain.py`:

| site | RVA | prologue | what it does |
| --- | --- | --- | --- |
| `FUN_140b50f20` | `0x00b50f20` | `40 55 41 54 41 56 41 57 48 83 ec 68` | gives the infusion container a tenth child |
| `FUN_1400bc850` | `0x000bc850` | `48 89 5c 24 18 55 56 57` | shows it when the requirements are unmet |

The container hook substitutes the builder's **argument**, not `FeLayoutDocument::findDefinition`'s
return, because `ds2-menu-row` already owns that lookup and MinHook binds one detour per address.
`FUN_140b50f20` hands the same definition pointer on to the `FeComponentSprite` it allocates, which
keeps it at `+0x48` as the display list's capacity, so substituting the argument raises the walk
and the capacity together -- which is the whole of what the lookup substitution does.

### The X is the game's own, and it costs one rect

The mark was asked for as a red X and it is one -- the same  the game already draws on an unusable
quick-slot weapon, not art this repo invented and not a tinted stand-in.

`l01_05_L_key.flo` shape `0x002a` samples `(740.65, 164.05)-(769.65, 195.55)` of **`waku_03`** and
is drawn by def `0x0036` child `[6]`, element `0x5f5c3e6`, at `(180.70, 501.25)` over an item icon
whose own box is `(183.15, 419.20)` plus `64 x 128`. So the game puts its  in the lower-left of
the icon it marks, which is where `FE_ITEM_WARN_OFFSET` puts this one.

**Finding it needed no eye.** The earlier reading here -- that the atlas could not be inspected and
so the art had to be a clone -- was wrong twice over. `scripts/ds2-tpf.py` pulls a named texture out
of `GameDataEbl` by walking BND4 member names (the archive keys on a path hash and stores no names,
so no path lookup could ever have found it), and `scripts/ds2-atlas-find.py` reports art as rects by
thresholding on colour and labelling the connected blobs. Cross-correlated against the  cut out of
a screenshot of that HUD slot, this rect scores `+0.79` and the next best candidate in `waku_03`
scores `+0.35`. Its ink is `(745, 169)-(767, 191)`, 367 opaque pixels, mean `rgb(181, 44, 16)` --
already red, so nothing is tinted any more.

**Why a rect write reaches it.** The cloned infusion glyph samples `waku_03` too: shape `0x005e` in
`l02_02_Inventory.flo`, `0x0059` in `l02_03_equipment.flo`, `0x010f` in `l02_01_In-Game.flo`, one
rect `(934.70, 52.50)-(960.30, 78.50)` in all three. A `FeComponentTextureShape` resolves its
texture at draw time from its shape-table entry's quad, which is shared with every other user of
that shape -- but `FUN_140b70200` gives each component its own copy of the destination and source
rects, at `+0x50` and `+0x58`. `crates/ds2-item-warn/src/place.rs` re-points the source copy at the
 and leaves the entry alone, so one badge changes and nothing else in the document does. Same
atlas, different rect, no texture of this repo's own.

The destination rect is still measured off the glyph's shipped rect rather than the 's, because
the per-quad matrix at `+0x48` carries the glyph quad's own offset (`-934.70, -52.50`) and cancels
it. Re-pointing the source changes which pixels arrive and moves nothing.

### What the hooks refuse

* the container hook, unless the definition has exactly nine children carrying
  `FLO_INFUSION_CONTAINER_IDS` in order, and unless the child it clones is `kind & 4` -- a nested
  definition, because that is the subtree `place` walks to reach the component's rect arrays
  (`FUN_140b50bc0` sends `kind & 1` straight to `FUN_140b51270`, with no definition under it);
* the placement, unless the component it found carries exactly one quad and its source rect is
  either the glyph's shipped `(934.70, 52.50)-(960.30, 78.50)` or the  already written over it --
  so a component this has no business in keeps the rects the game built;
* both hooks, unless the bytes at the site are the ones recorded, with what was actually found
  printed beside what was wanted;
* the pair, together: if the cell-bind hook refuses, the container hook is disarmed and every
  definition passes through untouched.

### What is not measured

* **Any of it, at runtime.** Not one line has been in front of a running game.
* **Whether the frontend stat table holds base or modified stats.** Every cross-reference to
  `FUN_1404ffb20` is a reader; its writer was not found. On the gameplay side the equivalent block
  is provably `clamp(base + modifiers, 1, 99)`, and this one has no such proof, so whether rings
  and spEffects are in these numbers is open. A badge computed from base stats would light up on a
  weapon the player can swing while wearing a Ring of Blades.
* **That style `0x98` renders red.** It is not a `FeColorSetParam` row -- that param's ids are
  `1`, `2`, `100`, `101`, `10000..10009`, `20000..20002`, `30000..30003` -- so the style-id space
  is FeLayout element states inside `.flo` payloads, which `scripts/ds2-flo.py` does not decode.
  What is proved is that `0x98` is the same style the comparison panels use for "this stat got
  worse" (`0x1400bcecf`, `0x140091f04`, `0x1400922bc`, `0x14009275b`, `0x140097e85`) against
  `0x99` for better and `0x67` for unchanged. The badge's own colour is set by this mod and does
  not depend on it.
* **Shop lists.** `FUN_14003c2d0`'s descriptor covers a shop entry as well as a bag entry, but only
  the bag path has been traced, and the gate reads the bag entry's `+0x1e`. A shop row is expected
  to fall out as "cannot ask", which shows no badge.
* **Armour and rings.** The columns and stat indices exist (`0x11..0x14`, `0x42`, `0x43`) and are
  not used: the badge lives inside the infusion container, which only weapons and shields have.
* **The pause menu's own two item cells** put the infusion container up to `7.1` units from where
  the inventory and equip cells put it, so the badge lands correspondingly off there.
