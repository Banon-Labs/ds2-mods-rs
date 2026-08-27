package cupcake.policies.claude.git_require_fresh_origin_main_test
import rego.v1
import data.cupcake.policies.claude.git_require_fresh_origin_main as guard

same := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
stale := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
event(command, oids) := {"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": command}, "signals": {"origin_main_oids": oids}}
blocked(command, oids) if {
    denials := guard.deny with input as event(command, oids)
    count(denials) == 1
}
allowed(command, oids) if {
    denials := guard.deny with input as event(command, oids)
    count(denials) == 0
}

test_deny_stale_rebase if { blocked("git rebase origin/main", stale) }
test_deny_stale_force_with_lease if { blocked("git push --force-with-lease origin feature/x", stale) }
test_deny_stale_short_force if { blocked("git push -f origin feature/x", stale) }
test_deny_stale_plus_refspec if { blocked("git push origin +HEAD:refs/heads/feature/x", stale) }
test_deny_missing_signal if { blocked("git rebase origin/main", "") }
test_allow_fresh_guarded_commands if { allowed("git rebase origin/main", same); allowed("git push --force origin feature/x", same) }
test_allow_unrelated_commands_when_stale if { allowed("git rebase origin/release", stale); allowed("git push origin feature/x", stale); allowed("printf 'git push --force'", stale) }

# --- Shell-wrapper payloads (2026-08-26, bd er-effects-rs-dt2e) ---------------
#
# This guard's three patterns anchor on `(^|[;&|][[:space:]]*|\n)`, which no more
# admits a quote than the main-push guard's class did, so every case below ran
# against an unverified origin/main while producing ZERO denials.

test_deny_wrapped_force_push_when_stale if { blocked("bash -c 'git push --force origin feature/x'", stale) }
test_deny_wrapped_force_with_lease_when_stale if { blocked(`sh -c "git push --force-with-lease origin feature/x"`, stale) }
test_deny_wrapped_short_force_when_stale if { blocked("bash -lc 'git push -f origin feature/x'", stale) }
test_deny_wrapped_plus_refspec_when_stale if { blocked("zsh -c 'git push origin +HEAD:refs/heads/feature/x'", stale) }
test_deny_wrapped_rebase_origin_main_when_stale if { blocked(`eval "git rebase origin/main"`, stale) }
test_deny_nested_wrapped_force_push_when_stale if { blocked(`bash -c 'bash -c "git push --force origin feature/x"'`, stale) }

test_allow_wrapped_force_push_when_fresh if { allowed("bash -c 'git push --force origin feature/x'", same) }

# Unreadable payload, inside this guard's own jurisdiction (git + force/rebase).
test_deny_unquoted_wrapper_payload_naming_force_when_stale if { blocked("bash -c $GIT_FORCE_CMD", stale) }
test_deny_unquoted_wrapper_payload_naming_rebase_when_stale if { blocked("eval $GIT_REBASE_CMD", stale) }
test_allow_unquoted_wrapper_payload_outside_jurisdiction if { allowed("bash -c $BUILD_CMD", stale) }
test_allow_unquoted_wrapper_payload_naming_force_when_fresh if { allowed("bash -c $GIT_FORCE_CMD", same) }

# --- Quoted TEXT is not an executed payload -----------------------------------

test_allow_quoted_prose_naming_a_force_push_when_stale if { allowed(`echo "never run git push --force here"`, stale) }
test_allow_bd_memory_body_naming_a_force_push_when_stale if { allowed("bd remember --key k \"before\ngit push --force origin x\nafter\"", stale) }
test_allow_heredoc_documenting_a_force_push_when_stale if { allowed("cat > docs/g.md <<'EOF'\ngit push --force origin x\nEOF", stale) }
