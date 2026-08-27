# The DS2 boot flow, and where an intro skip would cut into it

Everything below was read statically from `darksoulsii-deobf.bin` (SOTFS build 9527516) with
`scripts/ds2-rtti.py`, `scripts/ds2-xrefs.py` and `objdump`. No game was launched. Tracking
issue: `ds2-mods-rs-3rr`.

## The screens are not videos

`Game/movie/` holds exactly one file, `prologue.wmv` (516 MB). That is the attract-loop
cinematic the title menu plays when left idle, reached through `FeSubStateTitlePrologue` -- it is
not a boot logo. Replacing or truncating movie files, the usual trick for this genre of mod,
therefore does nothing here. Every boot screen is rendered in-engine.

## The flow is a state machine, and it is symbolized

`FeStateTitle` owns a family of substates, all deriving `FeSubStateBase`, all sharing one 8-slot
vtable shape. DS2 carries 5271 MSVC RTTI type descriptors, so finding them is a string search
rather than an inference:

| class | vtable |
| --- | --- |
| `FeSubStateBase` | `0x1410bcfa8` |
| `FeSubStateWarningNoCopy` | `0x1410bd9a8` |
| `FeSubStateTitleLogo` | `0x1410bd958` |
| `FeSubStateTitleUserPolicy` | `0x1410bd2c8` |
| `FeSubStateTitleInitBranch` | `0x1410bd908` |
| `FeSubStateTitleMain` | `0x1410bd9f8` |
| `FeSubStateTitlePrologue` | `0x1410bdba8` |

There are 24 `FeSubStateTitle*` classes in total; `scripts/ds2-rtti.py` will locate any of them.

## What the vtable slots mean

Read from the `FeSubStateBase` implementations, not guessed from names:

| slot | meaning | base implementation |
| --- | --- | --- |
| v0 | destructor | `0x1400f7210` |
| v1 | enter | `0x1400f72b0`, empty `ret` |
| v2 | leave | `0x1400f8970`, empty `ret` |
| v3 | update, per frame | `0x1400f8990`, empty `ret` |
| v4 | unused | `0x1400f89a0`, empty `ret` |
| v5 | debug registration, publishes the phase field | `0x1400f8950` |
| v6 | debug walk over what v5 registered | `0x1401043a0` |
| v7 | bool query, default false | `0x1400f72c0`, `xor al,al; ret` |

Four of the eight are empty in the base, so **any override is that substate's real logic**. v7 is
overridden by exactly one class, `FeSubStateTitleStartIngame`, which is why it is not the
"am I finished" query it superficially resembles.

`FeSubStateTitleInitBranch` overrides only v0, v1 and v5 -- no update, no leave. It is a pure
branch decision taken on entry.

## `FeSubStateTitleLogo`, fully traced

Object layout, read from its own code rather than from a struct definition:

| offset | meaning |
| --- | --- |
| `+0x18` | scene reference pointer |
| `+0x20` | phase, 1 to 4 |
| `+0x24` | float, hold duration |
| `+0x28` | float, elapsed timer |
| `+0x2c` | bool |

`FeSubStateTitleLogo::v3` (update) at `0x1400febf0` switches on the phase:

- **phase 1** (`0x1400fed10`) plays sequence `0x67`, sets phase 2, zeroes the timer.
- **phase 2** (`0x1400fec9b`) accumulates the frame delta into `+0x28` and compares it against
  `+0x24`. Below the threshold it returns and waits. At the threshold it plays sequence `0x68`,
  sets phase 3, zeroes the timer.
- **phase 3** (`0x1400fec14`) asks whether sequence `0x68` is still playing. If yes it returns.
  If no it sets phase 4.
- **skip path** (`0x1400fec6c`), reached from phases 2 and 3, writes `1` to a global at
  `[0x14160de10]+0x568`, plays sequence `0x66`, sets phase 4.

**Phase 4 is terminal**, and three independent paths reach it. The third is `Logo::v1` (enter) at
`0x1400fd980`, which sets phase 4 immediately when the scene pointer at `+0x18` is null -- the
game's own shipped "there is no logo to show" path. That is the single most useful fact here: a
transition to phase 4 is something the game already does to itself.

### The player's skip is an input poll

Phases 2 and 3 both test bit 4 of a state word reached through the singleton at `0x1416751f8`.
That is the single button press that accelerates a scene. It sets the `+0x568` global, and
`Logo::v1` reads that global on entry to decide whether to start at phase 1 or phase 2 -- which
is precisely why pressing the button once makes the *following* screens shorter but does not
remove them.

## The boot-once flag

`0x14160de1a` is touched by exactly three instructions in the whole image (`scripts/ds2-xrefs.py`):

- `0x1400fd932`, a read, and `0x1400fd938`, a write of `1` -- both inside
  `FeSubStateTitleInitBranch::v1` at `0x1400fd930`, which captures the previous value into the
  substate's `+0x10` and then latches the global to `1`.
- `0x1400febe0`, a write of `0`, inside `FeSubStateWarningNoCopy`'s leave.

So its only consumer is `InitBranch`'s own `+0x10`. It is worth understanding but it is not, on
this evidence, a master switch for the logo chain.

## Where to cut

In preference order:

1. **Hook each unwanted substate's v1 (enter), call the original, then write the terminal phase.**
   This reuses a transition the game performs on itself and invents no new state.
2. Zero the hold duration at `+0x24`. Only shortens the scenes; they still appear.
3. Drive the input poll so every scene self-skips. Reproduces the manual behaviour exactly, but
   the poll is shared input plumbing and hooking it reaches far beyond the title flow.

## The blocking unknown

**Nothing found so far reads phase 4 to advance the flow.** `FeStateTitle`'s own virtuals
(`v1=0x1400f72a0`, `v2=0x1400f8960`, `v3=0x1400f8980`) are empty stubs, so the sequencer lives
elsewhere -- `FeOperatorTitle` is the obvious candidate. Until the code that polls the phase field
and selects the next substate is found, "force phase 4" is an inference about what 4 means to the
owner, not a measurement. Find that first.

Related: the terminal phase value is `FeSubStateTitleLogo`'s. Do **not** assume 4 is terminal for
`FeSubStateWarningNoCopy` or `FeSubStateTitleUserPolicy`; each one's update must be read the same
way.

## The caveat that governs every address here

These addresses come from the deobfuscated image, which is not the byte stream that runs. At the
286 Arxan-redirected functions the deobf image shows recovered code where the live process has a
stub. Vtable slots are data and are trustworthy; the function *at* a slot must be checked against
the Arxan set before it is detoured. See `docs/ARXAN-FOOTPRINT.md`.

## The skip, as built (`ds2-mods-rs-3rr`)

`crates/ds2-intro-skip`, **on by default**. `[intro_skip] enabled = false`, or
`ds2-run.py --no-intro-skip`, turns it off.

The off switch is the part worth keeping, not the default. This patches executable memory in
three places during startup, so if a run ever fails to boot, ruling this feature out has to cost
one edited line rather than a rebuild and a re-stage. A default that cannot be switched off is a
default that cannot be ruled out. A misspelled value leaves the feature ON, which is the harmless
direction: only an exact `false` disables it.

It detours each screen's `enter` (vtable slot 1), lets the original run, then writes that class's
terminal phase. **Every one of the three already has a shipped path where `enter` does exactly
that and returns**, so the transition is one the game performs on itself:

| class | `enter` RVA | phase field | terminal | the game's own path to it |
| --- | --- | --- | --- | --- |
| `FeSubStateWarningNoCopy` | `0x000fded0` | `+0x10` | 4 | a virtual on the `0x1416751f8` singleton returns nonzero |
| `FeSubStateTitleLogo` | `0x000fd980` | `+0x20` | 4 | the scene reference at `+0x18` is null |
| `FeSubStateTitleUserPolicy` | `0x000f9040` | `+0x10` | 4 | the persisted `[sys+0x136d]` flag is set |

**The phase offset is per class and is not a base-class field.** Logo keeps its phase at `+0x20`;
the other two keep theirs at `+0x10`. Each was read from the field that class's own `update`
switches on. One offset applied to all three would put a `4` into an unrelated member of two of
them, and nothing would report that it had.

The consumer of the terminal phase was never located, and does not need to be: a player pressing
the skip button reaches phase 4 by the game's own path and the flow advances, every time. That is
the evidence that something reads it.

### Why the original `enter` still runs

Two of the three shipped paths return before touching their scene, so skipping the original
entirely is tempting and would give a cleaner "nothing at all". It is not done because `leave`
(slot 2) closes the scene reference `enter` opened, and a close with no matching open is an
unbalanced lifetime on an object this crate does not own. Letting `enter` run keeps them
symmetric and costs at most one frame.

### First run, measured

`--intro-skip`, build 9527516, Proton Experimental 11.0-100:

```
ds2-intro-skip: install installed=3/3
ds2-intro-skip: skipped screen=warning-no-copy phase-offset=0x10 value=4 count=1
ds2-intro-skip: skipped screen=logo           phase-offset=0x20 value=4 count=1
ds2-intro-skip: skipped screen=logo           phase-offset=0x20 value=4 count=2
ds2-intro-skip: skipped screen=logo           phase-offset=0x20 value=4 count=3
```

The game reached the title and stayed up, with no crash artifacts written.

**`FeSubStateTitleLogo` is entered three times** -- there are three logo screens in sequence, not
one, which is why a player pressing skip has to press it once per screen.

`user-policy` never fired: that screen is not reached on a profile that has already accepted the
policy, because the game's own `[sys+0x136d]` early-out takes it first. The hook is installed
either way, which is the right shape -- a screen that does not occur costs nothing, and a fresh
profile that does show it is covered.

The fire counts are logged precisely so "no logo appeared" and "the hook never installed" cannot
be confused for one another from the outside.

## The message boxes after the boot screens (`ds2-mods-rs-j3b`)

Skipping the three boot screens does not reach the title menu. The flow then stops on message
boxes that wait for a button, so the intro skip on its own just moves the button presses later.

Everything below was read from `darksoulsii-deobf.bin` with `scripts/ds2-disasm.py`. No game was
launched to establish any of it.

### Six classes, one `update`

Scanning every RTTI-described vtable in the image for slot `v3` finds exactly six classes sharing
`FeSubStateCommonWindowBase::v3` at `0x140105150`:

| class | vtable | `v1` enter | slot 8 | slot 9 |
| --- | --- | --- | --- | --- |
| `FeSubStateCommonWindowBase` | `0x1410bdde8` | shared | inert | inert |
| `FeSubStateCommonWindow` | `0x1410bcff8` | shared | inert | inert |
| `FeSubStateOfflineModeWindow` | `0x1410bd388` | shared | inert | inert |
| `FeSubStateTitleOnlineCheckFailWarn` | `0x1410bd7d8` | `0x1400fd370` | inert | inert |
| `FeSubStateTitleInformationFailWarn` | `0x1410bd848` | `0x1400fd230` | inert | inert |
| `FeSubStateTitleDeleteProfile` | `0x1410bd6c8` | shared | **`0x1400fcf30`** | inert |

"inert" means the slot still holds `FeSubStateCommonWindowBase`'s own implementation, which is
`ret 0` and nothing else (`0x1400f89d0` for slot 8, `0x1400f89c0` for slot 9).

This is a **different base class from the boot screens above, with a different layout.** These
dialogs keep their phase at `+0x30`; `FeSubStateWarningNoCopy` keeps its at `+0x10` and
`FeSubStateTitleLogo` keeps its at `+0x20`. Nothing about the boot-screen trace transfers here.

### Object layout, read from `v1` and `v3`

| offset | meaning | where it was read |
| --- | --- | --- |
| `+0x10` | WORD, caption/message id | `v5` registers it at `0x140104f69` |
| `+0x12` | **signed** WORD, option count | `cmp WORD PTR [.. +0x12], 0` + `jl`, in both `v1` and `v3` |
| `+0x14` | float, auto-close timeout (0 = none) | `0x1401051e5` |
| `+0x18` | float, elapsed | `addss xmm1, [rcx+0x18]` at `0x1401051a2` |
| `+0x20`, `+0x28` | option strings, null → defaults `0x64`/`0x65` | `v1` at `0x140104e13`/`0x140104e28` |
| `+0x30` | byte, phase | `movsx edx, BYTE PTR [rcx+0x30]` at `0x140105161` |
| `+0x31` | byte, result | `cmp BYTE PTR [rbx+0x31], 2` at `0x14010518b` |

`v1` ends with `mov WORD PTR [rdi+0x30], 1` -- a **16-bit** store that sets the phase to 1 and the
result to 0 in one instruction. That is what establishes that the two are adjacent and in that
order, rather than two offsets that happen to have been read separately.

`update` takes a second argument: `addss xmm1, [rcx+0x18]` runs against an XMM1 the function never
initialises, so the signature is `void update(this, float delta)` with the delta in XMM1.
`FeSubStateTitleLogo::v3` does the same thing to its own `+0x28`, which is what makes this the
family's signature rather than a quirk of one function. **A detour that declares only `this` is
free to clobber XMM1 and the dialog's timer then accumulates garbage.**

### What a button press actually does

`v3`'s phase-1 branch, in full:

```text
this->elapsed += delta;
if (input_pressed(ui)) {
    if ((int16)this->options < 0)     this->result = 1;   // one-button box
    else if (highlighted is confirm)  this->result = 2;
    else                              this->result = 1;
} else if (this->timeout > 0 && this->elapsed >= this->timeout) {
    this->result = 1;                                     // shipped auto-close
}
switch (this->result) {
    case 0: return;                                       // still waiting
    case 1: this->vtable[8](this); break;
    case 2: this->vtable[9](this); break;
}
close_window(ui); this->phase = 2;
```

**The dispatch reads the result byte and nothing else about the press.** So writing `+0x31` is not
an approximation of a button press, it is the button press minus the polling -- the close, the
animation, the phase transition and the handler call all remain the game's own code. Phase 2 then
waits on the window's own close check before moving to 3 or 4, so the close animation is respected.

The shipped auto-close path is worth noting separately: a dialog with a positive `+0x14` closes
itself with no press at all. Closing one without a press is therefore a behaviour the game already
has, not one invented here.

### Where to cut, and the two locks

`crates/ds2-dialog-skip`, **on by default**. `[dialog_skip] enabled = false`, or
`ds2-run.py --no-dialog-skip`, turns it off -- separately from `[intro_skip]`, so a boot failure
can be pinned on one feature without rebuilding either.

One detour, on `FeSubStateCommonWindowBase::v3` (RVA `0x00105150`). Not Arxan-redirected
(`scripts/ds2-arxan-chain.py` terminates at hop 0), `0xed` bytes long per `.pdata`, and its first
instruction `48 89 5c 24 08` is exactly five bytes -- the same trivial MinHook case as the already
proven `ARXAN_PROBE_HOOK_SITE`.

The detour writes the result and lets the original run in the same call. The value is **computed
from the object, not chosen**: `(int16)this->options < 0` means a one-button box, where the game
itself only ever produces `1`; otherwise `2`. A constant would be right for one kind of box and
silently wrong for the other.

**`FeSubStateTitleDeleteProfile` is why this is not "answer every common window".** It shares the
same update and is reached exactly when a player has asked to delete a save profile. Two
independent conditions must hold before anything is written:

1. an **allowlist of vtables** -- the three boot dialogs and nothing else;
2. a **runtime inertness check** on the object's own vtable: slots 8 and 9 must still be the base
   class's `ret 0` stubs.

The second is the one that carries the weight, because it is a property of the bytes in front of
the code rather than a belief about a class name. `DeleteProfile` overrides slot 8 and fails it on
its own merits, allowlist or no allowlist. Any common window that is seen and not answered is
logged once by vtable address with the reason, so the set of dialogs that actually appear at boot
is measured across runs instead of assumed.

### What the three allowlisted dialogs are

* `FeSubStateTitleOnlineCheckFailWarn` and `FeSubStateTitleInformationFailWarn` both come from
  `..\..\Source\Frontend\Operator\Title\FeSubStateServerFailWarn.cpp` -- the source path is still
  in the image at `0x1410bd8b0` -- and each formats that path with an error code (`0x35b62` and
  `0x33453`) into the message it shows. They are network-failure boxes.
* `FeSubStateOfflineModeWindow` is the "playing offline" notice; its slot-11 getter at
  `0x1400f9800` picks text `0x67` or `0x65` from category `0x19` depending on `[0x14160de10]+0x56b`.

All three have inert handlers, so answering them closes them and does nothing else.
