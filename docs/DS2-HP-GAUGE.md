# The floating HP bar and damage number

What `ds2-hp-gauge` hooks, and the evidence for each piece. Static facts are from
`darksoulsii-deobf.bin`; tuning values are from a live Frida run on 2026-09-25
(`scripts/frida/hp-text-scale.js`), in which the user adjusted each value by eye during a
seamless invasion until they said it looked good.

## The object

`FeSceneEnemyHpGuage`: constructor `0x140065820` (alloc `0x1640`, built by the HUD builder
`0x140507ea0`), vtable `0x1410b0fe8`, update `0x140066c60`. It keeps twenty per-slot scenes at
`this + 0x18 + slot * 8`. The collector `0x140067cd0` fills the slots from the lock-on target
and then `CharacterManager`'s entity list; the filter `0x1400660f0` leaves out the player's own
character.

For each live slot the update calls, in this order:

| call | RVA | places |
|---|---|---|
| bar | `0x663b0` | frame `0x113e10`, fills `0x113e11`/`0x113e12`, through `0x66290` |
| text | `0x669c0` | element `0x5f5b9f2`, the damage number |

Both take `(gauge, u32 slot, float4* pos, float scale, float2* offset)`, and the scale sits in
XMM3. It is render height / 720, read from `0x1410ae458`, which 22 other readers share -- so the
constant itself is not patched.

## Why a bigger scale did not make the number bigger

The text call submits its matrix through vtable `+0x120` (`FeComponentBase`, `0x140b6aa10`,
4x4 -> 3x4), which forwards to `+0x118`, `FeComponentObject::setMatrix` at `0x140b6aa70`:

```text
140b6aabc  movaps xmm0,[rdi]           ; copy the 3x4 to *(this+0x58)
...
140b6aae1  mov    eax,7
140b6aae6  mov    byte [rbx+0x99],1
140b6aaed  mov    byte [rbx+0x9a],sil  ; the flag
140b6aaf4  cmovnz eax,ecx              ; flag ? 1 : 7
140b6aaf7  mov    byte [rbx+0x30],al   ; apply mask
```

The text call passes flag 1, so the mask is 1 (translation only) and the stored scale is
ignored. The bar's calls pass 0. Live, a scale of 50 on the text call drew the number at its
normal size until the mask was set to 7 and the flag to 0 after the call, after which it drew
very large. The matrix is already stored by then, so those two bytes are the whole difference.

## The number's width

`0x66190(gauge, scene, i32 value)` is the only writer of the number's text: `value < 1` clears
it, otherwise it is clamped to 99999 and formatted in decimal. It is called from `0x140067900`
only when the number changes, so the digit count is recorded there, keyed by scene.

The number's pivot is the left edge of its text (seen live: with the pivot on the bar's centre,
the number's left edge sat on the centre). The layout gives no glyph advance, so centring uses a
tuned `digit_advance` of 8 layout units per digit.

## The bar's width

`l01_13_hp_enemy.flo` (from `/menu/01.febnd.dcx`), root def `0x12`: frame `0x113e10` def 7 at
`(0, 24)`, the two fills, and the text `0x5f5b9f2` def `0x11` at `(3.05, 0.3)`. The frame's
shape `0x0006` has atlas rect x `153.1 .. 252.7`, a width of 99.6, left-anchored at the pivot.

## Element lookup

`0x140afda00(scene, id, id, 0, ...)` reads all eleven arguments -- four registers and seven
stack slots `[rsp+0x80] .. [rsp+0xb0]` -- packs them into a zero-terminated id path and searches
the scene. The detour calls it exactly as the text call does.

## Arxan

`scripts/ds2-arxan-chain.py` finds a real prologue at `0x1400663b0` and `0x140066190`. At
`0x1400669c0` it stops at `4c 8b dc 48 81 ec f8 00` -- `mov r11,rsp; sub rsp,0xf8`, which is a
prologue, not a redirect.
