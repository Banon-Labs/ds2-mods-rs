#!/usr/bin/env bash
# Cupcake signal: last_assistant_authority_agreement
#
# Scans the most recently completed assistant turn of the current session transcript and returns a
# tagged banned-phrase marker, or empty if the turn is clean. Consumed by two policies:
#   * no_authority_agreement (Stop): halts turn-end so the agent must correct.
#   * no_authority_agreement_reminder (UserPromptSubmit): injects a mandatory correction directive.
#
# Two banned classes (2026-07-17 directives):
#   Category A -- Authority-coded agreement ("You're right", "That's right", "Correct,", "Exactly,",
#     "Absolutely,", "Precisely,"). Banned outright. Emitted as  AUTH:<phrase>.
#   Category B -- Feedback-acknowledgement / receipt-announcement prose ("Point taken", "Got it",
#     "Understood", "Noted", "Fair point", "Makes sense", ...). Banned outright. Emitted as
#     ACK:<phrase>.
#
#     This used to carry an exception: the prose was allowed when the same turn recorded a beads
#     memory, on the theory that announcing you internalized feedback is fine if you actually did.
#     User directive 2026-09-20 removed it. The exception made a memory the price of replying to a
#     correction, so corrections turned into memories rather than into behaviour, and the store grew
#     by a memory per slip. Applying the correction silently was always the better answer, and the
#     repo's own rule already said so: `bd remember` is for findings "worth keeping (validated,
#     durable, non-obvious) -- not eagerly for every intermediate hypothesis or run".
# Category A always wins over Category B (checked first).
# A clean turn emits empty. Consumers also treat any non-empty UNTAGGED value as a Category-A hit
# (backward compat with crafted/bare signal values).
#
# Why a whole-turn scan, not just the last message (2026-07-17 fix): the old signal kept only the last
# assistant text block, so (a) a slip in an earlier message of a multi-message turn was overwritten by a
# later clean block and escaped, and (b) when the user interrupts a turn, the Stop event never fires at
# all -- the halt could not catch it. Scanning the whole last-completed turn fixes (a); routing the same
# signal into the UserPromptSubmit reminder (which always runs on the next prompt, even after an
# interrupt) fixes (b). "Last completed turn" = the last non-empty run of assistant text bounded by real
# user prompts; on UserPromptSubmit the just-submitted prompt opens a new empty run, so the prior turn is
# still the last non-empty one -- the same value both events need.
#
# Double-quoted spans are stripped before matching so quoting the ban (this file, the reminder text, or a
# meta-discussion like `the phrase "You're right"`) does not false-trip; a real unquoted slip
# (`You're right, ...`) still matches. Fail-open (empty output) on any error so a transcript hiccup
# cannot wedge the session.
set -uo pipefail
python3 - <<'PY' 2>/dev/null || true
import glob, json, os, re, sys

cwd = os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd()
key = cwd.replace("/", "-")
tdir = os.path.join(os.path.expanduser("~/.claude/projects"), key)
files = sorted(glob.glob(os.path.join(tdir, "*.jsonl")),
               key=lambda p: os.path.getmtime(p), reverse=True)
if not files:
    sys.exit(0)


def is_real_user_prompt(ev):
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
        # any non-tool_result content (text block, or plain) counts as a prompt
        return True
    return False


def assistant_text(ev):
    out = []
    for block in ev.get("message", {}).get("content", []) or []:
        if isinstance(block, dict) and block.get("type") == "text" and block.get("text"):
            out.append(block["text"])
    return "\n".join(out)


# Bucket assistant text into turns delimited by real user prompts; keep the last bucket that has any
# text.
turns = [{"text": []}]
try:
    with open(files[0], encoding="utf-8", errors="replace") as fh:
        for line in fh:
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if is_real_user_prompt(ev):
                turns.append({"text": []})
            elif ev.get("type") == "assistant":
                t = assistant_text(ev)
                if t:
                    turns[-1]["text"].append(t)
except OSError:
    sys.exit(0)

last_turn = ""
for bucket in reversed(turns):
    if bucket["text"]:
        last_turn = "\n".join(bucket["text"])
        break

# Strip double-quoted spans so quoting the ban does not count as using it (single quotes are left alone
# because the phrases themselves contain apostrophes, e.g. you're).
scrubbed = re.sub(r'"[^"]*"', " ", last_turn)


def phrase(m):
    for g in m.groups():
        if g:
            return g.strip()
    return m.group(0).strip()


# Category A: authority-coded agreement, not incidental words ("the correct offset").
AUTH_RE = re.compile(
    r"\b(you'?re\s+right|you\s+are\s+right|that'?s\s+right|you'?re\s+correct|you\s+are\s+correct)\b"
    r"|(?:^|[.!?]\s+|\n)\s*(correct|exactly|absolutely|precisely)[,.! ]",
    re.IGNORECASE | re.MULTILINE,
)

# Category B: feedback-acknowledgement / receipt-announcement prose.
# Distinctive multi-word receipts occur anywhere (word-boundaried); they do not appear incidentally in
# technical prose.
ACK_ANYWHERE_RE = re.compile(
    r"\b(point taken|good point|fair point|fair enough|duly noted|message received"
    r"|lesson learned|i hear you|will do|take that on board|i'?ll internalize"
    r"|(?:that|this|it)\s+makes sense)\b",
    re.IGNORECASE,
)
# Ambiguous words/short phrases: only when they open a sentence as a standalone receipt, so
# "as noted above" / "I understood the code" / "get everyone on board" do not false-trip.
ACK_SENTENCE_INITIAL_RE = re.compile(
    r"(?:^|[.!?]\s+|\n)\s*"
    r"(got it|understood|noted|makes sense|on board)"
    r"[.!,: ]",
    re.IGNORECASE | re.MULTILINE,
)

mA = AUTH_RE.search(scrubbed)
if mA:
    sys.stdout.write("AUTH:" + phrase(mA))
    sys.exit(0)

mB = ACK_ANYWHERE_RE.search(scrubbed) or ACK_SENTENCE_INITIAL_RE.search(scrubbed)
if mB:
    sys.stdout.write("ACK:" + phrase(mB))
PY
