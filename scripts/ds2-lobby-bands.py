#!/usr/bin/env python3
"""Look at DARK SOULS II Seamless Co-op's Steam lobbies straight from Steam, with no game running.

    python3 scripts/ds2-lobby-bands.py --dump [--filter KEY=VALUE ...]   # every key of every lobby
    python3 scripts/ds2-lobby-bands.py --once                            # one census, JSON

# How

The process initialises the Steamworks API as app 335300 (env `SteamAppId`) through a Linux
`libsteam_api.so` borrowed from another installed game, so it needs the Steam client running and
nothing else. Ported from er-mods-rs `scripts/er-lobby-bands.py`. Steam shows the account as in
DARK SOULS II while this runs.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import pathlib
import sys
import time

APP_ID = "335300"
LOBBY_MATCH_LIST_CALLBACK = 510  # k_iSteamMatchmakingCallbacks 500 + 10
COMPARISON_EQUAL = 0
DISTANCE_WORLDWIDE = 3
RESULT_CAP_REQUEST = 1000

STEAM_ROOT = pathlib.Path(os.environ.get("STEAM_ROOT", pathlib.Path.home() / ".local/share/Steam"))
LIB_CANDIDATES = [
    "steamapps/common/Crusader Kings III/binaries/libsteam_api.so",
    "steamapps/common/Valheim/valheim_Data/Plugins/libsteam_api.so",
    "steamapps/common/Dwarf Fortress/libsteam_api.so",
]


def find_lib() -> pathlib.Path:
    override = os.environ.get("DS2_STEAM_API_LIB")
    if override:
        return pathlib.Path(override)
    for rel in LIB_CANDIDATES:
        path = STEAM_ROOT / rel
        if path.is_file():
            return path
    raise SystemExit("no libsteam_api.so with SteamAPI_Init found; set DS2_STEAM_API_LIB")


class Steam:
    def __init__(self) -> None:
        os.environ["SteamAppId"] = APP_ID
        os.environ["SteamGameId"] = APP_ID
        lib = ctypes.CDLL(str(find_lib()))
        u64, p, c, i = ctypes.c_uint64, ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int

        def fn(name, res, *args):
            f = getattr(lib, name)
            f.restype, f.argtypes = res, list(args)
            return f

        self.init = fn("SteamAPI_Init", ctypes.c_bool)
        self.run_callbacks = fn("SteamAPI_RunCallbacks", None)
        mm = fn("SteamAPI_SteamMatchmaking_v009", p)
        utils = fn("SteamAPI_SteamUtils_v010", p)
        self.request = fn("SteamAPI_ISteamMatchmaking_RequestLobbyList", u64, p)
        self.string_filter = fn("SteamAPI_ISteamMatchmaking_AddRequestLobbyListStringFilter", None, p, c, c, i)
        self.numerical_filter = fn("SteamAPI_ISteamMatchmaking_AddRequestLobbyListNumericalFilter", None,
                                   p, c, i, i)
        self.distance = fn("SteamAPI_ISteamMatchmaking_AddRequestLobbyListDistanceFilter", None, p, i)
        self.count_filter = fn("SteamAPI_ISteamMatchmaking_AddRequestLobbyListResultCountFilter", None, p, i)
        self.by_index = fn("SteamAPI_ISteamMatchmaking_GetLobbyByIndex", u64, p, i)
        self.data_count = fn("SteamAPI_ISteamMatchmaking_GetLobbyDataCount", i, p, u64)
        self.data_by_index = fn("SteamAPI_ISteamMatchmaking_GetLobbyDataByIndex", ctypes.c_bool,
                                p, u64, i, c, i, c, i)
        self.num_members = fn("SteamAPI_ISteamMatchmaking_GetNumLobbyMembers", i, p, u64)
        self.member_limit = fn("SteamAPI_ISteamMatchmaking_GetLobbyMemberLimit", i, p, u64)
        self.owner = fn("SteamAPI_ISteamMatchmaking_GetLobbyOwner", u64, p, u64)
        self.completed = fn("SteamAPI_ISteamUtils_IsAPICallCompleted", ctypes.c_bool, p, u64,
                            ctypes.POINTER(ctypes.c_bool))
        self.result = fn("SteamAPI_ISteamUtils_GetAPICallResult", ctypes.c_bool, p, u64, p, i, i,
                         ctypes.POINTER(ctypes.c_bool))
        if not self.init():
            raise SystemExit("SteamAPI_Init failed: is the Steam client running and logged in?")
        self.mm, self.utils = mm(), utils()

    def query(self, filters: list[tuple[str, str]],
              numeric: list[tuple[str, int, int]] = ()) -> list[int]:
        """One RequestLobbyList with these equality filters and (key, value, comparison) numerical
        filters, worldwide; the lobby ids."""
        for key, value in filters:
            self.string_filter(self.mm, key.encode(), value.encode(), COMPARISON_EQUAL)
        for key, value, comparison in numeric:
            self.numerical_filter(self.mm, key.encode(), value, comparison)
        self.distance(self.mm, DISTANCE_WORLDWIDE)
        self.count_filter(self.mm, RESULT_CAP_REQUEST)
        call = self.request(self.mm)
        failed = ctypes.c_bool(False)
        deadline = time.monotonic() + 20
        while not self.completed(self.utils, call, ctypes.byref(failed)):
            if time.monotonic() > deadline:
                raise TimeoutError(f"lobby query {filters} never completed")
            self.run_callbacks()
            time.sleep(0.05)
        matching = ctypes.c_uint32(0)
        ok = self.result(self.utils, call, ctypes.byref(matching), 4, LOBBY_MATCH_LIST_CALLBACK,
                         ctypes.byref(failed))
        if not ok or failed.value:
            raise RuntimeError(f"lobby query {filters} failed")
        return [self.by_index(self.mm, index) for index in range(matching.value)]

    def lobby_data(self, lobby: int) -> dict[str, str]:
        data = {}
        for index in range(self.data_count(self.mm, lobby)):
            key = ctypes.create_string_buffer(256)
            value = ctypes.create_string_buffer(8192)
            if self.data_by_index(self.mm, lobby, index, key, 256, value, 8192):
                data[key.value.decode("latin-1")] = value.value.decode("latin-1")
        return data


def parse_filters(raw: list[str]) -> list[tuple[str, str]]:
    out = []
    for item in raw:
        key, sep, value = item.partition("=")
        if not sep:
            raise SystemExit(f"--filter wants KEY=VALUE, got {item!r}")
        out.append((key, value))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--dump", action="store_true", help="every key/value of every lobby returned")
    ap.add_argument("--filter", action="append", default=[], help="KEY=VALUE equality filter")
    ap.add_argument("--numeric", action="append", default=[],
                    help="KEY:COMPARISON:VALUE numerical filter (comparison as Steam's ELobbyComparison: "
                         "-2 <=, -1 <, 0 ==, 1 >, 2 >=, 3 !=)")
    args = ap.parse_args()
    steam = Steam()
    numeric = []
    for item in args.numeric:
        key, comparison, value = item.rsplit(":", 2)
        numeric.append((key, int(value), int(comparison)))
    if args.dump:
        lobbies = steam.query(parse_filters(args.filter), numeric)
        out = [{"id": lobby, "members": steam.num_members(steam.mm, lobby),
                "limit": steam.member_limit(steam.mm, lobby),
                "owner": steam.owner(steam.mm, lobby), "data": steam.lobby_data(lobby)}
               for lobby in lobbies]
        print(json.dumps(out, indent=1))
        return 0
    ap.error("nothing to do")
    return 2


if __name__ == "__main__":
    sys.exit(main())
