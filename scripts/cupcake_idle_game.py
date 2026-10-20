#!/usr/bin/env python3
"""Whether this session left DARK SOULS II running after it read the run's evidence.

User, 2026-10-19: "Is there a reason the game is still up? If not, we lack a rego policy to tear it
down as soon as evidence is collected". Earlier, 2026-09-26: "I just want the game to go down when
you're done collecting evidence."

The two existing teardown paths both key on a commit:

  * scripts/ds2-teardown-after-evidence-push.py tears down after the `git push` that spends a run;
  * DS2-MODS-NO-UNSPENT-RUN-EVIDENCE halts a turn end while the running build is an unpushed
    game-code commit held in a local worktree HEAD.

The 2026-10-19 instance slipped past both. Commit d5e5404 on feat/config-hot-reload was launched at
03:21:27 UTC, its `live-config` lines were read out of ds2-loader.log at 03:21:49 and 03:22:00, and
then the commit was amended twice (847fd0c, c5ec9a5) for rustfmt and rustdoc. The running build was
now a commit no worktree HEAD contained, so the unspent-evidence signal went silent, no push was
coming for it, and the game sat idle through the 03:25:17 turn end until the next launch at 03:27:23.

This module does not ask about commits. It speaks when all of these hold:

  1. the session launched the game: a real `ds2-run.py` launch (cupcake_launch_deferral's
     `is_launch_command`) is in the transcript;
  2. evidence was read after the LAST such launch: a later tool call names `ds2-loader.log` (a Bash
     command, or a Read/Grep path). A turn that launched and has not read the log yet is still
     collecting, and stays silent;
  3. `pgrep -x DarkSoulsII.exe` finds the game, and its process started within LAUNCH_WINDOW_S of
     that launch, so it is the run this session started and not one somebody launched later;
  4. nobody gave a reason to keep it:
       * the user, since the prompt that preceded the launch, asked to leave it running or said
         they are playing (KEEP_USER_RE);
       * the user wrote anything after the launch, within HAND_TEST_WINDOW_S of the turn end, that
         is not a complaint that it is still up -- they are using the run (the hand-test exemption
         DS2-MODS-NO-UNSPENT-RUN-EVIDENCE has for the same reason);
       * the agent, in any prose since the launch, said it is keeping the game up and why
         (KEEP_AGENT_RE: "leaving the game running because/until/for/so ...").
       Any prose since the launch, not only the closing message: a reply written straight after a
       Stop halt is not in the transcript yet when the next Stop hook reads it (cupcake_turn_scan's
       `last_text_turn`), so the reason is honoured one halt later instead of never;
  5. DS2-MODS-NO-UNSPENT-RUN-EVIDENCE is not already speaking. When it is, the push it demands tears
     the game down, and a second halt saying "tear it down" would contradict "push it now" (the push
     guard wants a run behind it).

Anything unknown, and any error, is silence: a signal that fails must not wedge a session.

The Stop policy `no_idle_game_after_evidence` halts and hands the agent the existing bare teardown,
`python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py`, which DS2-MODS-TEARDOWN-MUST-RELAUNCH
allows as a whole command. It does not run the teardown itself: a signal is a measurement, and a
kill from a Stop hook would fire on a misjudged turn with nobody to see why.

Memory ds2-always-launch-over-running-game-2026-09-25 ("always launch over") is about not WITHHOLDING
a launch because the game is up. This guard never asks for a launch to be withheld; it only asks for
a finished run to be closed.

Prints `IDLEGAME|pid=<pid>|launched=<iso timestamp of the launch>` or nothing.
Run `--selftest` to check the decision table.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from datetime import datetime, timezone

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

# How long after the launch command the game process may start and still be that launch's game.
# ds2-run.py builds nothing, tears the old session down and starts Steam's game; 2026-10-20 runs
# had the process up within 22 seconds.
LAUNCH_WINDOW_S = 300

# Same window DS2-MODS-NO-UNSPENT-RUN-EVIDENCE uses for "the user is hand-testing this run".
HAND_TEST_WINDOW_S = 30 * 60

# Test-only: `dead`, or `<pid>|<process start epoch seconds>`, stands in for pgrep and /proc so the
# Stop guard tests read the transcript half for real whatever is running on the test machine.
MACHINE_OVERRIDE_ENV = "CUPCAKE_IDLE_GAME_MACHINE_OVERRIDE"

GAME_EXE = "DarkSoulsII.exe"
LOG_NAME = "ds2-loader.log"

_OBJ = r"(?:the\s+|this\s+)?(?:game|ds2|dark\s+souls(?:\s+ii|\s+2)?|darksoulsii\.exe|it|run|session)"
_STATE = r"(?:running|up|open|alive|live|going)"

KEEP_USER_RE = re.compile(
    rf"\b(?:leave|keep)\s+{_OBJ}\s+{_STATE}\b"
    rf"|\b(?:don'?t|do\s+not|never)\s+(?:close|kill|tear\s*down|teardown|shut\s*down|stop|quit)\s+{_OBJ}\b"
    r"|\bi'?m\s+(?:playing|in\s+the\s+game|in-game|testing\s+it|using\s+it)\b",
    re.IGNORECASE,
)

# A user line about the game being up that is a complaint, not a request to keep it.
COMPLAINT_RE = re.compile(
    r"\b(?:still|why\s+is)\b[^.?!\n]{0,40}\b(?:up|running|open)\b"
    r"|\btear\w*\s+(?:it\s+)?down\b|\b(?:close|kill|shut)\w*\b[^.?!\n]{0,15}\bgame\b",
    re.IGNORECASE,
)

KEEP_AGENT_RE = re.compile(
    rf"\b(?:leav(?:e|es|ing)|keep(?:s|ing)?)\s+{_OBJ}\s+{_STATE}\b"
    r"[^\n]{0,200}?\b(?:because|for|until|so|while|since)\b",
    re.IGNORECASE,
)

# "no reason to keep it running", "I'm not leaving the game up" are the opposite of a reason.
_NEGATION_RE = re.compile(r"(?:\bno\s+reason\s+to|\bnot|n't|\bnothing\s+to|\bno\s+need\s+to)\s*$", re.IGNORECASE)


def agent_keeps(text: str) -> bool:
    for m in KEEP_AGENT_RE.finditer(text or ""):
        if not _NEGATION_RE.search(text[max(0, m.start() - 30):m.start()]):
            return True
    return False


def user_keeps(text: str) -> bool:
    return bool(KEEP_USER_RE.search(text or ""))


def complaint(text: str) -> bool:
    return bool(COMPLAINT_RE.search(text or ""))


# --- transcript ----------------------------------------------------------------------------------

def _when(ev: dict) -> datetime | None:
    try:
        return datetime.fromisoformat(str(ev["timestamp"]).replace("Z", "+00:00"))
    except (KeyError, TypeError, ValueError):
        return None


def _tool_uses(ev: dict) -> list[dict]:
    if ev.get("type") != "assistant":
        return []
    return [b for b in ev.get("message", {}).get("content", []) or []
            if isinstance(b, dict) and b.get("type") == "tool_use"]


def _reads_log(block: dict) -> bool:
    """A tool call that reads the loader's log: the evidence."""
    args = block.get("input") or {}
    fields = [args.get("command"), args.get("file_path"), args.get("path"), args.get("pattern")]
    return block.get("name") in ("Bash", "Read", "Grep") and any(
        isinstance(f, str) and LOG_NAME in f for f in fields)


def _user_text(ev: dict) -> str:
    content = ev.get("message", {}).get("content")
    if isinstance(content, list):
        content = "\n".join(b.get("text", "") for b in content
                            if isinstance(b, dict) and b.get("type") == "text")
    return content if isinstance(content, str) else ""


def scan(events: list[dict]) -> dict | None:
    """The transcript facts, or None when the session never launched the game."""
    from cupcake_launch_deferral import is_launch_command
    from cupcake_turn_scan import assistant_text, is_real_user_prompt
    from cupcake_unspent_evidence import user_words

    launch_i = None
    for i, ev in enumerate(events):
        for block in _tool_uses(ev):
            if block.get("name") == "Bash" and is_launch_command((block.get("input") or {}).get("command", "")):
                launch_i = i
    if launch_i is None or _when(events[launch_i]) is None:
        return None
    launched = _when(events[launch_i])
    # The prompt that preceded the launch opens the window the user's "leave it running" counts in.
    prompt_i = 0
    for i in range(launch_i, -1, -1):
        if is_real_user_prompt(events[i]) and user_words(events[i]):
            prompt_i = i
            break
    evidence = any(_reads_log(b) for ev in events[launch_i + 1:] for b in _tool_uses(ev))
    # The launch event itself may carry prose before its tool call.
    agent = any(agent_keeps(assistant_text(ev)) for ev in events[launch_i:] if ev.get("type") == "assistant")
    user_keep = any(user_words(ev) and user_keeps(_user_text(ev)) for ev in events[prompt_i:])
    end = max((w for w in (_when(ev) for ev in events) if w is not None), default=None)
    hand = any(
        user_words(ev) and not complaint(_user_text(ev))
        and (w := _when(ev)) is not None and w > launched and end is not None
        and (end - w).total_seconds() <= HAND_TEST_WINDOW_S
        for ev in events[launch_i + 1:]
    )
    return {"launched": launched, "evidence": evidence, "agent_reason": agent,
            "user_keep": user_keep, "hand_test": hand}


# --- machine -------------------------------------------------------------------------------------

def game_process() -> tuple[int, float] | None:
    """(pid, start epoch seconds) of the newest DarkSoulsII.exe, or None. Exact name match only."""
    try:
        out = subprocess.run(["pgrep", "-x", GAME_EXE], capture_output=True, text=True, timeout=5).stdout
        pids = [int(p) for p in out.split()]
        if not pids:
            return None
        with open("/proc/stat") as fh:
            btime = next(int(line.split()[1]) for line in fh if line.startswith("btime "))
        tick = os.sysconf("SC_CLK_TCK")
        newest = None
        for pid in pids:
            with open(f"/proc/{pid}/stat") as fh:
                # Field 22 (starttime) counted after the `)` that closes comm, which may hold spaces.
                rest = fh.read().rsplit(")", 1)[1].split()
            start = btime + int(rest[19]) / tick
            if newest is None or start > newest[1]:
                newest = (pid, start)
        return newest
    except (OSError, ValueError, StopIteration, IndexError, subprocess.SubprocessError):
        return None


def machine() -> tuple[int, float] | None | str:
    """The game process, None when there is none, or "unknown"."""
    override = os.environ.get(MACHINE_OVERRIDE_ENV)
    if override is not None:
        if override.strip() in ("", "dead"):
            return None
        try:
            pid, start = override.split("|", 1)
            return int(pid), float(start)
        except ValueError:
            return "unknown"
    return game_process()


def unspent_speaks() -> bool:
    """Whether DS2-MODS-NO-UNSPENT-RUN-EVIDENCE is halting this turn end already."""
    override = os.environ.get("CUPCAKE_UNSPENT_EVIDENCE_OVERRIDE")
    if override is not None and not os.environ.get("CUPCAKE_UNSPENT_EVIDENCE_MACHINE_OVERRIDE"):
        return bool(override.strip())
    from pathlib import Path

    import cupcake_unspent_evidence as unspent

    repo = Path(os.environ.get("CLAUDE_PROJECT_DIR") or os.path.dirname(SCRIPT_DIR))
    return unspent.gather(repo) is not None


# --- decision ------------------------------------------------------------------------------------

def decide(*, facts: dict | None, game: tuple[int, float] | None | str, unspent: bool) -> bool:
    """True when the game this session launched is idle after its evidence was read. Pure."""
    if facts is None or not facts["evidence"] or game is None or game == "unknown":
        return False
    if facts["agent_reason"] or facts["user_keep"] or facts["hand_test"] or unspent:
        return False
    _, start = game
    offset = start - facts["launched"].timestamp()
    return -10 <= offset <= LAUNCH_WINDOW_S


def gather() -> str | None:
    from cupcake_turn_scan import latest_transcript, load_events

    path = latest_transcript()
    if not path:
        return None
    facts = scan(load_events(path))
    if facts is None or not facts["evidence"]:
        return None
    game = machine()
    if game is None or game == "unknown":
        return None
    if not decide(facts=facts, game=game, unspent=False):
        return None
    if unspent_speaks():
        return None
    when = facts["launched"].astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return f"IDLEGAME|pid={game[0]}|launched={when}"


# --- selftest ------------------------------------------------------------------------------------

def selftest() -> int:
    bad = 0

    def check(ok: bool, what: str) -> None:
        nonlocal bad
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")

    def at(minute: int, second: int = 0) -> str:
        return f"2026-10-20T03:{minute:02d}:{second:02d}.000Z"

    def epoch(minute: int, second: int = 0) -> float:
        return datetime.fromisoformat(at(minute, second).replace("Z", "+00:00")).timestamp()

    def user(minute: int, text) -> dict:
        return {"type": "user", "timestamp": at(minute), "message": {"content": text}}

    def bash(minute: int, command: str, second: int = 0) -> dict:
        return {"type": "assistant", "timestamp": at(minute, second), "cwd": "/",
                "message": {"content": [{"type": "tool_use", "id": f"t{minute}{second}",
                                         "name": "Bash", "input": {"command": command}}]}}

    def said(minute: int, text: str) -> dict:
        return {"type": "assistant", "timestamp": at(minute),
                "message": {"content": [{"type": "text", "text": text}]}}

    wt = "/home/banon/projects/ds2-mods-rs-wt-hotreload"
    log = '"/home/banon/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/ds2-loader.log"'
    launch = f"cd {wt} && python3 {wt}/scripts/ds2-run.py --no-change-appearance 2>&1 | tail -30"
    # 2026-10-20 03:21-03:25, compressed: launch, read the log twice, amend for fmt, end the turn.
    incident = [
        user(10, "What about the bonfire menu items we add?"),
        bash(21, launch, 27),
        bash(21, f"grep -E 'live-config' {log}; pgrep -x DarkSoulsII.exe", 49),
        bash(22, f"for i in $(seq 1 40); do grep -q 'bonfire rows' {log} && break; done", 0),
        bash(22, f"cd {wt} && git commit -q --amend --no-edit -a && bash scripts/check.sh", 39),
        said(25, "Blocked on `scripts/check.sh` for the amended hot-reload commit."),
    ]
    alive = (760735, epoch(21, 40))
    facts = scan(incident)

    check(facts is not None and facts["evidence"], "incident: launch and a later ds2-loader.log read are found")
    check(decide(facts=facts, game=alive, unspent=False),
          "incident 2026-10-20: game alive, evidence read, turn ends with no reason -> speaks")
    check(not decide(facts=facts, game=None, unspent=False), "game not running -> silent")
    check(not decide(facts=facts, game="unknown", unspent=False), "game liveness unknown -> silent")
    check(not decide(facts=facts, game=alive, unspent=True),
          "unspent-evidence guard already demanding the push -> silent (no contradictory halt)")
    check(not decide(facts=facts, game=(1, epoch(21, 40) + LAUNCH_WINDOW_S + 60), unspent=False),
          "game started long after this session's launch (someone else's run) -> silent")
    check(not decide(facts=facts, game=(1, epoch(15)), unspent=False),
          "game started before this session's last launch -> silent")
    check(not decide(facts=None, game=alive, unspent=False), "session never launched the game -> silent")

    collecting = incident[:2] + [said(21, "Waiting on the launch script's testimony.")]
    check(not decide(facts=scan(collecting), game=alive, unspent=False),
          "launched, log not read yet (still collecting) -> silent")

    relaunched = incident[:3] + [bash(27, launch, 23), said(27, "Waiting on the launch.")]
    check(not decide(facts=scan(relaunched), game=(1, epoch(27, 40)), unspent=False),
          "evidence read for an EARLIER launch only, then relaunched -> silent until the new run's log is read")

    asked = [user(10, "launch it and leave the game running, I'll play after")] + incident[1:]
    check(not decide(facts=scan(asked), game=alive, unspent=False), "user asked to leave it running -> silent")
    playing = incident[:3] + [user(23, "i'm playing, don't close it")] + incident[3:]
    check(not decide(facts=scan(playing), game=alive, unspent=False), "user says they are playing -> silent")

    reason = incident[:-1] + [said(25, "Leaving the game running until the 30 s reload poll lands in the log.")]
    check(not decide(facts=scan(reason), game=alive, unspent=False),
          "agent states a reason to keep it up -> silent")
    noreason = incident[:-1] + [said(25, "There's no reason to keep the game up, so it can go.")]
    check(decide(facts=scan(noreason), game=alive, unspent=False),
          "'no reason to keep the game up' is not a reason -> speaks")
    bare = incident[:-1] + [said(25, "Leaving the game running.")]
    check(decide(facts=scan(bare), game=alive, unspent=False), "'leaving the game running' with no why -> speaks")

    hand = incident[:3] + [user(23, "the rename row is greyed out")] + incident[3:]
    check(not decide(facts=scan(hand), game=alive, unspent=False),
          "user wrote about the run after the launch (hand test) -> silent")
    stale = hand + [said(59, "Done.")]
    check(decide(facts=scan(stale), game=alive, unspent=False),
          "user's message is over HAND_TEST_WINDOW_S before the turn end -> speaks")
    why = incident[:3] + [user(23, "Is there a reason the game is still up?")] + incident[3:]
    check(decide(facts=scan(why), game=alive, unspent=False),
          "the user asking why the game is still up is not a hand test -> speaks")
    old_keep = [user(1, "leave the game running"), bash(2, launch), user(9, "now do the bonfire rows")] + incident[1:]
    check(decide(facts=scan(old_keep), game=alive, unspent=False),
          "a 'leave it running' before an earlier launch does not cover a later one -> speaks")

    read_tool = incident[:2] + [{"type": "assistant", "timestamp": at(21, 50), "message": {"content": [
        {"type": "tool_use", "id": "r", "name": "Read",
         "input": {"file_path": "/x/Game/ds2-loader.log"}}]}}, said(25, "Read it.")]
    check(decide(facts=scan(read_tool), game=alive, unspent=False), "a Read of ds2-loader.log is evidence too")
    dry = [user(1, "x"), bash(2, f"python3 {wt}/scripts/ds2-run.py --dry-run"),
           bash(3, f"grep x {log}"), said(4, "ok")]
    check(scan(dry) is None, "a --dry-run is not a launch")

    for text, want in (
        ("leave it running", True), ("keep the game up please", True), ("don't kill the game", True),
        ("I'm playing", True), ("keep up_enters_list on", False), ("Is there a reason the game is still up?", False),
    ):
        check(user_keeps(text) == want, f"user_keeps({text!r}) == {want}")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        line = gather()
    except Exception:  # noqa: BLE001 -- a broken signal must never take a session down
        return 0
    if line:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
