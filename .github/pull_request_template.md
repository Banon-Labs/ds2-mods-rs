<!--
Title is a commit header: `type(scope): subject`. Types and scopes: docs/COMMITS.md.
Open as a draft. Keep the three headings below spelled exactly as they are, and keep the
attribution footer -- global policy guards enforce both, and cap the whole body at 2500
characters. A body that will not fit is a change that should have been two changes.
The footer's Run-Stamp line names the head commit and its last run; a repo guard refuses
`gh pr create` without one for HEAD, and `gh pr ready` without a passing one for the live head.
Push again, stamp again (`gh pr edit <n> --body-file <file>`).
Fill the attribution line in plain text, with no backticks: the global guard matches
`authorized by @<name>` literally, and a backtick after the @ fails it.
-->

## What changed

One paragraph, written in behaviour.

## Why

The finding, the report or the run that made it necessary. Long-form reasoning belongs in the commit
message or a doc this can link in a line.

## Evidence

The gate, and the run inside the game -- quote the log line that proves it. A green gate is not
evidence a feature works; every crate here ships as a library injected into a running game. When
there has been no run, say so plainly. "Not proven: no row has been pressed on this build" is a
pass. Silence is not.

---

Run-Stamp: replace this line with the output of `python3 scripts/pr-run-stamp.py --from-last-check`
🤖 Written by <agent>, authorized by @<github-username>
