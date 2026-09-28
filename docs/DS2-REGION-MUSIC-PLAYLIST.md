# Region music playlist: feasibility (static RE, 2026-09-27)

Request: an in-game (imgui) panel that shows the BGM track that is playing, seeks it, stops it
repeating, lets a region's playlist gain extra in-game tracks or drop the default one, and keeps
all of that across reloads and quit-outs.

**Verdict: feasible, with runtime checks still open.** Every piece the design needs exists in the
shipped binaries. The main hook can sit on an import slot, so no Arxan-checked `.text` has to be
patched. What static reading cannot settle is listed under
[What still needs a runtime check](#what-is-proven-vs-what-still-needs-a-runtime-check). Nothing
here was launched or attached.

Tools used (all read-only): `scripts/ds2-ebl.py` (archive extraction),
`scripts/fsb5-list.py` (new: FSB5 subsound names, lengths and loop points),
`scripts/pe-import-callsites.py` (new: FMOD import -> IAT slot -> thunk -> call sites),
`scripts/ds2-fmod-disasm.py` (new: disassembles a `.pdata` function with FMOD thunk calls named),
`scripts/pe-call-xrefs.py`, `scripts/ds2-xrefs.py`, `scripts/ds2-rtti.py`,
`scripts/ds2-arxan-chain.py`. The three new scripts are uncommitted.

---

## 1. Engine, DLLs, banks, naming

### FMOD Ex / FMOD Designer 4.44.50, dynamically linked

| file (Game/) | FileVersion |
| --- | --- |
| `fmodex64.dll` | 4.44.50 |
| `fmod_event64.dll` | 4.44.50 |
| `fmod_event_net64.dll` | (not inspected) |

`DarkSoulsII.exe` imports from both. Every call goes through an MSVC `jmp [rip+IAT]` thunk. Beads
memory `ds2-sotfs-9527516-fmod-is-not-statically-linked` and `docs/DS2-CONTINUE.md` already record
this. The music-relevant imports
(`uv run --with pefile python3 scripts/pe-import-callsites.py <exe> fmod_event`):

| import | IAT slot | thunk | call sites |
| --- | --- | --- | --- |
| `Event::start` | `0x141aae764` | `0x140a04bb6` | 6 |
| `Event::stop` | `0x141aae75c` | `0x140a04bbc` | 2 |
| `Event::setPaused` | `0x141aae85c` | `0x140a04bf2` | 7 |
| `Event::setMute` | `0x141aae82c` | `0x140a04bfe` | 1 |
| `Event::getInfo` | `0x141aae844` | `0x1409fd438` | 9 |
| `Event::getChannelGroup` | `0x141aae74c` | `0x140a04bc8` | 3 |
| `EventSystem::getEvent` | `0x141aae7ec` | `0x1409fd420` | 4 |
| `EventSystem::getEventBySystemID` | `0x141aae83c` | `0x140a017a2` | 1 |
| `EventSystem::load` | `0x141aae7e4` | `0x1409fd3f6` | 5 |

No `ff 15` direct IAT calls exist for any of these. So replacing an IAT slot catches every game
call to that function.

`fmodex64.dll` exports everything seek and loop control need:
`Channel::setPosition/getPosition`, `Channel::setLoopCount/getLoopCount`, `Channel::setMode/getMode`,
`Channel::getCurrentSound`, `Channel::setCallback`, `ChannelGroup::getChannel/getNumChannels/getGroup/getNumGroups`,
`Sound::getLength/getName/getSubSound`, plus the C API `FMOD_Channel_SetPosition`. `fmod_event64.dll`
exports `EventSystem::getEventByGUID(String)`, `EventGroupI::getEventByIndex/getNumEvents/loadEventData`
and `Event::setPropertyByIndex`. The mod reaches all of them with `GetProcAddress`.

### Where the data lives

* **Event projects (`.fev`)** are packed in `GameDataEbl.bdt` under `/sound/frpg2.fevbnd.dcx`
  (`python3 scripts/ds2-ebl.py extract /sound/frpg2.fevbnd.dcx --out <dir>`). The bnd holds
  `frpg2.fev, frpg2_m.fev, frpg2_sm.fev, frpg2_sm_dlc.fev, frpg2_sm_best.fev, frpg2_xm.fev,
  frpg2_ps.fev, frpg2_pm.fev, frpg2_c1..c9.fev, frpg2_efx.fev, *_dlc.fev`, plus `*.itl` files.
  The `.itl` files are `ITLIMITER_INFO` tables (instance limits per group such as `10040000`), not
  a name index.
* **Music audio** is loose on disk, in `Game/sound/frpg2_smAABB.fsb`, one bank per map area. The
  archive entry `/sound/frpg2_sm1004.bnd` holds only a `Dummy` stub, so the loose FSB is the real
  one. The exe path format is `%ssound/frpg2_sm%02d%02d.bnd`, with siblings `frpg2_m` (SE),
  `frpg2_xm` and `frpg2_ps`.
* In `frpg2_sm.fev` every `frpg2_smXXXX` bank carries flag `0x80`. The in-archive `frpg2_m*` and
  `frpg2_main*` banks carry `0x200`. The loose-on-disk families (sm, xm, ps) are all `0x80`, which
  fits **stream from disk**. That is inferred from the correlation. The FEV spec was not read.
* The FSB5 bank hash `9c ac 44 f4 b5 a3 cd c6` at `frpg2_sm1004.fsb+0x24` matches the hash
  `frpg2_sm.fev` stores for bank `frpg2_sm1004`, so the event project and the loose file are tied
  together.

### Naming

Sound IDs are a type letter plus a 9-digit number. `0x140b15210` formats them with `"%c%09d"`
(`0x1411d78c0`) into a 0x100-byte buffer, indexing the type table `"smacpoz"` at `0x1411d78b8`
(0=s, 1=m, 2=a, 3=c, 4=p, 5=o, 6=z). **Music is type 1, `m`.** The FSB subsound names match
the event names:

```
frpg2_sm1004.fsb  m100400001 198.3s loop=(629700, 8745027)   <- Majula theme
                  m100400002 144.0s no loop
frpg2_sm1016.fsb  m101600001 201.6s, m101600002 162.5s, m101600003 156.8s
frpg2_sm2021.fsb  m102800001, m102800002, m202100001..3, m202100004_andeal, m760203000
frpg2_sm5036.fsb  m101900001 185.5s  (same name in frpg2_sm1019.fsb is 149.7s -- NOT unique)
frpg2_smain.fsb   m000000002 229.4s, m000000003 404.1s (title/menu), bossout, gamestart
```

(`python3 scripts/fsb5-list.py ".../Game/sound/"frpg2_sm*.fsb`; it prints the `m*`/`o*` entries.)
The pattern is `mAAAANNNNN`, where AAAA is the area of the bank's map and NNNNN is a per-area
index. Boss tracks sit in the boss's own map bank. **The bare name is not a unique key.**
`m101900001`, `m101900002` and `m503500003` each exist in two banks with different content, so a
playlist entry has to store `(bank, name)`.

`frpg2_xmAAAA.fsb` holds a single `yAAAA00000` subsound per area. That is ambience, not music.

## 2. How a region picks its BGM, and where the handle is

**Region to track is map data, not a param table.** `/map/m10_04_00_00/m10_04_00_00.msb`
(from `GameDataEbl`) has MSB2 POINT entries of region type `7` (Sound) that carry the sound ID:

```
0x4f10  80 00 .. 1d 00 07 05 01 00 ..      type 7, shape 5 (box 140 x 79 x 40)
0x4fb4  01 00 00 00  81 fb fb 05           -> 1, 100400001 (m100400001)
0x4e50  ... 1a 00 07 05 00 00 ..           type 7, shape 5 (box 60 x 129 x 15)
0x4eec  00 00 00 00  81 fb fb 05           -> 0, 100400001
0x52a4  00 00 00 00  82 fb fb 05           -> 100400002
```

The IDs are proven. The meaning of the preceding u32 (0 vs 1) is not proven. It may be a sound
type or a priority. So the game's sound-region system (`MapSoundComponent` vtable `0x1410ec488`,
`MapSoundSlotCtrl` `0x1410c98a0`, `MapSoundCtrl` `0x1410ec590`) starts `m100400001` when the
player is inside a Majula BGM box. Boss music is presumably raised by event script. That path
was not traced.

**The playback object is `DLMO::MOFmodCue`.** RTTI names it, vtable `0x141187c58`. Its
prologues are clean in `ds2-arxan-chain.py`.

```
MOFmodCue
  +0x370  FMOD::Event*          (every start/stop/pause site loads rcx from here)
  +0x1e8  FMOD::ChannelGroup*   (written by Event::getChannelGroup in slot 9)
  +0x1b4  u32 state flags
  vtable slot 9  0x1409f8b90  prepare: Event::start; setPaused(true); getChannelGroup
  vtable slot 6  0x1409f93b0  stop:    [start if +0x38c==1]; unpause; Event::stop
  vtable slot 2  0x1409f9660  contains another Event::start
0x1409f5120  resolve SoundID -> Event: getEvent(name, 4=INFOONLY), else name -> systemid map at
             [this+0x38], then getEventBySystemID(id, 4); then getInfo and reads info.systemid
```

The global chain is proven by the init code:

```
[0x14166dfa8]  MOFmodSoundManager*          (accessor 0x1409ddbc0)
   +0x9d8  FMOD::EventSystem*   1409de1e4: lea r12,[r15+0x9d8]; call FMOD_EventSystem_Create
   +0x9e0  FMOD::System*        1409de33b: lea r13,[r15+0x9e0]; call EventSystem::getSystemObject
   +0x9f8  FMOD::ChannelGroup*  master (docs/DS2-CONTINUE.md)
```

The `"music"` category index is cached at `this+0x1148` of the object built in `0x140b074d0`
(`lea rcx,"music"` at `0x140b076fd`).

**What a mod reads for "what's playing".** It does not need the cue layout. It uses the event
handle FMOD passes through the `Event::start` import:

* `Event::getInfo(&index, &name, &info)`. `name` is the event name (`m100400001`). The game's
  own use of this call at `0x1409f5243` reads the WORD at `info+0x24`, which is `systemid` in the
  4.44 `FMOD_EVENT_INFO` layout (`memoryused, positionms +4, lengthms +8, ..., projectid +0x20,
  systemid +0x24`). So the header layout is corroborated at one field.
* `positionms` is at `+4`. `lengthms` is at `+8`, and FMOD documents it as -1 for an event with
  looping sounds, which is expected here.
* For a real length and position: `Event::getChannelGroup`, then `ChannelGroup::getChannel(0)` or
  a walk through `getGroup(i)`, then `Channel::getCurrentSound` / `getPosition(ms)`, then
  `Sound::getLength(ms)`.

## 3. Looping and seek

* **The loop is in the audio, not only in the event.** `m100400001` has an FSB5 loop chunk
  `(629700, 8745027)` at 44.1 kHz. That is 14.28 s to 198.30 s, and the end is the last sample,
  so the track is an intro plus a loop body. Most `m*` tracks have loop chunks. A few don't
  (`m100400002`, `m103201040`, `m202100003`, `m702100002/3`). Whether the FEV sound definition
  also retriggers was not decoded from the FEV.
* **"Stop repeating" has two routes:**
  1. Deterministic: poll `Channel::getPosition`. When it wraps back (new < old), or comes within
     a frame of the loop end, advance the playlist or stop the event. This does not depend on how
     the event is authored.
  2. `Channel::setLoopCount(0)` / `setMode(FMOD_LOOP_OFF)` on the event's channel. That is
     cheaper, but the event system may retrigger the sound, so it needs a runtime check.
* **Seek:** the event API has no timeline seek, but `Channel::setPosition(ms, FMOD_TIMEUNIT_MS)`
  on the channel under the event's channel group is exported, and FMOD Ex supports seeking
  streams. Whether it behaves on these streamed Vorbis FSB5 subsounds without a glitch, and
  whether the event system resets the position, needs a runtime check.

## 4. Playing another region's track while in Majula

* All base-game `sm` banks are **one** event project, `frpg2_sm.fev`. The DLC banks sm5035 to
  sm5038 are in `frpg2_sm_dlc.fev`. The events for Heide's or a boss's track are in the same
  project as Majula's.
* Their audio is loose on disk and marked for streaming (`0x80`), so starting such an event
  opens the stream without a preloaded bank.
* The init function (`MOFmodSoundManager::v6`, `0x1409ddbe0`) calls `EventSystem::load` from a
  list configured by `FmodEventFilePathNum` / `FmodEventFilePath_%d`, and an `IsDivideFEV`
  switch exists. Three more `load` sites exist (`0x1409df9c6`, `0x1409e52f2`, `0x1409e55e7`).
  **Whether `frpg2_sm.fev` stays loaded for the whole session is not proven.** If it does, a
  foreign track is `getEvent`/`getEventBySystemID` away. If it doesn't, the mod calls
  `EventSystem::load("frpg2_sm.fev")` itself. The data is in the fevbnd the game already mounted.
* The mod resolves `(bank, name)` to an event without guessing path syntax. It enumerates
  `getNumProjects` / `getProjectByIndex`, then the project's groups, then
  `EventGroup::getEventByIndex(i, INFOONLY)` and `getInfo(name)`. It caches the result once.

## 5. Design

### Hooks (no `.text` patches)

1. **IAT slot `Event::start` (`0x141aae764`).** This is the same technique `ds2-offline` uses
   for WS2_32 and `ds2-continue` uses to call FMOD. In the detour, call `getInfo`. If the name
   matches `^m\d{9}` (or `m_death`, `m_training` and similar), record it as the current BGM
   instance, along with a game-cue flag and a timestamp. If the region has an override playlist:
   let the game's event start, `Event::setMute(true)` it, and start our own event for the
   playlist's current entry. The game's instance then carries its fades, boss stop and area
   change as before, and ours follows it.
2. **IAT slot `Event::stop` (`0x141aae75c`) and `Event::setPaused` (`0x141aae85c`).** When the
   game stops or pauses its muted BGM instance, do the same to ours. That covers boss fights,
   cutscenes and area exits.
3. **Per-frame tick on the overlay thread.** Poll the current instance's channel position.
   Handle end-of-track as repeat off / next / shuffle. The panel's seek bar calls
   `Channel::setPosition`. FMOD Ex takes its own locks, and `ds2-continue` measured inline
   `setVolume` from a non-FMOD thread working.

The region key is the **area of the game's default track** (`1004` from `m100400001`). A
secondary key could be the map ID `m10_04_00_00` once there is a reader for it. There is none in
the repo today. Keying on the event the game asked for works in every map with no map-ID RE:
"when the game wants `m100400001`, play my Majula list instead".

### Panel (a `ds2-overlay` panel crate)

* Now playing: `bank / name`, a human label if the user sets one, `position / length`, a seek
  slider, play/pause, and next/prev.
* Repeat mode: `loop track` (game default) / `play once then next` / `play once then silence`.
* Region playlist editor: the current region's default track (with a remove toggle), added
  tracks, and an "add track" picker. The picker is built at startup by running `fsb5-list`-style
  parsing over the headers of the loose `sound/frpg2_sm*.fsb` files. It is read-only and touches
  no game memory. It filters to `m*` names and shows length and bank.
* A "preview" button that starts a foreign track once, to check the resolve and stream path.

### Persistence

`Game/ds2-music-playlist.toml`, beside `DarkSoulsII.exe` like `ds2-mods.toml` and
`weapon-sync.json`. Do **not** put it in `ds2-mods.toml`, which `scripts/ds2-run.py` rewrites on
every launch.

```toml
[region.1004]            # key = area of the game's default BGM
default = "frpg2_sm1004/m100400001"
include_default = true
repeat = "next"          # loop | next | once
tracks = ["frpg2_sm1016/m101600001", "frpg2_sm2021/m202100004_andeal"]
```

The mod writes the file on every edit (atomic rename) and reads it at DLL init. Nothing touches
the save, so it survives quit-outs and reloads by construction.

### Risks

* **Arxan:** the design patches IAT slots only. That is the `ds2-offline` precedent, and it has
  been measured working with WS2_32. The cue functions a `.text` detour would use
  (`0x1409f8b90`, `0x1409f93b0`, `0x1409f9660`, `0x1409f5120`) have clean prologues in
  `ds2-arxan-chain.py`, so they are a fallback. An `Event::start` IAT hook also fires for every
  SE and voice event: `getInfo` plus a first-byte check has to stay cheap.
* **Other mods:** `ds2-continue` calls through the `ChannelGroup::setVolume` import slot and
  reads `MOFmodSoundManager+0x930/0x9f8`. It does not touch `Event::start`. Seamless Co-op and
  the DS2 Lighting Engine `dxgi.dll` were not checked for FMOD hooks. Chain rather than overwrite
  the slot: save the old pointer, like `ds2-offline` does.
* **Save safety:** audio only. There are no save or param writes.
* **Online:** client-local audio, nothing sent.
* **Thread safety:** FMOD Ex locks internally. Keep every FMOD call out of `EventSystem::update`
  re-entrancy, which means none from inside FMOD callbacks.

## What is proven vs. what still needs a runtime check

Proven statically (with the command or address above):
FMOD Ex 4.44.50 and its DLLs; the IAT-thunk-only call path; the music naming and type table;
per-area `sm` banks, their track lengths and loop chunks; the non-unique names; Majula's MSB
sound regions pointing at `100400001` / `100400002`; `MOFmodCue` and its `+0x370` event /
`+0x1e8` channel group; `MOFmodSoundManager+0x9d8` / `+0x9e0` / `+0x9f8`; the `FMOD_EVENT_INFO`
systemid offset; the seek and loop exports.

Still needs a runtime check:
1. `Event::start` via the IAT fires for the Majula BGM, and `getInfo` returns `m100400001`. Needs
   a DLL or Frida log line. Frida attach has frozen the game before.
2. `frpg2_sm.fev` stays loaded in Majula, so a foreign `m*` event can be resolved and started
   without `EventSystem::load`.
3. The channel under the BGM event's channel group is reachable (flat or nested), and
   `Channel::setPosition` seeks a streamed Vorbis FSB5 cleanly.
4. `setLoopCount(0)` on that channel ends the track without the event retriggering, or else the
   position-wrap fallback is the only route.
5. `Event::setMute` on the game's instance plus our own parallel instance doesn't break the
   game's fade and stop handling (boss arenas, bonfire warp, cutscenes).
6. The `0x80` = stream-from-disk reading of the FEV bank flag, and the meaning of the MSB sound
   region's u32 before the sound ID.
