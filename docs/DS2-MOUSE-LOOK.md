# Where DARK SOULS II's mouse-look actually comes from

Read out of `darksoulsii-deobf.bin` (Ghidra on port 8766, `objdump` over the flat image) with no
game launched. Every claim is tagged **verified in binary** (with the addresses it was read from)
or **inferred**. Runtime measurements quoted from elsewhere are marked as such.

## Short answer

- `WindowsMouseDevice+0x08` is the **menu pointer**, not the camera. Writing it moves the GUI
  cursor and nothing else. That is why authored `mouse-x`/`mouse-y` turned the camera 0.00 degrees
  (runtime measurement), focused or not, pad or no pad.
- The camera's mouse input is the **DirectInput** `DLUID::MouseDevice` relative delta, run through
  a DLUI analog mapping, copied to `cursorObj+0x18` by the function Ghidra calls
  `parseCameraInput`, and read by the follow camera's input stage `FUN_14049cbf0`.
- There is **no pad-versus-mouse arbitration flag**. Each frame the mouse, when it moved past a
  threshold, replaces the stick for that frame; otherwise the stick is used. A connected pad does
  not switch the mouse off.
- The mouse branch is gated on `BaseP1+0x30` (window active) and `BaseP1+0x133` (a menu-scoped
  "keyboard/mouse enabled" byte). Neither is set by pad activity.

## The objects

```text
BaseP1                        = [0x1416751f8]          KatanaMainApp
inputObj                      = [BaseP1 + 0x60]
cursorObj                     = [inputObj + 0x08]      0x110-byte object, ctor FUN_140b0c5d0
cursorObj+0xd0 .. +0xd8       vector of pointer devices   -> holds the WindowsMouseDevice
cursorObj+0xf0                DLUID::MouseDevice          (DirectInput, relative axes)
cursorObj+0x18 / +0x1c        mouse-look delta, f32 x / f32 y   <- what the camera reads
cursorObj+0x00 / +0x08        pointer position now / previous   <- menu pointer only
```

- **verified in binary**: `FUN_140af3f60` allocates `0x110` bytes, builds it with `FUN_140b0c5d0`,
  and stores it at `inputObj+0x08`.
- **verified in binary**: `FUN_140b0c5d0` constructs a `0x30`-byte object with `FUN_140b5c130`,
  whose first store is the vtable `0x1411dcc38` (WindowsMouseDevice, `0x140b5c133`), and pushes
  it onto the vector at `+0xd0`. Its `+0x20` is the HWND (`0x140b5c14c`).
- **verified in binary**: the same constructor stores `DLUserInputManagerImpl` vtable slot `+0x18`
  called with `2` at `cursorObj+0xf0`. That slot is `0x140ef5dc0`; for type `2` it allocates
  `0x118` bytes and runs `FUN_140f06fb0`, whose vtable store at `0x140f06ff5` is `0x141272158`,
  `DLUID::MouseDevice`. It sets the DirectInput axis mode to relative (`DIPROP_AXISMODE`, data
  `1`) through the device's `SetProperty`.
- **verified in binary**: the constructor then builds a mapping at `cursorObj+0xf8`
  (`FUN_140ef8be0`), calls `FUN_140ef9840(map, 0, 0)` and `FUN_140ef9840(map, 1, 1)`, binds it with
  `FUN_140efafc0`, and attaches it to the MouseDevice through its vtable `+0x60`.
  **inferred**: those calls map mouse axis 0 to virtual analog 0 and axis 1 to virtual analog 1.

## Question (a): which device does each read?

### The pull at `0x140b0d0e0` reads the WindowsMouseDevice -- for the menu pointer

- **verified in binary**: `parseCameraInput` (`0x140b0c950`) walks `cursorObj+0xd0..+0xd8`, calls
  each device's slot 1 (the poll, `0x140b5c1f0`) and remembers the first whose slot 3 returns
  true. WindowsMouseDevice's slot 3 is `0x140b5c1d0`, `mov al,1; ret` -- always true. The only
  device ever pushed there is the WindowsMouseDevice built above.
- **verified in binary**: `FUN_140b0d0e0` copies that device's `+0x08` into `cursorObj+0x00`, its
  buttons into `cursorObj+0x30/+0x34`, its wheel into `cursorObj+0x20`.
- **verified in binary**: what then consumes `cursorObj+0x00..` is pointer logic. `FUN_140b0cfc0`
  compares the position against the press point using `GetSystemMetrics(SM_CXDRAG)` and
  `SM_CYDRAG` (click versus drag); `parseCameraInput` keeps per-button repeat counters and a
  `QueryPerformanceCounter` "last moved" stamp at `cursorObj+0x28`. Readers found by scanning for
  `[[BaseP1+0x60]+8]`: `AppGUISystem`'s `FUN_140b4ad00` (`0x140b4ad1d`), `FeSubStateTitleLogo`'s
  `FUN_1400febf0`, `FUN_14010b8f0`, `FUN_1400ff420` -- menus and title screen.
- **inferred**: the Ghidra name `parseCameraInput` is a guess someone typed. The function is the
  pointer update; the camera takes only the floats at `cursorObj+0x18/+0x1c` out of it (below).

### The camera reads `cursorObj+0x18`, which comes from the DirectInput MouseDevice

- **verified in binary**, `parseCameraInput` at `0x140b0c9b3..0x140b0c9ee`, before the pointer
  loop:

  ```text
  dev = cursorObj->[+0xf0]                      DLUID::MouseDevice
  if dev->slot2() == 0:                          0x140f072c0: 0 when the IDirectInputDevice8 at
                                                 +0xe8 is non-null, -1 when it is null
      dev->slot7(dt)                             0x140f16cc0, DLUserInputDeviceImpl::Update
      cursorObj+0x18 = ((f32*)dev[+0x20])[0]     0x140b0c9eb
      cursorObj+0x1c = ((f32*)dev[+0x20])[1]     0x140b0c9ee
  ```

- **verified in binary**: `Update` (`0x140f16cc0`) clears the virtual input buffer
  (`FUN_140f1b990` at `0x140f16d7e`), then calls vtable `+0xb8` -- slot 23, the poll
  `0x140f074a0` -- at `0x140f16d89`, then runs the attached mapping contexts
  (`FUN_140f1c670` at `0x140f16e1e`). `dev+0x20` is that buffer's float array (set up by the base
  constructor `FUN_140f1ba10` at `dev+0x10`).
- **verified in binary**: the poll writes the relative `DIMOUSESTATE2` deltas as floats to
  `+0x108`/`+0x10c`/`+0x110`; slot 24 (`0x140f071c0`) is the device's axis getter and reads
  `+0x108 + 4*index`.
- **inferred**: the mapping reads slot 24, so a float written to `+0x108`/`+0x10c` after the poll
  returns is what lands in `cursorObj+0x18`/`+0x1c`. The order poll-then-map is verified; the
  mapping's internal read of slot 24 is not traced instruction by instruction.
- **verified in binary**: when `slot2` is non-zero (device pointer nulled), `cursorObj+0x18` is
  **not cleared** -- it keeps its last value. The poll nulls `+0xe8` when `GetDeviceState` fails
  with anything but a non-negative result (the `DIERR_INPUTLOST` arm re-acquires and still falls
  through to the null store).

### The camera input stage

`FUN_14049cbf0` is called from the follow camera and fills the per-frame camera input record at
`param_2`.

- **verified in binary**, stick: `block = FUN_140af3f30(inputObj)` (`0x14049cca7`); if
  `cam+0x822`, `out[8..9] = (block+0x12c, block+0x128)` with per-axis inversion sign from
  `CameraManager+0x3b4/+0x3b5`.
- **verified in binary**, mouse (`0x14049cd15..`): runs only if `cam+0x823`, `BaseP1+0x30` and
  `BaseP1+0x133` are all non-zero. It loads `*(u64*)(cursorObj+0x18)` at `0x14049cd55`. If
  `|x|` or `|y|` exceeds `0.1` (`[0x1410ac9f4]`) it scales by `0.017453` (`[0x1410ac9f0]`,
  degrees to radians), low-pass filters into `cam+0x840/+0x844` with factor `0.5`, and if the
  filtered value exceeds `0.01` (`[0x1410acb08]`) it writes the turn to `out[0xc..0xd]`, scaled by
  `(sensitivity_byte * 0.01 * 1.9 + 0.1) * 0.36` (the byte is `GameDataManager+0xc8 -> +0x14`),
  sets `out+9 = 1` (`0x14049cebd`) and **zeroes `out[8..0xb]`, the stick** (`0x14049cec1`).
- **verified in binary**: the camera constructor stores `0x01010101` to `cam+0x820..+0x823`
  (`0x14049a1c7`), and no other store to `+0x822`/`+0x823` was found in the image.
  **inferred**: the stick and mouse enables are always on.

## Question (b): arbitration

- **verified in binary**: there is no stored "last used device". The only coupling is per frame
  inside `FUN_14049cbf0`: mouse motion above threshold overwrites the stick with zero for that
  frame. A pad at rest does not suppress the mouse; a moving mouse suppresses the pad.
- **verified in binary**: `BaseP1+0x30` is written by `FUN_140aeef30` at `0x140aeef45` as
  `LOWORD(wParam) != 0`, reached from the window procedure `FUN_140af0380` for `WM_ACTIVATE` and
  `WM_MOUSEACTIVATE`. It is the window-active flag.
- **verified in binary**: `BaseP1+0x30` gates the stick too. `FUN_140af3f30` asks for user `0`
  when it is set and user `5` (invalid, which returns the empty block at `inputObj[0]+0x20`) when
  it is clear. So focus alone cannot explain "pad turns, mouse does not".
  **inferred**: the runtime note that the pad turned the camera with the window unfocused means
  `BaseP1+0x30` was still `1` then -- the game had not received `WM_ACTIVATE(WA_INACTIVE)`.
- **verified in binary**: `BaseP1+0x133` is cleared by `FUN_14002a2b0` (`0x14002a3f4`) and set by
  `FUN_14002a420` (`0x14002a5e6`) for menu entries whose table byte `+8` is set
  (`0x14155d760 + 0xc*index`), and toggled by the debug command `0x10009` in `FUN_140b4b4d0`.
  It also gates the keyboard bindings in `FUN_140b09170` (`0x140b09829`). **inferred**: it is a
  "keyboard/mouse input enabled" flag that some menus switch off, not a device-arbitration flag.

## What the input layer re-centres, and what it does not

- **verified in binary**: `SetCursorPos` (`0x140af4525`) fires only when `inputObj+0x680` bit 0 is
  set and `inputObj+0x684` is zero, and clears `+0x680` afterwards (`0x140af452b`). The only
  writer of bit 0 is `FUN_140af4230`, whose only call site is `FUN_1400533d0` (`0x1400534af`), the
  `FeSceneCommonCursor` show/hide path, which requests the window centre when the menu cursor is
  hidden. There is no per-frame re-centre.
- **verified in binary**: `FUN_140af4230` also writes the requested point into each input block's
  previous-position field (`FUN_140b08540`: `+0x22c` and `+0x4b4`) so that re-centring does not
  register as motion.
- **inferred**: an absolute cursor that is clamped to the client rect and never re-centred during
  play cannot drive an unbounded turn; that is consistent with the camera using DirectInput
  relative deltas and not the Win32 cursor.

## Why the WindowsMouseDevice write never turned the camera

- **verified in binary**: nothing in `FUN_14049cbf0` reads `cursorObj+0x00..+0x0c`; it reads
  `cursorObj+0x18` only, and that is fed from the DirectInput device, not from the
  WindowsMouseDevice.
- Therefore the harness's virtual cursor at `WindowsMouseDevice+0x08` moves the menu pointer and
  nothing else. It also cannot be what stopped the player's real mouse from turning the camera
  after the virtual cursor engaged (runtime report): that write does not reach the camera path.
  **inferred** candidates for that report, each checkable by reading memory: the DirectInput
  device pointer at `MouseDevice+0xe8` went null (then `cursorObj+0x18` freezes at its last value),
  or `BaseP1+0x133` was left at `0` by a menu.
- **verified in binary**: left-click attacking proves nothing about the DirectInput mouse. The
  input blocks take button state from window messages (`FUN_140b08ef0`, event types 0..8 into
  `block+0x228`), dispatched from `KatanaMainApp`'s handler `FUN_1402ef110`.

## What the harness has to write, and when

To turn the camera with `mouse-x`/`mouse-y`, pad connected or not:

1. Write **`DLUID::MouseDevice+0x108` (x) and `+0x10c` (y)** as `f32` relative counts, **after the
   original poll `0x140f074a0` returns** -- i.e. in the existing `dinput-mouse` detour. For the
   player's own motion, leave the poll's values in place and add the authored delta on top; to
   block, zero them (as today). **verified in binary** that this is before the mapping runs.
2. It only reaches the camera while `MouseDevice+0xe8` is non-null (else `parseCameraInput` skips
   `Update` and the detour is not even reached through this path), `BaseP1+0x30 != 0`, and
   `BaseP1+0x133 != 0`. A pad does not need to be switched off.
3. Per-frame magnitude must clear `0.1` in virtual-analog units (**inferred**: roughly mouse counts
   per frame), then the filter at factor `0.5` and the `0.01` floor.
4. The WindowsMouseDevice write should stop publishing for the camera. Its "engaged virtual
   cursor" only drags the menu pointer outside the client rect.

Alternative, further downstream: write `cursorObj+0x18/+0x1c` after `parseCameraInput`
returns and before the camera update. It bypasses the device-pointer condition but skips the DLUI
mapping, so it is not a device write in the sense `ds2-input-harness` wants.

No pad-state change is needed. There is no active-device state to switch.

**Not yet runtime-proven.** An earlier live measurement wrote these same `+0x108` floats and saw
the published yaw stay put. Static reading does not explain that result. The gates in step 2 are
the candidates, and each is a single memory read on the next run: `BaseP1+0x30`,
`MouseDevice+0xe8`, `BaseP1+0x133`, and `cursorObj+0x18` itself.
