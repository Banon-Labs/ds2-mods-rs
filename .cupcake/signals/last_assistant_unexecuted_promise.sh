#!/usr/bin/env bash
# Cupcake signal: last_assistant_unexecuted_promise
#
# Consumed by:
#   * no_unexecuted_promise (Stop): halts turn-end when the turn ENDED on a first-person promise to do
#     something, and nothing in the world is going to do it.
#
# WHY THIS EXISTS (user directive 2026-08-22, in their words):
#   "How can I prevent you from ever saying 'I'll <statement of future action>' and then landing on no
#    shells running, no monitors, and no directive to me to explain why a user is required to
#    re-initiate the task."
# The instance: a turn ended "I'll re-record the directive with the shell metacharacters escaped
# rather than leave it unsaved" -- and then stopped. No tool call, no background task, no statement
# that the user had to do anything. The work evaporated and the user had to notice and re-ask. Prose
# promises are unenforceable by prose; only a Stop hook that refuses the stop closes this.
#
# THE VIOLATION IS THE CONJUNCTION OF FOUR FACTS. All four must hold, and any one missing is fine:
#   1. the turn's FINAL prose block contains a first-person future commitment to a CONCRETE action
#      ("I'll re-run the gate", "I'm going to patch it", "let me check the offsets");
#   2. no tool_use follows that prose in the turn -- nothing executed it;
#   3. nothing is carrying it: no watcher (detached shell / Monitor / SendMessage) in the turn, and
#      no live background job that the promise actually WAITS ON ("once it lands", "whatever it
#      finds"). A live job the promise never mentions is not cover -- a game session the user is
#      inspecting can be up for an hour, and the promise is deferred behind it, not carried by it;
#   4. the message does not hand the obligation to the user -- no question, no blocker statement, no
#      "once you've X", no "I'll need you to Y", no "next session".
# Emitted as  PROMISE:<the offending clause>  ; empty when the turn is clean.
#
# BIASED HARD TOWARD NOT FIRING, on purpose. A guard that cries wolf gets ignored, which is worse than
# no guard. Three deliberate narrowings:
#   * only the FINAL prose block is scanned. A mid-turn "I'll check the disassembly" that is followed
#     by the tool call doing exactly that is the normal, correct shape and must never be touched.
#   * the committed verb must be on a CONCRETE-ACTION allowlist. Stance and mental verbs ("I'll keep
#     that in mind", "I'll treat it as unproven"), verbs the same message already fulfils ("I'll
#     summarise", "I'll explain"), hedges ("I'll try to", "I'll probably") and negations ("I'll never")
#     are not promises of work and are all excluded.
#   * quoted spans, backticked spans and fenced code are stripped before matching, so quoting the ban
#     -- this file, the policy text, a test fixture -- cannot trip it.
#
# The shared half of the classification (turn bucketing, the blocked-on-user phrasing, live
# background work) is NOT reimplemented here: it comes from scripts/cupcake_turn_scan.py. That module
# is the single owner of "what did the last turn actually do", so the next Stop guard ported into
# this repo takes the same answer from it rather than growing a second copy that can drift.
# Fail-open (empty output) on any error.
set -uo pipefail
CUPCAKE_SIGNAL_REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")/../.." && pwd)"
export CUPCAKE_SIGNAL_REPO_ROOT
python3 - <<'PY' 2>/dev/null || true
import os, re, sys

sys.path.insert(0, os.path.join(os.environ.get("CUPCAKE_SIGNAL_REPO_ROOT", "."), "scripts"))
try:
    import cupcake_turn_scan as scan
except Exception:
    sys.exit(0)  # fail open: a missing helper must never wedge a session

path = scan.latest_transcript()
if not path:
    sys.exit(0)
events = scan.load_events(path)
turn = scan.last_text_turn(scan.split_turns(events))
if turn is None:
    sys.exit(0)

# (2) Anything after the closing prose executed it. At Stop the last block is prose by construction,
# but an INTERRUPTED turn can end on a tool call, and that turn did act.
idx = turn.last_text_index
if idx < 0 or turn.tool_after(idx):
    sys.exit(0)
final = turn.blocks[idx][1]

# Strip fenced code, inline-backticked spans and DOUBLE-quoted spans before prose matching, so
# quoting the ban does not count as committing it. Single quotes are left alone: the phrases
# themselves contain apostrophes (I'll / I'm).
scrubbed = re.sub(r"```.*?```", " ", final, flags=re.DOTALL)
scrubbed = re.sub(r"`[^`]*`", " ", scrubbed)
scrubbed = re.sub(r'"[^"]*"', " ", scrubbed)

# --- (1) first-person future commitment --------------------------------------------------------
# The openers an assistant actually uses to commit itself. "let's" is deliberately absent: it
# proposes joint action rather than committing the agent.
OPENER_RE = re.compile(
    r"\b(?:i['’]?ll"
    r"|i\s+will"
    r"|i['’]?m\s+going\s+to|i\s+am\s+going\s+to"
    r"|i['’]?m\s+about\s+to|i\s+am\s+about\s+to"
    r"|let\s+me)\b",
    re.IGNORECASE,
)

# Words between the opener and the verb that carry no commitment either way.
FILLER = {
    "then", "now", "also", "just", "first", "next", "go", "immediately", "still", "quickly",
    "actually", "already", "instead", "finally", "again", "simply", "promptly", "right", "away",
    "ahead", "and", "straight", "also", "briefly", "properly", "fully", "quietly", "one", "more",
}

# A hedge is not a commitment, and a negation is the opposite of one. Either kills the match.
HEDGE = {
    "probably", "likely", "maybe", "perhaps", "possibly", "hopefully", "try", "attempt", "consider",
    "think", "might", "may", "plan", "intend", "want", "hope", "aim", "prefer", "need", "have",
    "should", "could", "would", "must", "can", "expect", "guess", "suppose",
}
NEGATION = {"not", "never", "no", "nothing", "avoid", "stop", "refrain", "skip", "leave", "hold"}

# Verbs that name work only a TOOL CALL can do. Everything outside this list is left alone on
# purpose: stance verbs ("keep", "treat", "use", "follow", "remember"), verbs a message fulfils by
# itself ("summarise", "explain", "describe", "list", "note", "mention", "show", "tell"), and vague
# ones ("do", "get", "take", "handle", "continue") are not evidence of unexecuted work.
ACTIONS = {
    "run", "rerun", "launch", "relaunch", "build", "rebuild", "compile", "check", "recheck",
    "reread", "recapture", "reinstall", "rerecord", "redo", "reopen",
    "verify", "validate", "test", "retest", "fix", "patch", "repair", "correct", "add", "remove",
    "delete", "drop", "write", "rewrite", "record", "save", "update", "edit", "create", "commit",
    "push", "pull", "rebase", "merge", "open", "file", "land", "implement", "apply", "revert",
    "restore", "capture", "read", "inspect", "investigate", "search", "look", "dig",
    "trace", "disassemble", "decompile", "hook", "probe", "measure", "diff", "compare", "clean",
    "stage", "package", "install", "generate", "regenerate", "wire", "refactor", "rename", "move",
    "copy", "kill", "restart", "retry", "send", "post", "reply", "document", "extend", "factor",
    "migrate", "port", "bump", "pin", "split", "tag", "publish", "deploy", "sync", "fetch",
    "download", "upload", "convert", "parse", "dump", "scan", "grep", "find", "count", "render",
    "screenshot", "audit", "lint", "format", "benchmark", "profile", "instrument", "emit",
    "register", "enable", "disable", "toggle", "configure", "tune", "adjust", "tweak", "resolve",
    "ship", "harden", "delegate", "dispatch", "spawn", "teardown", "follow",
    # In-game input IS agent work here (~/AGENTS.md: during runtime research the agent must
    # self-drive deterministically and the user's input is blocked), so a promise to
    # press/drive/navigate/inject is a promise of a tool call like any other.
    "press", "drive", "navigate", "inject",
}

WORD_RE = re.compile(r"[A-Za-z][A-Za-z’'\-]*")


def committed_verb(tail):
    """The concrete-action verb this opener commits to, or None."""
    for raw in WORD_RE.findall(tail)[:6]:
        word = raw.lower().replace("’", "'").strip("-'")
        if word in FILLER:
            continue
        if word in NEGATION or word in HEDGE:
            return None
        if word in ACTIONS:
            return word
        # A HYPHENATED re- form is the same verb ("re-record" -> "record"). A MERGED one is not
        # derivable: stripping a bare "re" turns "report" into "port" and invents a promise that was
        # never made (caught by scripts/audit-unexecuted-promise-false-positives.py against real
        # transcripts). Merged forms are enumerated in ACTIONS instead.
        if word.startswith("re-") and word[3:] in ACTIONS:
            return word
        return None
    return None


def clause(text, start):
    """The offending sentence, collapsed and clipped, for quoting back at the agent."""
    end = len(text)
    for m in re.finditer(r"[.!?\n]", text[start:]):
        end = start + m.start() + 1
        break
    return " ".join(text[start:end].split())[:110]


promise = None
for m in OPENER_RE.finditer(scrubbed):
    if committed_verb(scrubbed[m.end():m.end() + 120]):
        promise = clause(scrubbed, m.start())
        break
if not promise:
    sys.exit(0)

# --- (4) the obligation was handed to the user -------------------------------------------------
# A question, a stated blocker, a dependency on something the user must do, or an explicit
# "this needs re-initiating" all mean the user knows the ball is theirs. Checked over the whole
# closing message, not just the promising sentence.
HANDOFF_RE = re.compile(
    r"\?"
    r"|\bonce\s+(?:you|your|the\s+user)\b"
    r"|\bwhen\s+(?:you|your|the\s+user)\b"
    r"|\bafter\s+(?:you|your|the\s+user)\b"
    r"|\bif\s+(?:you|your|the\s+user)\b"
    r"|\bas\s+soon\s+as\s+you\b"
    r"|\b(?:while|until)\s+you\b"
    r"|\bunless\s+you\b"
    r"|\byou'?(?:ll|d)\s+(?:need|have)\s+to\b|\byou\s+will\s+need\s+to\b"
    r"|\byou\s+(?:need|have)\s+to\b"
    r"|\bneed\s+you\s+to\b|\bneed\s+the\s+user\s+to\b"
    r"|\blet\s+me\s+know\b|\btell\s+me\b|\bping\s+me\b|\bcome\s+back\s+to\s+me\b"
    r"|\byour\s+call\b|\bsay\s+the\s+word\b|\bup\s+to\s+you\b"
    r"|\bre-?initiate\b|\bre-?ask\b|\bask\s+me\s+again\b|\bask\s+again\b"
    r"|\bnext\s+(?:session|turn|time)\b|\bfollow-?up\s+turn\b"
    r"|\bblocked\s+on\b|\bcan'?t\s+proceed\b|\bcannot\s+proceed\b"
    r"|\brequires?\s+(?:you|your|a\s+user|the\s+user)\b"
    r"|\bneeds?\s+(?:you|your)\b",
    re.IGNORECASE,
)
if HANDOFF_RE.search(scrubbed) or scan.blocked_on_user(scrubbed):
    sys.exit(0)

# --- (3) something is already carrying it ------------------------------------------------------
# "Something is running" is not by itself cover. An Elden Ring session the user is inspecting can be
# up for an hour, and a promise to go fix an unrelated file is not being CARRIED by it -- it is being
# deferred behind it, which is the disappearance this guard exists to stop (measured: the reported
# instance was silenced by an unrelated game launch two turns earlier). A live job covers a promise
# in exactly two cases: THIS turn started it (the harness wakes the agent when it exits, so the agent
# genuinely comes back), or the promise explicitly waits on its result.
WAITS_ON_RESULT_RE = re.compile(
    r"\b(?:when|once|after|as\s+soon\s+as|the\s+moment|while|until)\b[^.\n]{0,60}?"
    r"\b(?:it|that|they|its|their|the\s+\w+)\b"
    r"|\b(?:its|their)\s+(?:output|log|logs|result|results|findings?|verdict)\b"
    r"|\bwhatever\s+(?:it|they)\s+\w+"
    r"|\bif\s+(?:it|that|they)\s+\w+",
    re.IGNORECASE,
)
live = scan.live_background_work(events)
if live and (live.watcher or WAITS_ON_RESULT_RE.search(scrubbed)):
    sys.exit(0)

sys.stdout.write("PROMISE:" + promise)
PY
