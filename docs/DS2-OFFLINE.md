# Playing DARK SOULS II offline

Everything below was read out of `darksoulsii-deobf.bin` with `scripts/ds2-rtti.py`,
`scripts/ds2-disasm.py`, `scripts/ds2-xrefs.py` and `scripts/ds2-arxan-chain.py`. No game was
launched to establish any of it. Where a runtime measurement exists it is marked as one.

## Why this exists

Every other crate in this workspace patches `.text` in a running copy of DARK SOULS II. That is
what FromSoftware's matchmaking servers watch for, and a modded client that logs in is a client
that can be soft-banned. Offline is the prerequisite for the rest of the repo rather than a
feature beside it, which is why it is the only feature here that defaults to on for a reason that
is not convenience.

It is also the only setting under which a boot measurement cannot be interrupted by an invasion.

## The object

```text
GameManagerImp   = [0x1416148f0]
netService       = [GameManagerImp + 0x22f0]
netService->online : u8 at +0x3a
```

`[GameManagerImp + 0x22f0]` was already established from the other end, in
[`DS2-BOOT-WORK.md`](DS2-BOOT-WORK.md): the four network substates (`0x20` SteamNetworkCheck,
`0x4e` OnlineCheck, `0x39` GameServerLogin, `0x44` Information) all reach their work through it,
while the storage chain uses `[GameManagerImp + 0xb8]`.

| what | address | body |
| --- | --- | --- |
| getter | `0x140513600` | `movzx eax, BYTE PTR [rcx+0x3a]; ret` |
| setter | `0x140513820` | `mov BYTE PTR [rcx+0x3a], dl; ret` |
| constructor | `0x140512f30` | writes `mov BYTE PTR [rbx+0x3a], 0` at `0x140512f5a` |

Neither the getter nor the setter is an Arxan redirect; `scripts/ds2-arxan-chain.py` terminates at
hop 0 on each one's own prologue.

### The flag is born zero, and that is the whole design

The constructor is identified by the vtable `0x1410d13e8` it installs at `[this]`, and four
instructions in it zeroes `+0x3a`. **Offline is the state this object is constructed in.** Every
online run is one that left it, through the setter.

So `ds2-offline`'s primary patch replaces the *setter* with `ret`. That is a materially different
claim from forging the getter: it does not impose a value on the game, it prevents a departure
from the game's own initial one, and any reader that never goes through the getter still sees a
value the game itself wrote.

## The getter is the master online gate, and it has 34 readers

`scripts/ds2-xrefs.py 0x140513600` finds 34 `e8` displacements that resolve there. Every one is
followed by `test al,al` and a branch. Three were disassembled to check the polarity:

* **`FeSubStateTitleOnlineCheck::v8`** (`0x1400f98c0`) -- the substate's own work starter. Calls
  the getter and, on a zero, returns `false` **without starting anything**. Forcing the getter to
  zero does not fake the online check; it takes the shipped path where the check never runs.
* **The top-menu builder** at `0x1400f433b`. The result becomes `r14b`, which enables row 2
  (server information) and disables row 3 (go online), or the reverse. See
  [`DS2-TITLE-FLOW.md`](DS2-TITLE-FLOW.md) for the six-row menu this feeds.
* **`0x1400fe739`**, inside `FeSubStateTitleTopMenu::v5`, gating whether the row-3 transition is
  registered at all.

## `0x14160de19` is NOT the switch -- `ds2-mods-rs-rk4`, answered

`DS2-BOOT-WORK.md` recorded a byte the game reads to force the online flag to zero and asked
whether setting it removes the network boot chain. **It does not.** It is read at exactly one
instruction in the whole image, `0x1400f431f`:

```text
cmp BYTE PTR [0x14160de19], 0
je  ask_the_gate          ; zero -> fall through and call 0x140513600
xor r14b, r14b            ; non-zero -> force "not online" here and skip the call
jmp merge
```

It is a local override of one boolean in one function -- the top-menu builder -- and the boot
chain, which calls `0x140513600` directly on its own, never sees it. `0x140513600` is the read it
was shadowing, and patching that is the superset of setting this byte.

## The half that a flag patch does not reach

`FeSubStateTitleGameServerLogin::v8` (`0x1400f9820`, vtable slot 8) is the login work starter, and
it **never reads the online flag**:

```text
rcx = [GameManagerImp + 0x22f0]
call 0x1405132a0            ; -> [0x141616cf8] + 0x30, a NetSvrManager
call [that->vtable + 0x60]  ; NetSvrManager slot 12 (0x140290040)
call [that->vtable + 0x08]  ; NetSvrManager slot 1  (0x140290810)
test al,al
jne  skip                   ; -> mov eax,7; ret
...build the login job...   ; -> mov eax,7; ret
```

Slot 12 opens by calling the setter with `edx` zeroed, and
**`FeSubStateTitleSetOfflineMode::v1` (`0x1400f8f80`) is nothing but a tail-jump into that same
slot** -- which is how the setter was found in the first place, from the substate whose name says
what it does rather than by searching for a write to `+0x3a`.

The consequence is the one that shapes the crate: a build that only patched the flag would have
told the player they were offline while the login went out on the wire. That is why there is a
second layer, and why it is not decoration.

## The socket layer

`ds2-offline::winsock` fronts four of `DarkSoulsII.exe`'s own `WS2_32` imports -- `connect`,
`sendto`, `getaddrinfo`, `gethostbyname` -- and refuses anything that is not loopback.

**It patches the import table, not `ws2_32` itself.** Three consequences:

1. No code is modified, so Arxan's `.text` integrity checks have nothing to react to. Same
   argument `ds2-boot-timeline` makes for its `Sleep` counter.
2. Only this executable's calls are affected. `steamclient64.dll` and `GameOverlayRenderer64.dll`
   are in the process with their own import tables, so Steam's connection, the overlay and
   achievements are untouched. **This is not a firewall and must not be described as one.**
3. It is reversible by a pointer write, which is what makes `enabled = false` a real switch.

### The slots are found by asking `ws2_32`, not by trusting an ordinal table

DS2 imports 43 functions from `WS2_32.dll` and all but ten are imported **by ordinal** --
`connect` and `sendto` among them. Nothing in the crate parses hints or ordinals. It walks the
import descriptors for the `WS2_32.dll` entry, asks the already-loaded `ws2_32.dll` for each name
with `GetProcAddress`, and patches whichever IAT slot currently holds that pointer. The loader
filled those slots from the same export table `GetProcAddress` reads, so an equal pointer is an
identification.

Cross-checked against Proton Experimental's own
`files/lib/wine/x86_64-windows/ws2_32.dll`: it exports 135 of its 500 functions by name, and all
four wanted names are among them (`connect` = ordinal 4, `sendto` = 20, `gethostbyname` = 52,
`getaddrinfo` = 130). The game's ordinal imports at IAT `0x141aae624` and `0x141aae6ec` are
therefore `connect` and `sendto` -- but the code does not depend on that, because it matches by
address.

### The refusals use errors the game already handles

`WSAENETUNREACH` (10051) for the two send paths and `WSAHOST_NOT_FOUND` (11001) for the two
resolvers -- what a machine with no route produces. The game has shipped handling for exactly that
condition; it is what raises `FeSubStateTitleOnlineCheckFailWarn` and the "could not retrieve
information" box, both of which `ds2-dialog-skip` already answers. An invented error code, or a
silent success, would drive a path nobody has tested.

`send` and `recv` are deliberately left alone: they operate on a socket a refused `connect` never
handed over, and blanket-failing them would reach loopback traffic this crate has no business
touching. Loopback (`127.0.0.0/8`, `::1`) is allowed through for the same reason -- Proton, Wine
and the Steam API all use local sockets.

## Configuration

`<Game>/ds2-mods.toml`, written by `scripts/ds2-run.py` on every launch:

```toml
[offline]
enabled = true
pin_flag = true         # setOnline -> ret
report_offline = true   # isOnline  -> xor eax,eax; ret
block_sockets = true    # front the WS2_32 imports
```

| launcher flag | effect |
| --- | --- |
| *(default)* | all four true |
| `--no-offline` | all four false -- **plays online with a modded client, on purpose** |
| `--offline-no-socket-block` | flag patches on, socket guard off |

The third is the measurement arm. Because the login starter does not read the flag, it is the run
that says how much traffic the flag layer never reaches.

## What a real run says

Verbatim from `<Game>/ds2-loader.log`, build `fabe6fc0`, Proton Experimental 11.0-100,
2026-08-27:

```text
ds2-offline: config [offline] enabled=true pin_flag=true report_offline=true block_sockets=true
ds2-offline: set-online va=0x0000000140513820 wrote=c3 90 90 live=c3 90 90 landed=true
ds2-offline: is-online  va=0x0000000140513600 wrote=31 c0 c3 live=31 c0 c3 landed=true
ds2-offline: fronted import=WS2_32!connect       slot=0x0000000141aae624 original=0x00006fffff95b6b0
ds2-offline: fronted import=WS2_32!sendto        slot=0x0000000141aae6ec original=0x00006fffff9627e0
ds2-offline: fronted import=WS2_32!getaddrinfo   slot=0x0000000141aae64c original=0x00006fffff95e1a0
ds2-offline: fronted import=WS2_32!gethostbyname slot=0x0000000141aae5dc original=0x00006fffff95f1c0
ds2-offline: install pin_flag=true report_offline=true sockets=4/4 flag=<not-constructed-yet> found_ws2_32=true
ds2-offline: refused api=getaddrinfo host=frpg2-steam64-ope-login.fromsoftware-game.net error=11001 count=1
```

**The last line is the whole argument, measured.** Both flag patches were live and verified, the
flag itself read zero -- and the game still went looking for FromSoftware's DS2 login host. That is
`FeSubStateTitleGameServerLogin::v8` not reading the flag, exactly as the disassembly said, and it
is why a flag-only build would have been a mod that lies to the player.

`landed=` is a read-back, not the write's return value. `patch_3byte_stub` succeeding means the
expected byte was found and `VirtualProtect` allowed the write; it does not mean the stub is in
memory, because another mod can own the same address. Every other feature here fails visibly; this
one fails by telling the player they are offline when they are not, so it compares the bytes.

`flag=<not-constructed-yet>` at install is expected: `GameManagerImp` does not exist yet at the
post-Arxan callback. That read deliberately does not call the getter -- after `report_offline` the
getter is a lie by construction, so asking it would prove nothing.

### Cross-checked from outside the process

The same three facts, read out of `/proc/<pid>/mem` from Linux while the game sat at the title
menu. No debugger, no injection, nothing that could contaminate the run:

```text
0x140513600  live 31 c0 c3 3a c3     (xor eax,eax; ret -- the trailing bytes are unreachable)
0x140513820  live c3 90 90 c3 cc     (ret; the rest unreachable)
GameManagerImp = 0x7fffe7fa0260 -> netService = 0x7ffff03a8df0 -> +0x3a = 0
```

### The detach line is NOT yet observed

```text
ds2-offline: detach refused connect=N sendto=N resolve=N allowed-loopback=N
```

It is written from `DLL_PROCESS_DETACH`, which fires on an orderly `ExitProcess` and **not** on
`TerminateProcess`. The verification runs above were ended with a signal, so it never ran. Quitting
through the game's own QUIT GAME row is what would produce it, and until someone does that this
line is code that has compiled and never executed. It is the run-total that would say how much
traffic the socket layer catches over a whole session rather than just at boot.

## What this does not do

* **It does not touch Steam.** See above.
* **It does not remove the network boot substates.** `0x20`, `0x39` and `0x44` still run; they now
  fail early instead of waiting on a server. Removing them is `ds2-mods-rs-rk4`'s business.
  Whatever boot time that saves is a side effect, and this document does not claim a number for it
  -- `DS2-BOOT-WORK.md` measured the noise floor at +/-300 ms, so any such claim needs several
  runs before it is believed.
* **It does not stop a determined online path.** Four imports are fronted, not all 43. A code path
  that used `WSASendTo` or `WSAConnect` would go straight through -- neither is in this build's
  import table, which is why neither is fronted, but that is a fact about build 9527516 rather
  than a guarantee about the design.
