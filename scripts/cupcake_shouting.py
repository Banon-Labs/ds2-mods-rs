"""Deciding whether a span of text shouts -- emphasis put in the capitalisation rather than in
the sentence.

Owned by one module so the Stop signal, its regression test and any false-positive audit cannot
drift into three different answers, the same reason `cupcake_status_table` is shared. The two
patterns below are the single definition of the offence, and `.cupcake/policies/claude/
docs_no_shouting.rego` carries them verbatim for the PreToolUse arm; the equality of the three
copies is asserted by `scripts/test-shouting-signal.py`, because a guard whose two halves disagree
about what they forbid is worse than one half.

The user's directive, 2026-09-23, written in the style it is complaining about: "When I TALK LIKE
THIS everyone thinks the WORDS ARE IMPORTANT but really it distracts from the SUBSTANCE OF THE
MESSAGE which is burried by font style and EXCESSIVE PROSE". Capitals used as volume are skimmed
exactly like a wall of text is skimmed, and the sentence they were meant to rescue goes with them.

The test is two tests, and both were tuned by sweeping this repo's own prose rather than guessed.
The sweep is the 120 git-tracked Markdown files and Rust sources in the main checkout, read at the
scope `doc_text` below defines, with verbatim spans already removed.

  1. A run of `MIN_RUN` or more capitalised words in a row. Four, and the number is measured. At
     four the sweep finds 54 runs and every one of them is the habit -- "the field is opened
     here", "answered, and the answer is that this field does nothing". At three it finds 86 more,
     and 80 of those are nobody shouting: the game's name written out (68), a title-screen prompt
     (8) and the beads integration markers in CLAUDE.md (4). Three costs thirteen false positives
     for every real one it adds; four costs none.

  2. One capitalised word drawn from a closed list of function words and absolutes. This is what
     catches "NOTHING in ds2-save-file has been run" and "STARTUP-ONLY, both of them", which are
     single words and are unmistakably shouted. It works precisely because the list contains
     nothing that could be a name: `DLL`, `RVA`, `AES`, `TOML`, `CPU` and `OK` are things, while
     `the`, `not`, `every` and `only` are volume. The list is read out of the same sweep -- it
     fires 542 times there, on 70 distinct words, led by 110 capitalised "NOT"s, 62 "THE"s, 44
     "IS"s and 27 "AND"s, and every sampled one is the habit. The words deliberately left off it
     were read out of the sweep too: `ON`/`OFF`, `YES`/`NO`, `TRUE`/`FALSE` and `NEXT` are
     table-cell values here, `LEFT`/`RIGHT` are hand positions, `BE`/`LE` are byte orders, `OUT`
     is a parameter direction, `ONE`/`TWO` are counts, and `OR` appears only as "OR'd together".

Why keep the run rule when the word list already reaches all 54. Because the word list is a list,
and a list is only ever as good as the last person to extend it. A shouted phrase built entirely
out of content words -- "one row pitch per added row" -- is invisible to it by construction, and
the run rule catches that without knowing any vocabulary at all. It costs nothing to keep: on this
repo it has no false positives of its own.

Residue, said out loud rather than left to be discovered.

  * `AND` and `NOT` name logic operations as well as volume, so "an AND and a compare" written
    without backticks is refused. Backticks are the fix and are already the habit here for an
    opcode. Measured once in 2,347 blocks of the agent's own chat prose.
  * A hyphenated identifier in capitals inherits the word list through its parts, so a rule id
    like `DS2-MODS-NO-SHOUTING-AT-TURN-END` would be refused in prose. None of the 47 such tokens
    in the swept documentation is one; the three that carry a list word -- `STARTUP-ONLY`,
    `SECOND-HAND`, `STAND-IN` -- are all the habit, and the first is one of the cases the user
    named.
  * A quoted span is exempt wholesale, so a shouted heading somebody quotes back survives. That is
    the right trade: punishing an agent for quoting the user verbatim would be worse.
  * A launch banner is not exempted. AGENTS.md requires the banner to sit immediately before the
    launch call, which puts it mid-turn where the Stop arm never looks, so a banner this guard can
    see has already been written in the wrong place. Measured over 478 turn-closing prose runs in
    this project's transcripts, 19 would halt and 12 of those are banners written after the launch
    rather than before it.

Everything else that looks capitalised is excluded by the word boundary alone, which is why there
is no allow-list of identifiers: `SAVE_DIR_BUILD`, `FE_INGAME_MENU_ITEM_VECTOR_CAPACITY` and
`FUN_1402e67f0` are one word to a regex engine (an underscore is a word character) and never match
`\\b[A-Z]{2,}\\b`; `DS2`, `BND4`, `MD5`, `UTF-8` and `DS2SOFS0000.sl2` fail it on their digits;
`DllMain` fails it on its lowercase. Backticked spans, fenced blocks and quoted spans are removed
before any of it runs, so a log line or a menu string reproduced verbatim is never judged.
"""

from __future__ import annotations

import re

#: A run of this many capitalised words in a row is shouting whatever the words are. See the
#: measurement in the module docstring; `.cupcake/policies/claude/no_shouting_at_turn_end.rego`
#: holds the same number as `min_run` and re-applies it to the signal rather than trusting it.
MIN_RUN = 4

#: Function words and absolutes: capitalised, each is emphasis by volume on its own, and none of
#: them can be a name, an acronym, a register or a type. Kept as a tuple so the pattern below is
#: built in one place and the list can be read in one screenful.
EMPHASIS_WORDS = (
    # negation and absolutes
    "NOT", "NEVER", "NOTHING", "NONE", "NOBODY", "NOWHERE",
    "ALWAYS", "ALL", "EVERY", "EVERYTHING", "ONLY", "BOTH", "EITHER", "NEITHER",
    "MUST", "MANDATORY", "CANNOT", "CRITICAL", "IMPORTANT",
    # copulas and auxiliaries
    "IS", "ARE", "WAS", "WERE", "BEEN", "BEING",
    "DO", "DOES", "DID", "HAS", "HAVE", "HAD",
    "CAN", "COULD", "WILL", "WOULD", "SHOULD", "SHALL",
    # determiners and pronouns
    "THE", "AN", "THIS", "THAT", "THESE", "THOSE",
    "IT", "ITS", "YOU", "YOUR", "WE", "OUR", "THEY", "THEM", "THEIR",
    "WHAT", "WHY", "HOW", "WHICH", "WHO", "WHOSE", "WHEN", "WHERE",
    # prepositions and conjunctions
    "AND", "BUT", "IN", "AT", "TO", "FROM", "WITH", "WITHOUT", "INTO", "ONTO",
    "BY", "FOR", "OF", "AS", "THAN", "BECAUSE", "SO", "IF", "UNLESS", "UNTIL", "EXCEPT",
    # adverbs of emphasis
    "ACTUALLY", "REALLY", "EXACTLY", "DELIBERATELY", "ENTIRELY", "COMPLETELY",
    "PRECISELY", "GENUINELY", "SIMPLY", "MERELY", "WHOLLY", "PURELY", "STRICTLY",
    "ALREADY", "STILL", "INSTEAD", "ALSO", "EVEN", "JUST", "VERY", "EVER", "RATHER",
    "ONCE", "TWICE", "AGAIN",
    # comparatives and positions in an argument
    "FIRST", "SECOND", "LAST", "SAME", "OWN", "WRONG",
    "HERE", "THERE", "NOW", "THEN", "MORE", "MOST", "LESS", "LEAST", "BETTER", "WORSE",
)

#: One capitalised word, bounded so an underscore, a digit or a lowercase letter touching it
#: disqualifies it. `\b` treats `_` as a word character, which is the whole reason identifiers
#: need no allow-list here.
EMPHASIS_PATTERN = r"\b(?:%s)\b" % "|".join(EMPHASIS_WORDS)

#: `MIN_RUN` capitalised words separated by spaces or tabs. Punctuation between two words ends the
#: run, which is what keeps a comma-separated list of acronyms -- "PE, COFF, IAT, TLS" -- from
#: reading as one shouted phrase.
RUN_PATTERN = r"\b[A-Z]{2,}\b(?:[ \t]+\b[A-Z]{2,}\b){%d,}" % (MIN_RUN - 1)

#: A maximal run of capitalised words, used to score a line rather than to decide it.
_ANY_RUN_RE = re.compile(r"\b[A-Z]{2,}\b(?:[ \t]+\b[A-Z]{2,}\b)*")
_WORD_RE = re.compile(r"\b[A-Z]{2,}\b")
_EMPHASIS_RE = re.compile(EMPHASIS_PATTERN)

_EMPHASIS_SET = frozenset(EMPHASIS_WORDS)

#: Markdown doc comments in Rust. Ordinary `//` comments are not documentation and are not judged,
#: the same scope `docs_no_size_metrics` uses.
DOC_COMMENT_RE = re.compile(r"^\s*//[!/]")


def _split_on_balanced(text: str, delimiter: str) -> str:
    """Text with every `delimiter`-fenced span replaced by a space.

    Parity, not a regex, and deliberately the same trick `.cupcake/system/commands.rego` uses to
    read quoted spans: split on the delimiter and keep the pieces at even indices, which are the
    ones outside it. Rego has no working `regex.replace` -- it is host-dispatched and Cupcake's
    WASM runtime does not implement it, measured 2026-09-23 -- so the policy has to do it this way
    and this mirrors the policy rather than the other way round.

    The surviving pieces are rejoined with a NEWLINE rather than a space, and that is not
    cosmetic: everything downstream works line by line, so a space would weld the words on either
    side of a removed span into a run that was never written. "LOAD GAME `FeSubState...` NEW GAME"
    is two runs of two, not one run of four.

    An odd number of delimiters means the parity read says nothing, so the text is returned
    untouched and gets judged. That is the fail-closed direction: an unbalanced backtick should
    not be a way to hide a shouted sentence behind half a code span.
    """
    parts = text.split(delimiter)
    if len(parts) % 2 == 0:  # odd number of delimiters -> unbalanced
        return text
    return "\n".join(part for index, part in enumerate(parts) if index % 2 == 0)


def strip_verbatim(text: str) -> str:
    """Remove the spans that are somebody else's words rather than the author's prose.

    Three of them, and each earns its place:
      * fenced code blocks, removed across the whole text before it is cut into lines;
      * inline code spans, per line, which is where an identifier or a log line normally sits;
      * double-quoted spans, per line -- a verbatim quotation is a quotation. This is also what
        keeps the one false positive the run rule has in this repo silent: the game's full title
        is written "DARK SOULS II: SCHOLAR OF THE FIRST SIN", in quotes, both times it appears.
    """
    text = _split_on_balanced(text, "```")
    out = []
    for line in text.split("\n"):
        line = _split_on_balanced(line, "`")
        line = _split_on_balanced(line, '"')
        out.append(line)
    return "\n".join(out)


def _run_score(run: str) -> int:
    """How loud one run of capitalised words is.

    An ordinary capitalised word is worth one; a function word from `EMPHASIS_WORDS` is worth the
    whole threshold, because on its own it is already the offence. So the single number below says
    exactly what the two tests in the module docstring say: `MIN_RUN` words in a row, or one word
    that has no business being capitalised at all.
    """
    total = 0
    for word in _WORD_RE.findall(run):
        total += MIN_RUN if word in _EMPHASIS_SET else 1
    return total


def shouting_score(text: str) -> tuple[int, str]:
    """(how loud the loudest line is, that line).

    The sample is the whole line rather than the matched fragment, so the correction can quote
    something the author will recognise and find.
    """
    best = 0
    sample = ""
    for line in strip_verbatim(text).split("\n"):
        line_best = max((_run_score(run) for run in _ANY_RUN_RE.findall(line)), default=0)
        if line_best > best:
            best = line_best
            sample = " ".join(line.split())[:160]
    return best, sample


def shouts(text: str) -> bool:
    """Whether `text` shouts at all -- the boolean the two Rego policies compute."""
    score, _ = shouting_score(text)
    return score >= MIN_RUN


def doc_text(path: str, text: str) -> str:
    """The part of an edit that is documentation, in the scope `docs_no_size_metrics` established.

    Markdown is documentation end to end. In Rust only `//!` and `///` are: an ordinary `//`
    comment is a note to whoever is reading the code beside it, and a `println!` is a log line,
    neither of which is prose anybody skims.
    """
    lower = path.lower()
    if lower.endswith(".md"):
        return text
    if lower.endswith(".rs"):
        return "\n".join(line for line in text.split("\n") if DOC_COMMENT_RE.match(line))
    return ""
