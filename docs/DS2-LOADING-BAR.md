# A loading bar driven by the boot's own state

A design, not an implementation. No Rust was changed and no game was launched to write it. Every
fact below carries one of three tags:

* **read in source** -- read out of this repo's code or out of the binary by an earlier trace,
  with the file named;
* **measured** -- a number from a real run, with the run named;
* **inferred** -- a conclusion the evidence fits and nothing has proven.

The question it answers: where would a bar get a total and a position from, how does the position
stay monotonic when the boot takes a different path, and what can actually draw it.

## The boot, as the bar sees it

Two halves, and they are different kinds of thing.

The first half is the engine starting up. There is no title state machine yet, no substate id, and
for most of it no Direct3D device. The second half is the title flow: `FeOperatorTitle` loads its
resources, starts `FeStateFlow`, and the flow walks substates until the top menu.

The latest partitioned run, with Seamless on and the mod's skips on (measured: `e216b7e`,
`--boot-timeline`, the phase-mark table in the draft PR for branch `boot-timeline-boot-waits`;
milliseconds from the loader's `DllMain`):

| span | from | to | ms |
| --- | --- | --- | --- |
| Arxan neuter (ours) | 17 | 319 | 302 |
| our crate installs | 319 | 925 | 606 |
| installs done to `WinMain` (CRT, static init) | 925 | 1666 | 741 |
| app setup | 1681 | 1740 | 59 |
| archive mounts | 1807 | 2075 | 268 |
| graphics init | 2117 | 2214 | 97 |
| sound init | 2306 | 2485 | 178 |
| Katana init | 2512 | 3052 | 539 |
| first frame to first substate | 3062 | 3170 | 108 |
| first substate to `0x47` TopMenu | 3170 | 3550 | 380 |

What each phase contains is read in source on that branch (`ds2_rva::BOOT_PHASES`): app setup
creates the DXGI factory, graphics init calls `D3D11CreateDevice` and makes the first presents,
Katana init builds the draw system and scene manager, and the app frame is one main-loop frame.
So no swap chain exists before graphics init (read in source), which on this run is most of the
wall time before the menu (measured, same table).

The phase marks themselves live in the measurement crate on a draft branch and are not on `main`.
They are used here to derive weights, not as a runtime signal (see "Where the signal comes from").

## The progress model

### Paths, chosen from the config

The title flow is a graph. Which path a run takes depends mostly on configuration the loader
already reads, so the bar picks a path profile at install time rather than guessing at runtime:

| profile | chosen when | path after `0x37` | source |
| --- | --- | --- | --- |
| `seamless` | `[seamless] enabled = true` | `0x39` refused, `0x3a` box, `0x2a` offline notice, `0x47` | measured: the game directory's `ds2-loader.log` and `.prev`, 2026-09-26, `kind=58 cancel-dest=0x2a` then `kind=42 cancel-dest=0x47` |
| `offline` | `[offline] enabled = true` | `0x39` refused, `0x3e` prompt answered offline, `0x2a`, `0x47` | measured: `docs/DS2-OFFLINE.md`, `kind=62 ... confirm-dest=0x2a` then `kind=42` |
| `online` | neither | `0x39` login, `0x44` announcements, `0x46` empty-list box, `0x47` | measured: `docs/DS2-BOOT-WORK.md`, both runs of 2026-08-27 |

Before `0x37` every profile shares the cold-boot chain `0x00 0x01 0x13 0x14 0x15 0x17 0x05 0x20
0x37` (measured, `docs/DS2-BOOT-WORK.md`). `0x38 SaveSystemData` is on the static chain but was
not entered on any measured boot: `0x37` took its phase-4 edge straight to `0x39` (measured, same
doc).

That `0x46` is the successful-but-empty announcement box and `0x45` the failure box is read in the
binary on the draft branch `docs-info-fetch-succeeds`; `main`'s copy of the boot doc still reads
`0x46` as a failure. The bar does not depend on which reading is right, because both ids lead to
`0x47` and the model below treats an off-path id the same way whatever it means.

### Weight table

Weights are expected milliseconds per step. Where a step's dwell has been measured on a build that
still behaves the same way, that is the weight. Where it has not, the row says what stands in.

| step | weight ms | source |
| --- | --- | --- |
| before first frame | 3062 | measured, `e216b7e` table: first app frame at 3062 |
| operator states 1 to 3 | 108 | measured, `e216b7e`: first frame to first substate. Not split by state; see open questions |
| `0x00` InitBranch | 0 | read in source: no update, branches in the same frame (`docs/DS2-BOOT-WORK.md`) |
| `0x01` WarningNoCopy | 4.2 | measured, 2026-08-27 run 2 |
| `0x13` Logo | 4.3 | measured, same run |
| `0x14` Logo | 8.9 | measured, same run |
| `0x15` Logo | 9.0 | measured, same run |
| `0x17` TitleMain | 29.8 | measured, same run |
| `0x05` SteamLoadSystemData | 114.5 | measured, the floor-lifted run in `docs/DS2-BOOT-WORK.md` |
| `0x20` SteamNetworkCheck | 8.9 | measured, 2026-08-27 run 2 |
| `0x37` UserPolicy | 9.4 | measured, same run |
| `0x39` on `online` | 721.2 | measured, mean of 763.8 and 678.6 from the two 2026-08-27 runs |
| `0x44` Information | 1010.4 | measured, the floor-lifted run |
| `0x46` box | 6.1 | measured, mean of 5.961 and 6.276 |
| `0x39` on `seamless` | 179 | derived: the `e216b7e` flow total of 380 minus every other row on that path. Not measured on its own |
| `0x3a`, `0x2a`, `0x3e` suppressed boxes | 6.0 | stand-in: the measured dwell of the suppressed `0x46` box, the same kind of object |
| `0x39` on `offline` | 179 | stand-in: the `seamless` figure. `docs/DS2-OFFLINE.md` says this span has not been measured |
| any id not on the profile's path | 250 | a design constant, not a measurement; sets how fast the crawl in an off-path step moves |

The rows mix runs from different builds. That is deliberate and it is the weakest part of the
table: only the pre-flow split and the flow total come from one run. The work items below end with
a run that replaces every derived and stand-in row with a measurement from the same build.

Cumulative floors, as fractions of each profile's total (arithmetic on the table above):

| step entered | `seamless` floor | `online` floor |
| --- | --- | --- |
| first `Present` (graphics init entry, 2117 ms) | 0.596 | 0.415 |
| first app frame | 0.863 | 0.601 |
| `0x01` | 0.893 | 0.622 |
| `0x17` | 0.900 | 0.627 |
| `0x05` | 0.909 | 0.633 |
| `0x20` | 0.941 | 0.656 |
| `0x37` | 0.944 | 0.657 |
| `0x39` | 0.946 | 0.659 |
| `0x3a` / `0x44` | 0.997 | 0.801 |
| `0x2a` / `0x46` | 0.998 | 0.999 |
| `0x47` | 1.000 | 1.000 |

The `online` column shows why the profile matters. On that path the title flow is a third of the
boot; on `seamless` it is a tenth. One table for both would crawl for a second on one and leap on
the other.

### The function

State: the value shown so far `shown`, the current step `s`, the time it was entered `t0`, and the
value it was entered at `e`.

```text
on entering step s at time t0:
    if complete:                      ignore                     // after 0x47 the bar is gone
    if s in {0x47, 0x55, 0x57, 0x6b}: complete; target = 1.0     // TopMenu, or past it
    else if s is on the profile path and hi(s) > shown:
        e = max(shown, lo(s));  c = hi(s)                        // normal step
    else:                                                        // fork, retry, unknown id
        e = shown;              c = shown + (top - shown) * crawl

progress(t) = e + (c - e) * ease(t - t0, weight(s))
shown       = max(shown, progress(now))                          // the only writer of shown

ease(x, w) = x / w                                       for x <= k * w
           = k + (1 - k) * (1 - exp(-(x - k*w) / ((1 - k) * w)))   otherwise
```

`lo(s)` and `hi(s)` are the step's floor and the next step's floor from the profile table. `k` is
`0.8`, `top` is just below `1.0` and `crawl` a fraction of the remaining gap; all three are design
constants, not measurements.

What each piece is for:

* **Monotonic by construction.** `shown` is only ever written through a `max`. An id whose band
  lies below `shown` -- a retry, a return to an earlier step, an id the table never heard of --
  cannot move it back.
* **Capped below the next floor.** `ease` is strictly below `1` for every finite `x`, so a step
  that runs long approaches `hi(s)` and never reaches it. The bar can only cross into the next
  band when the next id actually arrives.
* **No stall.** `ease` is strictly increasing, and its two halves meet with the same slope `1/w`,
  so a long step slows down smoothly instead of hitting a wall. A step that runs three times its
  weight has still moved.
* **No jump on a fork.** An off-path id starts from `shown`, not from a table floor, so entering it
  moves nothing. It then crawls toward `top`.
* **One jump that is allowed, and smoothed.** A step that finishes early leaves a gap between
  `shown` and the next floor, and `0x47` closes whatever is left. The renderer closes gaps at a
  bounded rate rather than in one frame; the rate is a rendering choice and belongs to the draw
  code, not to this function.

### The forks, and what the function does with each

Edges read in source from each substate's `v5` (`docs/DS2-BOOT-WORK.md`, `docs/DS2-OFFLINE.md`):

| from | to | what it is | treatment |
| --- | --- | --- | --- |
| `0x00` | `0x17` | boot-once flag set: a return to the title | the bar is already complete; ignored |
| `0x05` | `0x06` `0x07` `0x09` `0x0b` | storage failure or first-save paths | off-path: hold and crawl |
| `0x20` | `0x24` `0x29` | Steam network check failures | off-path |
| `0x37` | `0x38` | save system data; not reached on any measured profile | off-path |
| `0x37` | `0x2a` | persisted offline setting (`[sys+0x136e]`) | off-path on `online`; `0x2a` is on the other paths |
| `0x38` | `0x52` `0x53` | save failures | off-path |
| `0x39` | `0x3a` to `0x40` | login failure boxes | on-path for `seamless` (`0x3a`) and `offline` (`0x3e`); off-path otherwise |
| `0x3e` | `0x39` | cancel on the login prompt retries the login | band below `shown`: held, then crawls. Never goes back |
| `0x44` | `0x45` | announcement fetch failed | off-path |
| `0x44` | `0x46` | fetch succeeded, empty list | on-path for `online` |

**Steps that wait for the player** are not loading, and a bar that crawls over them lies. With the
press-any-button skip off, `0x17` waits for input; `0x37` shows its policy screen on a profile that
has not accepted it; `0x3e` waits for an answer when `[offline]` is off, because `ds2-dialog-skip`
never answers a question (read in source: `crates/ds2-dialog-skip/src/lib.rs`). The bar hides
while one of these is up and comes back at the value it held. `ds2-dialog-skip` already decides,
per box, whether it was suppressed or shown; publishing that decision is the cheapest input-wait
signal and is a work item.

## Where the signal comes from

### The per-frame read, and why it replaces a hook

Everything the bar needs after the first frame is reachable by reads, from the same thread that
writes it, once per `Present`:

```text
gmi   = [base + GAME_MANAGER_IMP]                          0x016148f0
root  = [gmi  + GAME_MANAGER_FRONTEND_ROOT_OFFSET]         +0x22e0
op    = [root + FRONTEND_TITLE_OPERATOR_OFFSET]            +0xd0
state = u32 [op + FE_OPERATOR_TITLE_STATE_OFFSET]          +0x30   1 resources, 2 operators, 3 fade, 4 flow
flow  = [op + 0x38]                                        no ds2-rva name yet
sub   = [flow + FE_STATE_FLOW_RESIDENT_SUBSTATE_OFFSET]    +0x10
id    = u32 [sub + FE_SUBSTATE_ID_OFFSET]                  +0x0c
```

* Every offset except the flow's is a named constant in `crates/ds2-rva` (read in source). The
  flow at `+0x38` is read in the binary in `docs/DS2-BOOT-WORK.md` ("v2 ... stores the flow at
  `+0x38`") and needs a name before any code uses it.
* The operator pointer at `+0xd0` was read live at the top menu (read in source, the constant's
  doc comment). That it is non-null and stable from its creation onward is inferred.
* A frame runs the per-frame callbacks, which include `KatanaMainApp`'s update, and then draws and
  presents (read in the binary, `docs/DS2-BOOT-WORK.md`, `FUN_140af09b0`). So a read at `Present`
  sees the state the frame's update left.
* The boot thread is the one inside the `Present` wrapper (measured: the sampler run in the notes
  on the boot partition, `0xaf02f0` on the boot thread's stack). The flow runs on the boot thread
  too: the flow line logs `on-boot-thread=`, but no quoted run records its value, so that half is
  inferred and goes on the runtime checklist below.
* Sampling once per frame sees every resident substate only if the flow makes at most one
  transition per update. The boot timeline's two hooks never disagreed after the first substate
  (measured: `mismatch=false` on every leave line after `seq=0`, the first 2026-08-27 run), which
  fits that; whether a per-frame read sees the same ids is a runtime check, not a given.

### Which existing hooks see what, and which the bar may lean on

| signal | who sees it today | on by default | safe for the bar |
| --- | --- | --- | --- |
| `t = 0` at `DllMain` | `ds2_boot_timeline::mark_origin`, called unconditionally from the loader | yes | yes, once the crate exposes the elapsed time instead of keeping `now_us` private |
| entry point | the loader's Arxan callback, which is where every feature installs | yes | yes: a position the loader owns |
| `DirectInput8Create` | the loader's own proxy export | yes | yes, for the same reason |
| boot phases `win-main` to `app-frame` | measurement crate, draft branch only | no | no: an instrument, off by default; the bar uses the weights it produced |
| `FeStateFlow::update` `0x140104540` | `ds2-boot-timeline` | no | no, see below |
| `FeSubStateBase::v6` `0x1401043a0` | `ds2-boot-timeline` | no | no, same reason |
| `FeOperatorTitle::v4` `0x1400ef390` | `ds2-intro-skip`'s fade finish | with intro skip | no second hook; read `[op+0x30]` instead |
| common-window `enter`, `show_process_window`, the `0x05`/`0x44` floor enters | `ds2-dialog-skip` | with dialog skip | only as the source of the input-wait flag; none of them sees every step |
| `IDXGISwapChain::Present` | `ds2-invasion-path`, with a one-consumer clock seam used by `ds2-input-harness` | no | yes, after it moves to a shared crate |

Why the flow hook is off the table even though it sees everything: `FeStateFlow::update` takes
its frame delta in `xmm1` (read in source, `crates/ds2-boot-timeline/src/install.rs`), and
`ds2-hook`'s union carries four integer arguments and says it cannot carry a float (read in
source, `crates/ds2-hook/src/lib.rs`). So the timeline hooks it with a bare `MhHook`, and every
feature is linked into the one loader DLL with one MinHook instance, where a second
`MH_CreateHook` on the same address fails (read in source, the union's own comment). A bar that
hooked it would break the timeline on exactly the runs used to check the bar. Reading the same
field from `Present` needs no hook at all.

## Rendering

### When anything can be on screen at all

Using the `e216b7e` spans above:

| span | what exists | what a bar could show |
| --- | --- | --- |
| `DllMain` to the entry point | nothing of the game's; we are under the loader lock | nothing. This repo keeps work out of the loader lock (read in source: the loader's `DllMain` comments and `crates/ds2-invasion-path/Cargo.toml`) |
| entry point to app setup | our code runs; no game window | only a window of our own |
| app setup to graphics init | the game window, no device | only GDI on a window, ours or the game's |
| graphics init | device, swap chain, the first presents (read in source, `ds2_rva::BOOT_PHASES` on the draft branch) | a `Present` overlay, for however many frames are presented |
| sound init, Katana init | unknown whether any frame is presented | if none is, whatever was last drawn stays frozen |
| first app frame to `0x47` | a frame per vsync (measured: the sampler, boot thread in `Present`) | a `Present` overlay, every frame |

The honest shape: a swap-chain bar first appears when the boot is already most of the way done on
the `seamless` profile, and a bit under half on `online` (the first-`Present` row of the floors
table). With the time-weighted model that is correct rather than embarrassing -- the bar's first
frame already says how much of the wait is behind the player. What would be embarrassing is a bar
that appears at graphics init and then freezes through Katana init, and whether that happens is
the first open question.

### (a) The `Present` overlay, extracted into a shared crate

`ds2-invasion-path` already has everything a bar needs to draw (read in source,
`crates/ds2-invasion-path/src/render.rs`): a detour on `IDXGISwapChain::Present` found through a
throwaway swap chain, compiled shaders for coloured triangles in pixel coordinates, a
dynamic vertex buffer, and a full save and restore of the pipeline state it touches, with every
failure disabling the overlay instead of faulting inside `Present`. `lines.rs` turns segments into
quads, and a bar is quads. `frame_hook.rs` is a per-`Present` clock that `ds2-input-harness`
already rides.

What stops a second feature using it today: it is private to the invasion-path crate, it is only
installed when `[invasion_path]` is on, and the clock seam holds one consumer, last writer wins
(read in source, `frame_hook.rs`).

Limits: no text. The pipeline draws coloured triangles and there is no font, so the bar is a bar,
not a percentage or a phase name. And the probe device is built at the entry point, before the
game's own; what that costs the boot has not been measured.

### (b) The game's own process window or a game gauge

Looked for, not found:

* `FeSubStateProcessWindowBase` is a wait window, not a gauge. Its enter calls
  `show_process_window(ui, caption, 0, 1)`; its state is a minimum-display floor at `+0x10`, an
  elapsed timer at `+0x14`, a phase at `+0x20`, and slot 10 answers only "still working" (read in
  source, `ds2_rva::FE_PROCESS_WINDOW_*`). There is no fraction to drive.
* `ds2-dialog-skip` hides these windows during the title on purpose (read in source,
  `docs/DS2-TITLE-FLOW.md`). Driving one would undo a shipped feature.
* The only gauge element the repo has named is `FeSceneEnemyHpGuage`, an in-world enemy bar
  (read in source, `ds2_rva::HP_GAUGE_*`); nothing places it on the title.
* `FeOperatorNowLoading` exists from `GameManagerImp`'s init, but its fields have never been
  examined (`docs/DS2-FRAME-COUNTER.md`), and the last attempt to drive an operator screen read a
  scene sequence id as an operator slot and crashed the game (read in source, the correction in
  `ds2_rva::FE_GROUP_CLOSE`'s doc comment).
* Any front-end widget needs the menu archive `17.febnd.dcx`, which the title operator only
  requests in its state 1 (read in the binary, `docs/DS2-BOOT-WORK.md`). So a game-drawn bar would
  cover less of the boot than (a), not more.

Rejected.

### (c) A borderless window of our own, before the swap chain

A top-level popup created on a thread of our own from the Arxan callback, painted with GDI, and
destroyed on the first `Present`. It is the only option that can show anything between the entry
point and graphics init.

Against it (all inferred; none of it has been tried here): under Proton every Win32 top-level
window becomes a separate window for the compositor, so where it lands is the compositor's choice
and not the game's monitor unless it is placed by hand; it needs a message pump of its own; it
must not take focus from the game window being created underneath it; and all it buys is the span
from the entry point to graphics init in the table above, after which the overlay takes over. It is also the one option that puts a visible
artifact on the desktop if the game dies during startup.

Kept as a follow-up only if the bar appearing at the first frame turns out to be unacceptable.

### Recommendation: (a)

Extract the overlay, add the bar as a second consumer, and read the boot state per frame.

```text
crates/ds2-overlay/              new rlib, moved out of ds2-invasion-path without behaviour change
  src/lib.rs                     install(), register_layer(fn(&mut Frame)), first-frame log line
  src/present.rs                 the probe and the Present detour         (render.rs)
  src/pipeline.rs                shaders, buffers, state save and restore (render.rs)
  src/vertex.rs                  Vertex, push_segment, push_rect          (lines.rs)
  src/clock.rs                   the per-Present clock, now a fixed table of consumers (frame_hook.rs)
crates/ds2-loading-bar/          new rlib
  src/lib.rs                     LOG_PREFIX, install(Request)
  src/weights.rs                 host: the per-profile tables, each row with its source
  src/progress.rs                host: the function above
  src/replay.rs                  host: the recorded boots, as test data
  src/sample.rs                  windows: the per-frame read chain, through safe reads
  src/draw.rs                    windows: quads through ds2-overlay, gap closing, hide on input wait
crates/ds2-loader/src/loading_bar.rs   [loading_bar] enabled, default off; profile from [seamless], [offline], [intro_skip]
crates/ds2-rva                   FE_OPERATOR_TITLE_FLOW_OFFSET for the +0x38 above
scripts/frida/loading-bar-signals.js   the measurement below, before any DLL
scripts/ds2-run.py               --loading-bar
```

Work items, in order:

1. **Frida first.** An agent that counts `Present` calls and records, per frame, whether
   `GameManagerImp` and the title operator exist, the operator state and the resident substate id,
   with a timestamp. Run it with `--allow-early --boot-timeline` on the phase-mark branch. It
   answers the open questions below without a build.
2. Name the flow offset in `ds2-rva`.
3. Expose the `DllMain` origin from `ds2-boot-timeline` (or move it to `ds2-game-base`) so a second
   crate can read elapsed time without the timeline being on.
4. Extract `ds2-overlay`. Invasion-path and the input harness become its first consumers; run
   `--invasion-path --invasion-path-on` to show the arrows still draw before anything else lands.
5. Make the clock and the draw list multi-consumer: a fixed table, no allocation in `Present`.
6. `ds2-loading-bar`: weights, progress and replay on the host first, then sampling and drawing.
7. Publish the shown-or-suppressed decision from `ds2-dialog-skip` as the input-wait flag.
8. Loader switch and launcher flag, off by default until a run has shown the bar and a clean frame
   after it.
9. A run per profile with `--boot-timeline`, replacing the derived and stand-in rows of the weight
   table with measurements from one build.

## Test plan

### Host

`progress.rs` and `weights.rs` are plain arithmetic and run under `cargo test` on Linux. Each
recorded boot becomes a replay: a list of `(time, step)` events fed to the function at a simulated
frame rate, checking after every frame.

| replay | events | source |
| --- | --- | --- |
| online, before the floor lift | the cold-boot chain through `0x44 0x46 0x47`, both runs' dwells | measured, `docs/DS2-BOOT-WORK.md`, 2026-08-27 |
| online, floor lifted | same chain, `0x05` at 114.5 and `0x44` at 1010.4 | measured, same doc |
| engine block, early milestones | entry point, `dinput8-create`, first substate | measured, the run-4 table in the same doc |
| pre-flow phases, default Seamless | the `e216b7e` phase table | measured, the draft PR table |
| slow outlier | first substate at 8116 and 8927 ms instead of about 4200 | measured, the boot-partition notes of 2026-09-26 |
| Seamless path | `0x39 0x3a 0x2a 0x47` | measured ids, the game directory's loader logs; timings from the weight table |
| offline path | `0x39 0x3e 0x2a 0x47` | measured ids, `docs/DS2-OFFLINE.md` |
| offline setting persisted | `0x37 0x2a 0x47`, skipping the login | read in source, `0x37`'s phase-3 edge |
| login retry | `0x39 0x3e 0x39 0x3e 0x2a 0x47` | the cancel edge read live in `docs/DS2-OFFLINE.md`; the loop is constructed |
| unknown id | an id outside the static id space, mid-flow | constructed |
| after completion | `0x47 0x55 0x57 0x6b`, and `0x00 0x17` on a later return | measured, `docs/DS2-CONTINUE.md`; read in source for the return |

Properties checked on every replay, every frame:

* `shown` never decreases.
* `shown` stays below `hi(s)` until the id after `s` arrives, and below `1.0` until completion.
* `shown` strictly increases on every frame while a step is resident and the bar is not complete
  or hidden, including a step that runs far past its weight (the slow outlier).
* Entering an off-path or re-entered id changes `shown` by nothing on that frame.
* The per-frame change of the displayed value never exceeds the renderer's gap-closing rate.
* After completion nothing moves it, and the return to title does not restart it.

Plus a generated run: random sequences over the whole static id space with random dwells, checked
against the same properties, so an edge nobody recorded is covered by the rules rather than by a
row.

`weights.rs` gets its own checks: every profile's floors are strictly increasing along its path,
end at `1.0`, and every row names a source.

### In the game

A run with `--loading-bar --boot-timeline` on each profile. The bar's lines and the timeline's
lines land in the same `ds2-loader.log`, which is the cross-check:

```text
ds2-loading-bar: config enabled=true profile=seamless
ds2-overlay: Present hooked at 0x... consumers=loading-bar
ds2-loading-bar: first-frame t=<ms> shown=<fraction> op=<null|state> id=<none|0x..>
ds2-loading-bar: operator state=<1..4> t=<ms> shown=<fraction>
ds2-loading-bar: step id=0x05 path=on lo=<f> hi=<f> entered=<f> t=<ms>
ds2-loading-bar: step id=0x3a path=off entered=<f> t=<ms>
ds2-loading-bar: complete id=0x47 t=<ms> frames=<n> backwards=0 max-frame-step=<f> longest-still=<frames>
ds2-loading-bar: hidden reason=complete
```

What the run has to show:

* A `step` line for every `enter` the timeline logs between the first substate and `0x47`, in the
  same order. An id the timeline saw and the bar did not is a per-frame read that missed a step,
  and the bar logs it as `missed id=` when the next id skips over it.
* `backwards=0` on the completion line, counted by the renderer rather than asserted.
* `longest-still` small outside a hidden span: the number of consecutive frames the drawn value did
  not move.
* The timeline's `flow ... on-boot-thread=true`, which is the unproven half of the same-thread
  claim.
* The frame after `hidden reason=complete` renders the menu with no overlay state left bound, the
  same property invasion-path's restore already has to hold.

## Open questions, each answered by work item 1

* Does the game present any frame between graphics init's first presents and the first app frame?
  If not, the bar should not appear until the first app frame.
* When does the title operator first become reachable, relative to the phase marks?
* How the first-frame-to-first-substate span splits across operator states 1, 2 and 3 with the
  fade finished. The weight table carries them as one row until then.
* The dwell of `0x39` on the `seamless` and `offline` profiles on the current build.
* What building the probe swap chain at the entry point costs the boot.
