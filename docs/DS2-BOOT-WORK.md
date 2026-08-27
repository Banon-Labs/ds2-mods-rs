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

Both of the questions that gated that claim have now been read out of the binary. The answers
narrow it: **within-storage overlap is impossible, cross-chain overlap survives.** See below.

## The storage service is a one-request-at-a-time machine, and the binary enforces it

`[GameManagerImp + 0xb8]` is **`SaveLoadSystem`** (vtable `0x1410da4b8`, constructor `0x1402e5f30`).
Its own constructor spawns nothing; the asynchrony lives one layer down, in the `SaveLoad2`
namespace at `0x140a8xxxx`.

`SaveLoadSystem` holds a **`SaveLoad2::SLSessionManager`** at `+0x38`. That manager
(`0x140a8ba40`) is built with three empty slots and fills them once, in `0x140a8c490`:

| field | what goes in it |
| --- | --- |
| `mgr+0x40` | **one thread**, created by `0x14084ba00` with a `0x4000` stack and the literal name **`"SLSession"`** |
| `mgr+0x48` | **one `SLSessionRunnable`** (constructor `0x140a900c0`) |
| `mgr+0x50` | a `DLLifecycleAdapter<DLKR::DLPlainLightMutex>` — one mutex |

One runnable, one thread, one mutex. Not a pool.

The interlock is above that, in `SaveLoadSystem` itself. Every start entry point opens with the
same three lines — `0x1402e72c0` (`0x05 SteamLoadSystemData`'s start) and `0x1402e7170`
(`0x57 LoadProfile`'s) are the *same function* but for the request kind handed to `0x140a8a250`,
`0x07` against `0x1d`:

```text
if ([this+0x38] == 0)                    return false;   // no session manager
if ([this+0x08] != 0 || [this+0x0c] != 0) return false;   // BUSY -- refuse
... build the request on [this+0x30], hand it to [this+0x38] via 0x140a89820 ...
```

And the pollers read that same `+0x08` word, each accepting a different set of values:
`0x1402e6230` (LoadProfile's) takes `(state-2) & ~2 == 0`, i.e. `{2, 4}`; `0x1402e67f0`
(SaveSystemData's) tests `bt 0x6a, state`, i.e. `{1, 3, 5, 6}`.

**So a second storage request issued while one is outstanding is not a race — it is refused, and
returns false.** `0x05` and `0x38` cannot overlap each other, and no hook can make them.

## `0x38` really does consume what `0x05` read

They share more than the service. Both reach the savedata block at
`[[GameManagerImp + 0xa8] + 0xd8]`:

* `0x05 SteamLoadSystemData`'s update (`0x1400fbdb0`, a 13-case jump table on its phase) reads
  `[block + 0x1370]` to decide whether there is any system data to load at all, and takes its
  terminal phase when that byte is zero.
* `0x38 SaveSystemData`'s enter (`0x1400fc3d0`) writes *from* that block, using `[block + 0x1368]`
  as the slot index into `0x1f0`-stride records and writing `[block + 0x1360]`.

Read, then write, on one buffer, through a service that accepts one request at a time. **The
storage chain must stay in order.**

## What survives: the two chains are on different OS threads

The game creates 15 named threads (`0x14084ba00`, 15 call sites). Two of them matter here:

| thread | stack | created in |
| --- | --- | --- |
| `SLSession` | `0x4000` | `0x140a8c490` — the storage worker |
| `NexusRevolution Socket` | `0x10000` | `0x140a69820` — the network worker |

Different objects, different workers, no shared pool. `0x44 Information`'s update (`0x1400ff710`)
touches `[GameManagerImp + 0x22f0]` and nothing else; `0x39 GameServerLogin`'s start and poll
(`0x1400f9820` / `0x1400f9940`) likewise. Nothing on the network path was seen reaching
`SaveLoadSystem` or the savedata block.

So the achievable shape is **not** "five steps at once". It is:

```
storage:  0x05 SteamLoadSystemData ──► 0x38 SaveSystemData          (ordered, forced)
network:  0x20 SteamNetworkCheck ──► 0x39 GameServerLogin ──► 0x44 Information
                                                             (ordered, logically)
floor = max(storage chain, network chain)   instead of   sum of all five
```

The network chain is inherently ordered too — you cannot fetch server information before logging
in — so nothing is lost by leaving it alone. The whole available win is running the two chains
*against each other* rather than end to end.

**Still not established:** whether the network service has its own single-request interlock (it
does not matter for the shape above), and whether anything deeper in the network call tree reaches
storage. Only the three entry points named above were walked, not the service's own call graph.

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

## The boot writes back system data it has not changed

`0x38 SaveSystemData`'s enter (`0x1400fc3d0`) has three parts, two of them gated on constructor
flags at `+0x2c` and `+0x2d`:

```text
if ([this+0x2c]) { [block+0x1360] = <imported call 0x141aae1e4>; ... }   // stamp
if ([this+0x2d]) { copy 0x1f0 bytes from block+index*0x1f0; 0x140059940(block, index, buf); }
0x1402e7f10(SaveLoadSystem, [this+0x2c]);                                // issue the save
```

**For the boot instance both flags are zero.** It is constructed at `0x1400f7bbb` with
`xor r9d,r9d` (→ `+0x2c = 0`) and `r12b` (→ `+0x2d`), and `r12d` is written exactly once in the
whole of `FeStateTitle::v6` — the `xor r12d,r12d` at `0x1400f7363`. Nothing sets it between there
and the construction site.

So at boot this substate does not stamp a time and does not touch a character slot. It does one
thing: **issue a save of the system data**. Which `0x05 SteamLoadSystemData` read moments earlier,
and which nothing between them necessarily modified — on a profile that has already accepted the
policy, `0x37 UserPolicy` early-outs on `[sys+0x136d]` and changes nothing at all.

That makes `0x38` the first piece of *real work* on the chain with a plausible claim to being
redundant, and it is the interesting target — far more so than replacing the storage system, which
would mean owning the format of an 8 MB `DS2SOFS0000.sl2` to save an unmeasured number of
milliseconds.

**What would have to be true before cutting it**, none of it established:

* That nothing else in the boot dirties the system-data buffer between `0x05` and `0x38` — a
  play counter, the graphics config, the online flag. Compare the buffer at both points; do not
  reason about it.
* That the write is not what *creates* system data on a fresh profile. Gate any skip on `0x05`
  having found existing data (`[block + 0x1370] != 0`), which is the same byte `0x05`'s own update
  tests.

The cut itself is the shape `ds2-intro-skip` already uses — hook `enter`, write the terminal phase,
let the flow advance to `0x39` — and it needs its own config switch like every other patch here.

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

One instrumented run answers all of it, and **that instrument is now built**:
`crates/ds2-boot-timeline`, off by default, on with `ds2-run.py --boot-timeline`.

It hooks machinery rather than screens, for the reason this repo has already paid for once
(`docs/DS2-TITLE-FLOW.md`, "hook the drawing, not the class"):

* **`FeStateFlow::update`** (RVA `0x00104540`) drives whichever substate is resident. The detour
  samples the resident pointer at `+0x10` before and after the original, so every arrival is seen.
* **`FeSubStateBase::v6`** (RVA `0x001043a0`), the slot the flow calls immediately before every
  `leave`. **Not one of the 36 substate vtables overrides it**, so that single address is every
  departure in the game.

The two are deliberately redundant, and that redundancy is the integrity check: arrivals and
departures must interleave, and a `leave` line carrying `mismatch=true` says the sampler missed a
transition and every duration after it is attributed to the wrong step. A log that is wrong is
worth much less than a log that says it is wrong.

Timestamps are milliseconds from the loader's `DllMain`, which runs during import resolution and
therefore **before the game's entry point** — so the gap between `t=0` and the first substate line
is the engine bringing up D3D11, mounting archives and starting audio. Under Proton that may be
the largest number in the file, and a timeline anchored at the first substate would have hidden it.

The run also logs the substate count read live off the flow's own list, which turns "64 substates
are constructed in `FeStateTitle::v6`" from a static claim into a measured one and gives a loading
bar its denominator.

## MEASURED, one run, 2026-08-27

Build 9527516, Proton Experimental 11.0-100, `ds2-run.py --boot-timeline`, with intro-skip,
dialog-skip and title-skip all **on** -- so this is the flow as the mod currently ships, not
vanilla. No crash artifacts; the game reached the top menu and stayed up.

| # | id | substate | dwell |
| --- | --- | --- | --- |
| 1 | `0x00` | `TitleInitBranch` | — (see the artifact note below) |
| 2 | `0x01` | `WarningNoCopy` | 4.2 ms |
| 3 | `0x13` | `TitleLogo` #1 | 4.8 ms |
| 4 | `0x14` | `TitleLogo` #2 | 8.9 ms |
| 5 | `0x15` | `TitleLogo` #3 | 9.2 ms |
| 6 | `0x17` | `TitleMain` | 27.8 ms |
| 7 | `0x05` | `TitleSteamLoadSystemData` | **1011.0 ms** |
| 8 | `0x20` | `TitleSteamNetworkCheck` | 9.2 ms |
| 9 | `0x37` | `TitleUserPolicy` | 8.4 ms |
| 10 | `0x39` | `TitleGameServerLogin` | **763.8 ms** |
| 11 | `0x44` | `TitleInformation` | **1026.0 ms** |
| 12 | `0x46` | its message box | 6.0 ms |
| 13 | `0x47` | `TitleTopMenu` | reached at **t = 6827.5 ms** |

**The engine, not the title flow, is the larger half.** `DllMain` to the first substate transition
is **3843.9 ms** — 56% of the whole 6.83 s. Nothing in this document can touch it: it is D3D11 /
DXVK bring-up, archive mounting and audio, before the title state machine exists. That number is
the reason the timeline is anchored at `DllMain` and not at the first substate.

**What is left is almost entirely I/O.** The title flow costs 2983.6 ms, and the three slow steps
account for **2800.8 ms of it — 94%**. Every screen, gate and animation on the chain together comes
to about 78 ms. There is nothing left to skip; what remains is work.

### Two things this run corrected

* **`0x38 SaveSystemData` never ran.** The chain went `0x37 UserPolicy` straight to
  `0x39 GameServerLogin`. `UserPolicy`'s `v5` publishes three edges — phase 2 to `0x38`, phase 3 to
  `0x2a`, phase 4 to `0x39` — and this boot took the phase-4 one. The redundant-write finding above
  still describes the code correctly, but on this profile the write does not happen at boot at all,
  so there is nothing there to save. `ds2-mods-rs-cz6` is moot until a profile is found that takes
  the phase-2 edge.
* **66 substates are registered, not 64.** The live count read off the flow's own list
  (`registered=66`) beats the 64 allocation/constructor pairs the static parse of
  `FeStateTitle::v6` found. The parse missed two. Nothing above depends on the total, but a loading
  bar would have.

### The integrity check did its job, and once it cried wolf

Every `leave` line reads `mismatch=false` except the first: `seq=0 id=0x00 dwell=3843.907ms
mismatch=true`. That is the expected artifact and not a missed transition — `0x00 InitBranch` was
already resident when the hooks went in, so the sampler never saw it arrive and its "dwell" is just
time-since-origin. Every subsequent transition was caught by both hooks. **The sampler missed
nothing**, so the second hook at `0x140104b80` is not needed.

### What the numbers say about the two proposals

```
storage chain   0x05                      = 1011.0 ms
network chain   0x20 + 0x39 + 0x44        = 1799.0 ms
                                   serial = 2810.0 ms
                       max(storage, network) = 1799.0 ms
```

* **Overlapping the chains (`ds2-mods-rs-7on`) is worth ~1011 ms** — the storage chain hides
  entirely inside the network chain. Boot to menu would go from 6.83 s to about 5.82 s.
* **Removing the network chain (`ds2-mods-rs-rk4`) is worth ~1799 ms**, and it is the bigger of the
  two: 6.83 s to about 5.03 s. Removing work still beats overlapping it.
* They are not additive. With the network chain gone there is nothing left to overlap the storage
  chain against.

**Scope, do not overstate.** One run, one profile, one Proton version, online and logged in. The
two network numbers are a server round-trip and will vary with the network; the storage number is
an 8 MB `sl2` on this machine's disk through Wine. A second run is the cheapest way to learn how
much of this is noise, and nothing above should be treated as a constant until there is one.

## SECOND RUN, and it overturns the reading of the first

Same build, same Proton, same profile, back to back. Run 1's log is preserved as
`<Game>/ds2-loader.log.prev` by the launcher's own rotate, so this is two files rather than two
recollections.

| step | run 1 | run 2 | spread |
| --- | --- | --- | --- |
| engine (`DllMain` → first transition) | 3843.907 ms | 3869.716 ms | 0.67% |
| `0x01` WarningNoCopy | 4.155 | 4.219 | 1.5% |
| `0x13` / `0x14` / `0x15` Logo | 4.845 / 8.942 / 9.209 | 4.261 / 8.868 / 9.035 | ≤ 12% |
| `0x17` TitleMain | 27.816 | 29.813 | 7.2% |
| **`0x05` SteamLoadSystemData** | **1011.023** | **1008.729** | **0.23%** |
| `0x20` SteamNetworkCheck | 9.205 | 8.899 | 3.3% |
| `0x37` UserPolicy | 8.443 | 9.399 | 11.3% |
| **`0x39` GameServerLogin** | **763.780** | **678.561** | **11.2%** |
| **`0x44` Information** | **1026.008** | **1027.477** | **0.14%** |
| `0x46` message box | 5.961 | 6.276 | 5.3% |
| **total to top menu** | **6827.515** | **6771.052** | 0.83% |

### Two of the three "slow steps" are not doing anything

**`0x05` reproduces to 2.3 ms and `0x44` to 1.5 ms.** A disk read of an 8 MB `sl2` and a server
fetch do not repeat to within a quarter of one percent. `0x39 GameServerLogin` — the one step known
to be a real round-trip — moved **85 ms, 11%**, between the same two runs, which is what genuine
I/O looks like next to them.

So `0x05` and `0x44` are **timers, not work**: roughly 1.009 s and 1.027 s of waiting for a clock.
Together that is **2.04 s of a 6.8 s boot spent on nothing at all.**

The shape of the bug is already familiar here. `ds2-dialog-skip` removes a 1.0 s
`min-duration` floor from the process windows, and the log shows it doing so on `0x39`
(`shortened screen=process-window kind=57 min-duration=1.000->0`) — which is exactly why `0x39` is
the one of the three that comes in *under* a second. **Neither `0x05 TitleSteamLoadSystemData` nor
`0x44 TitleInformation` derives `FeSubStateProcessWindowBase`** — both are plain `FeSubStateBase`
with 8 virtuals — so that de-flooring never reached them.

**Where the floor is has NOT been found.** Both update functions were read in full and neither
contains a float comparison: `SteamLoadSystemData::v3` (`0x1400fbdb0`, 329 bytes) accumulates
elapsed into `+0x18` and never tests it, and `Information::v3` (`0x1400ff710`, 189 bytes) is a
state machine on the object at `[title+0xa0]` with no threshold. So the wait is one level down —
inside `SaveLoadSystem`/`SaveLoad2` for `0x05`, inside the downloader for `0x44`. That is the hunt,
and it now has a very specific target: a one-second constant on each of two paths.

### The engine block is not shader compilation

3843.9 ms and 3869.7 ms — **0.67% apart**. A cold DXVK shader cache against a warm one would differ
by seconds, not by 26 ms. Whatever those 3.86 s are, they are steady-state startup work that
happens on every launch, and they remain 56% of the boot and completely uninvestigated.

### What this does to the priority order

The earlier ranking was built on the assumption that all three slow steps were I/O. Two of them are
not, and that reorders everything:

| target | worth | status |
| --- | --- | --- |
| the two one-second floors (`0x05`, `0x44`) | **~2.04 s** | mechanism not found; same class of fix already shipped for `0x39` |
| the engine block | 3.86 s, unknown reducibility | never investigated |
| `rk4`, remove the network chain | **~0.69 s**, not 1.8 s | `0x44`'s second was a floor, not a fetch |
| `7on`, overlap storage against network | **~0** | `0x05` is a timer; overlapping a timer buys nothing — delete it instead |

`ds2-mods-rs-7on` is effectively dead as written, and `rk4` is worth a third of what the first run
suggested. Removing the two floors would take boot-to-menu from ~6.8 s to about **4.8 s**, and it
is the only item on the list whose fix has a proven precedent in this repo.

## Inside the engine block

Two milestones were added that cost no hook at all, because the loader already occupies both
positions: the Arxan callback runs at the game's entry point, and `DirectInput8Create` is our own
proxy export, called once during input initialisation. Measured (run 4 of 4):

| phase | span | cost |
| --- | --- | --- |
| `DllMain` → entry point | 0 → 418.2 ms | **418.2 ms** |
| entry point → input init | 438.5 → 855.1 ms | **416.6 ms** |
| the `DirectInput8Create` forward itself | 855.1 → 862.1 ms | 7.0 ms |
| **input init → first substate** | 862.1 → 3925.2 ms | **3063.1 ms** |
| title flow | 3925.2 → 6643.9 ms | 2718.7 ms |

So the engine block is not one lump: about 835 ms of it is before the game's own initialisation
gets going, and **3.06 s — 46% of the entire boot — sits in one window after input init and before
the title state machine runs.** That window is where D3D11 device creation, archive mounting, FMOD,
Steam and param loading all live, and no free milestone falls inside it.

**One of those numbers may be ours.** `neuter_arxan` runs inside the first 418 ms, and nothing has
measured it. Until a milestone brackets that call, some fraction of the pre-entry-point cost is
this mod's own overhead being counted against the game.

### The window is neither disk-bound nor CPU-bound

Rather than hook file APIs — a naked thunk around a ten-argument import, in a process carrying 48
Arxan stubs — the next measurement was taken from **outside** the process.
`scripts/ds2-io-sample.py` samples `/proc/<pid>/io` and `/proc/<pid>/stat` every 20 ms. It attaches
to nothing and signals nothing, so the run is the run it would have been anyway.

Over a 9.7 s window covering the whole boot:

```
rchar (bytes returned by read())        128.0 MB
read_bytes (fetched from the device)     19.2 MB
cpu (utime+stime)                         5.08 s over 9.7s  =  52% of ONE core
```

Two things fall straight out of that:

* **The archives are already in the page cache.** The game reads 128 MB and the disk serves 19 MB
  of it, 18.5 MB of that in the first 250 ms. Loading is not waiting on the disk on a warm boot,
  so "make the archive reads faster" is not the lever it looked like.
* **Nothing is compute-bound.** CPU never exceeds roughly one core, and averages half of one, in a
  process with many threads.

And the shape matters more than the totals. **Every byte of reading happens in an early ~2 s
burst**; from about 2 s onward the `rchar` column is flat zero while CPU sits at ~50%. A stretch
that is neither reading nor computing hard is a stretch that is *waiting* — the same signature the
two one-second floors already showed in the title flow.

### The alignment is NOT established, and it bounds what the above can claim

The sampler's `t = 0` is the moment it first found the process; the loader's `t = 0` is `DllMain`.
Nothing has measured the offset between them, so mapping a bucket in the sampler onto a milestone
in the timeline is an inference, not a reading. The claims that survive without the alignment are
the ones above — total bytes, cache warmth, CPU ceiling, and the fact that reading stops early.
The claim that does **not** yet survive is "the 3.06 s window contains ~1.9 s of idle", which the
shape suggests and nothing here proves.

Fixing it is one line on each side: a wall-clock stamp in the loader's first log line and in the
sampler's header. Do that before drawing any conclusion that depends on where the boundaries fall.

## The caveat that governs every address here

These come from the deobfuscated image, which is not the byte stream that runs. Vtables and the
transition objects are data and are trustworthy; any function to be detoured must be checked
against the Arxan redirect set with `scripts/ds2-arxan-chain.py` first. See
`docs/ARXAN-FOOTPRINT.md`.
