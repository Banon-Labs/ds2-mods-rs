# The per-frame counter a hang watchdog would watch

Elden Ring's `hang.rs` (in `er-crash-logging-core`) rests on one dword: the one `MainUpdate`
increments once per main-loop tick, at a fixed RVA in `eldenring.exe`. This is DARK SOULS II's
equivalent, found statically. Nothing here has been read from a running game yet.

Every claim is labelled:

- **verified-in-binary**: read directly from the bytes or the disassembly, in the Ghidra database
  of `DarkSoulsII.exe` (image base `0x140000000`), and where it matters cross-checked against
  `darksoulsii-deobf.bin`.
- **inferred**: follows from the code but still needs a runtime read to confirm.

## What `hang.rs` needs

| detector | Elden Ring input | DS2 input |
| --- | --- | --- |
| stall | a dword that advances once per frame | `GameManagerImp+0x104`, below |
| frame drop | the same dword, sampled every 50 ms against a calibrated baseline fps | the same field; the engine also keeps its own fps and frame-time fields, below |
| loading screen | `CS::LoadingScreenData` target and elapsed clock | not found here; see the end |
| host check | `GAME_MODULE_NAME = "eldenring.exe"` | `DarkSoulsII.exe` |

## The main loop, top to bottom

All verified-in-binary unless marked.

| VA | RVA | what it is |
| --- | --- | --- |
| `0x1402eb630` | `0x002eb630` | WinMain-level setup and message pump. It constructs the application object on its own stack, then loops: one message-queue poll; when the queue is empty it calls `0x140aeeed0` once, otherwise it translates and dispatches the message. The import names on those four calls are inferred from their argument shapes; the IAT slots were not resolved to names. |
| `0x140aeeed0` | `0x00aeeed0` | One frame. Calls `0x140af09b0`, then checks the quit byte at `app+0x13a` (the same byte the title menu's quit row writes). |
| `0x140af09b0` | `0x00af09b0` | Frame body, guarded against re-entry by `app+0x138`. Calls the frame timer `0x140b4a380` on `app+0xd0`, then the update `0x140af0090` with the frame time in `xmm1`, then the draw `0x140aedb60`. Exactly one update per call; there is no catch-up loop. |
| `0x140af0090` | `0x00af0090` | Engine update. Among others it calls `INPUT_UPDATE` (`0x140af42b0`) and the app's virtual at `+0xb0`, which is `0x1402ef1f0`. |
| `0x1402ef1f0` | `0x002ef1f0` | Game-app update. When `app+0x338` is set it calls `0x140b31530`, which calls `GameManager`'s virtual at `+0x10`. |
| `0x1401c32d0` | `0x001c32d0` | That virtual: `GameManagerImp`'s per-frame update. It runs the game-state handler table indexed by `GameManagerImp+0x24ac`, and the state handler at body `0x1401bf84a` is the one that drives `mapManUpdate`, `damageManUpdate`, `bulletManUpdate`, `demoManager`, `saveRequest` and the frontend (already described in `DS2-INGAME-MENU.md`). |

The application object's pointer is `FE_SYSTEM_SINGLETON` in `ds2-rva` (RVA `0x016751f8`). The
base constructor at `0x140aed610` stores `this` there (`0x140aed705`), and the destructor at
`0x140aed7c0` clears it. That constructor also builds the frame timer at `this+0xd0`. So the name
`FeSystem` in `ds2-rva` undersells it: it is the main application object, the one whose frame
function is `0x140aeeed0`. Verified-in-binary.

## The counter: `GameManagerImp+0x104`

```text
0x141cf2d05:  ff 81 04 01 00 00    inc dword ptr [rcx+0x104]     ; rcx = GameManagerImp
```

- **The increment**, verified-in-binary. It follows directly on the register saves, the frame setup and
  the load of the frame time in `GameManagerImp`'s per-frame update, before the state-handler loop
  and with no branch ahead of it, so every call to the update advances it by exactly one. The same bytes are at the same
  place in `darksoulsii-deobf.bin`.
- **It is the only write**, verified for the ranges scanned: a displacement scan for writes to
  `+0x104` over the whole Arxan section and over `0x1401b0000`-`0x1401d0000` (the
  `GameManagerImp` update code) finds this instruction and nothing else. A scan of all of `.text`
  was not done.
- **It is used as a frame count by the game itself**, verified-in-binary: the state handler reads
  it at `0x1401bfd47` and does work when it is a multiple of 15.
- **Reaching it**: `[[exe_base + 0x016148f0]] + 0x104`, where `0x016148f0` is `GAME_MANAGER_IMP`
  in `ds2-rva`. That is a pointer, then a dword; Elden Ring's counter is a static dword. Inferred:
  the pointer stays the same object for the life of the process.
- **Once per frame**, inferred. The chain above calls the update once per pump iteration, but
  whether `GameManager`'s slot is reached on every frame depends on `app+0x338` and
  `GameManager+0x8` being set, which is runtime state.

### It sits in Arxan's code

The update's entry at `0x1401c32d0` starts `e9 61 29 9c 01`: a redirected entry, one of the
functions `ARXAN-FOOTPRINT.md` describes. Arxan took its prologue, and the `inc` above is part of
that prologue, so the instruction lives in Arxan's own `.text` at `0x141cf2d05`, not in the game's
`.text`. The fragment ends in a `jmp` back to `0x1401c3303`, inside the original function
(`.pdata` range `0x1401c32d0`-`0x1401c3392`). All verified-in-binary. Two hops from the entry were
read (`0x141b85c36`, a stack-swap thunk to `0x141cf2cc5`); the rest of the chain from there to
`0x141cf2ce7` was not walked, and Ghidra's resolution of the virtual calls at `0x140b31555` and
`0x14030cdb0` straight to `0x141cf2ce7` stands in for it.

This does not matter for a watchdog, which only reads the field and never hooks the code. It does
mean nobody should detour the increment site, and that the runtime check below must happen with
dearxan's patches in place, the way the game actually runs under the loader. Inferred: dearxan
patches stub checks and decrypts regions but leaves relocated prologues running, so the
increment still executes.

## Frame timer fields, for the frame-drop detector

The frame timer at `app+0xd0` (constructor `0x140b4a320`, per-frame `0x140b4a380`) keeps its own
measurements. The field layout is verified-in-binary; the meanings are inferred from the
arithmetic and not yet read live:

| offset from `app+0xd0` | meaning |
| --- | --- |
| `+0x18` | counter value at the previous frame |
| `+0x20` | counter frequency, the divisor for every duration |
| `+0x30` | last frame time in seconds, as measured |
| `+0x34` | frame time handed to the update, clamped |
| `+0x38` | frames in the current one-second window, reset to zero when the window closes |
| `+0x48` | frames per second over the last closed window |

`+0x38` also advances once per frame, but it resets every second, so it cannot stand in for a
monotonic counter. `+0x48` is a ready-made baseline for the frame-drop detector's calibration.

## What a runtime confirmation needs

Nothing below has been run. A Frida agent through `scripts/ds2-frida-watch.py` would do it without
a DLL:

1. Read `[[base+0x016148f0]]+0x104` twice about a second apart at the title menu, in a world, and
   during a loading screen. It must advance by roughly the frame rate each time, and it must keep
   advancing through the load: that is what makes it a stall signal rather than a loading signal.
2. Attach an `Interceptor` to `0x140aeeed0` and count its calls over the same window as the two
   reads. The counter delta must equal the call count. Anything else means the update is skipped
   or repeated on some frames.
3. Record the `GameManagerImp` pointer at the title menu, in a world, and after a quit to title.
   If it changes, the watchdog must re-resolve it every sample, not once at arm time.
4. Read `app+0xd0+0x48` and compare it with the rate measured in step 1.
5. Note how long after launch the pointer first becomes non-null. `hang.rs` waits 5 s and then
   gives up arming after 15 s more without an advance; DS2's boot may be longer than that.

## Not found here

- **A loading-screen oracle.** The candidate object is `FeOperatorNowLoading`, reachable as
  `[[GAME_MANAGER_IMP]+0x22e0]+0xc8` (already in `ds2-rva`). Its fields were not examined, and
  `CS::LoadingScreenData`'s design does not carry over.
- **A static-address counter.** No `inc dword ptr [rip+x]` on the main-loop path turned up; the
  path was read function by function, not scanned exhaustively, so one may still exist elsewhere.
- **The `Present` call path.** Not needed for the counter, and not traced.
