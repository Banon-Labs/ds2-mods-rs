#!/usr/bin/env python3
"""Read the game's own IDXGISwapChain out of a running DarkSoulsII.exe, read-only.

Walks the chain `crates/ds2-rva` records (`GRAPHICS_DEVICE_PTR` -> `+GRAPHICS_DEVICE_SWAP_CHAIN`
-> `+SWAP_CHAIN_HOLDER_CHAIN`) through /proc/<pid>/mem and prints each pointer with the module it
lands in, then the swap chain's vtable and the first bytes of its `Present` slot. Nothing is
attached and nothing is written, so it is safe against a live session.

    python3 scripts/ds2-swapchain-read.py <pid>
"""

import struct
import sys

GRAPHICS_DEVICE_PTR = 0x1416751F0
GRAPHICS_DEVICE_SWAP_CHAIN = 0xD20
SWAP_CHAIN_HOLDER_CHAIN = 0x0
PRESENT_VTABLE_SLOT = 8


def main() -> int:
    pid = int(sys.argv[1])
    maps = []
    with open(f"/proc/{pid}/maps") as handle:
        for line in handle:
            parts = line.split()
            lo, hi = (int(x, 16) for x in parts[0].split("-"))
            name = parts[5].split("/")[-1] if len(parts) > 5 else "[anon]"
            maps.append((lo, hi, name))

    def owner(address: int) -> str:
        for lo, hi, name in maps:
            if lo <= address < hi:
                return f"{name}+{address - lo:#x}"
        return "unmapped"

    with open(f"/proc/{pid}/mem", "rb") as mem:

        def qword(address: int) -> int:
            mem.seek(address)
            return struct.unpack("<Q", mem.read(8))[0]

        device = qword(GRAPHICS_DEVICE_PTR)
        print(f"graphics device {device:#x} {owner(device)}")
        holder = qword(device + GRAPHICS_DEVICE_SWAP_CHAIN)
        print(f"swap chain holder {holder:#x} {owner(holder)}")
        chain = qword(holder + SWAP_CHAIN_HOLDER_CHAIN)
        print(f"IDXGISwapChain {chain:#x} {owner(chain)}")
        vtable = qword(chain)
        print(f"vtable {vtable:#x} {owner(vtable)}")
        for slot in range(PRESENT_VTABLE_SLOT + 3):
            entry = qword(vtable + 8 * slot)
            print(f"  slot {slot:2} {entry:#x} {owner(entry)}")
        present = qword(vtable + 8 * PRESENT_VTABLE_SLOT)
        mem.seek(present)
        print(f"Present prologue {mem.read(16).hex(' ')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
