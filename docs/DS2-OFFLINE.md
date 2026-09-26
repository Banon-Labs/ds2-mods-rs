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

## `0x14160de19` is not the switch -- the network-chain removal question, answered

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

Re-checked with `scripts/ds2-xrefs.py 0x14160de19` and each candidate decoded by hand
(verified in binary). The script lists six code candidates, but only one of them resolves to this
byte once the instruction's own immediate is counted:

| site | instruction | real target |
| --- | --- | --- |
| `0x1400eff88` | `cmp byte [rip+..], 0` | `0x14160de18` |
| `0x1400effa5` | `mov byte [rip+..], 0` | `0x14160de18` |
| `0x1400f011c` | `mov byte [rip+..], 1` | `0x14160de18` |
| `0x1400f431f` | `cmp byte [rip+..], 0` | **`0x14160de19`** |
| `0x1400fd938` | `mov byte [rip+..], 1` | `0x14160de1a` (the boot-once flag) |
| `0x1400febe0` | `mov byte [rip+..], 0` | `0x14160de1a` |

So the byte has one reader and no RIP-relative writer, and nothing connects it to `netService+0x3a`:
the one site that reads it writes nothing, and the setter `0x140513820` takes its value from `dl`
at each caller. The two are alternatives inside the top-menu builder and nowhere else. A write
through a computed pointer is not excluded by this scan (inferred unlikely: the neighbours
`0x14160de18` and `0x14160de1a` are both written RIP-relative).

## The switch the boot chain does consult: system data `+0x136e`

The flag that takes the network work out of the pre-Continue path is not an online flag at all. It
is a byte in the system-data block, and the substate that reads it is `0x37 UserPolicy`, which sits
between the Steam check and the login.

```text
sys = [[[0x1416148f0] + 0xa8] + 0xd8]     ; the same block 0x05 SteamLoadSystemData fills
```

`FeSubStateTitleUserPolicy::v1` (enter, `0x1400f9040`, not an Arxan redirect per
`scripts/ds2-arxan-chain.py`), verified in binary:

```text
0x1400f9070  cmp byte [sys+0x136e], 0
0x1400f9077  je  0x1400f908b              ; 74 12
0x1400f9079  mov dword [this+0x10], 3     ; phase 3
             ret
0x1400f908b  ...                          ; zero: test [sys+0x136d] (policy accepted -> phase 4),
                                          ; else build the policy screen (phase 1)
```

and `FeSubStateTitleUserPolicy::v5` (`0x1400f9510`) publishes, verified in binary:

| phase | destination |
| --- | --- |
| 2 | `0x38` SaveSystemData |
| 3 | `0x2a` FeSubStateOfflineModeWindow |
| 4 | `0x39` GameServerLogin |

`0x2a` is the "playing offline" notice, and its one edge goes to `0x47` the top menu -- measured,
from the `ds2-dialog-skip` line quoted further down (`kind=42 cancel-dest=0x47 confirm-dest=0xffff`),
and already suppressed by that crate. So with `+0x136e` non-zero the boot runs

```text
0x05 -> 0x20 -> 0x37 -(phase 3)-> 0x2a -> 0x47
```

and `0x38`, `0x39` and `0x44` are never entered. This is the game's own route, not a forged one: it
is the same `0x2a -> 0x47` tail the login-failure prompt takes when it plays offline.

### Where `+0x136e` comes from and who changes it

Every disp32 reference to `+0x136e` in the image, from a raw byte search for `6e 13 00 00` with
each hit decoded (verified in binary; the three other raw hits are `call`/`jae` displacements):

| site | function | access |
| --- | --- | --- |
| `0x1400f9070` | UserPolicy enter | read -- the branch above |
| `0x1400ff347` | `FeSubStateTitleTopMenu::v3` (`0x1400ff300`) | write `0`, when the menu's phase is 4 |
| `0x14019bbb1` | `0x14019bb30`, a system-data reader | write, from byte 3 of a 16-byte header read off the stream |
| `0x14019be38` | `0x14019bd90`, the other system-data reader | write, from the stream |
| `0x14019c77e` | `0x14019c190`, the system-data writer | read, to serialise it |

The default-initialiser `0x14019b960` writes the dword at `+0x136c` as `0x00000001`, so a fresh
block has `+0x136e = 0` (verified in binary). Top-menu phase 4 is row 3, "go online" (see
[`DS2-TITLE-FLOW.md`](DS2-TITLE-FLOW.md)), so choosing to go online clears it.

The reading that fits, inferred: `+0x136e` is a persisted "start in offline mode" setting. It
reaches memory only from the save file, the game clears it when the player asks to go online, and
it is written back whenever system data is saved. No writer of a non-zero value was found other
than the two readers loading it from disk; a bulk copy into the block would not show in this scan.

### `0x20` is not network work

`FeSubStateTitleSteamNetworkCheck::v1` (`0x1400f8fb0`) decides its phase inside `enter` and its
update is the shared `FeSubStateBase::v3`, a bare `ret` (verified in binary). It asks two
questions -- `[[0x141616cf8]]->vtable[1]` and whether the `NetSvrManager` state word is `3`
(`0x1405135a0`) -- and publishes phase 1 to `0x37`, 2 to `0x24`, 3 to `0x29`. Nothing is started and
nothing is waited on, which matches its measured 9 ms dwell. It stays on the path under either
change below and costs one frame.

### The two ways to set it, and why one is better

1. **Data write** -- set `[sys+0x136e] = 1` after `0x05` has loaded system data and before `0x37`
   enters. It uses the game's own branch and patches no code. The catch (inferred from the writer
   at `0x14019c77e`): the next system-data save persists it, so an unmodded launch afterwards
   would also boot offline until the player picks "go online".
2. **Code patch** -- `0x1400f9077`: `74 12` to `90 90`. The `je` falls through, UserPolicy takes
   phase 3 every time, and nothing is written to the save. Two bytes, in a function that is not an
   Arxan redirect, gated like the other `ds2-offline` patches.

The code patch is the smaller and safer change. What it also does, verified in binary: it skips
the policy screen on a profile that has never accepted it (the `+0x136d` test comes after the
branch), and it skips `0x38 SaveSystemData`, which the measured profile never reached anyway. Both
are exactly what the game already does when the persisted byte is set.

One more thing it skips, inferred: `NetSvrManager` slot 12 (`0x140290040`) is reached on the boot path
from the login starter (and otherwise from `FeSubStateTitleSetOfflineMode`), and besides the setter
it calls `0x140291390` with a fresh state (the body is verified in binary; the caller list is not
exhaustive). On this route it never runs. The `NetSvrManager` has presumably not left its initial state at that point, but that
is not traced.

Nothing here is a measurement of the saving. Under the current `ds2-offline` build, `0x44` is
already off the path -- the refused login raises the `0x3e` prompt and its offline edge goes to
`0x2a`, not to `0x44` (measured, the dialog-skip lines below). What this change removes is the
login's start, its failure and that prompt; how long those take with the sockets refused has not
been measured, and the noise floor is +/-300 ms.

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

## The login-failure prompt, and the trap in it

With the login refused, the title flow raises a two-option box:

```text
The DARK SOULS II service is not available. Please try again later.
Please see the following URL for more information: ...
Select "OK" to attempt to log in again.
Select "CANCEL" to start the game in offline mode.
```

`ds2-dialog-skip` correctly declined to answer it -- its rule was "suppress notices, never answer a
question" -- and logged `options=42 action=shown reason=has-a-real-choice`. Its own source comment
had predicted that a two-option boot dialog would surface exactly this way.

### `+0x12` was never an option count

`FeSubStateCommonWindowBase::v5` (`0x140104f30`) publishes one transition per edge:

```text
movsx edx, WORD PTR [rdi+0x10]     ; cancel destination
mov   [rax+0x18], &this[0x30]      ; watch the phase
mov   [rax+0x08], edx
mov   BYTE PTR [rax+0x20], 3       ; ...on phase 3, the cancel-closed phase

cmp   WORD PTR [rdi+0x12], 0
jl    done                         ; negative -> no confirm edge at all
movsx ecx, WORD PTR [rdi+0x12]     ; confirm destination
mov   [rax+0x08], ecx
mov   BYTE PTR [rax+0x20], 4       ; ...on phase 4
```

So `+0x10` and `+0x12` are **destination substate ids**, not a caption id and an option count.
`options=42` meant destination `0x2a`. The old reading was behaviourally right -- a negative
confirm destination really does mean a one-button box -- which is exactly why it survived: it never
produced a wrong answer until something wanted to know *where the edges went*.

### Reasoning from the button labels gives the wrong answer

The text says CANCEL starts offline mode, so the obvious move is to write
`FE_DIALOG_RESULT_CANCEL`. **That is backwards.** Read live out of the object on a running game
(`kind=0x3e`):

```text
cancel-dest  = 0x39   FeSubStateTitleGameServerLogin   <- retries the login
confirm-dest = 0x2a   FeSubStateOfflineModeWindow      <- plays offline
```

Writing the cancel result would have retried the login, in a mod whose entire purpose is not to.
The fix is to stop reasoning about buttons: the answer is chosen by comparing the two destination
ids against `FeSubStateOfflineModeWindow`, and only when **exactly one** of them matches. If both
or neither did, the box is shown and the player decides.

That capability lives in `ds2-dialog-skip`, which owns the `enter` detour; the loader relays
`[offline] enabled` to it via `set_answer_offline_prompt`. A run with offline off keeps the old
"never answer a question" behaviour exactly.

Measured, same boot:

```text
ds2-dialog-skip: suppressed screen=common-window kind=62 cancel-dest=0x39 confirm-dest=0x2a edge=confirm-goes-offline result=2 phase=4 total=1
ds2-dialog-skip: suppressed screen=offline-mode-window kind=42 cancel-dest=0x47 confirm-dest=0xffff edge=only-edge result=1 phase=3 total=2
```

The prompt routes to the "playing offline" notice, which is a one-edge box and is suppressed the
way the other notices always were, and that edge lands on `0x47` -- the top menu. The boot now
reaches a usable menu, offline, with no button presses.

## What this does not do

* **It does not touch Steam.** See above.
* **It does not remove the network boot substates.** `0x20` and `0x39` still run, and `0x39`
  now fails early instead of waiting on a server; `0x44` is already skipped because the failure
  prompt's offline edge goes to `0x2a`. Taking `0x39` off the path too is the two-byte UserPolicy
  patch described above, which is not built. Whatever boot time that saves has not been measured
  -- `DS2-BOOT-WORK.md` measured the noise floor at +/-300 ms, so any such claim needs several
  runs before it is believed.
* **It does not stop a determined online path.** Four imports are fronted, not all 43. A code path
  that used `WSASendTo` or `WSAConnect` would go straight through -- neither is in this build's
  import table, which is why neither is fronted, but that is a fact about build 9527516 rather
  than a guarantee about the design.
