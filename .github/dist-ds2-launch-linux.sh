#!/usr/bin/env bash
# Start ds2-launcher.exe inside DARK SOULS II's own Proton prefix, on Linux.
#
#   ./ds2-launch-linux.sh            run it
#   ./ds2-launch-linux.sh --print    print the command and environment, run nothing
#
# Put this file beside DarkSoulsII.exe and ds2-launcher.exe, with Steam running. It resolves the
# same chain Steam uses for this game rather than assuming one:
#
#   compatdata/335300/config_info    line 2 names a path inside the Proton this prefix was made by
#   <proton>/toolmanifest.vdf        require_tool_appid names the container runtime it needs
#   appmanifest_<runtime>.acf        installdir names where that runtime lives
#
# and runs  <runtime>/_v2-entry-point --verb=run -- <proton>/proton run ds2-launcher.exe
# with WINEDLLOVERRIDES=dinput8=n,b, so Wine loads this package's dinput8.dll instead of its own.
#
# Steam does not start the game this way, so the session records no playtime, has no overlay and
# does no cloud sync. Steam can start ds2-launcher.exe itself and keep all three -- see "Starting
# the launcher from Steam" in the README for the launch option.
set -euo pipefail

APPID=335300

die() {
    printf 'ds2-launch-linux: %s\n' "$*" >&2
    exit 1
}

print_only=false
case "${1:-}" in
    --print) print_only=true; shift ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac

here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
[[ -f "$here/DarkSoulsII.exe" ]] || die "no DarkSoulsII.exe beside this script ($here) -- put it in the game's Game folder"
[[ -f "$here/ds2-launcher.exe" ]] || die "no ds2-launcher.exe beside this script ($here)"

# <library>/steamapps/common/<game>/Game -> <library>/steamapps
steamapps="$(readlink -f "$here/../../..")"
[[ "$(basename "$steamapps")" == steamapps ]] || die "$here is not inside a Steam library's steamapps/common"

compat="$steamapps/compatdata/$APPID"
config_info="$compat/config_info"
[[ -r "$config_info" ]] || die "cannot read $config_info -- start the game through Steam once so Proton builds its prefix"

# Line 2 is a path inside the tool, `<tool>/files/share/fonts/`: three directories up is the tool.
inside="$(sed -n '2p' "$config_info")"
inside="${inside%/}"
[[ -n "$inside" ]] || die "$config_info does not name a Proton directory"
tool="$(dirname "$(dirname "$(dirname "$inside")")")"
proton="$tool/proton"
[[ -f "$proton" ]] || die "no proton at $proton (the Proton this prefix was made by is gone -- start the game through Steam once)"

steam_root=""
for candidate in "$HOME/.steam/root" "$HOME/.local/share/Steam" "$HOME/.steam/steam"; do
    if [[ -d "$candidate" ]]; then
        steam_root="$(readlink -f "$candidate")"
        break
    fi
done
[[ -n "$steam_root" ]] || die "cannot find the Steam client directory under $HOME"

chain=("$proton" run)
runtime_appid="$(grep -m1 require_tool_appid "$tool/toolmanifest.vdf" 2>/dev/null | cut -d'"' -f4 || true)"
if [[ -n "$runtime_appid" ]]; then
    # The runtime can sit in any library: this game's, the Proton's, or Steam's own.
    entry=""
    for library in "$steamapps" "$(dirname "$(dirname "$tool")")" "$steam_root/steamapps"; do
        manifest="$library/appmanifest_$runtime_appid.acf"
        [[ -r "$manifest" ]] || continue
        installdir="$(grep -m1 '"installdir"' "$manifest" | cut -d'"' -f4)"
        if [[ -n "$installdir" && -f "$library/common/$installdir/_v2-entry-point" ]]; then
            entry="$library/common/$installdir/_v2-entry-point"
            break
        fi
    done
    [[ -n "$entry" ]] || die "the runtime this Proton needs (appid $runtime_appid) is not installed -- start the game through Steam once"
    chain=("$entry" --verb=run -- "$proton" run)
fi

export STEAM_COMPAT_DATA_PATH="$compat"
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$steam_root"
export SteamAppId="$APPID"
export SteamGameId="$APPID"
export WINEDLLOVERRIDES="dinput8=n,b"

if $print_only; then
    for name in STEAM_COMPAT_DATA_PATH STEAM_COMPAT_CLIENT_INSTALL_PATH SteamAppId SteamGameId WINEDLLOVERRIDES; do
        printf '%s=%q\n' "$name" "${!name}"
    done
    printf 'cd %q\n' "$here"
    printf '%q ' "${chain[@]}" "$here/ds2-launcher.exe" "$@"
    printf '\n'
    exit 0
fi

if pgrep -x DarkSoulsII.exe >/dev/null 2>&1; then
    die "DarkSoulsII.exe is already running -- close it first; the launcher starts its own copy"
fi

cd "$here"
exec "${chain[@]}" "$here/ds2-launcher.exe" "$@"
