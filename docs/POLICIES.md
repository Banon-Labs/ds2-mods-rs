# Cupcake policies: what was ported from er-mods-rs, and what was not

The `.cupcake/` tree is executable enforcement for agent behaviour in this repo. It is evaluated by
the *global* hook wrapper at `~/.claude/hooks/cupcake-hook.sh`, which is already wired into
`PreToolUse`, `PostToolUse`, `SessionStart` and `UserPromptSubmit` in `~/.claude/settings.json`.
Nothing repo-local needs to install a hook: dropping a `.cupcake/` directory in the project root is
sufficient, and it takes effect on the very next tool call.

## Starting state

Measured 2026-08-26 with `cupcake verify --harness claude` from the repo root, before any of this
landed:

```
=== Project Routing Map ===
                                 <- empty
=== WASM Compilation ===
  Project WASM: MISSING
```

The only policies applying to this repo were the two global ones, `github_pr_draft_guard` and
`github_attribution_guard` (both already documented in `AGENTS.md`). Every behavioural guard the
sibling repo `er-mods-rs` had accumulated -- 62 git-tracked files under its `.cupcake/` -- applied
to that repo only.

## The porting rule

Same rule as the crate port: **a policy is game-specific until proven otherwise.** A guard carries
its authoring repo's assumptions in its *premises*, not just in its strings, and a premise that is
false here makes the guard actively harmful rather than merely useless.

Measured over the 22 policy files in `er-mods-rs/.cupcake/policies/`: only 2 contain
Elden-Ring-specific text. The other 20 are about git, the filesystem, and agent conduct, and port
cleanly. The two exceptions are the interesting ones, and neither is a port.

## Wave 1 -- ported (9 policies, 219 interpreter tests, 15 live-eval cases)

Copied with only two mechanical changes: rule IDs `ER-EFFECTS-*` became `DS2-MODS-*` (a deny
message quoting an Elden Ring rule ID in a Dark Souls II repo is the exact sloppiness this project
is trying to avoid), and `authors` was retargeted. Comments citing `bd er-effects-rs-*` issue IDs
were **kept deliberately** -- they point at the investigation that produced each rule, and deleting
them would destroy the evidence trail that makes the rule reviewable.

| Policy | What it does |
| --- | --- |
| `git_block_main_commit` | No committing while the checkout is on `main`. |
| `git_block_main_push` | No pushing to `main`, including via `HEAD` while on `main`. |
| `git_require_fresh_origin_main` | Rebase/force-push only against a just-fetched `origin/main`; fails closed. |
| `builtins/git_block_no_verify` | No bypassing commit hooks. |
| `builtins/git_pre_check` | Runs validation before git operations. |
| `builtins/protected_paths` | `/etc/`, `/System/`, `~/.ssh/` are read-only: reads allowed, writes blocked. |
| `edit_no_tmp_scripts_guard` | Source and scripts belong in the repo; `/tmp` is for data artifacts only. |
| `block_askuserquestion` | Gate on questionnaire prompts (currently inert by design; see its header). |
| `block_askuserquestion_reminder` | The `UserPromptSubmit` companion that carries the actual nudge. |

Note the interaction between `edit_no_tmp_scripts_guard` and the session scratchpad: writing a
`.py`/`.sh`/`.rs` into `/tmp` is denied, writing a `.json`/`.log`/`.bin` there is fine. That split
is the guard's whole point and it is not a bug to work around.

## NOT ported: `bash_elden_ring_launch_guard` (1445 lines)

It forbids launching Elden Ring through Steam at all, because of EAC bans. **Ported verbatim it
would block this repo's own launcher.** `scripts/ds2-run.py` launches appid 335300 through
`steam -applaunch` by design, and DS2 SOTFS has no EAC to get banned from.

The intent still transfers, with the mechanism inverted: deny a *raw* `steam -applaunch 335300`
issued outside `scripts/ds2-run.py`. The launcher is testimony-gated -- it stages the DLL, records
both the built and the staged sha256, and prints a success block only after reading the DLL's own
log line. A raw applaunch skips all of that, which is precisely the shape of a false "it launched
with our DLL" claim. Shipped as `ds2_launch_guard.rego` (`ds2-mods-rs-yx2`).

It denies `steam -applaunch 335300`, the `steam://run/` and `steam://rungameid/` URL forms, and a
wine/proton invocation naming `DarkSoulsII.exe` -- and fails closed on an unreadable `-c`/`eval`
payload that names both Steam and the appid. `scripts/ds2-run.py` needs no exemption: it runs its
applaunch from inside a file on disk, and a file on disk is never an agent Bash command.

Two things it deliberately does *not* touch. A different appid (`1245620` is Elden Ring) is
somebody else's business -- grabbing every applaunch would reproduce the ER guard's overreach in
mirror. And `objdump`, `strings`, `xxd` and `sha256sum` all name `DarkSoulsII.exe` without
launching it; static RE on that binary is the most encouraged activity in this repo, so the wine
rule is gated on a launcher verb rather than on the exe name.

## NOT ported: `block_manual_pgrep` (272 lines)

Its premise is explicitly WSL: on that box Steam and the game are native Windows processes that
`pgrep` cannot see, so `pgrep -x steam` is a false *negative* and the guard bans `pgrep` outright.

**That premise is false here.** This is native CachyOS/Arch, Steam and `DarkSoulsII.exe` are real
Linux processes, and `pgrep -x DarkSoulsII.exe` works correctly -- it is how the game was confirmed
closed on 2026-08-26. Porting the ban would forbid a technique that is *correct* on this machine.

The real hazard here is a different one, measured twice in a single session:

1. `pgrep -f DarkSoulsII.exe` matched the agent's own command line and produced a fabricated
   "process is ALIVE" claim.
2. `pkill -f 'ds2-run.py --probe neuter'` matched and killed the agent's own shell (exit 144).

Both are the `-f` flag matching the full command line, the agent's included. So the DS2 guard bans
`-f` and steers to `-x`, rather than banning the tool. Shipped as `block_pgrep_full_match.rego`
(`ds2-mods-rs-hst`).

It catches the cluster spellings (`-af`, `-fl`), `--full`, and the flag arriving after an option
that takes an operand (`pgrep -u banon -f ...`). `-F` is left alone on purpose -- that is
`--pidfile`, a different option that cannot self-match, and denying it would be a false positive.
`grep -f` and `tail -f` are unrelated tools and stay allowed, which is what the single-simple-command
bound on the pattern is for.

## What both DS2 guards taught us about `commands.rego`

Both were first written against `commands.executed_texts`, and both were wrong, in a way worth
recording because it is not obvious from the name.

`executed_texts` blanks *anchors* inside a quoted span -- separators, parens, the quotes themselves
-- but **not spaces**. So a mid-sentence mention inside a quoted operand still has a space in front
of it, still satisfies a space-anchored pattern, and still denies. Measured while writing the launch
guard: a `bd create --description "Deny a raw steam -applaunch 335300 ..."` was denied *by the very
rule it described*, which would have made the guard impossible to document or file issues about.

The right primitive is `commands.executed_unquoted_texts`, which the helper's own comment labels
"for substring/flag tests". It deletes quoted spans outright, so the mention disappears while a real
bare launch -- which is not quoted -- does not. Both guards use it.

The limit, stated rather than papered over: quoting the payload itself
(`steam -applaunch "335300"`) deletes it from the scanned text and defeats the pattern. That is a
deliberate-evasion spelling, not an accidental one. A text-scanning guard cannot beat someone trying
to get past it; these exist to stop the accidental bare launch and the reflexive `-f`, and the
fail-closed rules are what refuse to guess when the text genuinely cannot be read.

## Not yet ported: the behavioural Stop guards (Wave 2)

`no_authority_agreement`, `no_unbacked_claim`, `no_unexecuted_promise`, `no_stall_on_friction`,
`wall_of_text`, `idle_hold`, `native_ownership_vocab_reminder`. These are not self-contained: every
signal in `.cupcake/signals/` shells into `scripts/cupcake_turn_scan.py` (512 lines) and
`cupcake_unbacked_claim.py`, which have to come across first, along with the audit scripts that
prove the guards do not cry wolf against real transcripts. Tracked as `ds2-mods-rs-68n`.

## How it is tested

`scripts/check.sh` gates all three layers, and a missing `opa` is a hard failure rather than a
silent skip:

1. `opa test` per policy, each against only its own file plus `system/commands.rego`. Never the
   whole tree at once -- one policy's rules can satisfy another's assertions and turn a test green
   for the wrong reason.
2. `cupcake verify` compiles the tree to WASM. `opa test` proves the rules are right; it does not
   prove cupcake can load them.
3. `scripts/test-cupcake-policies.py` drives the real `cupcake eval` binary with real `PreToolUse`
   events -- the only layer that exercises what the live hook actually runs.

That harness is a committed file rather than an inline command on purpose. Several of its deny
cases must contain the literal text of a command the policies exist to block, and typing that into
an agent Bash command is itself intercepted. Measured during this port:

```
This command wraps a shell payload the guard cannot read (an unquoted or substituted
`-c`/`eval` argument) while naming git and push, so it cannot be shown not to push to main.
```

That is the guard working correctly, on a command of mine, minutes after being installed. A
committed file is never an agent Bash command, which is the same reasoning the sibling repo uses to
sanction `scripts/steam-running.sh` as the one legitimate home for a `pgrep` call.

## Known gap: `rulebook_security_guardrails` is configured but OFF

Under cupcake 0.5.2 this builtin needs an explicit `enabled: true`. Configuring it is not enough,
despite the rulebook template's own comment claiming otherwise. Verified by diffing the
`Enabled builtins:` line from `cupcake verify` with and without the key.

**er-mods-rs is in exactly that state**, so its `.cupcake/` directory has never actually been
protected -- which is also why agents there can edit policy files freely. Every other builtin in
that rulebook does carry `enabled: true`.

It is left off here too, deliberately and for now: when enabled it *halts* any Bash command whose
text merely references a protected path, with no read whitelist, which would block Waves 2 through
4 of this port. `bash scripts/check.sh` keeps working either way, because that command text does
not contain the protected string. Tracked as a decision, not a bug.
