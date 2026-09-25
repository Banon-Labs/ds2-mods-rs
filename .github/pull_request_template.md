<!--
Title is a commit header: `type(scope): subject`. Types and scopes: docs/COMMITS.md.
Open as a draft. Keep the three headings below spelled exactly as they are, and keep the
attribution footer -- global policy guards enforce both, and cap the whole body at 2500
characters. A body that will not fit is a change that should have been two changes.
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

Written by `<agent>`, authorized by @`<github-username>`
