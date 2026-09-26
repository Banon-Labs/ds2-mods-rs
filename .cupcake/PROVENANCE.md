# Where this guard layer came from

Most of `.cupcake/` was written in [`Banon-Labs/er-mods-rs`](https://github.com/Banon-Labs/er-mods-rs)
and ported here on **2026-09-21** from that repo at commit `f6d52baa`. This file records what was
taken, what was changed on the way in, and -- the part that matters most when a guard misfires --
**what was deliberately left behind, and why**.

The reason to keep this list is narrow and practical: a ported guard carries the premises of the
box it was written on. `block_pgrep_full_match.rego` already says this out loud about the one guard
that was rewritten rather than copied. Everything below is the same judgement applied file by file.

## Ported unchanged in substance

Rule ids were rebranded `ER-EFFECTS-*` -> `DS2-MODS-*` and `authors:` now lists both repos. Nothing
else in these files was altered.

**Behavioural (Stop / UserPromptSubmit) guards** -- `idle_hold`, `idle_hold_reminder`,
`native_ownership_vocab_reminder`, `no_admission_with_defence`, `no_authority_agreement`,
`no_authority_agreement_reminder`, `no_deferred_evidence_read`, `no_described_next_step`,
`no_diagnosis_without_fix`, `no_explanation_instead_of_correction`, `no_false_ci_green`,
`no_future_tense_commitment`, `no_narrated_action`, `no_proof_without_observation`,
`no_repo_network_banners_prompt_context`, `no_stall_on_friction`, `no_unbacked_claim`,
`wall_of_text`, and the sixteen `signals/last_assistant_*.sh` classifiers behind them.

**Command guards** -- `bash_no_python_file_write`, `block_compositor_input_injection`,
`guard_layer_destructive_guard`, `git_block_detached_push`, `monitor_rate_limit`,
`require_scoped_cargo`, and the hardened `edit_no_tmp_scripts_guard` (which fixes two measured
false positives the older local copy had: reading third-party JS out of `/tmp`, and `cp -f
/tmp/planner/*.js target/`, both denied because an interpreter name and a `/tmp` path merely
co-occurred).

**Vendored builtins** -- `protected_paths` (shell-wrapper payloads: `bash -c "rm -rf /"` was allowed
before) and `git_block_no_verify` (the `core.hooksPath` and hook-removal arms, which used to deny
*installing* hooks because a removal verb in one statement met a hooks path in another).

**Infrastructure** -- `system/commands.rego`, `scripts/cupcake-hook.sh`, `scripts/beads-prime.sh`
+ `gen-beads-prime.py`, and the test harnesses `test-cupcake-hook-shim.py`,
`test-cupcake-delivered-shape.py`, plus the `audit-*-false-positives.py` diagnostics.

## Ported with a deliberate divergence

| File | Divergence |
|---|---|
| `system/evaluate.rego` | Explicit dispatcher (er-mods-rs's design, which replaced upstream's `walk()` after the WASM runtime crashed inside `collect_verbs` on ordinary long Bash payloads -- a crash returns `{}` at exit 0, i.e. **allow**). Routes this repo's policy set, not er-mods-rs's. |
| `system/commands.rego` -> `heredoc_body_blanked` | The heredoc body is wrapped in a synthetic `"` so `quotes_removed` drops it whole. er-mods-rs emits it bare and buys the cases back with per-policy exemptions. Measured when the bare form was imported: a commit message *documenting* the pgrep rule was denied by the rule it documents. |
| `system/commands.rego` -> `command_position_prefix_pattern` | `timeout` added to the wrapper set. A bare duration is neither option-shaped nor an assignment, so `timeout 5 rm -rf /` stood in nobody's command position -- in either repo. |
| `system/commands.rego` -> shell-payload patterns | The bare words `eval` / `source` / `.` now need **command position**, not just a preceding space. `cupcake eval ...` and `opa eval ...` were being read as unreadable shell payloads, so every guard that fails closed on `unparsed_shell_payload` denied them -- which is to say, denied the commands used to debug those guards. |
| `no_grep_for_build_errors.rego` | Build verb must be in command position. er-mods-rs anchors on any whitespace, so `grep -n x scripts/test-cupcake-policies.py \| grep -nE y` -- reading a repo script -- was denied as if it were adjudicating a build. |
| `scripts/cupcake-hook.sh` | Adds `--wasm-max-memory 100MB`, taken from `~/.claude/hooks/cupcake-hook.sh`. The 10MB default aborts on an ~11KB command, and an aborted evaluation returns `{}` at exit 0. Neither repo hook had it. |
| `scripts/test-cupcake-policies.py` | Kept this repo's own 28-case table (er-mods-rs's 191 cases are mostly about policies not ported here). Gained er-mods-rs's signal-contract gate, generalised from its two runtime-evidence signals to every signal, and replaced its hand-listed `ORPHANED_REGO_SUITES` with `opa test .cupcake/` over the whole tree -- the list's own comment records that four suites sat orphaned in it, accumulating 89 never-executed assertions. |
| `rulebook.yml` | Retracts upstream's false "builtins are enabled by default when configured" claim and makes `rulebook_security_guardrails: enabled: false` explicit. It was never running; the comment said it was. |

## Ported 2026-09-25, from er-mods-rs's tree as it stood that day

Each file says in its own header what changed on the way in; this is the index.

| Policy | What had to change |
|---|---|
| `no_fix_claim_without_runtime_evidence` | Evidence allowlist is DARK SOULS II's run artifacts (`ds2-loader.log`, `ds2-crash-*.txt`, `ds2-teardown.py --status`, `ds2-frida-watch.py`); the crate walk reads `[workspace.dependencies]`, which er's does not. Its five fixtures were er's and now describe a `ds2-menu-row` edit. |
| `no_restating_user_own_rule` | Pushing is not a user-owned action here: `CLAUDE.md` makes it the agent's job. |
| `no_rust_edit_without_frida_proof` | Reader renamed to `scripts/ds2-frida-evidence.py`; `ds2-frida-watch.py` was already calling it under er's name and recording nothing. |
| `teardown_must_relaunch` | `ds2-teardown.py` / `ds2-run.py`; er's two extra launchers and `--reason` dropped. `ds2-run.py` tears down on its own. |
| `no_mergeable_without_green_ci` | The reason it was left behind -- no CI -- stopped being true: `.github/workflows/release.yml` builds on every pull request. er's signal was inert (a property called as a function); this one is driven end to end. |
| `gh_pr_title_conventional` | Authority is `scripts/check-commit-message.py`, not a CI job; its type list and two header rules. |

`git_require_runtime_test_before_push` also changed that day, for a miss of its own rather than a
port: `git commit ... && git push` in one command went through, because the hook fires before the
commit exists. See that policy and `scripts/test-runtime-evidence-signal.py`.

## Deliberately not ported

| File | Why not |
|---|---|
| `block_manual_pgrep` | Bans `pgrep` outright on a WSL2 box where Steam and the game are native Windows processes `pgrep` cannot see. That premise is WSL's. `block_pgrep_full_match` is this repo's answer to the `-f` self-match. Open question, not settled here: `scripts/ds2-teardown.py`'s docstring (2026-09-23) says `pgrep -x DarkSoulsII.exe` answers "nothing running" about a game on screen, which contradicts the `-x` premise in `block_pgrep_full_match`. One of the two is wrong, and deciding which needs a measurement against a live session. |
| `bash_elden_ring_launch_guard` | `ds2_launch_guard` is the local equivalent. er's also refuses bundling Seamless Co-op's `ersc.dll`; no DS2 rule says the DS2 Seamless DLL may not be bundled, so that arm has nothing to encode here. |
| `git_block_any_push` | Encodes "you should never be pushing". `CLAUDE.md` here says the opposite: work is not complete until `git push` succeeds. What stands between that and unrun game code is `git_require_runtime_test_before_push`. |
| `no_whole_check_sh` | Its premise is "the pre-push hook runs it and CI runs it". Neither is true here: `core.hooksPath` is `.beads/hooks`, and `release.yml` says in its own header that it is not the gate. AGENTS.md asks for `scripts/check.sh` once, when the branch is going out; banning it would mean nothing runs it. |
| `edit_no_comment_caps_guard` | `docs_no_shouting` is this repo's version, measured against this tree (runs of four capitalised words, and a closed list of function words), and `no_shouting_at_turn_end` carries it into prose. er's list was tuned to a tree swept to zero on 2026-09-07; this one was not swept. Its `.py`/`.sh`/`.bash` scope, which `docs_no_shouting` never covered, was later adapted as `script_comments_no_shouting`: er's file scope, this repo's two patterns (copied, not imported, because `scripts/check.sh` tests each policy alone). |
| `git_require_runtime_evidence` | `git_require_runtime_test_before_push` covers it: same jurisdiction (`crates/`, plus `scripts/ds2-run.py` here), and it additionally demands the staged DLL be byte-identical to the one this checkout built, with no `ER_ALLOW_UNPROVEN_PUSH`-style escape hatch. The part of er's that is stronger is provenance: its DLLs print `build git=<sha>` on their first line and the signal matches that sha to the tip, where this one compares times and bytes. Porting that needs `ds2-loader` to embed its commit, which is a crate change and is filed rather than done here. |
| `no_source_edit_during_live_run` | Its premise is er's PostToolUse stale-run sentinel, which tears a live run down when an edit feeds a loaded DLL, so the edit kills the run. This repo has no such sentinel -- the PostToolUse hook only runs cupcake -- so an edit here does not end a session and there is nothing for the guard to prevent. |

## Keeping the two in step

Fixes to a shared classifier belong upstream too, or the copies start disagreeing about the same
turn. Where this repo diverged on purpose, the divergence is commented **in the file**, not only
here -- this page is an index, and an index goes stale.
