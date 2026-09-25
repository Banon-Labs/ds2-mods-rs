# Recipes for this repo. `just` on its own lists them.

default:
    @just --list

# Open the Dark Souls II build search page, mirroring soulsplanner first if needed.
builds:
    #!/usr/bin/env bash
    set -euo pipefail
    sp="{{justfile_directory()}}/scripts/soulsplanner-search.py"
    cache="${XDG_CACHE_HOME:-$HOME/.cache}/soulsplanner"
    page="$cache/ledger.html"
    builds="$cache/builds-darksouls2.json"
    if [ ! -s "$builds" ]; then
        echo "No local mirror yet. Fetching ~6.6k build pages at 2/s -- about an hour."
        "$sp" index
        "$sp" fetch
    fi
    # the page embeds the whole mirror, so it only needs rebuilding when the
    # mirror or the template moved on
    tpl="{{justfile_directory()}}/scripts/soulsplanner-page.template.html"
    if [ ! -s "$page" ] || [ "$builds" -nt "$page" ] || [ "$tpl" -nt "$page" ]; then
        "$sp" page "$page"
    fi
    echo "file://$page"
    setsid xdg-open "$page" >/dev/null 2>&1 < /dev/null &

# Pull builds published since the last mirror, then rebuild the page.
builds-refresh:
    #!/usr/bin/env bash
    set -euo pipefail
    sp="{{justfile_directory()}}/scripts/soulsplanner-search.py"
    cache="${XDG_CACHE_HOME:-$HOME/.cache}/soulsplanner"
    "$sp" index
    "$sp" fetch            # already-cached ids are skipped
    "$sp" page "$cache/ledger.html"

# Search the mirror from the terminal, e.g. just builds-query --class knight --max-level 20
builds-query *ARGS:
    @{{justfile_directory()}}/scripts/soulsplanner-search.py query {{ARGS}}
