#!/usr/bin/env python3
"""Whether this turn is stopping while the game is up, this session's Frida watcher is attached, and
nobody was handed anything.

User, 2026-11-02: "I didn't ask you to do anything except for ensure that I never have to ask you to
keep going when a project isn't completed while the game is up and Frida is attached". Before that:
"Yes, it is. And your point? Why did you pause? To make me mad?"

The instance. DarkSoulsII.exe was running and `scripts/ds2-frida-watch.py` was attached, started by
the session as a backgrounded `uv run --with frida python3 scripts/ds2-frida-watch.py --agent
$S/summon-sign-trace-frozen.js --log $S/summon-sign.jsonl`. The investigation (why an NPC summon sign
is invisible) was unfinished, and the turn ended on a finding with no question, no request and no
blocker: "Your position is pinned down, and it isn't guesswork. ... So the last sign really was made
1.5 m from you, and it's invisible for some reason other than where it is." The user had to say keep
going.

It speaks when all of these hold:

  1. `pgrep -x DarkSoulsII.exe` finds the game;
  2. a `ds2-frida-watch.py` process is alive. Found by reading /proc/*/cmdline, not `pgrep -f`
     (refused for agent commands by DS2-MODS-BLOCK-PGREP-FULL-MATCH, and a signal should not need a
     spelling its agent cannot use). A `--selftest` or `--help` run is not a watcher;
  3. this session started that watcher: the transcript has a Bash call that runs ds2-frida-watch.py
     with an `--agent` whose file name matches the live watcher's. Another session's watcher, or one
     this session never started, stays silent, so a session that never attached is never held;
  4. the closing message does not hand off: no question to the user, no request for something only
     they can do or see, no named blocker. The phrase families are `scripts/cupcake_handoff.py`, the
     same ones DS2-MODS-NO-DESCRIBED-NEXT-STEP reads, plus `IN_GAME_ASK_RE` below for an in-game
     instruction written as a bare imperative ("Walk up to the sign and press E.");
  5. the turn did not tear down the game or stop the watcher (`ds2-teardown.py`, a `kill`, a
     TaskStop), and no other background work this session started is still running (the agent is
     woken when it lands; the watcher itself never lands, so it is left out of that count);
  6. the user did not open the turn by telling the agent to stop, wait or hold, or by saying they are
     playing or about to look themselves.

Anything unknown, and any error, is silence: a signal that fails must not wedge a session.

The transcript is the one named by the Stop event's `transcript_path`, read from stdin, with
`cupcake_turn_scan.latest_transcript` only as a fallback. The newest file in the project directory is
whichever session wrote last, and with a Frida session logging hits next to it that is not reliably
the session whose turn is ending.

Prints `FRIDAATTACHED|game=<pid>|watcher=<agent file name>` or nothing.
Run `--selftest` to check the decision table.
"""
from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, SCRIPT_DIR)

GAME_EXE = "DarkSoulsII.exe"
WATCHER = "ds2-frida-watch.py"

# Test-only stand-in for pgrep and /proc: `dead` (no game), or `<game pid>|<watcher agent file
# names, comma separated, or ->`. The Stop guard tests read the transcript half for real whatever is
# running on the test machine.
MACHINE_OVERRIDE_ENV = "CUPCAKE_FRIDA_ATTACHED_MACHINE_OVERRIDE"

ANY_AGENT = "*"

# A watcher run that is not a watcher: it attaches to nothing and exits.
_NOT_A_RUN = {"--selftest", "--help", "-h"}

# Teardown or stopping the watcher in the turn being judged. `ds2-teardown.py` is the repo's
# teardown; a `kill`/`pkill`/`killall` is how a watcher is stopped by hand; TaskStop/KillShell stop
# the backgrounded Bash that holds it.
TEARDOWN_BASH_RE = re.compile(r"ds2-teardown\.py|\b(?:kill|pkill|killall)\b")
STOP_TOOLS = {"TaskStop", "KillShell", "KillBash"}

# An in-game instruction to the user written as a bare imperative. The handoff families in
# cupcake_handoff all name "you" or ask something; "Sit at a bonfire and pick Reallocate Stats" does
# neither, and it is exactly the request only the user can carry out. Every alternative here was a
# closing the replay (scripts/audit-frida-attached-false-positives.py) would otherwise have held:
# "Please start an invasion search again", "Load your character, then press F4", "To test it, sit at
# a bonfire and pick **Reallocate Stats**", "**Open the bonfire menu once more**, then say done",
# "so you can rest at a bonfire", "On the Equipment page, look for under 70% load".
_IN_GAME_VERB = (
    r"(?:walk|stand|sit|rest|warp|travel|summon|equip|unequip|press|pick|choose|select|approach"
    r"|interact|host|invade|kill\s+the|light\s+the|place\s+(?:the|a|your)|put\s+down"
    r"|go\s+(?:to|back|over|up|near)|move\s+(?:to|a\s+point)|step\s+on|look\s+(?:at|for|around)"
    r"|turn\s+around|stay\s+(?:at|in|near|on)|wait\s+(?:at|near|by)|reload\s+(?:the|your)|quit\s+to"
    r"|load\s+(?:the|your)\s+\w+|use\s+(?:the|a|an|your)|open\s+(?:the|your)\s+(?:\w+\s+){0,2}"
    r"(?:menu|dialog|screen|panel|inventory|page)|start\s+(?:an?|the)\s+(?:\w+\s+)?(?:search|invasion"
    r"|game|new\s+game)|try\s+(?:it|again|choosing|selecting|that))\b"
)
IN_GAME_ASK_RE = re.compile(
    r"(?:^|[.!:,\n]\s*|\*\*\s*|\b(?:so|then|now)\s+|\byou\s+can\s+(?:now\s+)?)" + _IN_GAME_VERB
    + r"|\bplease\b|\bsay\s+(?:done|when|go|ok)\b",
    re.IGNORECASE,
)

# The prompt that opened the judged turn told the agent to hold, or said the user is about to act.
USER_HOLD_RE = re.compile(
    r"(?:^|[.!\n]\s*)(?:ok(?:ay)?[,.]?\s+)?(?:stop|wait|pause|hold\s+(?:on|off|up)|halt|enough)\b"
    r"|\b(?:don'?t|do\s+not)\s+(?:continue|keep\s+going|go\s+on|do\s+anything)\b"
    r"|\bi'?m\s+(?:playing|in\s+the\s+game|in-game|testing\s+it|using\s+it|heading|going\s+to)\b"
    r"|\blet\s+me\s+(?:try|test|check|look|play|see)\b"
    r"|\bgive\s+me\s+a\s+(?:sec|second|minute|moment)\b",
    re.IGNORECASE,
)


# --- transcript ----------------------------------------------------------------------------------

def _segments(command: str) -> list[list[str]]:
    """Shell segments of a command as token lists. Quotes are honoured where shlex can parse them."""
    out = []
    for seg in re.split(r"&&|\|\||[;|\n]|(?<![>&0-9])&(?!>)", command or ""):
        try:
            tokens = shlex.split(seg, comments=False)
        except ValueError:
            tokens = seg.split()
        if tokens:
            out.append(tokens)
    return out


def _base(token: str) -> str:
    return os.path.basename(token.strip("'\""))


def watcher_agents_in_args(args: list[str]) -> set[str] | None:
    """The `--agent` file name of one watcher invocation's arguments, or None when it is not a run."""
    if any(a in _NOT_A_RUN for a in args):
        return None
    for i, a in enumerate(args):
        if a == "--agent" and i + 1 < len(args):
            return {_base(args[i + 1])}
        if a.startswith("--agent="):
            return {_base(a.split("=", 1)[1])}
    return {ANY_AGENT}


def watcher_runs(command: str) -> set[str]:
    """Agent file names of every ds2-frida-watch.py the command RUNS (not greps, reads or edits)."""
    agents: set[str] = set()
    for tokens in _segments(command):
        for i, tok in enumerate(tokens):
            if _base(tok) != WATCHER:
                continue
            ran = i == 0 or any(_base(t).startswith("python") for t in tokens[:i])
            if not ran:
                continue
            found = watcher_agents_in_args(tokens[i + 1:])
            if found:
                agents |= found
    return agents


def _bash_command(block: dict) -> str:
    if block.get("name") != "Bash":
        return ""
    cmd = (block.get("input") or {}).get("command")
    return cmd if isinstance(cmd, str) else ""


def _tool_uses(ev: dict) -> list[dict]:
    if ev.get("type") != "assistant":
        return []
    return [b for b in ev.get("message", {}).get("content", []) or []
            if isinstance(b, dict) and b.get("type") == "tool_use"]


def _without_tool_uses(events: list[dict], ids: set[str]) -> list[dict]:
    """The events with the named tool_use blocks removed, for a background-work count that leaves
    the watcher out."""
    out = []
    for ev in events:
        content = ev.get("message", {}).get("content") if isinstance(ev.get("message"), dict) else None
        if ev.get("type") == "assistant" and isinstance(content, list) and any(
                isinstance(b, dict) and b.get("id") in ids for b in content):
            ev = {**ev, "message": {**ev["message"], "content": [
                b for b in content if not (isinstance(b, dict) and b.get("id") in ids)]}}
        out.append(ev)
    return out


MOVED_RE = re.compile(r"moved to the background", re.IGNORECASE)


def _timed_out_into_background(events: list[dict], skip: set[str]) -> bool:
    """A foreground Bash the harness moved to the background at its timeout, still unreported.

    `cupcake_turn_scan.live_background_work` counts only calls that asked for `run_in_background`.
    The harness also backgrounds a foreground call that outlives its timeout ("Command did not
    complete within its 90s timeout and was moved to the background"), and it reports that one by
    task notification like any other. Measured on the replay: session 292f5203 ended a turn on "The
    launcher is regenerating the recommender data file before it starts the game." with exactly such
    a launch running, and its notification was the next thing in the transcript.
    """
    import cupcake_turn_scan as ts

    prompts = [i for i, ev in enumerate(events) if ts.is_real_user_prompt(ev)]
    cutoff = prompts[-ts.RECENT_TURNS] if len(prompts) >= ts.RECENT_TURNS else 0
    bash_at: dict[str, int] = {}
    pending: dict[str, int] = {}
    for index, ev in enumerate(events):
        raw = ts._task_notification_text(ev)
        if raw and ts.TASK_NOTIFICATION_RE.search(raw):
            m = ts.TASK_TOOL_USE_ID_RE.search(raw)
            status = ts.TASK_STATUS_RE.search(raw)
            if m and status and status.group(1).lower() in ts.FINISHED_STATUSES:
                pending.pop(m.group(1), None)
            continue
        if ev.get("type") not in ("assistant", "user"):
            continue
        content = ev.get("message", {}).get("content")
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use" and block.get("name") == "Bash":
                if (block.get("id") or "") not in skip:
                    bash_at[block.get("id") or ""] = index
            elif block.get("type") == "tool_result":
                tid = block.get("tool_use_id") or ""
                if tid in bash_at and MOVED_RE.search(ts._result_text(block)):
                    pending[tid] = bash_at[tid]
    return any(at >= cutoff for at in pending.values())


def _prompt_text(ev: dict) -> str:
    content = ev.get("message", {}).get("content")
    if isinstance(content, list):
        content = "\n".join(b.get("text", "") for b in content
                            if isinstance(b, dict) and b.get("type") == "text")
    return content if isinstance(content, str) else ""


def scan(events: list[dict]) -> dict | None:
    """The transcript facts, or None when this session never ran the watcher or there is no closing
    message to judge."""
    import cupcake_handoff as handoffs
    import cupcake_turn_scan as ts

    agents: set[str] = set()
    watcher_ids: set[str] = set()
    for ev in events:
        for block in _tool_uses(ev):
            found = watcher_runs(_bash_command(block))
            if found:
                agents |= found
                watcher_ids.add(block.get("id") or "")
    if not agents:
        return None

    turns = ts.split_turns(events)
    turn = ts.last_text_turn(turns)
    # Only the newest turn is judged. last_text_turn walks back past a newest turn that has no prose,
    # which at Stop means the reply to a halt is not in the transcript yet; judging the halted
    # message again would hold the session on a sentence it has already answered.
    if turn is None or not turns or turn is not turns[-1]:
        return None
    if turn.last_text_index < 0 or turn.tool_after(turn.last_text_index):
        return None
    closing = turn.text_runs[-1] if turn.text_runs else ""

    opening = ""
    for ev in reversed(events):
        if ts.is_real_user_prompt(ev):
            opening = _prompt_text(ev)
            break

    teardown = any(TEARDOWN_BASH_RE.search(_bash_command(b)) for b in turn.tools()) or any(
        b.get("name") in STOP_TOOLS for b in turn.tools())
    other_work = bool(ts.live_background_work(_without_tool_uses(events, watcher_ids))) or \
        _timed_out_into_background(events, watcher_ids)
    return {
        "agents": agents,
        "handoff": handoffs.hands_off(closing) or bool(IN_GAME_ASK_RE.search(handoffs.prose_only(closing))),
        "blocked": handoffs.names_blocker(closing),
        "teardown": teardown,
        "other_work": other_work,
        "user_hold": bool(USER_HOLD_RE.search(opening)),
    }


# --- machine -------------------------------------------------------------------------------------

def game_pid() -> int | None:
    try:
        out = subprocess.run(["pgrep", "-x", GAME_EXE], capture_output=True, text=True, timeout=5).stdout
        pids = [int(p) for p in out.split()]
    except (OSError, ValueError, subprocess.SubprocessError):
        return None
    return max(pids) if pids else None


def live_watcher_agents() -> set[str]:
    """Agent file names of every live ds2-frida-watch.py, read from /proc/*/cmdline."""
    agents: set[str] = set()
    try:
        entries = os.listdir("/proc")
    except OSError:
        return agents
    for entry in entries:
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/cmdline", "rb") as fh:
                argv = [a.decode("utf-8", "replace") for a in fh.read().split(b"\0") if a]
        except OSError:
            continue
        for i, a in enumerate(argv):
            if os.path.basename(a) == WATCHER and (
                    i == 0 or any(os.path.basename(t).startswith("python") for t in argv[:i])):
                found = watcher_agents_in_args(argv[i + 1:])
                if found:
                    agents |= found
    return agents


def machine() -> tuple[int, set[str]] | None | str:
    """(game pid, live watcher agents), None when the game is not running, or "unknown"."""
    override = os.environ.get(MACHINE_OVERRIDE_ENV)
    if override is not None:
        if override.strip() in ("", "dead"):
            return None
        try:
            pid, watchers = override.split("|", 1)
            names = {w.strip() for w in watchers.split(",") if w.strip() and w.strip() != "-"}
            return int(pid), names
        except ValueError:
            return "unknown"
    pid = game_pid()
    if pid is None:
        return None
    return pid, live_watcher_agents()


# --- decision ------------------------------------------------------------------------------------

def decide(*, facts: dict | None, game: tuple[int, set[str]] | None | str) -> str | None:
    """The agent file name of this session's live watcher when the turn end must be held. Pure."""
    if facts is None or game is None or game == "unknown":
        return None
    if facts["handoff"] or facts["blocked"] or facts["teardown"] or facts["other_work"] or facts["user_hold"]:
        return None
    _, live = game
    if not live:
        return None
    if ANY_AGENT in facts["agents"]:
        return sorted(live)[0]
    shared = (facts["agents"] & live) or (facts["agents"] if ANY_AGENT in live else set())
    return sorted(shared)[0] if shared else None


def transcript_path(event_text: str) -> str | None:
    try:
        path = json.loads(event_text).get("transcript_path")
    except (ValueError, AttributeError):
        path = None
    if isinstance(path, str) and os.path.isfile(path):
        return path
    from cupcake_turn_scan import latest_transcript

    return latest_transcript()


def gather(event_text: str) -> str | None:
    from cupcake_turn_scan import load_events

    path = transcript_path(event_text)
    if not path:
        return None
    facts = scan(load_events(path))
    if facts is None:
        return None
    game = machine()
    watcher = decide(facts=facts, game=game)
    if watcher is None:
        return None
    return f"FRIDAATTACHED|game={game[0]}|watcher={watcher}"


# --- selftest ------------------------------------------------------------------------------------

def selftest() -> int:
    bad = 0

    def check(ok: bool, what: str) -> None:
        nonlocal bad
        bad += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")

    n = [0]

    def at() -> str:
        n[0] += 1
        return f"2026-11-02T19:{n[0] // 60:02d}:{n[0] % 60:02d}.000Z"

    def user(text) -> dict:
        return {"type": "user", "timestamp": at(), "message": {"content": text}}

    def bash(command: str, background: bool = False, tid: str | None = None) -> dict:
        n[0] += 1
        inp = {"command": command}
        if background:
            inp["run_in_background"] = True
        return {"type": "assistant", "timestamp": at(), "message": {"content": [
            {"type": "tool_use", "id": tid or f"t{n[0]}", "name": "Bash", "input": inp}]}}

    def result(tid: str, text: str) -> dict:
        return {"type": "user", "timestamp": at(), "message": {"content": [
            {"type": "tool_result", "tool_use_id": tid, "content": text}]}}

    def tool(name: str, inp: dict) -> dict:
        n[0] += 1
        return {"type": "assistant", "timestamp": at(), "message": {"content": [
            {"type": "tool_use", "id": f"x{n[0]}", "name": name, "input": inp}]}}

    def said(text: str) -> dict:
        return {"type": "assistant", "timestamp": at(), "message": {"content": [{"type": "text", "text": text}]}}

    s = "/tmp/claude-1000/-home-banon-projects-ds2-mods-rs/ccc762ef/scratchpad"
    attach = (f"S={s}; cp -f scripts/frida/summon-sign-trace.js $S/summon-sign-trace-frozen.js; "
              "uv run -q --with frida python3 scripts/ds2-frida-watch.py --agent "
              "$S/summon-sign-trace-frozen.js --log $S/summon-sign.jsonl > $S/watch.log 2>&1")
    closing = ("Your position is pinned down, and it isn't guesswork. PlayerCtrl `+0x90` reads "
               "(−109.9, −17.1, −163.7), which is 3.7 m from MSB object `o00_0100_0005` at "
               "(−113.0, −17.0, −165.7). That object is one of the four `o00_0100` objects in "
               "m10_10_00_00, and the Forest of Fallen Giants has four bonfires. So the last sign really "
               "was made 1.5 m from you, and it's invisible for some reason other than where it is.")
    head = [
        user("Find out why the summon sign is invisible."),
        bash(attach, background=True, tid="watch"),
        result("watch", "Command running in background with ID: bxa3mgx0v. You will be notified when it completes."),
        user("The Crestfallen's Retreat. Until you can firmly identify where I am in that map, I think "
             "trying to co-locate a sign near me is fruitless"),
        bash(f"tail -5 {s}/summon-sign.jsonl", tid="peek"),
        result("peek", "{\"pos\": [-109.9, -17.1, -163.7]}"),
    ]
    incident = head + [said(closing)]
    alive = (766429, {"summon-sign-trace-frozen.js"})
    facts = scan(incident)

    check(facts is not None and facts["agents"] == {"summon-sign-trace-frozen.js"},
          "incident: the backgrounded uv-run watcher launch and its --agent file name are found")
    check(decide(facts=facts, game=alive) == "summon-sign-trace-frozen.js",
          "incident 2026-11-02 19:51: game up, this session's watcher attached, finding with no ask -> speaks")
    check(decide(facts=facts, game=None) is None, "game not running -> silent")
    check(decide(facts=facts, game="unknown") is None, "machine state unknown -> silent")
    check(decide(facts=facts, game=(766429, set())) is None, "game up, no watcher alive -> silent")
    check(decide(facts=facts, game=(766429, {"tick.js"})) is None,
          "the live watcher runs another agent (another session's) -> silent")

    # 2026-11-02 21:02: the next instance. A finding, then a results table, and nothing else.
    table = head + [said(
        "Pate wasn't spawned as a phantom, and the class of the returned object shows it.\n\n"
        "| entry | chr | class returned | where |\n|---|---|---|---|\n"
        "| 0x2824 (Pate) | 744009 | `CharacterCtrl` | his map spot, 210 m away, not moving |\n"
        "| 0x3024 | 866001 | `PlayerCtrl` | pooled generator, moving |")]
    check(decide(facts=scan(table), game=alive) == "summon-sign-trace-frozen.js",
          "incident 2026-11-02 21:02: a finding plus a results table, no ask, no blocker -> speaks")
    table_q = head + [said("Two spawns are traced.\n\n| question | answer |\n|---|---|\n"
                           "| phantom? | no, blocker: the class |")]
    check(decide(facts=scan(table_q), game=alive) is not None,
          "a `?` and the word blocker inside table cells do not exempt -> speaks")

    asked = head + [said(closing.replace("where it is.", "where it is. Is the sign on your screen right now?"))]
    check(decide(facts=scan(asked), game=alive) is None, "turn ends on a question to the user -> silent")
    imperative = head + [said(closing + "\n\nWalk up to where the sign should be and press E.")]
    check(decide(facts=scan(imperative), game=alive) is None,
          "turn ends on an in-game instruction only the user can carry out -> silent")
    needs = head + [said(closing + " Patching the param needs sudo for the mount, so that part is blocked on you.")]
    check(decide(facts=scan(needs), game=alive) is None, "turn names a genuine blocker -> silent")

    torn = head + [bash("python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py", tid="td"),
                   result("td", "torn down"), said("Torn down; the sign is invisible because of its flag.")]
    check(decide(facts=scan(torn), game=alive) is None, "turn ran the teardown -> silent")
    stopped = head + [tool("TaskStop", {"task_id": "bxa3mgx0v"}), result(f"x{n[0]}", "stopped"),
                      said("Watcher stopped; the finding is in the log.")]
    check(decide(facts=scan(stopped), game=alive) is None, "turn stopped the watcher's task -> silent")

    building = head + [bash("cargo build -p ds2-summon 2>&1 | tail -3", background=True, tid="bld"),
                       result("bld", "Command running in background with ID: b2. You will be notified when it completes."),
                       said(closing)]
    check(decide(facts=scan(building), game=alive) is None,
          "other background work this session started is still running (agent will be woken) -> silent")
    built = building[:-1] + [result("bld", "Finished dev profile"), said(closing)]
    check(decide(facts=scan(built), game=alive) is not None,
          "that background work has reported back -> speaks again")

    moved = head + [bash("python3 scripts/ds2-run.py", tid="run"),
                    result("run", "Command did not complete within its 90s timeout and was moved to the "
                                  "background (ID: b11b6kjo3). You will be notified when it completes."),
                    said("The launcher is regenerating the data file before it starts the game.")]
    check(decide(facts=scan(moved), game=alive) is None,
          "a foreground Bash moved to the background at its timeout is still running -> silent")
    landed = moved + [user("<task-notification>\n<task-id>b11b6kjo3</task-id>\n<tool-use-id>run</tool-use-id>\n"
                           "<status>completed</status>\n</task-notification>"),
                      said(closing)]
    check(decide(facts=scan(landed), game=alive) is not None,
          "its completion notification arrived -> speaks again")

    held = head + [user("stop, I want to look at it myself"), said("Stopped. The log is at summon-sign.jsonl.")]
    check(decide(facts=scan(held), game=alive) is None, "user told the agent to stop -> silent")
    why = head + [user("Yes, it is. And your point? Why did you pause?"), said(closing)]
    check(decide(facts=scan(why), game=alive) is not None,
          "'why did you pause?' from the user is not an order to hold -> speaks")

    never = [user("why is the sign invisible"), bash("grep -n attach scripts/ds2-frida-watch.py"),
             bash("uv run --with frida python3 scripts/ds2-frida-watch.py --selftest"), said(closing)]
    check(scan(never) is None, "grep of the watcher and a --selftest run are not this session attaching")

    stale = incident + [user("Stop hook feedback:\nThe game is up with Frida attached")]
    check(scan(stale) is None, "the halted message is not judged again while the reply is unwritten")
    mid = head + [said(closing), bash("tail -3 x", tid="m")]
    check(scan(mid) is None, "a turn that kept going after its prose did not end on it -> silent")

    for cmd, want in (
        ("python3 scripts/ds2-frida-watch.py --agent=scripts/frida/tick.js", {"tick.js"}),
        ("cd /x && timeout 600 uv run --with frida python3 /x/scripts/ds2-frida-watch.py --agent 'a b/c.js'", {"c.js"}),
        ("./scripts/ds2-frida-watch.py --pid 4", {ANY_AGENT}),
        ("sed -n 1,5p scripts/ds2-frida-watch.py", set()),
        ("python3 scripts/ds2-frida-watch.py --help", set()),
    ):
        check(watcher_runs(cmd) == want, f"watcher_runs({cmd[:60]!r}) == {want}")
    check(decide(facts={**facts, "agents": {ANY_AGENT}}, game=alive) == "summon-sign-trace-frozen.js",
          "a session watcher with no --agent matches any live watcher")
    print("selftest: OK" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        event_text = sys.stdin.read() if not sys.stdin.isatty() else ""
        line = gather(event_text)
    except Exception:  # noqa: BLE001 -- a broken signal must never take a session down
        return 0
    if line:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
