#!/usr/bin/env bash
# Cupcake signal: last_assistant_launch_deferral
#
# Consumed by:
#   * no_launch_deferral (Stop): halts a turn that withholds a DARK SOULS II launch because the game
#     is running or in use, or that ends with a committed game-code build nobody launched.
#
# Why this exists. User directive 2026-09-26, verbatim: "if you ever stop runtime testing new
# builds while we are running claude, I'm going to lobotomize you". The agent had a committed build
# the push guard required to be run, refused to launch it because the user's DS2 session was up and
# "the user plans to go back to it", and told a subagent not to launch or attach. The bd memory
# `ds2-always-launch-over-running-game-2026-09-25` already said to launch over a running game; it
# did not stop it.
#
# The lexicon, the quoting carve-out, what counts as a launch and how a commit is recognised all
# live in scripts/cupcake_launch_deferral.py, pinned by scripts/test-launch-deferral-signal.py.
#
# Emits one line, or nothing when the turn is clean:
#   LAUNCHDEFER|kind=prose|phrase=<the sentence>     closing prose withheld a launch
#   LAUNCHDEFER|kind=agent|phrase=<the instruction>  a subagent was told not to launch/attach
#   LAUNCHDEFER|kind=unrun|phrase=<sha>              a game-code commit this session, never launched
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, sys

root = os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", ".")
sys.path.insert(0, os.path.join(root, "scripts"))
try:
    import cupcake_turn_scan as scan
    import cupcake_launch_deferral as ld
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
events = scan.load_events(path)
if not events:
    sys.exit(0)

# The user can overrule the rule in their own latest prompt; nothing an agent writes can.
last_prompt = ""
for ev in events:
    if scan.is_real_user_prompt(ev):
        content = ev.get("message", {}).get("content")
        if isinstance(content, str):
            last_prompt = content
        else:
            last_prompt = "\n".join(
                b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"
            )
if ld.user_opted_out(last_prompt):
    sys.exit(0)


def emit(kind, phrase):
    phrase = " ".join(str(phrase).split())
    print("LAUNCHDEFER|kind=%s|phrase=%s" % (kind, phrase))
    sys.exit(0)


turns = scan.split_turns(events)
turn = scan.last_text_turn(turns)
if turn is not None:
    blocks = turn.blocks

    def launched_after(i):
        for kind, value in blocks[i + 1:]:
            if kind == "tool" and value.get("name") == "Bash":
                if ld.is_launch_command((value.get("input") or {}).get("command", "")):
                    return True
        return False

    for i, (kind, value) in enumerate(blocks):
        if kind == "text":
            phrase = ld.defers_launch(value)
            if phrase and not launched_after(i):
                emit("prose", phrase)
        elif kind == "tool" and value.get("name") in ("Agent", "Task", "SendMessage"):
            inp = value.get("input") or {}
            prompt = "\n".join(str(v) for v in inp.values() if isinstance(v, str))
            phrase = ld.agent_prompt_withholds_launch(prompt)
            if phrase and not launched_after(i):
                emit("agent", phrase)

# A game-code commit this session with no launch after it. Every Bash call in the transcript, in
# order, paired with its result so the `[branch sha]` line git commit prints names the commit.
results = {}
for ev in events:
    if ev.get("type") != "user":
        continue
    content = ev.get("message", {}).get("content")
    if not isinstance(content, list):
        continue
    for b in content:
        if isinstance(b, dict) and b.get("type") == "tool_result":
            results[b.get("tool_use_id")] = (scan._result_text(b), "1" if b.get("is_error") else "0")
steps = []
for ev in events:
    if ev.get("type") != "assistant":
        continue
    for b in ev.get("message", {}).get("content", []) or []:
        if isinstance(b, dict) and b.get("type") == "tool_use" and b.get("name") == "Bash":
            command = (b.get("input") or {}).get("command", "")
            result, errored = results.get(b.get("id"), ("", "0"))
            steps.append((command if isinstance(command, str) else "", result, errored))

repo = os.environ.get("CLAUDE_PROJECT_DIR") or root
sha = ld.unrun_game_commit(steps, lambda s: ld.commit_touches_game_code(s, repo))
if sha:
    emit("unrun", sha)
PY
