#!/usr/bin/env python3
"""Detect a closing message that calls a change a fix when no run showed the change working.

User, 2026-09-11: "If someone ever says real fix to me and doesn't put it in airquotes, I look at
them like this" -- and then, when nothing stopped it: "We are supposed to have a rego policy that
stops you from saying 'fix' without runtime evidence". The turn that prompted it had edited a crate
that ships inside the game, never run the game, and closed on "the real fix". It failed live on the
next launch.

Ported from `../er-mods-rs/scripts/cupcake_fix_claim.py` on 2026-09-25. The prose half -- what
counts as a claim, a hedge, a blocker, host work -- is unchanged, because it is about English and
not about either game. The two halves that are about the game were rewritten for this one:

  * `_RUNTIME_ARTIFACT` names what a DARK SOULS II run leaves behind: `<Game>/ds2-loader.log` and
    its `.prev`, the `ds2-crash-*.txt` family `ds2-crash-logging-core` writes, and the scripts
    that read a live session (`ds2-frida-watch.py`, `ds2-teardown.py --status`, and the
    live-process readers under `scripts/`).
  * `runtime_crates` walks this workspace's manifests, which name their path dependencies through
    `[workspace.dependencies]` (`ds2-x.workspace = true`) rather than `path = "../ds2-x"`. The er
    walk reads only the second form and would find the two cdylib shells and nothing else here.

Kept as an importable module rather than an inline heredoc so it is unit-testable without a live
transcript, the way `cupcake_narrated_action.py` and `cupcake_unbacked_claim.py` are.
`.cupcake/signals/last_assistant_fix_claim_without_runtime_evidence.sh` imports it, and so does
`scripts/test-fix-claim-classifier.py`, so the test cannot pass against a classifier production
does not run.

The vocabulary this module owns, and the boundary it must not cross: the present participle
"fixing" is left alone, because `DS2-MODS-NO-FUTURE-TENSE-COMMITMENT` and the promise guards own
announcements of work in flight. Charging one sentence twice with two different corrections is how
a guard layer becomes noise, so no pattern here matches it.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

# --- (1) the closing prose called a change a fix ------------------------------------------------
# An explicit alternation rather than the bare stem. A turn that edits a runtime crate and mentions
# the word at all is not the failure: "this needs a fix in the loader too" names remaining work and
# "the fix for the other bug is unrelated" is ordinary reference. What the directive is about is a
# change being asserted to be the fix, so every branch below carries that assertion in its grammar.
#
# `real fix` stands alone because it is the phrase the user quoted, twice, and it is the one that
# cannot be said honestly about an unrun change.
_FIX_CLAIM = re.compile(
    r"\breal\s+fix\b"
    r"|\b(?:this|that|it|here)(?:'s|\s+is|\s+was)\s+(?:the|a|my|our)\s+"
    r"(?:real\s+|actual\s+|genuine\s+|proper\s+|right\s+|correct\s+|only\s+|true\s+|whole\s+)?fix\b"
    r"|\b(?:the|a|my|our)\s+"
    r"(?:real\s+|actual\s+|genuine\s+|proper\s+|right\s+|correct\s+|only\s+|true\s+|whole\s+)?"
    r"fix\s+(?:works|worked|holds|holds\s+up|landed|is\s+in\b|is\s+live\b|is\s+done\b)"
    # Naming the mechanism as the answer: "the fix is `CloseAsFailed`". The lookahead is what keeps
    # the honest plan out of it -- "the fix is to gate it, which I have not done yet" is remaining
    # work, and so is "the fix is needed in the loader too". The difference between a plan and a
    # claim is the infinitive, so that is what the pattern reads.
    r"|\b(?:the|this|that|my|our)\s+"
    r"(?:real\s+|actual\s+|genuine\s+|proper\s+|right\s+|correct\s+|only\s+|true\s+|whole\s+)?"
    r"fix\s+is\b(?!\s+(?:to|needed|required|still|pending|not|going|unclear|unknown|coming"
    r"|obvious|simple|hard|easy|probably|likely|maybe)\b)"
    r"|\b(?:this|that|it)\s+fixes\b"
    r"|\bfixes\s+(?:it|that|this|the)\b"
    r"|\bI(?:'ve|\s+have)?\s+(?:just\s+|now\s+|already\s+)?fixed\b"
    r"|\b(?:is|are|was|were|now)\s+fixed\b"
    r"|^\s*fixed\b"
    # The synonyms the 2026-09-13 escape used, added because the vocabulary was the narrow half of
    # that miss: "Solved and user-confirmed" said the same thing as "Fixed." and no branch read it.
    # A user confirming something is still not a measurement this turn read, so it is a claim like
    # any other; the honest form of it cites what was measured, or hedges.
    r"|^\s*(?:solved|resolved)\b"
    r"|\b(?:is|are|was|were|now)\s+(?:solved|resolved)\b"
    r"|\b(?:this|that|it)\s+(?:solves|resolves)\b"
    r"|\b(?:solves|resolves)\s+(?:it|that|this|the)\b"
    # An edit asserted to have produced a runtime effect. The subject has to be the change itself,
    # so "restored the file from git" and "I restored the backup" stay ordinary reporting while
    # "This single edit restored both the chrome and the six-cell grid" is read as the claim it is.
    r"|\b(?:this|that|the)\s+(?:\w+\s+){0,2}"
    r"(?:edit|edits|change|changes|commit|patch|line|one-liner|diff|rename)\s+"
    r"(?:fixed|fixes|restored|restores|solved|solves|resolved|resolves|brought\s+back)\b"
    # Present-tense behaviour asserted to have arrived. Narrow on purpose: a bare "works" is
    # ordinary ("the approach works for host code"), and only the adverb makes it a report that the
    # behaviour changed.
    r"|\bnow\s+works\b|\bworks\s+now\b",
    re.IGNORECASE,
)

# The stems that are names rather than claims. `\b` already excludes `prefix`, `suffix`, `bugfix`
# and `hotfix`, because a word character sits in front of the stem in each; these are the ones it
# does not exclude on its own. A branch (`fix/harness-repl-reach`), a slug (`fix-the-loader`), a
# file (`fix.md`) and `fixture` all read as "fix" to a word-boundary match and none of them is a
# claim about anything.
_NAME_STEM = re.compile(r"\bfix(?:ture|/|-[A-Za-z0-9]|\.[A-Za-z0-9])")

# An honest hedge suppresses the hit outright, and that is the whole point: saying a change is
# unverified is the behaviour being asked for and must never be punished. Scanned over the entire
# closing prose run rather than the matched sentence, so a claim in one paragraph and its hedge in
# the next still passes -- a guard that demanded the two sit in one sentence would teach people to
# stop hedging rather than to hedge better.
_HEDGE = re.compile(
    r"\bunverified\b|\bunproven\b|\buntested\b|\bunconfirmed\b"
    r"|\bnot\s+(?:yet\s+)?(?:proven|verified|confirmed|tested|run|launched|validated|observed)\b"
    r"|\bno\s+(?:run|runtime\s+evidence|evidence|proof|oracle|telemetry)\b"
    r"|\bneeds?\s+(?:a\s+)?(?:run|launch|runtime\s+proof|runtime\s+evidence|validation|testing)\b"
    r"|\bhas\s+not\s+(?:run|been\s+run|been\s+launched)\b"
    r"|\bI\s+(?:have\s+not|haven't|did\s+not|didn't)\s+(?:run|launch|test|verify|observe)\b"
    r"|\byet\s+to\s+(?:be\s+)?(?:run|prove|proven|verify|verified)\b"
    r"|\bawaiting\s+(?:a\s+)?run\b|\buntil\s+(?:a\s+|the\s+)?(?:run|launch)\b"
    r"|\battempt(?:s|ed)?\b|\bcandidate\b|\bnot\s+a\s+fix\b|\bmay\s+not\b"
    # Denying the fix outright. Measured, not guessed: one corpus turn closed on "This build
    # doesn't fix that case -- it makes it legible", and the earlier sentence "either route I've
    # fixed" convicted it. A message that says plainly what it does not fix is the behaviour being
    # asked for.
    r"|\b(?:does|do|did|will|would|can|could)(?:n'?t|\s+not)\s+fix\b"
    r"|\bshould\s+fix\b|\bmight\s+fix\b|\bmay\s+fix\b|\bwould\s+fix\b|\bif\s+it\s+works\b"
    # Measured, not guessed: this spelling closed the one 2026-08 turn in the corpus whose closing
    # line already said the change had not run, in the repo's own idiom rather than in the
    # dictionary's -- "the table now exists once ... (DLL 798414f3, ready for the next run)".
    r"|\b(?:ready\s+)?for\s+the\s+next\s+run\b|\bon\s+the\s+next\s+run\b"
    # Naming what would settle it is the behaviour the correction asks for, so a sentence that does
    # it must pass even when it uses the word. Same for work described but not done.
    r"|\bwhat\s+would\s+prove\b|\bwould\s+prove\s+it\b|\bproof\s+would\s+be\b"
    r"|\b(?:have\s+not|haven't|has\s+not|hasn't)\s+done\b|\bnot\s+done\s+yet\b",
    re.IGNORECASE,
)

# A dependency the agent cannot dissolve by working harder. Same family the neighbouring guards
# exempt, and for the same reason: a claim that cannot be tested yet is not a claim made carelessly.
_BLOCKED = re.compile(
    r"\bsudo\b|\bcredential|\bpassword\b|\blog\s*in\b|\blogin\b|\bpurchase\b"
    r"|\bsteam\s+is\s+(?:not|down)\b|\bno\s+(?:game|session)\s+is\s+(?:up|running)\b"
    r"|\bneeds?\s+(?:the\s+)?game\s+running\b|\bwith\s+the\s+game\s+down\b"
    r"|\bwait(?:ing)?\s+for\s+(?:the\s+)?(?:user|you)\b"
    r"|\bblocked\s+on\s+(?:the\s+)?(?:user|you)\b"
    r"|\btell\s+me\s+what\s+you\s+(?:saw|see)\b"
    r"|\bonce\s+you\b|\bover\s+to\s+you\b"
    # A decision handed to the user is a dependency like any other, and the corpus turn that forced
    # this branch names a real fix it deliberately did not make: "... which I've left for you to
    # call since it changes behaviour". Charging that sentence would punish the handback the rest of
    # this guard layer asks for.
    r"|\bleft\s+(?:it\s+|that\s+|them\s+|this\s+)?(?:for|to)\s+you\b"
    r"|\bfor\s+you\s+to\s+call\b|\byour\s+call\b",
    re.IGNORECASE,
)

# --- the claim is about host work, which a run cannot show either way ---------------------------
# Two of the five real turns this classifier convicted on a first pass had fixed a GATE: "Two gates
# broke on the way and I fixed them rather than shaving around them", and "The integration gate came
# back red and I've fixed all four failures". Both are honest reports of host work whose proof is the
# gate going green, and demanding a game launch for a clippy lint is how a guard earns a reputation
# for being wrong.
#
# The exemption needs both halves. A host noun on its own would swallow "the fix works and the tests
# pass", which is the exact substitution the directive is about -- a passing test is not a run. So it
# applies only when the sentence names host machinery and names nothing that lives inside the game.
_HOST_OBJECT = re.compile(
    r"\b(?:gate|gates|check|checks|selftest|selftests|test|tests|suite|suites|lint|lints"
    r"|clippy|rustfmt|fmt|compile|compiles|compiler|warning|warnings|workflow|ci|typo|docstring"
    # Added with the `solved`/`resolved` vocabulary above, and required by it: "resolved the merge
    # conflict" and "fixed the formatting" are the commonest sentences carrying those verbs, and a
    # run can say nothing about either.
    r"|conflict|conflicts|merge|rebase|comment|comments|formatting|import|imports"
    r"|fixture|fixtures|regression|regressions|policy|policies|signal|signals)\b",
    re.IGNORECASE,
)

# Host machinery a run cannot speak about at all, however many game nouns share the sentence. The
# ordinary `_HOST_OBJECT` test below is deliberately defeated by a game noun, which is right for
# "the fix works and the tests pass" -- but "resolved the merge conflict in rows.rs" names a file
# and is not a claim about the row it holds, and the stem match reads `rows.rs` as the game noun
# `row`. These few phrases are unambiguous enough to settle the sentence on their own.
_HOST_ONLY = re.compile(
    r"\bmerge\s+conflicts?\b|\bconflicts?\s+in\b|\brebase\b|\bclippy\b|\brustfmt\b"
    r"|\bcargo\s+fmt\b|\btypo\b|\bdocstrings?\b|\bcomment-caps\b|\bimport\s+order\b",
    re.IGNORECASE,
)

# Deliberately stem-matched rather than word-matched, so "hooked", "loading" and "rendered" count.
_RUNTIME_NOUN = re.compile(
    r"\b(?:crash|hang|softlock|game|load|menu|row|button|hook|detour|dll|player|save|boot"
    r"|launch|frame|render|animation|popup|invasion|warp|world|character|runtime|live"
    r"|oracle|telemetry)",
    re.IGNORECASE,
)

# --- (2) what a run leaves behind ---------------------------------------------------------------
# Deliberately an allowlist of artifact shapes rather than a loose notion of "ran something". The
# distinction the directive turns on is that a build is not a run and a launch is not a run: only
# something a run wrote, read back afterwards, says the change behaved.
#
# What is in it, measured by grepping `crates/` for the files the DLLs write on 2026-09-25:
#   ds2-<name>.log       the log a loaded DLL writes beside `DarkSoulsII.exe`. Today that is
#                        `ds2-loader.log` (LOG_NAME in scripts/ds2-run.py), rotated to
#                        `ds2-loader.log.prev` on the next run's first write; the pattern is the
#                        family so a second DLL log is covered the day it lands;
#   ds2-crash-<name>.txt what `ds2-crash-logging-core` leaves after a crash: ds2-crash-log.txt,
#                        ds2-crash-latest.txt, ds2-crash-breadcrumb-latest.txt,
#                        ds2-crash-modules.txt;
#   ds2-frida-watch.py   an attached agent reading the running game;
#   ds2-stop-works.py    the same, for the stop-call probe;
#   ds2-player-kind.py   a read of the live roster out of the running process;
#   ds2-io-sample.py     the running process's I/O and CPU counters;
#   ds2-harness-liveness.py  whether `ds2-input-harness` in the running game still answers;
#   ds2-teardown.py --status the live-session state read, which needs both tokens together.
#
# What is deliberately absent, for the reason the er original gives: `cargo build`, `cargo xwin
# build`, `cargo test`, `cargo check`, `scripts/check.sh`, a `sha256sum` of the artifact, and
# `scripts/ds2-run.py` itself. Building proves it compiles. Launching proves the game starts --
# and ds2-run.py's success block proves the DLL attached, which is a launch fact, not the change
# doing what the sentence says. `ds2-input-harness-cmd.txt` and `ds2-load-next-save.txt` are also
# absent: the game reads them, it does not write them, so opening one observes nothing.
_RUNTIME_ARTIFACT = re.compile(
    r"\bds2-[a-z0-9-]+\.log(?:\.prev)?\b"
    r"|\bds2-crash-[a-z0-9-]*\.txt(?:\.prev)?\b"
    r"|ds2-frida-watch\.py"
    r"|ds2-stop-works\.py"
    r"|ds2-player-kind\.py"
    r"|ds2-io-sample\.py"
    r"|ds2-harness-liveness\.py",
    re.IGNORECASE,
)

# The artifact read that needs two tokens in one command. `ds2-teardown.py` alone ends a session,
# so only the status form counts.
_STATUS_READ = re.compile(r"ds2-teardown\.py[^\n]*--status")

# Arming a watch is not reading a measurement, and this is the gap that let the 2026-09-13 escape
# through. That turn edited three gate predicates, cross-compiled, launched, and armed a `Monitor`
# on `tail -F er-quickload-autoload-debug.log` -- a file the process it had just started had not
# written a line of yet. The filename sat in the tool input, `reads_run_artifact` matched it, and
# `evidence` came back 1 on a log nobody had read. The rule the guard exists to state, one level
# further down than it was written: a launch is not proof, and neither is subscribing to what a
# launch might later write.
#
# Three shapes, all of them a promise of future output rather than a record of past output:
#   * the `Monitor` tool, whose entire purpose is text that has not arrived;
#   * `run_in_background`, which returns an id rather than output;
#   * a follow/detach in the command itself -- `tail -f`, `tail -F`, `--follow`, `nohup`, `setsid`.
# A foreground `tail -n 40 <log>` is untouched by all three and stays the ordinary way to read one.
_FOLLOW_OR_DETACH = re.compile(
    r"\btail\b[^|;&\n]*\s-[A-Za-z]*[fF]\b|--follow\b|\bnohup\b|\bsetsid\b|\bdisown\b"
)
_SUBSCRIPTION_TOOLS = {"monitor", "sendmessage", "taskstop"}

_WRITE_TOOLS = {"edit", "write", "multiedit", "notebookedit"}

# A command that produces a DLL. What makes it matter is stated in `runtime_evidence`: a build does
# not count as proof of anything, but it does stale the proof that came before it.
_BUILD_CMD = re.compile(r"cargo\s+(?:\+\S+\s+)?(?:xwin\s+)?build\b")

# A Bash write whose target is a crate path, matched as one span so the target is what decides.
# Narrower than the list the diagnosis signal carries, and narrowed on purpose: there a heredoc
# counting as an edit exonerates a turn, here it would convict one, and `python3 - <<'PY'` reading
# three files under `crates/` is how most of the reading in this repo is done. A bare heredoc is
# therefore not a write here -- only a redirect, `tee`, `patch` or an in-place `sed` that names a
# crate path.
_BASH_WRITE_TARGET = re.compile(
    r">>?\s*[^\s|&;<>]*crates/[A-Za-z0-9_-]+/[^\s|&;<>]*"
    r"|\b(?:tee|patch)\b[^|;]*?crates/[A-Za-z0-9_-]+/[^\s|&;<>\'\"]*"
    r"|\bsed\s+(?:-[^\s]*\s+)*-i\b[^|;]*?crates/[A-Za-z0-9_-]+/[^\s|&;<>\'\"]*"
)

# A path inside one workspace crate. The capture is the crate directory name, which is what decides
# whether the edit can reach the game at all.
_CRATE_PATH = re.compile(r"(?:^|[\s\"'=/])crates/([A-Za-z0-9_-]+)/([^\s\"']*)")

# Crate subtrees that are host work even in a crate that ships in a DLL.
_HOST_SUBTREE = ("tests/", "benches/", "examples/")


def scrub(text: str) -> str:
    """Drop fenced, backticked and double-quoted spans, as every neighbouring signal does.

    Quoting the word is the airquotes the directive asks for, so a scare-quoted "fix" and a
    backticked one both stop being claims here rather than needing a rule of their own.
    """
    text = re.sub(r"```.*?```", " ", text or "", flags=re.DOTALL)
    text = re.sub(r"`[^`]*`", " ", text)
    return re.sub(r'"[^"]{0,400}"', " ", text)


def fix_claim(closing_text: str) -> str:
    """The first sentence of the closing prose that calls a change a fix, or '' when there is none."""
    for raw in re.split(r"(?<=[.!?;])\s+|\n", scrub(closing_text)):
        sentence = raw.strip()
        if not sentence or sentence.startswith("|"):
            continue  # a table cell most often holds a label, not a claim
        if _FIX_CLAIM.search(_NAME_STEM.sub(" ", sentence)):
            return " ".join(sentence.split())[:220].replace("|", "/")
    return ""


def hedged(closing_text: str) -> bool:
    """True when the closing prose admits somewhere that the change has not been shown to work."""
    return bool(_HEDGE.search(scrub(closing_text)))


def externally_blocked(closing_text: str) -> bool:
    """True when the closing prose names a dependency the agent cannot dissolve by working harder."""
    return bool(_BLOCKED.search(scrub(closing_text)))


def host_object(claim_sentence: str) -> bool:
    """True when the claim is about host machinery a run cannot speak to either way.

    Both lists read the claim sentence and nothing else. Widening either to the whole closing prose
    was tried and measured wrong on a real turn: one that closed on "Fixed and built -- that blank
    row now has a third place-name source" listed its green gates two sentences later ("`fmt` 0,
    comment-caps 0"), and the lint name bought silence for a claim about a row in the game. A gate
    mentioned elsewhere in the message is not the object of the sentence.
    """
    if not claim_sentence:
        return False
    if _HOST_ONLY.search(claim_sentence):
        return True
    return bool(_HOST_OBJECT.search(claim_sentence)) and not _RUNTIME_NOUN.search(claim_sentence)


def runtime_crates(repo_root: str | Path) -> set[str]:
    """Crate directory names whose source can end up inside a DLL the game loads.

    Measured from the workspace manifests rather than listed by hand, because a hand-written list
    rots the first time a crate is added and rots silently. A crate qualifies when it declares a
    `cdylib` artifact -- `ds2-loader`, which ships as `dinput8.dll`, and `ds2-crash-logging`, which
    ships as `ds2_crash_logging.dll` -- or when one of those reaches it through a non-dev
    dependency. `ds2-launcher` is an injector `.exe` and is in only if a DLL depends on it.

    Two dependency spellings, because this workspace uses the second one almost everywhere:
    `ds2-x = { path = "../ds2-x" }` names the directory, and `ds2-x.workspace = true` names a key
    in the root manifest's `[workspace.dependencies]`, which carries `path = "crates/ds2-x"`. The
    er walk read only the first and would have reported the two shells and nothing else here.
    `[dev-dependencies]` are skipped: a test-only dependency does not ship.

    Returns an empty set on any read failure, which makes the whole guard silent. That is the safe
    direction for a rule whose false positive gags an honest report, and the classifier regression
    asserts the walk still finds the crates it is supposed to find.
    """
    root = Path(repo_root) / "crates"
    manifests: dict[str, tuple[bool, set[str]]] = {}
    try:
        workspace_paths: dict[str, str] = {}
        root_manifest = Path(repo_root) / "Cargo.toml"
        if root_manifest.is_file():
            for key, directory in re.findall(
                r'^([A-Za-z0-9_-]+)\s*=\s*\{[^}\n]*path\s*=\s*"crates/([A-Za-z0-9_-]+)"',
                root_manifest.read_text(encoding="utf-8", errors="replace"),
                re.MULTILINE,
            ):
                workspace_paths[key] = directory
        for toml in sorted(root.glob("*/Cargo.toml")):
            text = toml.read_text(encoding="utf-8", errors="replace")
            cdylib = bool(re.search(r'^\s*crate-type\s*=\s*\[[^\]]*"cdylib"', text, re.MULTILINE))
            deps: set[str] = set()
            section = ""
            for line in text.splitlines():
                stripped = line.strip()
                if stripped.startswith("["):
                    section = stripped
                    continue
                if "dependencies" not in section or "dev-dependencies" in section:
                    continue
                deps.update(re.findall(r'path\s*=\s*"\.\./([A-Za-z0-9_-]+)"', line))
                key = re.match(r"([A-Za-z0-9_-]+)\s*(?:\.workspace\s*=\s*true|=\s*\{[^}]*workspace\s*=\s*true)", stripped)
                if key and key.group(1) in workspace_paths:
                    deps.add(workspace_paths[key.group(1)])
            manifests[toml.parent.name] = (cdylib, deps)
    except OSError:
        return set()
    reachable = {name for name, (cdylib, _deps) in manifests.items() if cdylib}
    growing = True
    while growing:
        growing = False
        for name in list(reachable):
            for dep in manifests.get(name, (False, set()))[1]:
                if dep in manifests and dep not in reachable:
                    reachable.add(dep)
                    growing = True
    return reachable


def _written_paths(block: dict) -> list[str]:
    """Paths a single tool_use block put bytes into. Empty for a read, a build or a launch."""
    if not isinstance(block, dict):
        return []
    name = str(block.get("name") or block.get("tool_name") or "").strip().lower().replace("_", "")
    raw = block.get("input") or {}
    if not isinstance(raw, dict):
        return []
    if name in _WRITE_TOOLS:
        return [str(raw.get("file_path") or raw.get("notebook_path") or "")]
    if name != "bash":
        return []
    return _BASH_WRITE_TARGET.findall(str(raw.get("command") or ""))


def wrote_runtime_source(block: dict, crates: set[str]) -> bool:
    """True when this tool_use changed a file that can end up inside a loaded DLL."""
    for candidate in _written_paths(block):
        for crate, tail in _CRATE_PATH.findall(candidate):
            if crate not in crates:
                continue
            if tail.startswith(_HOST_SUBTREE):
                continue
            if tail.endswith((".rs", ".toml", ".json", ".tsv")) or tail == "":
                return True
    return False


def arms_watch(block: dict) -> bool:
    """True when this tool_use subscribes to output that has not been produced yet.

    See the note on `_FOLLOW_OR_DETACH`: the 2026-09-13 escape was a `Monitor` armed on a log the
    run it had just started had not written to, and it read as evidence.
    """
    if not isinstance(block, dict):
        return False
    name = str(block.get("name") or block.get("tool_name") or "").strip().lower().replace("_", "")
    if name in _SUBSCRIPTION_TOOLS:
        return True
    raw = block.get("input") or {}
    if not isinstance(raw, dict):
        return False
    if raw.get("run_in_background"):
        return True
    return bool(_FOLLOW_OR_DETACH.search(str(raw.get("command") or "")))


def builds_artifact(block: dict) -> bool:
    """True when this tool_use compiled a DLL, which stales every measurement taken before it.

    Deliberately only a real build. `cargo check` and `cargo xwin check` produce no artifact and
    therefore stale nothing, and neither does a test run; including them would move the anchor past
    reads that are still describing the loaded DLL.
    """
    if not isinstance(block, dict):
        return False
    name = str(block.get("name") or block.get("tool_name") or "").strip().lower().replace("_", "")
    if name != "bash":
        return False
    raw = block.get("input") or {}
    if not isinstance(raw, dict):
        return False
    return bool(_BUILD_CMD.search(str(raw.get("command") or "")))


def reads_run_artifact(block: dict) -> bool:
    """True when this tool_use opened something a run of the game produced.

    Opened, not subscribed to. A call that arms a watch names the same filenames and has read none
    of them, so it is excluded before the allowlist is consulted rather than after.
    """
    if not isinstance(block, dict):
        return False
    if arms_watch(block):
        return False
    try:
        payload = json.dumps(block.get("input") or {})
    except (TypeError, ValueError):
        return False
    return bool(_RUNTIME_ARTIFACT.search(payload) or _STATUS_READ.search(payload))


# A sentence that names an artifact in the future tense is the prose half of the same substitution
# `arms_watch` refuses: "the monitor will tell me if `muted=true` appears" has read nothing. The
# 2026-09-13 escape said exactly that and was saved from this branch only by the accident that the
# filename stayed in the tool input; a sentence away from silence is not a margin worth keeping.
_WATCH_FRAMING = re.compile(
    r"\bwill\s+(?:tell|show|say|report|confirm|land|appear|print|carry)\b"
    r"|\bshould\s+(?:tell|show|say|report|confirm|land|appear|print)\b"
    r"|\b(?:monitor|monitoring|watching|tailing|watcher|subscribed|streaming)\b"
    r"|\bonce\s+(?:it|the|they)\b|\bwhen\s+(?:it|the|they)\s+(?:lands?|appears?|arrives?)\b"
    r"|\bas\s+soon\s+as\b|\bif\s+(?:it|they)\s+(?:appears?|shows?|lands?)\b"
    r"|\bwaiting\s+(?:on|for)\b|\bnot\s+written\s+yet\b|\bhas\s+not\s+written\b",
    re.IGNORECASE,
)


def cites_run_artifact(text: str) -> bool:
    """True when the prose itself reports a run artifact, an oracle field or a run id.

    Searched unscrubbed and over the whole turn's prose, the way the proof-without-observation
    guard searches its evidence half: a run id most often arrives inside a fenced block, and a
    generous evidence search fails toward silence.

    Sentence by sentence rather than over the whole blob, because the distinction is per sentence:
    one that reports what a log said is evidence, one that says a watcher will report it later is
    the promise of evidence. A turn that does both still counts -- any one reporting sentence is
    enough -- so the generosity this branch was written for survives.
    """
    for sentence in re.split(r"(?<=[.!?;])\s+|\n", text or ""):
        if not (_RUNTIME_ARTIFACT.search(sentence) or _STATUS_READ.search(sentence)):
            continue
        if _WATCH_FRAMING.search(sentence):
            continue
        return True
    return False


def runtime_evidence(blocks: list[tuple], crates: set[str]) -> tuple[bool, bool]:
    """(changed, evidence) for one turn's ordered block stream.

    `changed` is a write to a crate that ships in a DLL. `evidence` is a run artifact opened at or
    after that write -- ordering is the whole point, because a log read before the edit describes
    the code that was there before it. Measured from the first such write rather than the last, so
    a turn that reads the log and then makes one more small edit still counts as having looked.

    The anchor moves forward again for a rebuild. A measurement taken before the artifact was built
    again describes the previous artifact, so the read has to come after the last build in the turn:
    the claim is about the DLL that is loaded now, not about the one that was loaded when the log
    was written. That is the letter of the correction -- since the most recent build of the artifact
    in question, has a measurement actually been read -- and it is the one tightening here that a
    green build cannot satisfy on its own, because a build only moves the anchor, never clears it.
    """
    first_write = None
    for index, (kind, block) in enumerate(blocks):
        if kind == "tool" and wrote_runtime_source(block, crates):
            first_write = index
            break
    if first_write is None:
        return False, False
    anchor = first_write
    for index, (kind, block) in enumerate(blocks):
        if index > anchor and kind == "tool" and builds_artifact(block):
            anchor = index
    evidence = any(
        kind == "tool" and reads_run_artifact(block)
        for kind, block in blocks[anchor:]
    )
    return True, evidence
