# METADATA
# scope: package
# title: Cupcake Hybrid Model Aggregation
# description: |
#   The single entrypoint the engine evaluates. It collects every decision verb exported by
#   every loaded policy and hands the engine one object with a stable shape.
# authors: ["Cupcake Engine", "er-mods-rs agents", "ds2-mods-rs agents"]
package cupcake.system

import rego.v1

# DO NOT REINTRODUCE `walk(data.cupcake.policies, ...)` HERE.
#
# Upstream's dispatcher collected verbs by walking the whole virtual policy tree and keeping the
# values whose last path element was "deny"/"halt"/etc. It is elegant and it fails open. Measured
# in er-mods-rs on 2026-09-18: the WASM runtime crashed inside `collect_verbs` /
# `opa_value_transitive_closure` while evaluating ORDINARY long Bash payloads. A crashed
# evaluation is not a refusal -- the engine returns `{}` at exit 0, which the harness reads as
# allow -- so every policy in this directory went silent at once, with nothing in the transcript
# to say so. The walk is what does it: it traverses every helper rule and every intermediate value
# in the tree, not just the decision verbs, so the cost scales with the size of the policy set and
# the length of the command being judged rather than with the number of decisions.
#
# The same failure arrives from the other end through WASM linear memory; see the
# `--wasm-max-memory` note in scripts/cupcake-hook.sh. Both are fixed, and both are fail-open, so
# neither fix announces itself when it is working.
#
# The cost of the explicit form is one line per decision-exporting policy, and forgetting that line
# means the policy is inert. scripts/check-cupcake-wasm-builtins.py exists to catch exactly that:
# it fails when a policy exports a verb this file does not route. Add the line with the policy.

evaluate := {
	"halts": [decision | some decision in all_halts],
	"denials": [decision | some decision in all_denials],
	"blocks": [],
	"asks": [],
	"modifications": [],
	"add_context": [decision | some decision in all_add_context],
}

# Halt decisions.
all_halts contains decision if { some decision in data.cupcake.policies.builtins.git_pre_check.halt }

all_halts contains decision if { some decision in data.cupcake.policies.builtins.protected_paths.halt }

all_halts contains decision if { some decision in data.cupcake.policies.builtins.rulebook_security_guardrails.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.guard_layer_destructive_guard.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.idle_hold.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.native_ownership_vocab_reminder.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_admission_with_defence.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_authority_agreement.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_deferred_evidence_read.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_described_next_step.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_diagnosis_without_fix.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_explanation_instead_of_correction.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_fix_claim_without_runtime_evidence.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_future_tense_commitment.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_narrated_action.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_proof_without_observation.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_restating_user_own_rule.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_shouting_at_turn_end.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_stall_on_friction.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_status_table_at_turn_end.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_unbacked_claim.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_unchecked_game_alive_claim.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_unexecuted_promise.halt }

all_halts contains decision if { some decision in data.cupcake.policies.claude.no_user_property_grant.halt }

# Deny decisions.
all_denials contains decision if { some decision in data.cupcake.policies.builtins.claude_code_enforce_full_file_read.deny }

all_denials contains decision if { some decision in data.cupcake.policies.builtins.git_block_no_verify.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.bash_no_python_file_write.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.block_askuserquestion.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.block_askuserquestion_reminder.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.block_compositor_input_injection.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.bd_bug_needs_qa_steps.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.block_pgrep_full_match.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.docs_no_size_metrics.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.docs_no_shouting.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.ds2_launch_guard.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.pr_requires_run_stamp.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.edit_no_tmp_scripts_guard.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.git_block_detached_push.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.git_block_main_commit.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.git_block_main_push.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.git_require_fresh_origin_main.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.git_require_runtime_test_before_push.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.monitor_rate_limit.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.no_rust_edit_without_frida_proof.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.teardown_must_relaunch.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.no_grep_for_build_errors.deny }

all_denials contains decision if { some decision in data.cupcake.policies.claude.require_scoped_cargo.deny }

# Block and ask decisions. No current project policy exports these, but keep the shape stable.

# Modify decisions. No current project policy exports these, but keep the shape stable.

# Prompt-context decisions.
all_add_context contains decision if { some decision in data.cupcake.policies.builtins.claude_code_always_inject_on_prompt.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.block_askuserquestion_reminder.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.idle_hold_reminder.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.native_ownership_vocab_reminder.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.no_authority_agreement_reminder.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.no_false_ci_green.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.claude.wall_of_text.add_context }

all_add_context contains decision if { some decision in data.cupcake.policies.no_repo_network_banners_prompt_context.add_context }
