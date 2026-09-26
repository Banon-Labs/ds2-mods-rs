# Where the game reads a pad's buttons

The input harness's `buttons` verb writes `PadDevice+0x198` as a `u16` after the pad poll
returns. An earlier static pass concluded that nothing in the game reads that field, and that the
verb was therefore pressing a button nothing listens to. That conclusion is wrong. The field is
read every frame, through a virtual method that tests it with a `test` instruction, and
`scripts/ds2-field-xrefs.py` cannot see a `test`: it only matches `mov`-family opcodes.

Every claim below is tagged. "Verified in binary" means read out of `darksoulsii-deobf.bin` or the
Ghidra project, with the address given. "Inferred" means reasoned from verified facts but not read
directly.

## The reader: `PadDevice` vtable slot 27 (`+0xd8`), `0x140f04d40`

`DLUID::PadDevice`'s vtable is at `0x141271aa8`. Slot 27 (byte offset `0xd8`) is `0x140f04d40`,
a pad-specific override (keyboard and mouse have their own at the same slot). Verified in binary.

It is `bool IsButtonDown(this, int keyId)`, with one arm per pad backend. It picks the arm the same
way the poll does (verified in binary, `0x140f04d40`..`0x140f04dcc`):

| Arm | Condition | Key ids | Read |
|---|---|---|---|
| third backend | `[this+0x314] >= 0` | `0x36..=0x45` | `test dword [this+0x2f8], u32_table[keyId]`, table at `0x1415f6438 + 4*keyId` |
| XInput | `[this+0x314] < 0` and `[this+0x19c] >= 0` | `0x28..=0x35` | `test word [this+0x198], u16_table[keyId]` at `0x140f04d94`, table at `0x1415f64a0 + 2*keyId` |
| DirectInput | `[this+0x100] != 0` | `0x08..=0x27` | `test byte [this+0x170+keyId], 0x80`, which is `DIJOYSTATE.rgbButtons[keyId-8]` at `+0x178..` |

The XInput table, read out of the image (verified in binary):

| key id | mask | XInput name |
|---|---|---|
| `0x28` | `0x0001` | `DPAD_UP` |
| `0x29` | `0x0002` | `DPAD_DOWN` |
| `0x2a` | `0x0004` | `DPAD_LEFT` |
| `0x2b` | `0x0008` | `DPAD_RIGHT` |
| `0x2c` | `0x0010` | `START` |
| `0x2d` | `0x0020` | `BACK` |
| `0x2e` | `0x0040` | `LEFT_THUMB` |
| `0x2f` | `0x0080` | `RIGHT_THUMB` |
| `0x30` | `0x0100` | `LEFT_SHOULDER` |
| `0x31` | `0x0200` | `RIGHT_SHOULDER` |
| `0x32` | `0x1000` | `A` |
| `0x33` | `0x2000` | `B` |
| `0x34` | `0x4000` | `X` |
| `0x35` | `0x8000` | `Y` |

The masks are exactly `XINPUT_GAMEPAD.wButtons` bits, so the encoding of `+0x198` is the raw
`wButtons` word the poll stores at `0x140f05b85`. START is key id `0x2c`, mask `0x0010`. Verified in
binary.

Slot 28 (`+0xe0`, `0x140f05410`) is the matching setter: the same three arms, `or`/`and` on the same
fields (`or word [rcx+0x198],ax` at `0x140f05473`, `and word [rcx+0x198],ax` at `0x140f0547e`).
Verified in binary. Nothing in this pass found a caller for it; it is recorded so the next reader
knows the engine itself treats `+0x198` as writable button state.

Which arm a connected Xbox pad takes: `0x140f06740` is the XInput connect path. It runs only while
`[this+0x100] == 0` and `[this+0x314] < 0`, stores the XInput user index into `+0x19c`, and sets the
capability word `+0x214` to `0x31b`. The third backend's connect path, `0x140f06540`, sets
`+0x214` to `0xffffc1b` instead. Verified in binary. So an Xbox pad polled through
`XInputGetState` has `+0x314 < 0` and `+0x19c >= 0`, and slot 27 reads `+0x198` for it. Inferred
from those two paths; not read live.

## When it is read: after the poll, in the same call

`DLUserInputDeviceImpl`'s update is shared slot 7 (`+0x38`), `0x140f16cc0`. Verified in binary:

1. `0x140f16d89`: `call [vtable+0xb8]`, the pad poll `0x140f05540`.
2. `0x140f16d8f`: `test al,al` on the poll's return. The poll returns `al=1` after a successful
   `XInputGetState` (`0x140f05b8c`, right after the `+0x198` store) and `al=0` when it skips
   itself or finds no device (`0x140f05f06`, and the focus gate at `0x140f05599`).
3. On a non-zero return, the listener list at `[this+0x98]..[this+0xa0]` runs:
   `0x140f1c540` (`0x140f16e4e`) calls `0x140f1c470`, which calls every converter's vtable
   `+0x28` with the device as its first argument.
4. `DLUserInputDirectConverter`'s `+0x28` (vtable `0x141277c18`) is `jmp 0x140f1a9d0`; that body
   calls the device's `+0xd8` at `0x140f1aaf1` for each binding's key id and sets one bit per
   pressed binding. `DLUserInputDAConverter`'s `+0x28` (vtable `0x141277a18`) is
   `jmp 0x140f1a040`, which calls `+0xd8` at `0x140f1a0dc`.

So a value stamped into `+0x198` after the original poll returns, and before the detour itself
returns, is what slot 27 sees that frame. The harness's detour (`crates/ds2-input-harness/src/device.rs`,
`poll`) does exactly this and passes the original's return value through, so the listener loop
still runs. Inferred from the call order above.

On a zero return the listener loop is skipped entirely (the `jne 0x140f16df5` is not taken), so a
stamp on a frame where the poll skipped itself reaches nobody. Verified in binary. With a pad
connected and the pad's focus option clear this does not apply; the harness's own drive notes
record that option reading 0 live.

## What the harness must write for START

- Object: the `DLUID::PadDevice` whose poll the detour just ran (`this` in `rcx`).
- Offset: `+0x198`.
- Width and encoding: `u16`, raw `XINPUT_GAMEPAD.wButtons` bits. START is `0x0010`.
- Timing: after the original poll returns, in the detour, with the original's return value handed
  back unchanged.

That is what `device.rs` already writes (verified by reading `write_pad`). The field, width,
encoding and timing are all correct for the XInput arm.

The gap is the other arms. When `[this+0x314] >= 0` slot 27 reads `+0x2f8` instead, and when the
pad is DirectInput it reads `rgbButtons` at `+0x178`; an authored `+0x198` is invisible in both.
Verified in binary. The harness should pick the field by the same test slot 27 uses.

## Measured live: an authored START opens the pause menu

On 2026-09-26, with an Xbox controller connected (the XInput arm), a `block 600` confirmed in the
log, then `buttons 0x0010 12`: `scripts/frida/pad-button-read.js`, hooked on `0x140f04d40`, logged
key id `0x2c` on the stamped device as

```
device=0xe53db8 third=-1 port=0 buttons=0x0 down=0
device=0xe53db8 third=-1 port=0 buttons=0x10 down=1
device=0xe53db8 third=-1 port=0 buttons=0x0 down=0
```

and `ds2-menu-row` built the pause menu for the first time in that session straight after the
hold. So the reading above holds end to end: slot 27 reads the harness's word, answers START down,
and the menu opens. The earlier failure below was in a session whose controller state was not
recorded.

## Why the earlier live START press did nothing: open

Since the field is read, the failed press is explained somewhere downstream of slot 27, and this
static pass did not settle where. Candidates, all inferred:

- The pad was not on the XInput arm in that session (`+0x314 >= 0`), so the stamp went to a field
  slot 27 was not reading.
- The pause-menu action is taken from a mapper or device the game treats as inactive, for example
  because keyboard and mouse were the last devices used, so the pad's converter output is not
  what the menu reads.
- The pause action is bound to a key id other than `0x2c` in the pad's binding table (the bindings
  come from `KeyConfigParam.param`, which this pass did not read).

The decisive measurement is a Frida hook on `0x140f04d40` while `buttons 0x0010 12` is held: log
`this`, the key id in `edx`, `[this+0x314]`, `[this+0x19c]`, and the return. If key id `0x2c` is
queried on the stamped device and returns 1, the problem is past the converter; if `0x2c` is never
queried, the binding is elsewhere; if the device's `+0x314` is non-negative, the arm is wrong.

## The scanner blind spot

`python3 scripts/ds2-field-xrefs.py 0x198 --width 2` lists the constructor's zeroing store and the
poll's store, and no read. Its opcode table holds `mov`, `movzx` and `movsx` forms only, so
`test m16,r16` (`66 85`), `or m16,r16` (`66 09`) and `and m16,r16` (`66 21`) are invisible to it.
Ghidra's `scan_x86_displacements` over `0x140ee0000..0x140f20000` finds all of them. Verified by
running both.
