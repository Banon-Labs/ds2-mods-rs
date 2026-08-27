# M1: does a MinHook detour survive Arxan in DARK SOULS II?

**Nothing here has been run.** This document describes an experiment that is built, gated and
dry-run, and whose two arms are waiting for someone to execute them. Until both have run, every
plan in this repo that involves a hook is unproven.

## The question

DS2 carries 48 Arxan stubs and 286 Arxan-redirected functions (`docs/ARXAN-FOOTPRINT.md`).
MinHook works by rewriting a function prologue in `.text`, which is precisely the thing an
integrity check exists to notice. Nobody has hooked anything in this game. So:

> If we patch a prologue and leave it patched, does anything put it back?

## Why a hit counter cannot answer it

The obvious experiment is to detour a hot function and count the calls. It does not work. A
counter reading zero is equally consistent with:

* Arxan reverted the patch, and the original function has been running ever since; and
* nothing touched the patch, and that function was simply never called.

Those two point in opposite directions, and no amount of waiting separates them. Worse, Arxan
could corrupt the **trampoline** while leaving the hook site pristine -- the counter would go
quiet, the site would look perfect, and the natural reading ("it stopped being called") would be
wrong in a way that looks like data.

So the probe reports four things, and it is the combination that is evidence:

| # | measurement | what it rules out |
| - | --- | --- |
| 1 | hit counter, incremented inside the detour | "the detour is not running" |
| 2 | hook-site bytes, re-read every second | "the function was never called" -- works even at zero hits |
| 3 | trampoline bytes, re-read every second | "the site is clean so the hook is fine" |
| 4 | which A/B arm ran | "the hook survived, therefore Arxan is harmless" |

## The two arms, and why one is not an experiment

Suppose the probe runs with `neuter_arxan` having patched all 48 stubs, and the detour survives
an hour. That result is compatible with two different worlds -- *Arxan would have reverted it and
dearxan is load-bearing*, or *Arxan never looks at this page and dearxan was irrelevant here*.
One run cannot tell them apart.

```bash
python3 scripts/ds2-run.py --probe neuter        # arm A: dearxan neuters Arxan first
python3 scripts/ds2-run.py --probe skip-neuter   # arm B: Arxan's 48 stubs left live
```

| | `DS2_ARXAN_PROBE` | `DS2_ARXAN_PROBE_SKIP_NEUTER` | logs `arm=` |
| --- | --- | --- | --- |
| arm A | `1` | unset | `neuter-arxan` |
| arm B | `1` | `1` | `skip-neuter-arxan` |

Arm B does **not** simply drop the `neuter_arxan` call and install from `DllMain`. That would
change two variables at once -- the Arxan patching *and* the moment the hook goes in. It calls
`dearxan::disabler::schedule_after_arxan` instead, which is the same scheduling machinery
`neuter_arxan` is built on (`neuter_arxan` is "patch the stubs before the Arxan entry stub" plus
"report after it"; arm B is the second of those two alone). Both arms therefore install at the
same point in the CRT entry sequence, on the same thread, with the same SteamStub handling.
Exactly one thing differs.

### Reading the pair

| arm A | arm B | conclusion |
| --- | --- | --- |
| survives | survives | Arxan never threatened this site. dearxan is not load-bearing *for hooking here*. |
| survives | reverted | Arxan reverts hooks and **dearxan is required**. The strongest possible result for the loader design. |
| reverted | reverted | Something reverts the patch that dearxan does not disable. Investigate before building anything on hooks. |
| reverted | survives | Incoherent. Suspect the run, not the game -- check the arms were what they claimed. |

## The hook site

**RVA `0x00832e70`** (`ds2_rva::ARXAN_PROBE_HOOK_SITE`), resolved as `module_base + RVA` at
runtime because `DllCharacteristics` is `0x8160` and the loader may relocate the image.

* 2052 static call sites -- rank 3 in the binary, so a detour that never fires is a real signal.
* Prologue `48 89 5c 24 08`, one 5-byte instruction: MinHook's trivial relocation case.
* `0x47` bytes long; not one of the 286 Arxan-redirected functions.

The top two functions by call count, `0x00832cb0` (12401 sites) and `0x00c2c9e0` (4866), are
**both Arxan-redirected** and are recorded in `ds2_rva::ARXAN_REDIRECTED_DO_NOT_HOOK` so nobody
rediscovers them and hooks one. Patching over Arxan's own `e9` would make the experiment fail for
a reason unrelated to the question.

The probe reads the prologue before it writes anything. If those five bytes are not the recorded
ones, it declares the run `VOID` and patches nothing -- a function someone else already patched
measures that person, not Arxan.

## The detour is hand-written assembly, deliberately

Nobody knows this function's signature; it was chosen for its call count and its prologue. A Rust
detour would have to *declare* one, and declaring it wrong is not cosmetic -- arguments past the
fourth live on the caller's stack at offsets relative to the return address, so a Rust detour
that builds its own frame silently hands the original function different stack arguments than its
caller passed. Float arguments and float returns go the same way.

```asm
lock inc qword ptr [rip + HIT_COUNT]
jmp  qword ptr [rip + TRAMPOLINE]
```

Fourteen bytes, no prologue, no register clobbers, flags only -- and flags are volatile at a
function entry under every x64 convention. The original function cannot tell it was called
through us. `lock` is deliberate: `hits=0` versus `hits=1` is the single most important bit this
experiment produces, and a plain `inc` can lose increments under contention.

## The log lines

Everything is written to `<Game>/ds2-loader.log`, one timeline, so a divergence at *t*=41s can be
lined up against what the loader said at *t*=3s.

```
ds2-loader: attach awaiting-arxan-callback probe=on arm=neuter-arxan DS2_ARXAN_PROBE="1" DS2_ARXAN_PROBE_SKIP_NEUTER=<unset>
ds2-loader: arxan status=ok detected=true blocking_entrypoint=true
ds2-probe: install arm=neuter-arxan base=0x0000000140000000 rva=0x00832e70 va=0x0000000140832e70
ds2-probe: install original=[48 89 5c 24 08 ...] expected=[48 89 5c 24 08] prologue-match=true
ds2-probe: install minhook=ok trampoline=0x... patched=[e9 ...] site-jmp=true
ds2-probe: install trampoline-baseline=[...]
ds2-probe: watching arm=neuter-arxan poll=1.0s heartbeat=10.0s site-window=16 trampoline-window=64
ds2-probe: heartbeat uptime=10.0s arm=neuter-arxan hits=48213 site=intact tramp=intact site-diverged=0 tramp-diverged=0
```

**The heartbeat is the line to read.** All four measurements are on it, so a verdict is the last
heartbeat in the file rather than a reconstruction from scattered lines.

A state change is logged the instant it is seen, with the bytes:

```
ds2-probe: SITE  uptime=41.0s arm=... state=DIVERGED prev=intact hits=... va=0x... expected=[...] observed=[...]
ds2-probe: TRAMP uptime=41.0s arm=... state=DIVERGED prev=intact hits=... addr=0x... expected=[...] observed=[...]
```

`state` is one of `intact`, `DIVERGED` or `UNREADABLE` -- the last meaning `ReadProcessMemory`
refused the range, which is a different fact from "the bytes changed" and is not reported as one.
The pollers read through `ds2_game_base::mem::read_bytes` precisely so that a page Arxan had torn
down returns `false` instead of faulting: a crashed game destroys the evidence.

Refusals are explicit, and there is a line for each:

```
ds2-probe: VOID prologue-mismatch va=0x... -- this function was already patched before the probe touched it, ...
ds2-probe: install-failed stage=MH_CreateHook status=MH_ERROR_UNSUPPORTED_FUNCTION
```

`stage=` is one of `module-base`, `read-original`, `MH_Initialize`, `MH_CreateHook`,
`MH_EnableHook`, `read-patched`, `read-trampoline`, `spawn-poller`.

### Crash versus clean exit

```
ds2-probe: detach uptime=612.4s arm=... hits=... site=intact tramp=intact site-diverged=0 tramp-diverged=0
```

`DLL_PROCESS_DETACH` fires on `ExitProcess` and **not** on `TerminateProcess`. So this line
present means the process wound down normally; absent, with the log ending at a heartbeat, means
it did not -- a crash, a kill, or a Wine teardown that skipped the notification. That is weaker
than a crash handler and is stated weakly on purpose: it is the difference between "closed" and
"died", not a cause of death. It is written with a plain append rather than through
`ds2_game_base::log`, because that module takes a process-wide mutex and a thread killed while
holding it would hang the game on exit.

## Running it

```bash
bash scripts/check.sh
cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader
python3 scripts/ds2-run.py --selftest              # tailer + contract + verdict logic
python3 scripts/ds2-run.py --dry-run --probe neuter
python3 scripts/ds2-run.py --probe neuter
python3 scripts/ds2-run.py --probe skip-neuter
```

The script prints a verdict block assembled from the DLL's own lines and from nothing else, and
**reads the arm back out of the log** to compare against the one requested. That guard is not
decoration: the probe variables reach the game through the same channel as `WINEDLLOVERRIDES`,
which a Steam client that was already running does not propagate. A run that lost them entirely
installs no probe and is caught by the missing install line -- but a run that lost only the skip
variable would quietly execute the *other* arm and produce a perfectly well-formed verdict for an
experiment nobody asked for. Exit code `4` means no verdict, for any of these reasons.

Exit `0` means the experiment **ran**, whether or not the detour survived. A reverted hook is a
successful experiment with an unwelcome answer, and exiting non-zero on it would train whoever
reads this into treating the real finding as a tooling failure.

## What this will not establish

* Anything past the observation window. `--observe` defaults to 180s; the verdict is scoped to it.
* That every Arxan check ran during the window. Some may be triggered by events a short session
  never reaches.
* Anything about any hook site other than RVA `0x00832e70`. Arxan covers 286 functions; a clean
  result here says nothing about hooking one of those, which is why they are named and excluded.
* Anything about hooking *several* functions, or about a hook installed later than the entry
  point. The probe installs one hook, once, at the earliest safe moment.
