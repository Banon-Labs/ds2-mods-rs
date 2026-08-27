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

### First run: the allowlist was wrong, and the log said so

Measured, build 9527516, Proton Experimental 11.0-100, with the three `FailWarn`/`OfflineMode`
vtables allowlisted:

```
ds2-dialog-skip: install ok rva=0x00105150 va=0x0000000140105150 dialogs=3
ds2-dialog-skip: seen screen=<not-allowlisted> vtable=0x00000001410bcff8 rva=0x010bcff8 action=left-alone
```

The three allowlisted classes never fired. The dialog that actually appears at boot is
`FeSubStateCommonWindow` -- the one left out precisely because its name sounded generic. That is
the "report, never answer" branch doing its job: an incomplete allowlist cost one log line and a
button press, not a silently auto-answered dialog.

**Answering it is safe, and the reason is three measured facts rather than a judgement about the
name:**

1. **There is exactly one such object in the game.** `scripts/ds2-xrefs.py 0x1410bcff8` finds a
   single code reference in the entire image -- the `lea r13` at `0x1400f75c1` -- and it sits
   inside `FeStateTitle::v6` at `0x1400f72e0`. That function is the title's **substate table
   builder**: an 88-slot array at `[state+8]` with its count at `[state+0x2c8]`, filled once. So
   this class is not "the generic box used all over the game"; it is one member of the title flow,
   and no in-game prompt can be an instance of it.
2. **It is a one-button acknowledgement box by construction.** Its constructor at `0x140104c00`
   runs `or eax,0xffffffff` then `mov WORD PTR [rcx+0x12],ax`, hardcoding the option count to
   `-1`. The update's input path can therefore only ever produce result `1` for it.
3. **Its handlers are inert** -- slots 8 and 9 are the base class's `ret 0`.

Its message is category `0x19` id `0x1adc0`, its caption id is `0x20`, its kind field `6`.

`FeStateTitle::v6` is also the sequencer this document previously listed as the blocking unknown.
It builds every title substate up front rather than raising them on demand, which is why nothing
in `FeStateTitle`'s other virtuals looked like a flow.

### Auto-advancing is not the same as not appearing

The first working version hooked the shared `update` and wrote the result byte. Every box answered
itself, and the log proved it -- but the box still had to be **drawn** before it could answer
itself, so the player watched dialogs flash past instead of pressing buttons. Measured:

```
ds2-dialog-skip: answered screen=common-window kind=6  caption=0x20 options=-1 timeout=0.000 result=1 total=1
ds2-dialog-skip: answered screen=common-window kind=70 caption=0x47 options=-1 timeout=0.000 result=1 total=2
```

Two useful facts fell out of those two lines. **There are exactly two boot notices**, and **they are
the same object** -- one reusable one-button box re-parameterised per message. The `kind=6`,
`caption=0x20` seen in the constructor is its construction default, not a fixed identity.

The runtime values also confirmed the static read of the constructor field for field: `kind=6` from
`mov edx,0x6`, `caption=0x20` from `lea r8d,[rdx+0x1a]`, `options=-1` from
`or eax,0xffffffff; mov [rcx+0x12],ax`, `timeout=0.000` from the zeroed `xmm6`.

### Where the cut actually belongs: `enter`

**All six classes funnel through `FeSubStateCommonWindowBase::v1` at `0x140104db0`** -- including
the two that override `v1`, which format their message and then `call 0x140104db0` (at
`0x1400fd471` in `OnlineCheckFailWarn`). That is the only place a title message box comes into
existence, so it is the only place one can be prevented rather than dismissed.

The detour returns **without calling the original** and writes result `1`, phase `3` -- the state
the game itself leaves such a box in once closed.

**Skipping an `enter` is normally the wrong shape**, and `ds2-intro-skip` deliberately does not do
it. The difference is in `leave`:

```text
if (this->phase == 1) { this->vtable[10](this); close_window(ui); }
this->phase = 0;
```

`leave` closes the window **only when the phase is 1**. So an `enter` that never ran and never
opened anything pairs with a `leave` that never closes anything. The boot screens' `leave` closes
unconditionally, which is why that crate lets the original run and only rewrites the phase
afterwards. The conditional here is what makes suppression sound, and it was read out of `leave`
rather than carried over from the sibling crate.

Measured, same two notices, now never created:

```
ds2-dialog-skip: install ok rva=0x00104db0 va=0x0000000140104db0 dialogs=4
ds2-dialog-skip: suppressed screen=common-window kind=6  caption=0x20 options=-1 total=1
ds2-dialog-skip: suppressed screen=common-window kind=70 caption=0x47 options=-1 total=2
```

### The rule that carries the safety argument

> **This mod suppresses notices. It never answers a question.**

A negative option count is the game's own marker for a one-button acknowledgement box: its input
path can only ever produce a cancel, and the closed phase it computes can only ever be 3, so
removing the box removes a keypress and nothing else. A non-negative count means a real decision
with a real affirmative -- those are **shown** and left for the player, logged
`reason=has-a-real-choice`. That converts the safety argument from a list of
class names into a property of the object in front of the code: "auto-confirmed something
consequential" becomes structurally impossible rather than merely guarded against by an allowlist.
The cost is that a genuinely two-option boot dialog would go unanswered -- and would show up as a
line to read and decide on, which is the right way to meet one.

## The last two stops: press-any-button, and the "please wait" windows

Suppressing the notice boxes still does not hand the player a menu. Two things remain, and they are
different in kind from the notices and from each other -- **neither is suppressed.**

### PRESS ANY BUTTON

`FeSubStateTitleMain::v3` (`0x1400fed90`) switches on a phase at `+0x10`. Its phase-1 branch does
three things in order:

```text
scene = [[0x14160de10]+0x80];
scene->vtable[4]();                  // tick the title scene
if (!0x1400f37f0(scene)) return;     // is the title sequence (0x67) up yet?
if (!0x1400ff420(this))  goto idle;  // was a button pressed?
... the whole of the game's own setup for the top menu ...
```

`0x1400ff420` is the press poll. It **ignores its argument** and reads globals -- the `0x1416751f8`
singleton, bit 16 then bit 4 of the word at `+0x10` of the object at its `+0x60`, falling back to
`[[+0x60]+8]+0x34 & 1`. That is the same state word `FeSubStateTitleLogo`'s skip path tests.

**It has exactly one caller in the entire image**: `0x1400fee6b`, inside that very update. Counted
by scanning every `e8 rel32` for the target and attributing each hit to its `.pdata` owner. That is
what makes detouring it a change to one gate rather than to input handling, and it is the fact the
decision rests on -- `docs/DS2-TITLE-FLOW.md` had previously ruled out driving the input poll for
the logo skip precisely because *that* poll is shared plumbing. This one is not.

So the cut is: **force the poll true, and touch nothing else.** The sequence gate above it still
holds, so the title screen still initialises normally; and the game's own phase-1 body -- which is
what builds the top menu -- runs in full. Forcing the substate's terminal phase instead would skip
that setup, which is why the phase is left alone here even though `ds2-intro-skip` writes phases for
the boot screens. Different situation, different cut.

### The "please wait" windows

`FeSubStateProcessWindowBase::v1` (`0x140104ed0`) is shared by six classes -- the two
`ProcessWindow` bases plus `FeSubStateTitleOnlineCheck`, `FeSubStateTitleGameServerLogin`,
`FeSubStateTitleSaveSystemData` and `FeSubStateTitleLoadProfile`:

```text
enter:  result = this->vtable[8]();               // STARTS THE ASYNCHRONOUS WORK
        if (result >= 0) { this->phase = 3; return; }   // nothing to do; no window at all
        show_process_window(ui, this->caption, 0, 1);
        this->timer = 0; this->phase = 1;

update: if (phase == 1) {
            this->timer += delta;
            if (this->timer < this->min_duration) return;   // +0x10, an ARTIFICIAL FLOOR
            if (this->vtable[10]()) return;                 // still working -> keep waiting
            close_window(ui); this->phase = 2;
        }
```

Layout (**different from the common windows** -- different base, phase is a DWORD at `+0x20`, not a
byte at `+0x30`):

| offset | meaning |
| --- | --- |
| `+0x0c` | kind |
| `+0x10` | float, minimum display duration |
| `+0x14` | float, elapsed |
| `+0x20` | DWORD phase: 1 showing, 2 closing, 3 done |
| `+0x24` | result |

**These must not be suppressed.** Slot 8 starts real work -- a network check, a server login, a
system-data save, a profile load -- and slot 10 is the wait for it. Skipping the substate would skip
the wait, not just the window. That is the whole reason this is treated differently from the notice
boxes, where nothing was pending.

The cut is `+0x10 = 0`: remove the floor that keeps the window up *before* the update will even ask
whether the work is done. The slot-10 wait is untouched, so the window still stays up for exactly as
long as the operation really takes, and it cannot outrun it.

### Measured, one run, no input at all

```
ds2-intro-skip:  skipped screen=warning-no-copy
ds2-intro-skip:  skipped screen=logo (x3)
ds2-dialog-skip: pressed    screen=title-main gate=press-any-button total=1
ds2-dialog-skip: suppressed screen=common-window kind=6  caption=0x20 options=-1 total=1
ds2-intro-skip:  skipped screen=user-policy
ds2-dialog-skip: shortened  screen=process-window kind=57 min-duration=1.000->0 total=1
ds2-dialog-skip: suppressed screen=common-window kind=70 caption=0x47 options=-1 total=2
```

**The floor was a full second.** That is the number that says the change was worth making rather
than merely correct: one process window, held up for 1.0s after its work was already done. The
previous value is logged for exactly this reason -- a floor shorter than the work would have shown
up here as a number explaining why nothing visibly changed.

### Four hooks, four switches

`[intro_skip] enabled`, `[dialog_skip] enabled`, `[title_skip] press_any_button` and
`[title_skip] process_windows`, with `--no-intro-skip`, `--no-dialog-skip`,
`--no-press-any-button-skip` and `--no-process-window-skip` on the launcher. Four things now patch
executable memory during startup, and the entire value of separate keys is that a run which fails to
boot can be pinned on ONE of them by editing one line. `ds2-run.py --selftest` asserts each switch
moves only its own key.

## Hiding the wait windows properly: hook the drawing, not the class

The first attempt at the wait windows hooked `FeSubStateProcessWindowBase::v1` and reproduced it
without its show call. It worked, and it hid **one** window. "Retrieving Information" kept
appearing.

The reason is structural, and it is why per-class hooking was never going to finish. Mapping every
call site of the three window-show functions to its owning vtable slot gives:

| shows | from |
| --- | --- |
| process window | `FeSubStateTitleSaveFirst::v1` |
| process window | `FeSubStateTitleSteamLoadSystemData::v1` |
| process window | `FeSubStateProcessWindowBase::v1` (six classes share it) |
| process window | a continuation chunk of `FeSubStateTitleInformation::v3` -- **its `update`** |
| process window | two further chunks, plus `FeAddProcessingMessageJob::v4` |

**`FeSubStateTitleInformation` shows its window from `update`, not `enter`.** No amount of hooking
`enter` reaches it. The classes differ, the slots differ, and the list is open-ended.

So the hook moved to the one place they all meet: `show_process_window` at `0x1404fe760`.

### Its signature, established rather than assumed

Four register arguments, **no stack arguments** -- the body reads nothing above its own frame. It
keeps RCX and forwards RDX, R8 and R9 untouched into `0x1405105f0`, which is why a detour must
carry all four even though the function looks like it uses one. All seven call sites were checked
and set exactly these four: six do `mov r9b,1; xor r8d,r8d`, and `0x1401088ae` does the mirror
`xor r9d,r9d; mov r8b,1`. None writes a fifth at `[rsp+0x20]`. They are forwarded as raw `u64` so
even the upper bits the callers leave undefined are reproduced.

Returning `0` is the function's own answer when there is nothing to draw on -- it opens
`mov rcx,[rbx+0xf0]; test rcx,rcx; jne` with `xor eax,eax; ret` on the null path -- and **no caller
uses the return value**; all seven ignore EAX and write their own phase field next. So a detour
that returns 0 without drawing is indistinguishable from a shipped path.

### The gate is the game's own flag

Hiding every process window in the game would take the in-game "Saving..." indicator with it, which
a player is entitled to see. So the detour is gated on `0x141614804`, a byte written `1` by
`FeOperatorTitle::v2` (`0x1400ef045`) and `0` by its `v3` teardown (`0x1400ef123`). The game reads
it itself at `0x140342251`, so it is real state and not a write-only leftover. That keeps the
change to the boot sequence without this mod inventing a notion of "still booting" or time-boxing
one.

### Measured

```
ds2-dialog-skip: hooked gate=show-process-window rva=0x004fe760 flag-rva=0x01614804
ds2-dialog-skip: pressed    screen=title-main gate=press-any-button total=1
ds2-dialog-skip: advanced   screen=title-main phase=2->4 total=1
ds2-dialog-skip: hidden     screen=process-window total=1
ds2-dialog-skip: suppressed screen=common-window kind=6  caption=0x20 total=1
ds2-dialog-skip: hidden     screen=process-window total=2
ds2-dialog-skip: shortened  screen=process-window kind=57 min-duration=1.000->0 total=1
ds2-dialog-skip: hidden     screen=process-window total=3
ds2-dialog-skip: suppressed screen=common-window kind=70 caption=0x47 total=2
```

**Three** wait windows, where the per-class hook had reached one. The other two came from slots that
hook could not see, which is the whole argument for moving it.

The `enter` hook stays, doing only the min-duration zeroing: a window that is invisible would
otherwise still hold its one-second floor before the flow could advance.

## The title-screen wait, and the one thing still not solved

The user's own decomposition is what made this tractable, and it corrected a misattribution: the
**three splash scenes** are one thing, and the **title screen's own logo and PRESS ANY BUTTON
prompt** are another. Time spent optimising `FeSubStateTitleLogo` for an animation that belongs to
`FeSubStateTitleMain` was time spent on the wrong function.

### Forcing the press poll alone does nothing visible

`FeSubStateTitleMain::v3` phase 1 will not even *look* for a press until
`0x1400f37f0` reports the scene's current sequence is `0x67`:

```text
scene->vtable[4]();                    // tick
if (!0x1400f37f0(scene)) return;       // <- THE WAIT IS THE ANIMATION
if (!0x1400ff420(this)) goto idle;     // the press poll
```

So the forced press was simply being taken *after* the animation finished on its own. Forcing the
gate too -- it has the same single caller -- lets phase 1 run on the first frame it is reached, and
**that removed the PAB prompt entirely** (user-confirmed).

### What the title text actually is

`FeSubStateTitleMain::v1` calls `0x1400f3e30` at `0x1400fda54`, which plays sequence **`0x66`** on
`[scene+8]`. That is the "DARK SOULS II SCHOLAR OF THE FIRST SIN" text animating in, and nothing in
the phase machine stops it.

The scene was named by measurement rather than inference: a one-shot probe read the live vptr at
`[[0x14160de10]+0x80]` and matched it against the RTTI vtable map -- **`FeSceneTitle`**, primary
vtable RVA `0x010bcab0`, with `0x010bcae0` a secondary (multiple-inheritance) vtable at `+0x18`,
which is why `[scene+0x28]` points back into the same object.

Sequence ids across the Fe scenes are `0x65`, `0x66`, `0x67`, `0x68`, from the 91 call sites of the
play forwarder `0x140afdb80`: `0x66`/`0x68` the in and out transitions, `0x67` the settled state.

### Two attempts at making it instant, both recorded as failures

1. **Call `0x140afe8a0` on the `+0x38` handle**, as phase 3 does. Retracted: that function is not a
   finish. It tail-calls `0x1409d5610`, which compares `[handle]` against a global and on mismatch
   emits a record tagged `"SMOM"` -- validation or telemetry. It returned success while nothing
   changed on screen. See `FE_SEQUENCE_NOT_A_FINISH_DO_NOT_USE`.
2. **Call `0x1400f3820` to play `0x67`**, the settled state the gate waits for. This function *does*
   do what its name says -- its body is unambiguous -- but the text animated in exactly as before.
   See `FE_SCENE_TITLE_PLAY_IDLE_INEFFECTIVE`.

Both calls were removed rather than left in on the chance they helped. Neither had a demonstrated
effect, and a mod carrying calls whose purpose cannot be shown is a mod nobody can reason about
later.

**The open question**, stated precisely so the next attempt does not start from scratch: playing
`0x67` does not replace `0x66`, so either the two run in parallel, or `0x67` has its own entry
animation, or the visible text is not driven by that sequence object at all. The next measurement
is the one this probe could not take -- `[[scene+0x28]+0x30]` was still null on the first
`FeSubStateTitleMain` update, so the player object attaches later. Probing it once the title screen
is settled would name the animation player class and expose whatever seek or rate control it has.

### What shipped

The gate forcing stays, because the outcome it produces is good on its own terms: the flow reaches
the menu as soon as the data is there, rather than pacing itself to an animation. The title text
still animates over the top of an already-interactive menu.

## The top menu: nothing is inserted, nothing is removed, and only one bit differs

Read statically from `darksoulsii-deobf.bin` with `scripts/ds2-rtti.py`, `scripts/ds2-disasm.py`
and the Ghidra MCP daemon. No game was launched to establish any of it.

**The premise this trace was started on is wrong in a useful way.** The title menu does not have a
different *set* of buttons online vs offline, or with a save vs without. It always has the **same
six rows, in the same order**. What changes is one byte per row, and that byte decides both whether
the cursor can land on the row and which sequence the row plays.

### The list is a fixed vector of six, rebuilt from scratch on demand

`FeGroupTitleTopMenu` derives `FrontendEx::FexGridControl`. Its rows come from
**`0x1400f4250`** (RVA `0x000f4250`), which fills a caller-supplied 352-byte stack buffer -- a
`DLFixedVector` of capacity **6**, element stride **0x38**, count at **`+0x158`**:

| offset | meaning | read from |
| --- | --- | --- |
| `+0x00` | label object, copied by `0x1400189f0` from `0x14001f090(.., .., text_id)` | the six copies in the builder |
| `+0x30` | **action id**, 1..6 | `mov r8d,[rax+rdx*1]` at `0x1400f4a8d`, index `cursor*0x38` |
| `+0x34` | **enabled**, one byte | `cmp BYTE PTR [rcx+rdi*1+0x34],0` at `0x1400f5063` |

Every one of the six rows is appended unconditionally. There is no branch that skips an append, and
the count is 6 on every path that does not hit the fixed-vector overflow `panic`. **Insertion and
removal are not mechanisms this menu has.**

### The six rows, and the three facts that gate them

The builder computes three booleans up front, at `0x1400f430a`-`0x1400f4355`:

* `r15b` -- **has a save**: `0x1400f0f60` walks the 10 save slots and appends `{slot, ptr}` for each
  with `[slot+0x1d9] & 1` set and `& 2` clear; the flag is `(end-begin)>>4 != 0`.
* `r14b` -- **online available**: `0x140513600(GameManagerImp[+0x22f0])`, which is nothing but
  `return *(u8*)(this+0x3a)`. This is the game's master online gate: 34 call sites, including
  `FeSubStateTitleOnlineCheck`'s own slot-8 work starter.
* `r12b` -- `!r14b`.

| row | action | text id | enabled when | on press, goes to |
| --- | --- | --- | --- | --- |
| 0 | 1 | 17010024 | always | `0x48` if **no free save slot**, else `0x4c` |
| 1 | 2 | 17010023 | a save exists | `0x55` `FeSubStateTitleLoadDataList` |
| 2 | 3 | 17010022 | online available | `0x66` `FeSubStateTitleInformation` |
| 3 | 4 | 17010021 | **not** online available | `0x20` `FeSubStateTitleSteamNetworkCheck` |
| 4 | 5 | 99992554 | always | pushes group **5** on the group stack, in place |
| 5 | 6 | 99992555 | always | writes `1` to `[[0x1416751f8]+0x13a]` |

Rows 2 and 3 are the pair the "different buttons online vs offline" impression comes from: they are
**both always present**, and exactly one of them is ever enabled.

### The row names were not read from the text; they were read from where each row goes

The label ids are in the FMG archives (`GameDataEbl.bdt`), which this trace did not open. The
identification below rests on the destination substate instead, which is stronger evidence than a
string would have been.

`FeSubStateTitleTopMenu::v5` at **`0x1400fe510`** (RVA `0x000fe510`) registers the transitions.
Each is a `0x28`-byte `FeTransitionEqualValue<int>`: `+0x08` destination substate id, `+0x18`
pointer to the watched int, `+0x20` the value to match. Every one of them watches
`&substate->_x10_unk` -- the substate's phase, which `FeSubStateTitleTopMenu::v3` (`0x1400ff300`)
sets from `[scene+0xE8]`, which is where the group parks the action id.

* **Row 0 is NEW GAME.** Its first transition is a *subclass*, `_TransitionNewGame` (vtable
  `0x1410bda40`, predicate `0x1400fdf70`), which matches only when the phase is 1 **and**
  `0x14019ba90` -- first free save slot, or `-1` when all ten are taken -- returns negative. So a
  full save list diverts to `0x48`, a message box. Otherwise the plain value-1 transition takes it
  to `0x4c` `FeSubStateTitleOptionScreen`, whose `enter` (`0x1400fdbf0`) early-outs to its terminal
  phase when `savedata[+0x136f]` is already set, then hands to `0x4d` `FeSubStateTitleOptionGame`
  and on to `0x4e` `FeSubStateTitleOnlineCheck`. That is the once-per-profile brightness/options
  walk in front of a new game, not a settings menu reached from the title.
* **Row 1 is LOAD GAME** -- `FeSubStateTitleLoadDataList`, gated on a save existing.
* **Row 2 is the server information screen** -- `FeSubStateTitleInformation`, gated on being online.
* **Row 3 is "go online"** -- `FeSubStateTitleSteamNetworkCheck`, gated on **not** being online.
  Its transition is the only one registered conditionally: the builder appends it only when
  `0x140513600` returns 0, so when you are already online the row is both disabled *and* has no
  destination.
* **Row 5 is QUIT GAME.** `[[0x1416751f8]+0x13a] = 1` is written by exactly one other place in the
  image, `FeSubStateTitleShutdown`'s own `0x1400fde20`, and is read from the main game-state loop.

Two things are **not** established. Row 4 creates group id 5 through the group factory
(`0x140056b70` then `0x140026e70`) rather than transitioning, and that group was not identified.
And the transitions on phase 5 (`-> 0x1b`) and phase 6 (`-> 0x35`) name substate ids that no
constructor in `FeStateTitle::v6` writes as a literal -- but the activate handler returns *before*
storing the result for actions 5 and 6, so neither can fire from the button press itself. What does
fire them was not traced.

### Disabled is one function, and it does two separate things

`FeGroupTitleTopMenu::v25` at `0x1400f4df0` clears the stored result, rebuilds the list, calls
**`0x1400f5000`** (RVA `0x000f5000`), then refreshes the grid. `0x1400f5000` is the whole of the
disabled-vs-enabled behaviour, and the decompiler drops an argument here, so this is read from the
disassembly at `0x1400f5000`-`0x1400f50b7`:

```text
for i in 0 .. list.count:
    cell = 0x140108060(group, i)          # the cell whose [+0x10] id == i; null is skipped
    desc = list + align + i*0x38
    tmp  = 0x140026790(group+0x100, &scratch, desc)   # RCX/RDX/R8 are set BEFORE the branch
    if desc[+0x34] != 0:                  # enabled
        tmp[+0x40]->vtable[0](tmp+0x40, 0x67, 0, 0.0)
        cell[+0x8] = 3 + (i == group[+0x28])          # 3 normal, 4 under the cursor
    else:                                 # disabled
        cell[+0x8] = 2
        tmp[+0x40]->vtable[0](tmp+0x40, 0x7a, 0, 0.0)
```

**The style and the clickability are two independent writes.** The greyed look is the explicit
sequence **`0x7a`** in place of the normal **`0x67`** -- the same sequence-id family as the scene
transitions (`0x65`, `0x66`, `0x67`, `0x68`) documented above. Nothing reads `cell[+0x8] == 2` to
decide how to draw.

### The cursor cannot land on a disabled row at all

`cell[+0x8]` is the cell's state, and the cell class is **`FeObjectButtonEx`** (vtable
`0x1410be398`). Two of its virtuals read that field, and both are four instructions long:

| slot | address | body |
| --- | --- | --- |
| v1 | `0x14004c5b0` | `cmp DWORD PTR [rcx+8],4; sete al; ret` -- "am I the cursor" |
| v16 | `0x14004c5c0` | `vtable[3]() == 1 && [rcx+8] == 3` -- **"can I be selected"** |

`FexGroupList<FeGroupGrid>`'s navigation search at `0x140107b40` calls **v16 on every candidate**
before accepting it, on all six of its direction branches and again at the shared accept point
`0x140107fb0`. State `2` fails it, so a disabled row is not "present but not clickable" -- it is
skipped by cursor movement entirely, and the activate handler `0x1400f4a60` never sees it. That
handler does **no** enable check of its own, which is exactly why the check has to be here.

State `2` is not sticky: `FexGroupList` v4 (`0x140106190`) resets `4 -> 3` and v5 (`0x140106290`)
resets `2 -> 3`, and v22 (`0x1401081f0`) rewrites every cell to 3, with 4 on the cursor. `0x1400f5000`
re-applies the 2s on every rebuild, which is what keeps a row disabled across cursor moves.

### The switch that is read once and never written

`0x14160de19` is read at exactly one instruction in the whole image -- `0x1400f431f`, in this
builder -- and forces the online flag to 0 when set. Ghidra finds no writer. It is the shortest
path to "make the menu believe it is offline", and it moves row 2 and row 3 together because both
are computed from that one flag.

### A correction to the vtable table above

`FeSubStateBase` v5 and v6 are **not** debug registration and a debug walk. `FeStateFlow`'s
dispatcher `0x140104540` calls v5 with the transition list immediately after a substate's `enter`,
and v6 with the same list immediately before its `leave`:

* **v5 = publish my transitions.** `FeSubStateTitleTopMenu::v5` (`0x1400fe510`) is where the whole
  top-menu routing table lives.
* **v6 = drop them.** The base at `0x1401043a0` releases every entry and zeroes the count, so the
  list is rebuilt from scratch on each substate change. Capacity is `0x2a`.

`FeStateFlow::FUN_140104930` (`0x140104930`) then returns the **first** transition whose predicate
matches, which is why `_TransitionNewGame` being registered ahead of the plain value-1 transition is
what makes the full-save-list diversion work.

## Drawing the rows the game hides (`ds2-mods-rs-<menu>`)

The trace above establishes that the menu never inserts or removes a row, and that "unavailable"
is two independent writes. On this layout the sequence half of that pair does not grey a row --
**it takes it off the screen**. That is why LOAD GAME is simply absent until a save exists, and it
is the shipped behaviour `crates/ds2-dialog-skip/src/menu.rs` changes.

### The evidence that the sequence is what hides it

Not a guess, and worth setting out because the layout resource itself was never opened:

1. Every row is always constructed. `0x1400f4250` has no conditional append, the count is 6 on
   every path, and the cell factory `0x1400f36b0` builds a cell for every row index below that
   count. **A missing row cannot be a missing row.**
2. A disabled row differs from an enabled one in exactly two writes: `cell[+8] = 2` instead of
   `3`/`4`, and sequence `0x7a` instead of `0x67`.
3. **Nothing in the image reads `cell[+8] == 2` to decide how to draw.** The only readers of that
   field compare it against `3` (`FeObjectButtonEx::v16`, selectability) and `4`
   (`FeObjectButtonEx::v1`, cursor), plus two helpers that reset it. So the state cannot be what
   removes the row.

That leaves the sequence, by elimination. `0x7a` is also the "off" half of a pair elsewhere in the
same flow -- `0x1400f5720` plays `0x70` on one element in one branch and `0x7a` on it in the other
-- so it is a layout-wide convention rather than a quirk of this one call.

### The change: swap which write carries the meaning

One detour on `FE_TOP_MENU_APPLY_STATES` (`0x000f5000`) brackets the original:

* **before** -- every descriptor's enable byte is forced to `1`, so the pass styles all six rows as
  available and every row is drawn;
* **after** -- the enable bytes are restored, and every row the game had marked unavailable has its
  cell state written straight back to `2`.

Both halves are the game's own values written to the game's own fields. Nothing here invents a
notion of "available": the byte the game computed is read, and the same verdict is re-applied
through the other one of the two mechanisms the game already has.

### Why the state is re-asserted twice

Once **synchronously**, inside that detour, because `FeGroupTitleTopMenu::v25` calls
`FexGridControl::FUN_140023690` immediately afterwards and that is what settles the cursor. This
matters more than it looks: **the activate handler `0x1400f4a60` does no enable check of its own.**
It reads the action id at the cursor and acts. The shipped invariant that stops you firing an
unavailable row is entirely upstream, in the navigation predicate, so a cursor allowed to rest on a
row this mod had just made look selectable would be able to fire it.

And once **per frame**, from `FE_TOP_MENU_UPDATE` (`0x000ff300`, `FeSubStateTitleTopMenu::v3`) --
the only per-frame function specific to this menu. `FexGroupList` v22 (`0x1401081f0`) rewrites
every cell to `3`/`4` from the cursor, and the input handling that reads the states runs inside the
scene tick the original performs, so re-asserting after it means navigation on the next frame sees
the corrected values.

### What this does not do

**The row is drawn in its normal style, not a greyed one.** Sequence ids index a layout resource
inside `GameDataEbl.bdt`; which id renders as greyed cannot be read out of the executable, and
`0x67` is used because it is the only id this exact element is *proven* to render -- by the
available branch of the very function being replaced. Establishing a real greyed-out sequence means
decrypting the layout archive with `GameDataKeyCode.pem` and reading its animation table. That is a
separate piece of work and is not guessed at here; what ships is **visible and inert**.

### The line to read on the first run

```
ds2-dialog-skip: shown screen=top-menu rows=6 unavailable-mask=0b001100 state=2 total=2
```

The mask is the point. It says which rows the game decided it could not offer on this machine, in
row order (bit 0 = NEW GAME), and it is the only way to tell a run where this feature changed
nothing from a run where the hook never fired. It also settles a question this document could not:
whether the menu is ever built **before** the save data is read. If a machine with a save reports a
mask with bit 1 set, the top menu was reached ahead of the save-slot read and the enable states are
a snapshot of a fact that was not yet true.
