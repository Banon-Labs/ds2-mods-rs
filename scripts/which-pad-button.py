#!/usr/bin/env python3
"""Name the controller button someone just pressed, in the spelling `ds2-mods.toml` accepts.

    python3 scripts/which-pad-button.py                 # first gamepad found, waits for one press
    python3 scripts/which-pad-button.py --device /dev/input/event22
    python3 scripts/which-pad-button.py --presses 3     # read three, for "press it a few times"

WHY THIS EXISTS. A hotkey config needs a button NAME, and the person who knows which button they
mean knows it as a place their thumb goes, not as a word. Asking them to name it invites the two
classic wrong answers -- the Xbox name for a PlayStation pad, or "the top one" -- and either sends
the wrong string into a config file where it fails silently as an unparsed value. Reading the
device says what was actually pressed.

It reads evdev directly, so it does NOT need the game running, does not need Steam, and cannot be
confused by whatever has focus. `/dev/input/event*` is usually readable without special rights on
this setup; if it is not, the fix is group `input`, not `sudo`.

THE NAMES IT PRINTS are `ds2-inventory-sort`'s, which are XInput's, which are Xbox's -- because
XInput is an Xbox API and its own constants are spelled that way. On a DualShock, `y` is Triangle
and `b` is Circle. The kernel's own `BTN_NORTH`/`BTN_WEST` aliases are deliberately NOT used here:
they disagree with the physical face letters on an Xbox pad, and `xpad` reports the letter codes.
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

#: `struct input_event` on 64-bit Linux: `timeval` (16) + type (2) + code (2) + value (4).
EVENT_FORMAT = "llHHi"
EVENT_SIZE = struct.calcsize(EVENT_FORMAT)

EV_KEY = 0x01
EV_ABS = 0x03

#: evdev button code -> the name the config file takes. `xpad` reports the LETTER aliases
#: (`BTN_A` 0x130 .. `BTN_Y` 0x134), which match the letters printed on the pad.
BUTTONS = {
    0x130: "a",
    0x131: "b",
    0x133: "x",
    0x134: "y",
    0x136: "lb",
    0x137: "rb",
    0x13A: "back",
    0x13B: "start",
    0x13C: "guide (NOT bindable -- Steam takes it)",
    0x13D: "lthumb",
    0x13E: "rthumb",
}

#: The d-pad arrives as a hat AXIS, not as buttons, so it needs its own decoding.
ABS_HAT0X = 0x10
ABS_HAT0Y = 0x11
HATS = {
    (ABS_HAT0X, -1): "dpad_left",
    (ABS_HAT0X, 1): "dpad_right",
    (ABS_HAT0Y, -1): "dpad_up",
    (ABS_HAT0Y, 1): "dpad_down",
}


def device_name(path: Path) -> str:
    """The kernel's own name for the device, or a placeholder."""
    sysfs = Path("/sys/class/input") / path.name / "device" / "name"
    try:
        return sysfs.read_text().strip()
    except OSError:
        return "unknown device"


def find_gamepad() -> Path | None:
    """First `/dev/input/event*` whose name looks like a pad. Order is the kernel's, not a guess."""
    wanted = ("controller", "gamepad", "xbox", "dualsense", "dualshock", "wireless controller")
    for event in sorted(Path("/dev/input").glob("event*"), key=lambda p: int(p.name[5:])):
        name = device_name(event).lower()
        # "Chatpad" is a KEYBOARD that ships attached to an Xbox pad and matches "xbox" happily.
        if "chatpad" in name:
            continue
        if any(word in name for word in wanted):
            return event
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--device", help="evdev node, e.g. /dev/input/event22. Default: autodetect.")
    parser.add_argument(
        "--presses",
        type=int,
        default=1,
        metavar="N",
        help="how many presses to report before exiting (default 1).",
    )
    args = parser.parse_args()

    path = Path(args.device) if args.device else find_gamepad()
    if path is None:
        print("no gamepad found under /dev/input -- is it plugged in?", file=sys.stderr)
        return 1
    if not path.exists():
        print(f"{path} does not exist", file=sys.stderr)
        return 1

    print(f"reading {path}  ({device_name(path)})", flush=True)
    print(f"press the button {args.presses} time(s)...", flush=True)

    seen = 0
    try:
        with path.open("rb") as device:
            while seen < args.presses:
                data = device.read(EVENT_SIZE)
                if len(data) < EVENT_SIZE:
                    break
                _sec, _usec, kind, code, value = struct.unpack(EVENT_FORMAT, data)
                name = None
                if kind == EV_KEY and value == 1:
                    # Only the PRESS edge. A release is value 0 and a repeat is 2; reporting either
                    # would name the same button twice for one push.
                    name = BUTTONS.get(code, f"unmapped evdev code {code:#x}")
                elif kind == EV_ABS and code in (ABS_HAT0X, ABS_HAT0Y) and value != 0:
                    name = HATS.get((code, 1 if value > 0 else -1))
                if name is None:
                    continue
                seen += 1
                print(f"  pressed: {name}", flush=True)
    except PermissionError:
        print(f"cannot read {path} -- add this user to the 'input' group", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
