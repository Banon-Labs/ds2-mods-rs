#!/usr/bin/env python3
"""Which checkout a pending `git push` would actually run in.

Read by:
  * `.cupcake/signals/runtime_evidence_for_head.sh`
  * `.cupcake/signals/runtime_evidence_note.sh`

both of which answer a question about one specific push -- has the code it carries ever run --
and both of which used to answer it about whichever directory the signal process happened to
start in.

# The defect this exists to close, measured 2026-09-13

A session whose working directory was the main checkout ran

    cd <another worktree> && git push --force-with-lease=... origin HEAD:chore/...

and `ER-EFFECTS-REQUIRE-RUNTIME-EVIDENCE` denied it with

    er-quickload-autoload-debug.log was built from 2703cd8d, not 0f309fd6

`0f309fd6` was the tip of the main checkout. The commit being pushed was a different one, in the
worktree named by the `cd`, whose diff against `origin/main` touches `.github/`, `.gitignore`,
`AGENTS.md`, `docs/` and `scripts/` and no crate at all -- a `NOTRUNTIME` push, refused on a
measurement of a repository it was not about.

The symmetric half is the one that matters more, and nothing in the denial message hinted at it:
the same mix-up waves a push through whenever the measured directory happens to carry evidence
while the pushed worktree does not. That is the push the guard exists to stop, and it was passing.

# Why the resolution happens here rather than in the policy

Rego can see `input.tool_input.command` and could refuse to deny when the command redirects git
at another checkout. That removes the false positive and leaves the false negative exactly where
it was, because a policy cannot run `git` in the directory it just identified. The signals can:
cupcake pipes the whole pending event to every signal on stdin, `tool_input.command` and `cwd`
included (measured against cupcake 0.5.2 -- `Writing 737 bytes of event data to signal stdin`,
and the dump carries both fields). So the repository is resolved here and the git reads move to
it, which closes both halves with one change.

# Contract

Reads the hook event JSON on stdin, or takes `--command` and `--cwd` directly. Prints one word,
and the caller branches on it:

    `SELF`          nothing in the command redirects git at another checkout, so the directory
                    the caller already measures is the one being pushed. The overwhelmingly
                    common answer, and the pre-change behaviour.
    `REPO <path>`   the push runs in <path>, a different working tree of this same repository.
                    Every git read that decides the verdict belongs there.
    `UNKNOWN`       a redirect is present and could not be resolved to a working tree of this
                    repository. The signals turn this into the `UNKNOWN` verdict, which never
                    denies -- the policy's own rule is that a guard which cannot see must not
                    invent one, and `scripts/check-runtime-evidence.sh` in the pre-push hook
                    still measures the push exactly, from the directory git hands it.

`UNKNOWN` covers a deliberate list of shapes rather than a guess: a command whose quoting will
not lex, a `cd` inside a subshell or a heredoc (where a linear walk cannot say whether the push
inherited it), a target that is not a git working tree, a target belonging to some other
repository, and two pushes aimed at two different checkouts -- one verdict cannot describe two
repositories, and the honest answer to "which repository" is then "more than one".
"""
from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

# The whole walk below is skipped unless one of these appears in the command text. Nothing else
# in a shell command can point git at another checkout, so their absence is proof that the
# caller's own directory is the right one -- which keeps the common case at zero git reads and,
# more importantly, keeps an unlexable command that contains no redirect on the pre-change path
# instead of degrading it to `UNKNOWN`.
REDIRECT_MARKERS = ("cd", "pushd", "-C", "--git-dir", "--work-tree", "GIT_DIR")

# Shapes a linear left-to-right walk cannot reason about. A `cd` inside `( ... )` is undone at the
# closing paren, and a heredoc body is data that lexes like commands, so a `cd` in either one may
# or may not apply to a push that follows it.
UNRESOLVABLE_MARKERS = ("(", ")", "<<")

# Wrappers whose argument after `-c` is itself a command. `commands.executed_texts` in
# `.cupcake/system/commands.rego` decomposes these before the policy matches a push, so the
# resolution has to follow them or the two halves disagree about the same command.
SHELL_WRAPPERS = {"bash", "sh", "zsh", "dash", "ksh"}

# Words that may sit in front of the real verb without changing it.
TRANSPARENT_PREFIXES = {"command", "builtin", "exec", "nohup", "time", "sudo", "env"}

# git's own options that take a value, so the subcommand is not mistaken for one of their
# operands. Only the value-taking spellings matter here; a flag with no operand cannot swallow
# the verb that follows it.
GIT_VALUE_OPTIONS = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env", "--exec-path"}

# Hard cap on every git read below, and not a synchronisation device: each one is a `rev-parse` or
# an `init` against a local object store, measured in milliseconds. It exists because this runs
# inside a signal on every Bash tool call, where a git command wedged on a lock would stall the
# session rather than fail it. A read that reaches it returns `None`, which becomes `UNKNOWN`.
GIT_TIMEOUT_SECONDS = 10.0


def has_redirect_marker(command: str) -> bool:
    """Whether the text contains anything capable of pointing git at another checkout.

    Deliberately a substring test over the raw text, matched on word boundaries for the bare
    words so that `scripts/cd-rom.sh` is not read as a `cd`. It errs toward doing the work: a
    false positive costs one lex, while a false negative would leave the defect in place.
    """
    for marker in REDIRECT_MARKERS:
        if marker.startswith("-"):
            if marker in command:
                return True
            continue
        for index in range(len(command)):
            if not command.startswith(marker, index):
                continue
            before = command[index - 1] if index else " "
            after = command[index + len(marker) :][:1] or " "
            if not (before.isalnum() or before in "_-./") and not (after.isalnum() or after in "_-."):
                return True
    return False


def lex(command: str) -> list[str] | None:
    """Tokens with shell operators kept as tokens of their own, or `None` if it will not lex."""
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        return list(lexer)
    except ValueError:
        return None


def segments(tokens: list[str]) -> list[list[str]]:
    """Split a token list at the operators that end one command and start the next."""
    out: list[list[str]] = [[]]
    for token in tokens:
        if token and all(character in ";&|\n" for character in token):
            out.append([])
        else:
            out[-1].append(token)
    return [segment for segment in out if segment]


def strip_prefixes(segment: list[str]) -> list[str]:
    """Drop leading environment assignments and wrapper words to reach the real verb."""
    index = 0
    while index < len(segment):
        word = segment[index]
        if "=" in word and not word.startswith("-") and word.split("=", 1)[0].isidentifier():
            index += 1
            continue
        if Path(word).name in TRANSPARENT_PREFIXES:
            index += 1
            continue
        break
    return segment[index:]


def git_c_directories(segment: list[str]) -> tuple[list[str], bool]:
    """The `-C` operands of a git invocation, and whether its subcommand is `push`.

    git applies repeated `-C` cumulatively, each relative to the last, which is why they come
    back as a list rather than a single value.
    """
    directories: list[str] = []
    index = 1
    while index < len(segment):
        word = segment[index]
        if word == "-C" and index + 1 < len(segment):
            directories.append(segment[index + 1])
            index += 2
            continue
        if word.startswith("-C") and len(word) > 2:
            directories.append(word[2:])
            index += 1
            continue
        if word in GIT_VALUE_OPTIONS:
            index += 2
            continue
        if word.startswith("-"):
            index += 1
            continue
        return directories, word == "push"
    return directories, False


class Unresolvable(Exception):
    """A redirect is present and the walk cannot say what it means."""


def push_directories(command: str, cwd: str) -> set[str]:
    """Every directory a `git push` in this command would run in.

    Raises `Unresolvable` for the shapes listed in the module docstring. An empty result means
    the command pushes nothing, which is the ordinary case for almost every Bash call.
    """
    tokens = lex(command)
    if tokens is None:
        raise Unresolvable("the command does not lex")
    if any(marker in command for marker in UNRESOLVABLE_MARKERS):
        raise Unresolvable("a subshell or heredoc hides which directory a push inherits")

    current = cwd
    found: set[str] = set()
    for segment in segments(tokens):
        words = strip_prefixes(segment)
        if not words:
            continue
        verb = Path(words[0]).name
        if verb in ("cd", "pushd"):
            operands = [word for word in words[1:] if word != "--"]
            if not operands:
                current = os.path.expanduser("~")
                continue
            if operands[0].startswith("-"):
                raise Unresolvable("`cd -` goes somewhere only the shell's history knows")
            current = os.path.normpath(os.path.join(current, operands[0]))
            continue
        if verb in SHELL_WRAPPERS and "-c" in words:
            payload_index = words.index("-c") + 1
            if payload_index < len(words):
                found |= push_directories(words[payload_index], current)
            continue
        if verb == "git":
            directories, is_push = git_c_directories(words)
            if not is_push:
                continue
            target = current
            for directory in directories:
                target = os.path.normpath(os.path.join(target, directory))
            found.add(target)
    return found


def git_read(directory: str, *arguments: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", "-C", directory, *arguments],
            capture_output=True,
            text=True,
            check=False,
            timeout=GIT_TIMEOUT_SECONDS,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def resolve(command: str, cwd: str) -> str:
    """The one line this tool prints. See the module docstring for the contract."""
    if not has_redirect_marker(command):
        return "SELF"
    try:
        targets = push_directories(command, cwd)
    except Unresolvable:
        return "UNKNOWN"
    if not targets:
        return "SELF"
    if len(targets) > 1:
        return "UNKNOWN"

    target = targets.pop()
    if not os.path.isdir(target):
        return "UNKNOWN"
    toplevel = git_read(target, "rev-parse", "--show-toplevel")
    if not toplevel:
        return "UNKNOWN"

    # The same repository, or a stranger. Every consumer of this answer goes on to read commits
    # and refs through the caller's own object store, which is shared between the working trees
    # of one repository and holds nothing at all of another's, so a foreign checkout would be
    # measured against objects it does not contain.
    here = git_read(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir")
    there = git_read(target, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if not here or not there or os.path.realpath(here) != os.path.realpath(there):
        return "UNKNOWN"

    own = git_read(cwd, "rev-parse", "--show-toplevel")
    if own and os.path.realpath(own) == os.path.realpath(toplevel):
        return "SELF"
    return f"REPO {toplevel}"


def selftest() -> int:
    """Cases that need no repository, plus two that build throwaway ones.

    The parsing half is what a regression would land in, so it is asserted against strings; the
    two git-shaped answers (`REPO` for a sibling working tree, `UNKNOWN` for a stranger) need
    real repositories and get them.
    """
    failures = 0

    def check(description: str, got: object, want: object) -> None:
        nonlocal failures
        if got == want:
            print(f"  ok    {description}")
        else:
            print(f"  FAIL  {description}: got {got!r}, wanted {want!r}")
            failures += 1

    cwd = "/repo"
    check(
        "a plain push names no other directory",
        push_directories("git push -u origin feat/x", cwd),
        {"/repo"},
    )
    check(
        "a cd before the push moves it",
        push_directories("cd /other && git push origin HEAD:feat/x", cwd),
        {"/other"},
    )
    check(
        "a relative cd resolves against the caller's directory",
        push_directories("cd sub/tree && git push", cwd),
        {"/repo/sub/tree"},
    )
    check(
        "`git -C` moves the push without a cd",
        push_directories("git -C /other push origin main", cwd),
        {"/other"},
    )
    check(
        "repeated `-C` operands compose, as git applies them",
        push_directories("git -C /other -C sub push", cwd),
        {"/other/sub"},
    )
    check(
        "a cd that is not followed by a push finds nothing",
        push_directories("cd /other && cargo test", cwd),
        set(),
    )
    check(
        "the cd applies to a push inside a shell wrapper",
        push_directories("bash -c 'cd /other && git push'", cwd),
        {"/other"},
    )
    check(
        "a command with no push at all resolves to SELF",
        resolve("cargo fmt --check", cwd),
        "SELF",
    )
    check(
        "a push with no redirect anywhere resolves to SELF without reading git",
        resolve("git push -u origin feat/x", cwd),
        "SELF",
    )
    check(
        "a path merely containing the letters cd is not a redirect",
        has_redirect_marker("bash scripts/abcd-check.sh"),
        False,
    )
    check(
        "a real cd is a redirect",
        has_redirect_marker("cd /other && git push"),
        True,
    )

    for description, command in (
        ("two pushes at two checkouts answer UNKNOWN", "git push && cd /other && git push"),
        ("a subshell hides what the push inherits", "(cd /other && git push)"),
        ("an unbalanced quote does not lex", "cd '/other && git push"),
    ):
        check(description, resolve(command, cwd), "UNKNOWN")

    import tempfile

    with tempfile.TemporaryDirectory() as scratch:
        main = Path(scratch) / "main"
        linked = Path(scratch) / "linked"
        stranger = Path(scratch) / "stranger"
        for path in (main, stranger):
            path.mkdir()
            subprocess.run(
                ["git", "init", "-q", str(path)],
                check=False,
                capture_output=True,
                timeout=GIT_TIMEOUT_SECONDS,
            )
            for key, value in (("user.email", "selftest@example.invalid"), ("user.name", "selftest")):
                subprocess.run(
                    ["git", "-C", str(path), "config", key, value],
                    check=False,
                    capture_output=True,
                    timeout=GIT_TIMEOUT_SECONDS,
                )
            (path / "file.txt").write_text("x\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(path), "add", "-A"],
                check=False,
                capture_output=True,
                timeout=GIT_TIMEOUT_SECONDS,
            )
            subprocess.run(
                ["git", "-C", str(path), "commit", "-qm", "first", "--no-verify"],
                check=False,
                capture_output=True,
                timeout=GIT_TIMEOUT_SECONDS,
            )
        subprocess.run(
            ["git", "-C", str(main), "worktree", "add", "-q", "-b", "side", str(linked)],
            check=False,
            capture_output=True,
            timeout=GIT_TIMEOUT_SECONDS,
        )
        check(
            "a sibling working tree of the same repository is measured there",
            resolve(f"cd {linked} && git push origin HEAD:side", str(main)),
            f"REPO {linked}",
        )
        check(
            "an unrelated repository is not this guard's to judge",
            resolve(f"cd {stranger} && git push", str(main)),
            "UNKNOWN",
        )
        check(
            "a cd back to the caller's own tree is SELF",
            resolve(f"cd {main} && git push", str(main)),
            "SELF",
        )
        check(
            "a directory that does not exist cannot be measured",
            resolve(f"cd {scratch}/absent && git push", str(main)),
            "UNKNOWN",
        )

    if failures:
        print(f"cupcake_push_target_repo selftest: {failures} failure(s)")
        return 1
    print("cupcake_push_target_repo selftest: PASS")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--command", help="the pending command, instead of reading the event")
    parser.add_argument("--cwd", help="the directory the command would run in")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args(argv[1:])

    if args.selftest:
        return selftest()

    command = args.command
    cwd = args.cwd or os.getcwd()
    if command is None:
        try:
            event = json.loads(sys.stdin.read() or "{}")
        except (ValueError, OSError):
            print("SELF")
            return 0
        tool_input = event.get("tool_input")
        if isinstance(tool_input, dict):
            command = tool_input.get("command")
        if args.cwd is None and isinstance(event.get("cwd"), str):
            cwd = event["cwd"]
    if not isinstance(command, str) or not command.strip():
        # No command to read means no redirect to find, which is the same answer as a command
        # that carries none. Fail toward the pre-change behaviour rather than toward `UNKNOWN`:
        # an engine that stops piping the event must not silently disarm the guard.
        print("SELF")
        return 0

    print(resolve(command, cwd))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
