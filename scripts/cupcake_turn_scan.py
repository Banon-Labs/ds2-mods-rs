#!/usr/bin/env python3
"""ONE definition of "what did the last assistant turn actually do", for the Stop-hook signals.

Ported from er-mods-rs, where the same module is shared by several `last_assistant_*` signals that
all have to answer "did that turn do real work, or did it just talk?". Two copies of that answer are
two rules: they drift, and then two guards disagree about the same turn. Here there is currently ONE
consumer -- `.cupcake/signals/last_assistant_unexecuted_promise.sh` -- and this file carries only the
parts that consumer needs. It is still a module rather than inlined code because the next Stop guard
ported over (idle-hold, stall-on-friction) needs the same turn bucketing, and the point is that it
gets it from here instead of writing its own.

What lives here (and NOWHERE else):
  * transcript discovery         -- `latest_transcript`
  * turn bucketing               -- `split_turns`; a turn is bounded by REAL user prompts, and a
                                    tool-result carrier "user" event does NOT split one
  * blocked-on-the-user prose    -- `USER_WAIT_RE`, `blocked_on_user`
  * live background work         -- `live_background_work`: a backgrounded Bash or an async Agent
                                    launch that has not reported back yet, and whether anything in
                                    the turn is deliberately watching work that outlives it

Signal-SPECIFIC prose classification (which phrases are banned, which verbs count as work) stays in
the individual signal: that is the part each guard genuinely owns.

Import contract: every consumer must fail OPEN (emit nothing) if this module cannot be imported, so a
missing/renamed file can never wedge a session.
"""
from __future__ import annotations

import glob
import json
import os
import re
from dataclasses import dataclass, field


# --- transcript discovery ---------------------------------------------------------------------

def latest_transcript(project_dir: str | None = None) -> str | None:
    """Newest session transcript for the project, or None. Mirrors Claude Code's own key scheme:
    ~/.claude/projects/<cwd-with-slashes-replaced-by-dashes>/*.jsonl."""
    cwd = project_dir or os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()
    tdir = os.path.join(os.path.expanduser("~/.claude/projects"), cwd.replace("/", "-"))
    files = sorted(
        glob.glob(os.path.join(tdir, "*.jsonl")),
        key=lambda p: os.path.getmtime(p),
        reverse=True,
    )
    return files[0] if files else None


def load_events(path: str) -> list[dict]:
    """Parse a transcript into events, skipping unparseable lines. Returns [] on any read error."""
    events: list[dict] = []
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    ev = json.loads(line)
                except ValueError:
                    continue
                if isinstance(ev, dict):
                    events.append(ev)
    except OSError:
        return []
    return events


# --- turn bucketing ---------------------------------------------------------------------------

def is_real_user_prompt(ev: dict) -> bool:
    """A genuine user prompt starts a new turn. Tool-result 'user' events do NOT (they are the harness
    handing tool output back mid-turn), so they must not split the assistant turn."""
    if ev.get("type") != "user":
        return False
    content = ev.get("message", {}).get("content")
    if isinstance(content, str):
        return content.strip() != ""
    if isinstance(content, list):
        for block in content:
            if isinstance(block, dict) and block.get("type") == "tool_result":
                return False  # tool-result carrier, not a prompt
        return True
    return False


@dataclass
class Turn:
    """One assistant turn: its text blocks and tool_use blocks, IN ORDER.

    `blocks` is the ordered stream, each entry ("text", str) or ("tool", block-dict). Order is what
    lets a caller ask the question that separates a kept promise from a broken one: was there a tool
    call AFTER the sentence that promised one?
    """

    blocks: list[tuple] = field(default_factory=list)

    @property
    def texts(self) -> list[str]:
        return [b[1] for b in self.blocks if b[0] == "text"]

    @property
    def last_text_index(self) -> int:
        for i in range(len(self.blocks) - 1, -1, -1):
            if self.blocks[i][0] == "text":
                return i
        return -1

    def tool_after(self, index: int) -> bool:
        """Any tool_use block after `index` -- i.e. the turn kept going after that prose."""
        return any(b[0] == "tool" for b in self.blocks[index + 1:])

    def tools(self, name: str | None = None) -> list[dict]:
        out = [b[1] for b in self.blocks if b[0] == "tool"]
        if name is not None:
            out = [b for b in out if b.get("name") == name]
        return out


def split_turns(events: list[dict]) -> list[Turn]:
    """Bucket events into turns delimited by real user prompts."""
    turns = [Turn()]
    for ev in events:
        if is_real_user_prompt(ev):
            turns.append(Turn())
            continue
        if ev.get("type") != "assistant":
            continue
        for block in ev.get("message", {}).get("content", []) or []:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "text" and block.get("text"):
                turns[-1].blocks.append(("text", block["text"]))
            elif block.get("type") == "tool_use":
                turns[-1].blocks.append(("tool", block))
    return turns


def last_text_turn(turns: list[Turn]) -> Turn | None:
    """The last turn that actually said something -- the turn whose end the Stop hook is judging."""
    for turn in reversed(turns):
        if turn.texts:
            return turn
    return None


# --- blocked on the user ----------------------------------------------------------------------

# Phrases that mean the turn is legitimately BLOCKED ON THE USER (awaiting their answer/drive). Kept
# specific so an incidental "you"/"your" in ordinary prose does not over-exempt.
USER_WAIT_RE = re.compile(
    r"\bwait(?:ing)?\s+for\s+(?:the\s+)?(?:user|you)\b"
    r"|\bblocked\s+on\s+(?:the\s+)?(?:user|you)\b"
    r"|\b(?:awaiting|await)\s+(?:the\s+)?(?:user'?s?|your)\b"
    r"|\bneed\s+(?:the\s+)?(?:user|you)\s+to\b"
    r"|\b(?:for|until)\s+you\s+to\b"
    r"|\bi'?ll\s+wait\s+for\s+you\b"
    r"|\bover\s+to\s+you\b"
    r"|\bhand(?:ing)?\s+(?:it\s+|this\s+)?(?:back\s+|off\s+)?to\s+you\b",
    re.IGNORECASE,
)


def blocked_on_user(text: str) -> bool:
    return bool(USER_WAIT_RE.search(text))


# --- live background work ---------------------------------------------------------------------

# A foreground Bash call can still leave a process running after it returns. These are the forms that
# deliberately detach one.
DETACHED_RE = re.compile(
    r"\bnohup\b|\bsetsid\b|\bdisown\b|\bsystemd-run\b"
    r"|\bscreen\s+-d\b|\btmux\s+new(?:-session)?\s+-d\b"
    r"|&\s*(?:$|\n)",
)

# How many turns back a background launch can be and still count as live. See live_background_work.
# Ten is deliberately generous: a genuinely long build or subagent stays covered across ten turns of
# conversation, while the failure this bound exists to stop -- a launch whose completion never reached
# the transcript -- was measured contaminating 46 consecutive turns before the guard was silenced on
# the very turn it exists to catch.
RECENT_TURNS = 10

# The harness's own words when a tool call was LAUNCHED rather than awaited. A backgrounded Bash and
# an async subagent both answer immediately with one of these and report for real later, through a
# `<task-notification>`. Treating that immediate acknowledgement as the result is what makes a guard
# think a running job has finished -- measured on a real transcript, where "Command running in
# background with ID: ..." was read as completion and the still-running job stopped covering the turn.
BACKGROUND_LAUNCH_RE = re.compile(
    r"async agent launched|agentId:"
    r"|command running in background|running in background with id"
    r"|you will be notified when it completes",
    re.IGNORECASE,
)

# The harness injects these when a background task stops. `<status>` distinguishes a finished task
# from a progress ping.
TASK_NOTIFICATION_RE = re.compile(r"<task-notification>", re.IGNORECASE)
TASK_TOOL_USE_ID_RE = re.compile(r"<tool-use-id>\s*([^<\s]+)\s*</tool-use-id>", re.IGNORECASE)
TASK_STATUS_RE = re.compile(r"<status>\s*([^<\s]+)\s*</status>", re.IGNORECASE)
FINISHED_STATUSES = {"completed", "complete", "failed", "error", "killed", "cancelled", "canceled"}


def _result_text(block: dict) -> str:
    content = block.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for sub in content:
            if isinstance(sub, dict) and sub.get("type") == "text":
                parts.append(sub.get("text") or "")
            elif isinstance(sub, str):
                parts.append(sub)
        return "\n".join(parts)
    return ""


def _user_content_string(ev: dict) -> str:
    content = ev.get("message", {}).get("content")
    if isinstance(content, str):
        return content
    return ""


@dataclass
class BackgroundWork:
    """Background work still running at turn-end. Falsy when there is none.

    `watcher` marks the kinds whose entire purpose is work continuing past this turn -- a Monitor, a
    SendMessage to a running agent, a shell the turn deliberately detached. Those cover a promise on
    their own. A plain background job does not: see live_background_work.
    """

    description: str = ""
    watcher: bool = False

    def __bool__(self) -> bool:
        return bool(self.description)


def live_background_work(events: list[dict]) -> BackgroundWork:
    """What work is still running at turn-end.

    A caller must NOT read "something is running" as "the promise is covered". A Dark Souls II session
    the user is inspecting stays up for an hour; a promise to go fix an unrelated file is not being
    carried by it, it is being deferred behind it -- the exact disappearance the promise guard exists
    to stop, and measured on the real transcript that prompted the guard, where a game launch two
    lines earlier would otherwise have excused it. Live work covers a promise only when the promise
    WAITS ON that work ("once it lands", "whatever it finds", "if it flags") or when the live thing is
    a `watcher`.

    Otherwise deliberately GENEROUS -- every branch here suppresses a halt, so a false "something is running" is
    a quiet non-event while a false "nothing is running" would accuse an agent that did cover itself.
    Three sources, all read straight out of the transcript:

      * a Bash tool_use with `run_in_background: true` whose tool_result has not arrived (the harness
        delivers that result when the command exits, so a missing one means it is still going);
      * an Agent tool_use whose result said "Async agent launched" and for which no
        `<task-notification>` with a finished `<status>` has arrived;
      * a foreground Bash call in the last turn that detached a process itself (nohup/setsid/`&`), or
        a Monitor/SendMessage call, which wake or watch work that outlives the turn.

    BUT ONLY RECENTLY LAUNCHED WORK COUNTS (RECENT_TURNS). A launch whose completion never made it
    into the transcript -- the notification was dropped, the session was resumed into a new file, the
    task was killed -- would otherwise stay "pending" forever and silently exempt every turn after it
    for the rest of the session. Measured on real transcripts: ONE subagent launched at line 658 and
    never notified suppressed the guard across the remaining 2,800 lines, including the exact turn the
    guard exists to catch. A guard that quietly stops firing is worse than one that never shipped, so
    a launch older than RECENT_TURNS turns is treated as finished. Genuine long-running work is
    re-covered every turn it is still waited on in prose, and ten turns of conversation is far longer
    than a background job normally survives without its result landing.
    """
    pending_bg: dict[str, tuple[int, str]] = {}   # tool_use_id -> (event index, description)
    agent_uses: dict[str, int] = {}
    pending_agents: dict[str, tuple[int, str]] = {}

    # Everything before this event index is too old to still be running (see RECENT_TURNS above).
    prompt_indices = [i for i, ev in enumerate(events) if is_real_user_prompt(ev)]
    cutoff = prompt_indices[-RECENT_TURNS] if len(prompt_indices) >= RECENT_TURNS else 0

    for index, ev in enumerate(events):
        content = ev.get("message", {}).get("content")

        # Harness-injected completion notice for a background task.
        raw = _user_content_string(ev)
        if raw and TASK_NOTIFICATION_RE.search(raw):
            m = TASK_TOOL_USE_ID_RE.search(raw)
            status = TASK_STATUS_RE.search(raw)
            if m and status and status.group(1).lower() in FINISHED_STATUSES:
                # One notification shape closes both kinds of launch: a backgrounded Bash and an
                # async subagent are both reported this way.
                pending_agents.pop(m.group(1), None)
                pending_bg.pop(m.group(1), None)
            continue

        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use":
                name = block.get("name") or ""
                inp = block.get("input") or {}
                if isinstance(inp, dict) and inp.get("run_in_background"):
                    pending_bg[block.get("id") or ""] = (index, "backgrounded %s" % (name or "tool"))
                elif name in ("Agent", "Task"):
                    agent_uses[block.get("id") or ""] = index
            elif block.get("type") == "tool_result":
                tid = block.get("tool_use_id") or ""
                text = _result_text(block)
                launched = bool(BACKGROUND_LAUNCH_RE.search(text))
                if tid in pending_bg and not launched:
                    # A real result (output, exit status) -- the job is done. A "running in
                    # background" acknowledgement is NOT a result and leaves it pending.
                    pending_bg.pop(tid, None)
                if tid in agent_uses:
                    if launched:
                        pending_agents[tid] = (agent_uses[tid], "async subagent")
                    agent_uses.pop(tid, None)

    candidates = [
        (at, desc)
        for at, desc in list(pending_bg.values()) + list(pending_agents.values())
        if at >= cutoff
    ]
    if candidates:
        # Report the MOST RECENT live job; with several in flight the newest is the representative one.
        _launched_at, description = max(candidates)
        return BackgroundWork(description)

    # Last-turn-only signals: a detached process, or a tool whose whole point is work that outlives
    # the turn. Scoped to the last turn so a long-dead `nohup` from an hour ago cannot exempt forever.
    turn = last_text_turn(split_turns(events))
    if turn is not None:
        for block in turn.tools():
            name = block.get("name")
            if name in ("Monitor", "SendMessage"):
                return BackgroundWork("a %s call (work continues outside this turn)" % name, True)
            if name == "Bash":
                cmd = (block.get("input") or {}).get("command", "")
                if isinstance(cmd, str) and DETACHED_RE.search(cmd):
                    return BackgroundWork("a detached shell command", True)
    return BackgroundWork()
