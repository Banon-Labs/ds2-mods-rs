#!/usr/bin/env python3
"""The remote branch each `git push` in a pending command would update.

Read by `.cupcake/signals/push_target_branches.sh` for DS2-MODS-BLOCK-MAIN-PUSH
(`.cupcake/policies/claude/git_block_main_push.rego`).

# The miss this closes (bd ds2-mods-rs-zmep, 2026-09-27)

With the hook's working directory at the main checkout, on `main`,

    cd /home/banon/projects/ds2-mods-rs-wt-jiy && git push -q --force-with-lease origin loader-build-sha

was refused with "Do not push directly to main". The worktree was on `loader-build-sha` and the
refspec named `loader-build-sha`, so nothing in that command could update remote main. The policy
judged the `current_branch` signal, which is the branch of the directory the HOOK runs in, and its
text exceptions did not recognise `-q --force-with-lease` in front of the remote.

# What is judged

The branch the push actually sends, per push:

  * An explicit refspec wins. `<src>:<dst>` sends `<dst>`; a bare `<src>` sends `<src>`, and
    `HEAD`/`@` there mean the branch checked out in the directory the push runs in.
  * With no refspec, the branch checked out in the directory the push runs in, which is where
    `push.default` = `current` or `simple` sends it. `upstream`/`tracking` send it to its upstream's
    name instead, and that is what is reported.

The directory comes from `scripts/cupcake_push_target_repo.py`'s walk of `cd <dir> &&`,
`git -C <dir>` and `bash -c '...'` -- the same walk the runtime-evidence guard uses, so the two
guards never disagree about where a push runs. Unlike that guard, a checkout of a DIFFERENT
repository is judged too: "do not push to main" does not depend on which repository it is, and a
branch name is readable in any checkout.

# Output

One line per destination, and the policy decides from the set:

    DEST <ref>          a ref this command pushes to, `refs/heads/` stripped
    UNKNOWN <reason>    something in the command could not be resolved

Nothing at all when the command runs no push. The policy treats any `UNKNOWN`, and a missing or
empty signal, as unresolved and denies unless its own text-only exceptions prove every push names a
non-main destination. That is the fail-closed direction: this script only ever lets a push through
by naming where it goes.

`UNKNOWN` covers: an unlexable command; a `cd` inside a subshell or heredoc; `--git-dir`,
`--work-tree`, `GIT_DIR` or `GIT_WORK_TREE`; a push directory that is not a git checkout; a detached
HEAD where a branch was needed; `--all`/`--branches`/`--mirror`/`--prune`; a glob refspec; an option
this parser does not know (it might take a value and shift the remote); `push.default=matching`;
a configured `remote.<name>.push`; and `push.default=upstream` with no upstream. A command that has
no `cd`/`-C` and whose only uncertainty is a substitution or subshell (a `$(...)` commit message)
reports the checkout's own branch as well, since any push hidden there runs in that checkout -- and
reports `UNKNOWN` instead when that branch is main, so the explicit-refspec text exceptions decide.

`CUPCAKE_CURRENT_BRANCH_OVERRIDE` stands in for the branch of the event's own directory, the same
override `.cupcake/rulebook.yml` gives `current_branch`, so the live-eval suite does not depend on
which branch this checkout happens to be on.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cupcake_push_target_repo as target_repo  # noqa: E402

GIT_TIMEOUT_SECONDS = target_repo.GIT_TIMEOUT_SECONDS

# Anything that points git at a repository or tree other than the directory the push runs in.
# The directory walk does not follow them, so a push carrying one is not resolved.
UNFOLLOWED_REDIRECTS = ("--git-dir", "--work-tree", "GIT_DIR", "GIT_WORK_TREE")

# Shapes that can hide a push from the lexer: it sees `$(...)` and backticks inside a quoted word
# as one word, and a subshell's `cd` may or may not reach a later push.
HIDING_MARKERS = ("(", ")", "`", "<<")

# `git push` options, from `git push -h` (git 2.5x). No-value flags first.
NO_VALUE_LONG = {
    "--verbose", "--quiet", "--delete", "--tags", "--dry-run", "--porcelain", "--force",
    "--force-with-lease", "--force-if-includes", "--thin", "--set-upstream", "--progress",
    "--no-verify", "--verify", "--follow-tags", "--signed", "--atomic", "--ipv4", "--ipv6",
}
# Options that take a value, either `--opt=v` or `--opt v`.
VALUE_LONG = {"--repo", "--recurse-submodules", "--receive-pack", "--exec", "--push-option"}
# Options that take a value only as `--opt=v`.
EQUALS_ONLY_LONG = {"--force-with-lease", "--signed"}
# Options that change what is pushed beyond the refspecs written, so no refspec reading holds.
UNRESOLVED_LONG = {"--all", "--branches", "--mirror", "--prune"}
NO_VALUE_SHORT = set("vqdnfu46")
VALUE_SHORT = {"o"}


class Unknown(Exception):
    """The command cannot be resolved to the refs it pushes."""


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


def push_arguments(words: list[str]) -> list[str]:
    """The words after the `push` subcommand of a git invocation that starts at `git`."""
    index = 1
    while index < len(words):
        word = words[index]
        if word in target_repo.GIT_VALUE_OPTIONS:
            index += 2
            continue
        if word.startswith("-"):
            index += 1
            continue
        if word == "push":
            return words[index + 1 :]
        break
    raise Unknown("no push subcommand where the walk found one")


def parse_push(arguments: list[str]) -> tuple[str | None, list[str], bool]:
    """(repository or None, refspecs, whether `--tags` was given)."""
    positional: list[str] = []
    repository: str | None = None
    tags = False
    index = 0
    options_done = False
    while index < len(arguments):
        word = arguments[index]
        index += 1
        if options_done or not word.startswith("-") or word == "-":
            positional.append(word)
            continue
        if word == "--":
            options_done = True
            continue
        if word == "--end-of-options":
            options_done = True
            continue
        if word.startswith("--"):
            name, has_value, value = word.partition("=")
            base = name
            if base.startswith("--no-") and base not in NO_VALUE_LONG:
                base = "--" + base[len("--no-") :]
                if base in UNRESOLVED_LONG or base in NO_VALUE_LONG or base in VALUE_LONG:
                    # `--no-<x>` negates and never takes an operand.
                    if base == "--tags":
                        tags = False
                    continue
                raise Unknown(f"unknown option {name}")
            if base in UNRESOLVED_LONG:
                raise Unknown(f"{base} pushes refs no refspec names")
            if base in VALUE_LONG:
                if not has_value:
                    if index >= len(arguments):
                        raise Unknown(f"{base} without its value")
                    value = arguments[index]
                    index += 1
                if base == "--repo":
                    repository = value
                continue
            if base in NO_VALUE_LONG:
                if has_value and base not in EQUALS_ONLY_LONG:
                    raise Unknown(f"{base} does not take a value")
                if base == "--tags":
                    tags = True
                continue
            raise Unknown(f"unknown option {name}")
        # Short options, possibly bundled (`-qu`), possibly with an attached value (`-ofoo`).
        letters = word[1:]
        position = 0
        while position < len(letters):
            letter = letters[position]
            position += 1
            if letter in NO_VALUE_SHORT:
                continue
            if letter in VALUE_SHORT:
                if position >= len(letters):
                    if index >= len(arguments):
                        raise Unknown(f"-{letter} without its value")
                    index += 1
                break
            raise Unknown(f"unknown option -{letter}")
    if positional:
        if repository is None:
            repository = positional[0]
        refspecs = positional[1:]
    else:
        refspecs = []
    return repository, refspecs, tags


def strip_heads(ref: str) -> str:
    for prefix in ("refs/heads/",):
        if ref.startswith(prefix):
            return ref[len(prefix) :]
    return ref


class Checkout:
    """The branch state of one directory a push runs in, read lazily."""

    def __init__(self, directory: str, override: str | None) -> None:
        self.directory = directory
        self.override = override
        self._branch: str | None = None

    def branch(self) -> str:
        if self._branch is None:
            if self.override:
                self._branch = self.override
            else:
                if git_read(self.directory, "rev-parse", "--is-inside-work-tree") != "true":
                    raise Unknown(f"{self.directory} is not a git checkout")
                self._branch = git_read(self.directory, "branch", "--show-current") or ""
        if not self._branch:
            raise Unknown(f"{self.directory} has no branch checked out")
        return self._branch

    def config(self, key: str) -> str | None:
        return git_read(self.directory, "config", "--get", key)

    def configured_push_refspecs(self) -> bool:
        return bool(git_read(self.directory, "config", "--get-regexp", r"^remote\..*\.push$"))


def destinations(checkout: Checkout, arguments: list[str]) -> list[str]:
    repository, refspecs, tags = parse_push(arguments)
    out: list[str] = []
    if tags:
        out.append("refs/tags/*")
    words = list(refspecs)
    index = 0
    while index < len(words):
        spec = words[index]
        index += 1
        if spec == "tag":
            if index >= len(words):
                raise Unknown("`tag` without a tag name")
            out.append(f"refs/tags/{words[index]}")
            index += 1
            continue
        spec = spec.lstrip("+")
        if "*" in spec:
            raise Unknown(f"glob refspec {spec}")
        source, colon, destination = spec.partition(":")
        if not colon:
            destination = source
        if destination in ("HEAD", "@"):
            destination = checkout.branch()
        if not destination:
            raise Unknown(f"refspec {spec} has no destination")
        out.append(strip_heads(destination))
    if refspecs or tags:
        return out

    # No refspec: push.default and remote.<name>.push decide.
    if checkout.configured_push_refspecs():
        raise Unknown("remote.<name>.push is configured, so a bare push follows it")
    branch = checkout.branch()
    mode = (checkout.config("push.default") or "simple").lower()
    if mode == "matching":
        raise Unknown("push.default=matching pushes every same-named branch")
    if mode in ("upstream", "tracking"):
        merge = checkout.config(f"branch.{branch}.merge")
        if not merge:
            raise Unknown(f"push.default={mode} and {branch} has no upstream")
        return [strip_heads(merge)]
    return [branch]


def resolve(command: str, cwd: str, override: str | None) -> list[str]:
    """The lines to print. See the module docstring."""
    if "push" not in command:
        return []
    if any(marker in command for marker in UNFOLLOWED_REDIRECTS):
        return ["UNKNOWN the command points git at another tree through a variable or option"]

    redirected = target_repo.has_redirect_marker(command)
    hiding = any(marker in command for marker in HIDING_MARKERS)
    if redirected and hiding:
        return ["UNKNOWN a subshell or heredoc hides which directory a push inherits"]

    # The walk refuses every parenthesis; without a `cd`/`-C` there is no directory for one to
    # hide, so that refusal is skipped here and the hidden-push case is handled below.
    try:
        invocations = target_repo.push_invocations(command, cwd, allow_hiding=hiding)
    except target_repo.Unresolvable as error:
        return [f"UNKNOWN {error}"]

    checkouts: dict[str, Checkout] = {}

    def checkout_for(directory: str) -> Checkout:
        key = os.path.normpath(directory)
        if key not in checkouts:
            own = key == os.path.normpath(cwd)
            checkouts[key] = Checkout(key, override if own else None)
        return checkouts[key]

    lines: list[str] = []
    try:
        for directory, words in invocations:
            for destination in destinations(checkout_for(directory), push_arguments(words)):
                lines.append(f"DEST {destination}")
        if hiding:
            # A push inside `$(...)` or a subshell runs in this same directory; the most it can
            # send without naming a ref (which the policy's own text match reads) is this branch.
            branch = checkout_for(cwd).branch()
            if branch == "main":
                return ["UNKNOWN a substitution or subshell may hide a bare push from main"]
            lines.append(f"DEST {branch}")
    except Unknown as error:
        return [f"UNKNOWN {error}"]
    if not lines and hiding:
        return ["UNKNOWN no push found outside a substitution or subshell"]
    return lines


def selftest() -> int:
    import tempfile

    failures = 0

    def check(description: str, got: object, want: object) -> None:
        nonlocal failures
        if got == want:
            print(f"  ok    {description}")
        else:
            print(f"  FAIL  {description}: got {got!r}, wanted {want!r}")
            failures += 1

    def run(*arguments: str) -> None:
        subprocess.run(arguments, check=True, capture_output=True, timeout=GIT_TIMEOUT_SECONDS)

    with tempfile.TemporaryDirectory() as scratch:
        main = Path(scratch) / "main"
        linked = Path(scratch) / "linked"
        onmain = Path(scratch) / "onmain"
        stranger = Path(scratch) / "stranger"
        for path in (main, stranger):
            run("git", "init", "-q", "-b", "main", str(path))
            for key, value in (
                ("user.email", "selftest@example.invalid"),
                ("user.name", "selftest"),
                ("push.default", "current"),
            ):
                run("git", "-C", str(path), "config", key, value)
            (path / "file.txt").write_text("x\n", encoding="utf-8")
            run("git", "-C", str(path), "add", "-A")
            run("git", "-C", str(path), "commit", "-qm", "first", "--no-verify")
        run("git", "-C", str(stranger), "switch", "-qc", "stranger-feature")
        run("git", "-C", str(main), "worktree", "add", "-q", "-b", "loader-build-sha", str(linked))
        # A second checkout of `main` itself: the worktree-on-main shape.
        run("git", "-C", str(main), "branch", "-q", "spare")
        run("git", "-C", str(main), "switch", "-q", "spare")
        run("git", "-C", str(main), "worktree", "add", "-q", str(onmain), "main")
        # The hook's directory: a checkout on main, as the main checkout was in the reported miss.
        cwd = str(onmain)

        def verdict(command: str, override: str | None = None, at: str = cwd) -> list[str]:
            return resolve(command, at, override)

        # The reported miss, verbatim in shape.
        check(
            "bd zmep: cd <worktree> && git push -q --force-with-lease origin <its branch>",
            verdict(f"cd {linked} && git push -q --force-with-lease origin loader-build-sha"),
            ["DEST loader-build-sha"],
        )
        check(
            "bare push after cd judges the worktree's branch, not the hook's",
            verdict(f"cd {linked} && git push"),
            ["DEST loader-build-sha"],
        )
        check(
            "git -C <worktree> push judges the worktree's branch",
            verdict(f"git -C {linked} push -u origin"),
            ["DEST loader-build-sha"],
        )
        check(
            "bash -lc 'cd <worktree> && git push' judges the worktree's branch",
            verdict(f"bash -lc 'cd {linked} && git push --force-with-lease'"),
            ["DEST loader-build-sha"],
        )
        check(
            "HEAD in a worktree resolves to that worktree's branch",
            verdict(f"cd {linked} && git push origin HEAD"),
            ["DEST loader-build-sha"],
        )

        # Every real push to main stays visible as main.
        check("bare push on main", verdict("git push"), ["DEST main"])
        check("origin main", verdict(f"cd {linked} && git push origin main"), ["DEST main"])
        check("HEAD:main", verdict(f"cd {linked} && git push origin HEAD:main"), ["DEST main"])
        check(
            "feature:refs/heads/main",
            verdict(f"cd {linked} && git push origin +loader-build-sha:refs/heads/main"),
            ["DEST main"],
        )
        check("HEAD from main", verdict("git push -u origin HEAD"), ["DEST main"])
        check("@ from main", verdict("git push origin @"), ["DEST main"])
        check(
            "cd from a feature worktree to a worktree on main",
            verdict(f"cd {onmain} && git push", at=str(linked)),
            ["DEST main"],
        )
        check(
            "git -C a worktree on main from a feature worktree",
            verdict(f"git -C {onmain} push origin", at=str(linked)),
            ["DEST main"],
        )
        check(
            "explicit feature refspec from a main checkout",
            verdict("git push -u origin feat/x"),
            ["DEST feat/x"],
        )
        check(
            "the override stands in for the event directory's branch",
            verdict("git push", override="main"),
            ["DEST main"],
        )
        check(
            "the override does not leak into another directory",
            verdict(f"cd {linked} && git push", override="main"),
            ["DEST loader-build-sha"],
        )
        check(
            "another repository's feature checkout is judged by its branch",
            verdict(f"cd {stranger} && git push"),
            ["DEST stranger-feature"],
        )
        check("a command with no push prints nothing", verdict("git status"), [])
        check(
            "a newline ends a push: a bare push on the next line is its own push",
            verdict(f"cd {linked} && git push origin loader-build-sha\ngit push", at=cwd),
            ["DEST loader-build-sha", "DEST loader-build-sha"],
        )
        check(
            "a newline-separated bare push from main is main",
            verdict("git push origin feat/x\ngit push"),
            ["DEST feat/x", "DEST main"],
        )
        check(
            "a newline inside a quoted argument is not a separator",
            verdict("git push origin 'feat/x\n' "),
            ["DEST feat/x\n"],
        )
        check(
            "two pushes, two checkouts, both reported",
            verdict(f"git -C {linked} push && git -C {onmain} push"),
            ["DEST loader-build-sha", "DEST main"],
        )
        check(
            "a $(...) commit message on a feature branch keeps a bare push resolvable",
            verdict('git commit -m "$(printf x)" && git push', at=str(linked)),
            ["DEST loader-build-sha", "DEST loader-build-sha"],
        )

        for description, command, at in (
            ("--all", "git push --all origin", cwd),
            ("--mirror", "git push --mirror origin", cwd),
            ("an unknown option", "git push --frobnicate origin feat", cwd),
            ("a glob refspec", "git push origin 'refs/heads/*:refs/heads/*'", cwd),
            ("--git-dir", f"git --git-dir={linked}/.git push", cwd),
            ("GIT_DIR", f"GIT_DIR={linked}/.git git push", cwd),
            ("a subshell cd", f"(cd {linked} && git push)", cwd),
            ("a directory that is not a checkout", f"cd {scratch} && git push", cwd),
            ("a missing directory", f"cd {scratch}/absent && git push", cwd),
            ("an unbalanced quote", "git push origin 'feat", cwd),
            ("a substitution from main", 'git commit -m "$(printf x)" && git push -u origin f', cwd),
        ):
            got = verdict(command, at=at)
            check(
                f"UNKNOWN: {description}",
                len(got) == 1 and got[0].startswith("UNKNOWN "),
                True,
            )

        run("git", "-C", str(linked), "config", "push.default", "upstream")
        check(
            "push.default=upstream with no upstream is UNKNOWN",
            verdict(f"cd {linked} && git push")[0].startswith("UNKNOWN "),
            True,
        )
        run("git", "-C", str(linked), "config", "branch.loader-build-sha.merge", "refs/heads/main")
        check(
            "push.default=upstream tracking main is main",
            verdict(f"cd {linked} && git push"),
            ["DEST main"],
        )

    if failures:
        print(f"cupcake_push_target_branch selftest: {failures} failure(s)")
        return 1
    print("cupcake_push_target_branch selftest: PASS")
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
    cwd = args.cwd
    if command is None:
        try:
            event = json.loads(sys.stdin.read() or "{}")
        except (ValueError, OSError):
            print("UNKNOWN the event did not parse")
            return 0
        tool_input = event.get("tool_input")
        if isinstance(tool_input, dict):
            command = tool_input.get("command")
        if cwd is None and isinstance(event.get("cwd"), str):
            cwd = event["cwd"]
    if not isinstance(command, str):
        return 0
    cwd = cwd or os.getcwd()
    override = os.environ.get("CUPCAKE_CURRENT_BRANCH_OVERRIDE") or None
    for line in resolve(command, cwd, override):
        print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
