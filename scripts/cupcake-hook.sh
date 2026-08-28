#!/usr/bin/env bash
# Run a cupcake hook evaluation against THIS repo's .cupcake tree.
#
# WHY THIS FILE EXISTS RATHER THAN `cupcake eval` DIRECTLY IN settings.json.
#
# 1. NEW PERMISSION MODES ARE FATAL. cupcake 0.5.2 deserializes `permission_mode` into a closed
#    enum and exits 1 on anything outside {default, plan, acceptEdits, bypassPermissions}. Claude
#    Code ships an `auto` mode, so with it active the hook fails with
#
#        Error: unknown variant `auto`, expected one of `default`, `plan`, ...
#
#    and the policy silently does not run. An unrecognised mode must degrade to "evaluate anyway",
#    never to "evaluate nothing", so it is rewritten to `default` -- the least privileged of the
#    four, because a mode we cannot interpret must not be treated as more permissive than the real
#    one. Measured live on 2026-08-24 in the sibling repo, where it took EVERY hook down at once
#    while the test suite stayed green.
#
# 2. THE DEFAULT LOG LEVEL IS `info`. Unset, cupcake writes ~60 INFO lines to stderr on every
#    single hook -- policy-by-policy parse chatter, WASM compilation, signal gathering.
#
# 3. THE POLICY DIRECTORY IS PINNED. Without `--policy-dir` cupcake discovers `.cupcake/` from the
#    process cwd, which is fine for the interactive session and wrong for anything that invokes the
#    hook from elsewhere (the regression test, a subagent with a pinned cwd, a git worktree).
#
# SCOPE. This wrapper is currently wired for the **Stop** event only (see .claude/settings.json);
# PreToolUse/PostToolUse/SessionStart/UserPromptSubmit go through the user's global
# ~/.claude/hooks/cupcake-hook.sh, which reaches the same .cupcake tree by cwd discovery. The
# sibling repo's copy of this file carries a fourth fix -- restoring unquoted newlines in Bash
# `tool_input.command` that the engine erases before evaluation -- which is deliberately NOT here:
# it exists for the command-text guards on PreToolUse, and a Stop payload has no `tool_input`.
#
# stdout, stdin and the exit code are passed through untouched: the exit code is how cupcake
# denies, so swallowing it would disable the guard just as thoroughly as bug 1.
set -uo pipefail

CUPCAKE_BIN="${CUPCAKE_BIN:-cupcake}"
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# The raw event is held so a normaliser failure can still be evaluated. Feeding cupcake an empty
# payload would fail the hook open, which is the one outcome this file exists to prevent.
raw_event=$(cat)
normalized=$(printf '%s' "$raw_event" | python3 -c '
import json, sys

KNOWN = {"default", "plan", "acceptEdits", "bypassPermissions"}
raw = sys.stdin.read()
try:
    payload = json.loads(raw)
except (ValueError, TypeError):
    sys.stdout.write(raw)
    sys.exit(0)
if isinstance(payload, dict) and payload.get("permission_mode") not in KNOWN:
    if "permission_mode" in payload:
        payload["permission_mode"] = "default"
sys.stdout.write(json.dumps(payload))
')
if [ $? -ne 0 ] || [ -z "$normalized" ]; then
	normalized=$raw_event
fi

printf '%s' "$normalized" | "$CUPCAKE_BIN" eval \
	--harness claude \
	--log-level error \
	--policy-dir "$repo_root/.cupcake" \
	--global-config "$repo_root/.cupcake/rulebook.yml"
exit "${PIPESTATUS[1]}"
