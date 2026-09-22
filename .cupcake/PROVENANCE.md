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

## Deliberately NOT ported

| File | Why not |
|---|---|
| `block_manual_pgrep` | Bans `pgrep` outright on a WSL2 box where Steam and the game are native Windows processes `pgrep` cannot see. False premise here: this is native Linux and `pgrep -x DarkSoulsII.exe` is correct. `block_pgrep_full_match` is this repo's answer to the real hazard, the `-f` flag. |
| `bash_elden_ring_launch_guard` | `ds2_launch_guard` is the local equivalent. |
| `git_block_any_push` | Encodes "you should NEVER be pushing". `CLAUDE.md` here says the opposite, in capitals: work is not complete until `git push` succeeds. |
| `no_whole_check_sh` | Its premise is "the pre-push hook runs it and CI runs it". Neither is true here -- there is no `.github/workflows/`, and `core.hooksPath` is `.beads/hooks`. Banning the agent from `scripts/check.sh` would mean nothing ever runs it. |
| `no_mergeable_without_green_ci` | Requires a green CI verdict that this repo has no CI to produce, so the halt would be unsatisfiable -- a permanent wedge. `no_false_ci_green` is kept instead: it handles the no-CI case correctly by refusing to let a branch be called green. |
| `edit_no_comment_caps_guard` | Enforces a zero-shouted-words convention er-mods-rs swept its tree to on 2026-09-07. This tree was never swept and its prose uses capitals throughout, so the guard would deny an edit to almost every file -- and its refusal text asserts a sweep that did not happen here. |
| `git_require_runtime_evidence`, `no_fix_claim_without_runtime_evidence`, `no_rust_edit_without_frida_proof`, `no_source_edit_during_live_run`, `teardown_must_relaunch` | All four depend on Elden Ring runtime plumbing -- Frida agents, `er-me3-runs` artifacts, a live-run sentinel. The equivalents here would have to be built against `scripts/ds2-run.py`, which is real work rather than a copy. |

## Keeping the two in step

Fixes to a shared classifier belong upstream too, or the copies start disagreeing about the same
turn. Where this repo diverged on purpose, the divergence is commented **in the file**, not only
here -- this page is an index, and an index goes stale.
