# What the game does before you can press Continue

Read statically from `darksoulsii-deobf.bin` (SOTFS build 9527516) with `scripts/ds2-rtti.py`,
`scripts/ds2-disasm.py` and the new `scripts/ds2-pe.py`. No game was launched to establish any of
it; one previously-measured run is used at the end to **cross-validate** the id space.

`docs/DS2-TITLE-FLOW.md` traces the screens. This traces the **work** — which steps are animation
that can be deleted, which are real I/O that cannot, what each one blocks on, and what could run at
the same time as something else.

## The state machine, and where the ids come from

`FeStateTitle` owns a `TMenuStateBaseList<FeSubStateBase, 0x58>` — 88 slots. **64 substates are
constructed**, in `FeStateTitle::v6` at `0x1400f72e0` (5744 bytes): allocate via `0x140833320`,
call a per-class constructor, append to `[list + count*8 + 8]` with the count at `+0x2c8`.

Every substate carries its **id as a DWORD at `+0x0c`**, written by its constructor — either as a
literal (`FeSubStateTitleMain` writes `0x17`) or from `edx` at the call site (the four
`FeSubStateTitleLogo` instances get `0x13`, `0x14`, `0x15`, `0x16`).

Transitions are published by each substate's **v5**, which `FeStateFlow`'s dispatcher
(`0x140104540`) calls right after `enter`. Each is a `0x28`-byte `FeTransitionEqualValue<int>`,
vtable `0x1410bd440`:

| offset | meaning |
| --- | --- |
| `+0x00` | vtable `0x1410bd440` |
| `+0x08` | **destination substate id** |
| `+0x0c` | byte, 1 |
| `+0x10` | `-1` |
| `+0x18` | pointer to the watched int — always the substate's own phase field |
| `+0x20` | **the value to match** |

So the entire boot graph is a static read: for every substate, disassemble v5 and pair `+0x08`
against `+0x20`. That is what the table below is.

### The id space is confirmed by a run, not just by the disassembly

`ds2-dialog-skip`'s existing log lines print a `kind=` for each window it acts on. Two of them from
the run recorded in `docs/DS2-TITLE-FLOW.md`:

```
ds2-dialog-skip: shortened  screen=process-window kind=57 min-duration=1.000->0
ds2-dialog-skip: suppressed screen=common-window  kind=70 caption=0x47
```

`57` is `0x39` and `70` is `0x46`. Statically, `0x39` is `FeSubStateTitleGameServerLogin` and
`0x46` is the message box constructed immediately after `FeSubStateTitleInformation`. The `kind`
that crate logs **is** the `+0x0c` id. The static id space and a real boot agree.

## The cold-boot chain, in order

Entered at `FeSubStateTitleInitBranch` (id `0x00`), whose v5 branches on the boot-once flag
`0x14160de1a`: first pass through this process → `0x01`; a later return to the title → `0x17`.

| # | id | substate | what it actually does | blocks on |
| --- | --- | --- | --- | --- |
| 1 | `0x00` | `TitleInitBranch` | branch only, no update, no work | — |
| 2 | `0x01` | `WarningNoCopy` | "do not copy" screen | a timer |
| 3 | `0x13` | `TitleLogo` #1 | splash, scene `[title+0xc0]` | a timer |
| 4 | `0x14` | `TitleLogo` #2 | splash, scene `[title+0xb8]` | a timer |
| 5 | `0x15` | `TitleLogo` #3 | splash, scene `[title+0xc8]` | a timer |
| 6 | `0x17` | `TitleMain` | title scene, sequence gate, PRESS ANY BUTTON | animation, then input |
| 7 | `0x05` | `TitleSteamLoadSystemData` | **reads system data** | storage service |
| 8 | `0x20` | `TitleSteamNetworkCheck` | **Steam network probe** | network service |
| 9 | `0x37` | `TitleUserPolicy` | EULA screen | input, or early-out |
| 10 | `0x38` | `TitleSaveSystemData` | **writes system data** | storage service |
| 11 | `0x39` | `TitleGameServerLogin` | **server login** | network service |
| 12 | `0x44` | `TitleInformation` | **fetches server information** | network service |
| 13 | `0x47` | `TitleTopMenu` | the menu. Continue / LOAD GAME lives here | input |

Edges, each read from the v5 of the substate on the left:

```
0x00 --(flag==0)--> 0x01 --(phase 4)--> 0x13 --> 0x14 --> 0x15 --(phase 4)--> 0x17
0x00 --(flag==1)--> 0x17
0x17 --(phase 4)--> 0x05 --(result 7)--> 0x20 --(result 1)--> 0x37
0x37 --(phase 2)--> 0x38 --(result 0)--> 0x39 --(result 0)--> 0x44 --(phase 5)--> 0x47
```

Every failure result forks to a message box or a fail-warn substate instead: `0x05` to
`0x06`/`0x07`/`0x09`/`0x0b`, `0x20` to `0x24`/`0x29`, `0x37` to `0x2a` (offline mode), `0x38` to
`0x52`/`0x53`, `0x39` to `0x3a`–`0x40`, `0x44` to `0x45`/`0x46`. Those are the boxes
`ds2-dialog-skip` already suppresses.

**After** Continue: `0x47` row 1 → `0x55 TitleLoadDataList` → `0x57 TitleLoadProfile` → `0x17`.
That is a separate chain and is not covered here.

## There are exactly two backends, and they are different objects

Every blocking step above resolves to one of two singletons hanging off `GameManagerImp`
(`0x1416148f0`):

* **Storage** — `[GameManagerImp + 0xb8]`. `SteamLoadSystemData` (`0x05`), `SaveSystemData`
  (`0x38`), `LoadProfile` (`0x57`), `SaveFirst` (`0x07`), `RegulationSave` (`0x36`).
  `TitleSaveSystemData`'s starter is `0x1400fc3d0` and its wait is `0x1400fc730`, both reaching
  `[app+0xb8]`; `TitleLoadProfile`'s are `0x1400fc370` / `0x1400fc5b0`, same object.
* **Network** — `[GameManagerImp + 0x22f0]`, fetched through `0x1405132a0`/`0x1405132c0`.
  `SteamNetworkCheck` (`0x20`), `OnlineCheck` (`0x4e`), `GameServerLogin` (`0x39`),
  `Information` (`0x44`). `TitleOnlineCheck`'s starter `0x1400f98c0` and wait `0x1400f9970` both
  tail into that object's vtable, as do `TitleGameServerLogin`'s `0x1400f9820` / `0x1400f9940`.

That split is the whole parallelism story, and it is why it is worth stating precisely.

## The work is already asynchronous. The state machine is what serialises it

`FeSubStateProcessWindowBase::v1` (`0x140104ed0`) is shared by six classes, and its shape is:

```text
enter:  result = this->vtable[8]();      // STARTS the operation, returns immediately
        ...
update: if (this->vtable[10]()) return;  // POLLS it; keep waiting while true
```

Slot 8 starts, slot 10 polls. Neither blocks the frame. **The engine is not making these
operations serial — the state machine is**, by only ever having one substate resident and only
starting the next one's slot 8 after the previous one's slot 10 has gone false.

Concretely, the boot spends `t(0x05) + t(0x20) + t(0x38) + t(0x39) + t(0x44)` where two of those
five are on a storage object nothing else is touching and three are on a network object nothing
else is touching. The lower bound for the same work is
`max( t(0x05) + t(0x38), t(0x20) + t(0x39) + t(0x44) )`.

**What is not established:** whether either service serialises its own queue internally, and
whether `0x38 SaveSystemData` depends on the result of `0x05 SteamLoadSystemData` (it plausibly
writes back what `0x05` read). Both are answerable — the first by reading the two service classes,
the second by reading what `0x38`'s slot 8 sources its buffer from. Neither has been read yet, and
the overlap claim above is a *shape*, not a licence to reorder anything.

## Skippable, real, and what already skips it

The user's question was which of these can be removed outright. Split three ways:

### Pure gating — no work happens, deleting it costs nothing

| step | what it is | already skipped by |
| --- | --- | --- |
| `0x01` WarningNoCopy | timed screen | `ds2-intro-skip` (writes terminal phase 4) |
| `0x13`/`0x14`/`0x15` Logo | three timed splashes | `ds2-intro-skip` |
| `0x17` sequence gate | waits for title sequence `0x67` | `ds2-title-skip` (`title_sequence_gate`) |
| `0x17` press poll `0x1400ff420` | PRESS ANY BUTTON | `ds2-title-skip` (`press_any_button`) |
| `0x37` UserPolicy | EULA, once per profile | `ds2-intro-skip`; the game early-outs on `[sys+0x136d]` anyway |
| process-window `+0x10` floor | 1.0s minimum display, **after** the work finished | `ds2-dialog-skip` (`process_windows`) |
| the `0x06`/`0x46`/… message boxes | notices with nothing pending | `ds2-dialog-skip` |
| the wait windows themselves | drawing only | `ds2-dialog-skip` (`show_process_window` hook) |

**Nothing in this group is unhandled.** That list is complete against the substates on the chain
above, and it is the answer to "is anything skippable still not skipped": on the *screen* side, no.

### Real work — cannot be skipped without changing behaviour

`0x05` SteamLoadSystemData, `0x38` SaveSystemData, `0x39` GameServerLogin, `0x44` Information,
`0x20` SteamNetworkCheck. Each starts an operation on one of the two services and waits for it.
Suppressing the substate suppresses the wait, not the work — the operation would still be in
flight, with the flow past it.

### Real work that is conditional — the biggest cut available, and it is not built

`0x20`, `0x39` and `0x44` are the network chain. An offline boot does not need any of them, and
the game already has the switch: **`0x14160de19`**, read at exactly one instruction in the whole
image (`0x1400f431f`, in the top-menu builder) and forcing the online flag to 0. Ghidra finds no
writer. Setting it makes the menu believe it is offline.

**Not established:** whether that byte also diverts the *boot chain* or only the menu rows. It is
read in the builder; whether `0x20`/`0x39`/`0x44` consult the same flag has not been traced. The
master online gate they do consult is `0x140513600`, which is `return *(u8*)(this+0x3a)` on the
network service — a different read. Trace that before assuming one byte removes the whole chain.

## For the loading bar

The bar wants a total and a position. Both are available without inventing anything:

* **The set of steps is finite and enumerable at runtime** — 64 substates, each with its id at
  `+0x0c`, all of them in the list at `FeStateTitle+0x08` with the count at `+0x2c8`.
* **The current step is one read.** `FeStateFlow`'s dispatcher `0x140104540` holds the resident
  substate; `FeOperatorTitle::v4` requests one by writing the id to `[state+0x48]` (it writes
  `0x17` there on the return-to-title path, at `0x1400ef3e4`).
* **Progress within a step is already modelled.** Each process-window substate exposes slot 10 —
  "am I still working" — and `FeSubStateProcessWindowBase` keeps elapsed at `+0x14`.

The honest caveat: the chain is a **graph, not a line**. Every step can fork to an error box, and
`0x00` skips the whole splash run on a return to title. A bar driven off "step N of 13" will be
wrong on any non-happy path. Drive it off a weight table keyed by id, with the weights measured,
and treat an unexpected id as "hold position" rather than "jump".

## What this trace cannot tell you, and the one run that would

**Nothing here is a duration.** Static analysis names the steps and proves the dependency shape; it
cannot say whether `GameServerLogin` costs 40 ms or 4 s, and without that the parallel-overlap idea
has no measured value and the loading bar has no weights.

One instrumented run answers all of it: a timestamped `enter`/`leave` line per substate id, taken
at the dispatcher so no per-class hook can miss a step — the same mistake per-class `enter` hooking
already made once with the wait windows (`docs/DS2-TITLE-FLOW.md`, "hook the drawing, not the
class"). That single log is simultaneously the parallelism evidence, the loading-bar weight table,
and the record of which steps are worth attacking.

## The caveat that governs every address here

These come from the deobfuscated image, which is not the byte stream that runs. Vtables and the
transition objects are data and are trustworthy; any function to be detoured must be checked
against the Arxan redirect set with `scripts/ds2-arxan-chain.py` first. See
`docs/ARXAN-FOOTPRINT.md`.
