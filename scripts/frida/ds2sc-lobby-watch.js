// What does DARK SOULS II Seamless Co-op publish on its lobby, and what does its own search ask for?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/ds2sc-lobby-watch.js`
//
// # Why at the interface
//
// The strings that build the matchmaking value ("ds2sc_yui_nexus_session_search_", "_bimm_",
// "_qmmm_", "_comm_") sit in ds2sc.dll's .rdata, but no code in its decrypted image references them
// by address (measured 2026-10-21 on a dump from scripts/frida/ds2sc-image-dump.js), so the value is
// read where it surfaces: the ISteamMatchmaking calls. Slot numbers are ISteamMatchmaking v009's,
// the interface the game's own steam_api64.dll `SteamMatchmaking()` export returns; ported from
// er-mods-rs scripts/frida/lobby-key-watch.js.
//
// Each publish and each search is printed beside this character's soul level, soul memory and the
// reinforcement level of its six weapon records, so the bracket can be tied to a stat by data.
//
// Observers only (`Interceptor.attach` on rarely-called matchmaking entries, never GetLobbyData).

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const GAME_MANAGER_IMP = 0x016148f0;
const PLAYER_CTRL_OFFSET = 0xd0;
const PLAYER_PARAM_OFFSET = 0x490;
const SOUL_LEVEL_OFFSET = 0xd0;
const SOUL_MEMORY_OFFSET = 0xf4;
const CHR_ASM_CTRL_OFFSET = 0x378;
const RECORD_TABLE_OFFSET = 0x20;
const RECORD_ARRAY_OFFSET = 0x08;
const RECORD_STRIDE = 0x14;
const RECORD_ITEM_OFFSET = 0x04;
const RECORD_LEVEL_OFFSET = 0x0e;
const WEAPON_RECORDS = 6;

const SLOTS = {
  4: 'RequestLobbyList',
  5: 'AddRequestLobbyListStringFilter',
  6: 'AddRequestLobbyListNumericalFilter',
  7: 'AddRequestLobbyListNearValueFilter',
  8: 'AddRequestLobbyListFilterSlotsAvailable',
  9: 'AddRequestLobbyListDistanceFilter',
  10: 'AddRequestLobbyListResultCountFilter',
  13: 'CreateLobby',
  14: 'JoinLobby',
  15: 'LeaveLobby',
  20: 'SetLobbyData',
  23: 'DeleteLobbyData',
  27: 'SetLobbyMemberData',
  30: 'SetLobbyMemberLimit',
  32: 'SetLobbyType',
  33: 'SetLobbyJoinable',
};

function rp(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return null;
  }
}

function str(p) {
  try {
    return p.isNull() ? null : p.readUtf8String();
  } catch (e) {
    return null;
  }
}

function character() {
  const gm = rp(image.base.add(GAME_MANAGER_IMP));
  if (gm === null || gm.isNull()) return { world: false };
  const ctrl = rp(gm.add(PLAYER_CTRL_OFFSET));
  if (ctrl === null || ctrl.isNull()) return { world: false };
  const out = { world: true };
  const param = rp(ctrl.add(PLAYER_PARAM_OFFSET));
  try {
    out.soul_level = param.add(SOUL_LEVEL_OFFSET).readU32();
    out.soul_memory = param.add(SOUL_MEMORY_OFFSET).readU32();
  } catch (e) {
    out.param = 'unreadable';
  }
  try {
    const asm = rp(ctrl.add(CHR_ASM_CTRL_OFFSET));
    const table = rp(asm.add(RECORD_TABLE_OFFSET));
    out.weapons = [];
    for (let i = 0; i < WEAPON_RECORDS; i++) {
      const rec = table.add(RECORD_ARRAY_OFFSET + i * RECORD_STRIDE);
      out.weapons.push([rec.add(RECORD_ITEM_OFFSET).readU32(), rec.add(RECORD_LEVEL_OFFSET).readU8()]);
    }
  } catch (e) {
    out.weapons = 'unreadable';
  }
  return out;
}

const steam = Process.getModuleByName('steam_api64.dll');
const accessor = new NativeFunction(steam.getExportByName('SteamMatchmaking'), 'pointer', []);
const iface = accessor();
const vtable = iface.readPointer();
send({ tag: 'ready', iface: iface.toString(), character: character() });

let calls = 0;
for (const [slotText, name] of Object.entries(SLOTS)) {
  const slot = Number(slotText);
  const target = vtable.add(slot * Process.pointerSize).readPointer();
  Interceptor.attach(target, {
    onEnter(args) {
      calls += 1;
      const msg = { tag: name, n: calls, self: args[0].toString(), caller: this.returnAddress.toString() };
      const mod = Process.findModuleByAddress(this.returnAddress);
      if (mod !== null) msg.caller = mod.name + '+0x' + this.returnAddress.sub(mod.base).toString(16);
      if (name === 'SetLobbyData') {
        msg.lobby = args[1].toString();
        msg.key = str(args[2]);
        msg.value = str(args[3]);
      } else if (name === 'DeleteLobbyData') {
        msg.lobby = args[1].toString();
        msg.key = str(args[2]);
      } else if (name === 'AddRequestLobbyListStringFilter') {
        msg.key = str(args[1]);
        msg.value = str(args[2]);
        msg.comparison = args[3].toInt32();
      } else if (name === 'AddRequestLobbyListNumericalFilter' || name === 'AddRequestLobbyListNearValueFilter') {
        msg.key = str(args[1]);
        msg.value = args[2].toInt32();
        msg.comparison = args[3].toInt32();
      } else if (name === 'RequestLobbyList') {
        msg.character = character();
      } else if (slot >= 8 && slot <= 10) {
        msg.value = args[1].toInt32();
      } else if (name === 'SetLobbyMemberLimit' || name === 'SetLobbyJoinable' || name === 'SetLobbyType') {
        msg.lobby = args[1].toString();
        msg.value = args[2].toInt32();
      }
      if (name === 'SetLobbyData') msg.character = character();
      send(msg);
    },
  });
}
send({ tag: 'hooked', slots: Object.values(SLOTS) });
