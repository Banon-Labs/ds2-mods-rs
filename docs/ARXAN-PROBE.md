# M1: does a MinHook detour survive Arxan in DARK SOULS II?

## ANSWERED 2026-08-27: yes, with Arxan live

Both arms ran on build 9527516 under Proton Experimental 11.0-100. The detour survived and fired
in both, with zero divergences at either window.

| arm | Arxan | window | heartbeats | hits | site | trampoline | divergences |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `--probe neuter` | neutered by dearxan first | 121s | 12 | 9 | intact | intact | 0 / 0 |
| `--probe skip-neuter` | **48 stubs left live** | 120s | 12 | 9 | intact | intact | 0 / 0 |

Install evidence was byte-identical in both:

```
ds2-probe: install base=0x0000000140000000 rva=0x00832e70 va=0x0000000140832e70
ds2-probe: install original=[48 89 5c 24 08 57 48 83 ec 20 48 8b d9 e8 ae 01] expected=[48 89 5c 24 08] prologue-match=true
ds2-probe: install minhook=ok trampoline=0x000000013fff0fc0 patched=[e9 5e e1 7b ff 57 48 83 ec 20 48 8b d9 e8 ae 01] site-jmp=true
```

**Reading, per "Reading the pair" below: Arxan never threatened this site.** Both arms survived, so
`dearxan` is *not* demonstrated to be load-bearing for hook-byte survival. The game also did not
crash in arm B, so nothing responded to the patch by killing the process either.

Two things this does NOT license. It does not retire `dearxan`: `neuter_arxan` still runs before
the entry point and may matter for reasons other than hook bytes. And it is one site over two
120-second windows -- it does not prove every Arxan check ran during them, since checks can be
path-triggered or time-delayed. See "What this will not establish".

The image base was `0x140000000` in both runs, so RVA->VA stays 1:1 with the Ghidra image, and
`prologue-match=true` confirms the site static analysis picked is the clean MSVC
`mov [rsp+8], rbx` prologue rather than one of the 286 Arxan-redirected `e9` stubs.

`hits` sat at 9-11 across both runs rather than climbing, so the hooked function is on the init
path and fires a few more times later -- not a per-frame function.

---

The rest of this document is the experiment as designed, kept as written.

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

**Both arms run back to back with no user action between them.** That is a property of the
configuration mechanism rather than a nicety -- see [How the probe is configured](#how-the-probe-is-configured).

| | `[arxan_probe] enabled` | `[arxan_probe] skip_neuter` | logs `arm=` |
| --- | --- | --- | --- |
| arm A | `true` | `false` | `neuter-arxan` |
| arm B | `true` | `true` | `skip-neuter-arxan` |

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

## How the probe is configured

`<Game>/ds2-mods.toml`, beside the DLL, written by `scripts/ds2-run.py` before every launch and
read by the DLL itself in `DllMain`.

```toml
[arxan_probe]
enabled = true
skip_neuter = false

# poll_interval_ms = 1000
# heartbeat_interval_ms = 10000
```

### It was two environment variables, and they measurably did not work

This is recorded because it is the kind of thing that gets reintroduced by someone reasoning from
first principles about how a launcher *should* pass a flag. It was `DS2_ARXAN_PROBE=1` and
`DS2_ARXAN_PROBE_SKIP_NEUTER=1`, set in the launcher's environment. A real run produced, from the
DLL's own attach line:

```
ds2-loader: attach awaiting-arxan-callback probe=off arm=neuter-arxan DS2_ARXAN_PROBE=<unset> DS2_ARXAN_PROBE_SKIP_NEUTER=<unset>
```

`steam -applaunch` hands the request to an **already-running Steam client** over IPC, and that
client starts the game from *its* environment -- not from the one the launcher set.
`WINEDLLOVERRIDES` survives only because it lives in the per-app Steam launch options, which is a
different channel; it is also the one setting that cannot move into this file, because Wine reads
it to decide whether to map our DLL at all, before there is a DLL running to read anything.

The two available fixes -- quit Steam before every run, or edit the launch options between the
arms -- are both manual steps *between the two halves of one experiment*, and a manual step there
is a step that eventually gets skipped, producing a well-formed verdict for an arm nobody ran. A
file beside the DLL travels through no IPC. **Do not put the arm back in the environment.**

The file is read with `ds2_hotkey_config::kv`, a `key = value` reader in a strict subset of TOML.
There is no TOML dependency: `ds2-hotkey-config` has no dependencies at all, deliberately, and the
whole surface here is four scalars. Anything the reader cannot use is reported with its line
number rather than skipped.

### What is live, and what is not

**The arm is not live, and cannot be.** `enabled` and `skip_neuter` are consumed in `DllMain`,
before `DarkSoulsII.exe`'s entry point, because that is the only moment the choice between the
arms exists: `skip_neuter` decides whether dearxan patches Arxan's 48 stubs *before the Arxan
entry stub runs*, and once that has happened or not happened there is no undoing it. There is no
un-neutering a live process. Editing either key while the game is running changes nothing, and the
probe says so rather than letting it look like it worked:

```
ds2-probe: config uptime=30.0s STARTUP-ONLY-IGNORED enabled="true" skip_neuter="true" -- this run is still arm=neuter-arxan. Both are read in DllMain before the game's entry point and cannot change afterwards; there is no un-neutering a live process. Restart the game to run the other arm.
```

**The two cadence knobs are genuinely live.** They go through
`ds2_hotkey_config::reload::HotFile`, which compares the file's **text** rather than its mtime --
a Proton prefix sits on filesystems that stamp mtime to a whole second, so two edits inside one
second are invisible to an mtime watcher, which reads as "changing the file did nothing". An edit
takes effect within one poll interval and is logged:

```
ds2-probe: config uptime=42.0s RELOADED poll=1000ms heartbeat=10000ms -> poll=1000ms heartbeat=60000ms
```

| key | live? | effect |
| --- | --- | --- |
| `enabled` | **no** -- `DllMain` | whether the detour is installed at all |
| `skip_neuter` | **no** -- `DllMain` | which arm; see above |
| `poll_interval_ms` | yes | how often the two byte windows are re-read (100..=60000) |
| `heartbeat_interval_ms` | yes | how often the heartbeat line is written (1000..=3600000) |

Neither live knob changes *what* is measured: the byte windows and their baselines are fixed when
the hook goes in. That is precisely why they are safe to move mid-run and the arm is not. A value
outside its range is clamped and the clamp is logged; a value that is not a whole number is
rejected, the default stands, and the rejection is logged with the text verbatim.

`scripts/ds2-run.py` writes the two live keys **commented out**, so the DLL's own defaults are
what run. It rewrites the whole file on every launch, so an edit to a startup-only key does not
survive into the next run -- deliberately, because the arm under test has to be the arm that was
requested.

### The config is echoed back into the log before anything acts on it

A missing file and a file that says `enabled = false` produce identical behaviour and must never
produce an identical log line: one means the launcher did not write it or wrote it somewhere else,
the other means it wrote it and asked for the probe to be off. So the DLL prints the path, the
status, and every key **as written**, before it decides anything:

```
ds2-loader: config file="/.../Game/ds2-mods.toml" status=found bytes=1487
ds2-loader: config [arxan_probe] enabled="true" skip_neuter="false" poll_interval_ms=<absent> heartbeat_interval_ms=<absent>
ds2-loader: config resolved probe=on arm=neuter-arxan poll=1000ms heartbeat=10000ms (enabled/skip_neuter are STARTUP-ONLY; poll/heartbeat are live)
```

`status` is `found`, `MISSING`, `UNREADABLE` or `NO-GAME-DIRECTORY`. Anything unusable follows on
its own line, with the text quoted so it can be found in the file:

```
ds2-loader: config REJECTED line=7 text="enabled = true" -- this key was already set; the FIRST value is the one in force
ds2-loader: config UNKNOWN [arxan_probe] "enbaled" -- not a key this DLL reads; it was ignored. Known keys: enabled, skip_neuter, poll_interval_ms, heartbeat_interval_ms
ds2-loader: config REJECTED [arxan_probe] enabled="yes" -- expected `true` or `false`; using the default false
ds2-loader: config CLAMPED [arxan_probe] poll_interval_ms=5 -- outside 100..=60000; using 100
ds2-loader: config REJECTED [arxan_probe] heartbeat_interval_ms="abc" -- expected a whole number of milliseconds; using the default 10000
```

`UNKNOWN` is separate from `REJECTED` because a misspelled key **parses perfectly**: `enbaled =
true` is a valid assignment to a key called `enbaled`, so the reader cannot reject it and only the
loader knows no such key exists. Without that line the only evidence of the typo would be
`enabled` reading as `<absent>` on the echo line, which says what did not happen but not why.

Booleans are `true`/`false` and nothing else. Strict on purpose, and for the same reason the
environment version accepted only `1`: a value that silently read as "off" produces a run that
looks like the probe never reported, which is the one failure this experiment must never confuse
with a real result.

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
ds2-loader: config file="/.../Game/ds2-mods.toml" status=found bytes=1487
ds2-loader: config [arxan_probe] enabled="true" skip_neuter="false" poll_interval_ms=<absent> heartbeat_interval_ms=<absent>
ds2-loader: config resolved probe=on arm=neuter-arxan poll=1000ms heartbeat=10000ms (enabled/skip_neuter are STARTUP-ONLY; poll/heartbeat are live)
ds2-loader: attach awaiting-arxan-callback probe=on arm=neuter-arxan config=found poll=1000ms heartbeat=10000ms
ds2-loader: arxan status=ok detected=true blocking_entrypoint=true
ds2-probe: install arm=neuter-arxan base=0x0000000140000000 rva=0x00832e70 va=0x0000000140832e70
ds2-probe: install original=[48 89 5c 24 08 ...] expected=[48 89 5c 24 08] prologue-match=true
ds2-probe: install minhook=ok trampoline=0x... patched=[e9 ...] site-jmp=true
ds2-probe: install trampoline-baseline=[...]
ds2-probe: watching arm=neuter-arxan poll=1000ms heartbeat=10000ms site-window=16 trampoline-window=64
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

Both arms are runnable back to back with nothing to do in between: the launcher rewrites
`ds2-mods.toml` before each launch and the DLL reads it off disk itself.

The script prints a verdict block assembled from the DLL's own lines and from nothing else, and
**reads the arm back out of the log** to compare against the one requested. That guard was written
for environment variables that could silently vanish, and moving to a file did not retire it -- a
file has its own ways to be wrong. It can fail to be written, be written to the wrong directory (a
game directory that moved, a second install), or be left over from a previous run. A run that lost
it entirely installs no probe and is caught by the missing install line; a run against a **stale**
file would quietly execute whichever arm that file names and produce a perfectly well-formed
verdict for an experiment nobody asked for. Exit code `4` means no verdict, for any of these
reasons.

The verdict block also reprints the config file verbatim, and reports any `ds2-probe: config` line
seen during the observation window -- a file edited mid-measurement means the window is not
uniform, and that has to be visible before two arms are compared.

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
