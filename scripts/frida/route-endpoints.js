// What is at the far end of every path `ds2-invasion-path` is drawing right now?
//
// THE QUESTION THIS ANSWERS, AND WHY THE LOG CANNOT.
//
// The crate routes to objects whose vtable is `PlayerCtrl`'s, and DARK SOULS II builds three
// different things out of that one class -- `0x140357920` formats `L"Player_%06u"` for the local
// player, and `0x1403572e0` formats `L"NetworkPlayer_%06u"` for a real remote player and
// `L"GhostPlayer_%06u"` for a bloodstain replay. The vtable test cannot separate the last two, so
// `roster: remotes=2` is equally consistent with two invaders and with two bloodstains.
//
// The formatted name CAN separate them, because the two factories write different literals. This
// reads it off each object and says which of the three each routed target is.
//
// NO TIMER, ON PURPOSE. Frida's JS runtime in this process can wedge permanently (see the
// `frida-js-thread-wedges-in-ds2` memory): timers stop firing while the game runs on. This agent
// does one pass at load, sends one message, and installs nothing -- so there is no heartbeat to
// misread and no hook to starve the thread with.

'use strict';

const GAME_MANAGER_IMP = 0x016148f0; // RVA; ds2_rva::GAME_MANAGER_IMP
const PLAYER_CTRL_OFFSET = 0xd0; // GameManagerImp -> the local PlayerCtrl
const CHARACTER_MANAGER_OFFSET = 0x18; // GameManagerImp -> CharacterManager
const ENTITY_BEGIN_OFFSET = 0x10; // CharacterManager -> roster begin
const ENTITY_END_OFFSET = 0x18; // CharacterManager -> roster end
const POSITION_OFFSET = 0x90; // CharacterCtrl -> f32[4], y up
const PLAYER_CTRL_VTABLE = 0x010e4bb8; // RVA
const CHARACTER_CTRL_VTABLE = 0x010df218; // RVA

//: `PlayerCtrl` is 0x4a0 bytes, so the name pointer is inside that. Walked in 8-byte steps.
const PLAYER_CTRL_SIZE = 0x4a0;
//: The roster bound the crate itself refuses past; a longer span is a torn read, not a roster.
const MAX_ROSTER = 8192;

function readPointer(at) {
  try {
    return at.readPointer();
  } catch (_) {
    return null;
  }
}

function readFloats(at, count) {
  try {
    return at.readByteArray(count * 4);
  } catch (_) {
    return null;
  }
}

function position(chr) {
  const raw = readFloats(chr.add(POSITION_OFFSET), 3);
  if (raw === null) {
    return null;
  }
  const view = new Float32Array(raw);
  for (let i = 0; i < 3; i += 1) {
    if (!isFinite(view[i])) {
      return null;
    }
  }
  return [view[0], view[1], view[2]];
}

function distance(a, b) {
  if (a === null || b === null) {
    return null;
  }
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  const dz = a[2] - b[2];
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

// The name the factory formatted, hunted rather than read from a known offset.
//
// No constant in `ds2-rva` records where the name lands in the object, and guessing one would be
// the sort of unverified offset this repo refuses. Scanning for a pointer whose target is a wide
// string matching the literals the two factories are known to format is a MEASUREMENT: a hit is
// the string itself, printed, and a miss says "not found" rather than inventing a field.
function nameOf(chr) {
  for (let offset = 0; offset < PLAYER_CTRL_SIZE; offset += 8) {
    const candidate = readPointer(chr.add(offset));
    if (candidate === null || candidate.isNull()) {
      continue;
    }
    let text;
    try {
      text = candidate.readUtf16String(32);
    } catch (_) {
      continue;
    }
    if (typeof text !== 'string') {
      continue;
    }
    if (/^(Network|Ghost)?Player_\d+/.test(text)) {
      return { at: offset, text: text };
    }
  }
  return null;
}

function classify(name) {
  if (name === null) {
    return 'PlayerCtrl, name not found';
  }
  if (name.text.indexOf('NetworkPlayer_') === 0) {
    return 'a REAL REMOTE PLAYER (NetworkPlayer factory)';
  }
  if (name.text.indexOf('GhostPlayer_') === 0) {
    return 'a bloodstain replay (GhostPlayer factory) -- not a live person';
  }
  if (name.text.indexOf('Player_') === 0) {
    return 'the local player (Player factory)';
  }
  return 'PlayerCtrl, unrecognised name';
}

function main() {
  // `Process.findModuleByName`, not `Module.findBaseAddress`: the latter is gone in Frida 17 and
  // fails as `TypeError: not a function`, which reads like a bad address rather than a bad API.
  const module = Process.findModuleByName('DarkSoulsII.exe');
  if (module === null) {
    send({ error: 'DarkSoulsII.exe is not loaded in this process' });
    return;
  }
  const base = module.base;
  const manager = readPointer(base.add(GAME_MANAGER_IMP));
  if (manager === null || manager.isNull()) {
    send({ error: 'GameManagerImp is null -- no world yet' });
    return;
  }
  const local = readPointer(manager.add(PLAYER_CTRL_OFFSET));
  const characters = readPointer(manager.add(CHARACTER_MANAGER_OFFSET));
  if (characters === null || characters.isNull()) {
    send({ error: 'CharacterManager is null' });
    return;
  }
  const begin = readPointer(characters.add(ENTITY_BEGIN_OFFSET));
  const end = readPointer(characters.add(ENTITY_END_OFFSET));
  if (begin === null || end === null || begin.isNull()) {
    send({ error: 'roster bounds unreadable' });
    return;
  }
  const span = end.sub(begin).toInt32();
  if (span < 0 || span % 8 !== 0 || span / 8 > MAX_ROSTER) {
    send({ error: 'roster span refused: ' + span + ' bytes' });
    return;
  }

  const playerVtable = base.add(PLAYER_CTRL_VTABLE);
  const characterVtable = base.add(CHARACTER_CTRL_VTABLE);
  const localAt = local === null || local.isNull() ? null : position(local);
  const roster = [];
  for (let index = 0; index < span / 8; index += 1) {
    const chr = readPointer(begin.add(index * 8));
    if (chr === null || chr.isNull()) {
      continue;
    }
    const vtable = readPointer(chr);
    if (vtable === null) {
      continue;
    }
    const isPlayer = vtable.equals(playerVtable);
    const at = position(chr);
    const entry = {
      ctrl: chr.toString(),
      vtable: vtable.toString(),
      klass: isPlayer
        ? 'PlayerCtrl'
        : vtable.equals(characterVtable)
          ? 'CharacterCtrl (NPC)'
          : 'other subclass (NPC)',
      position: at,
      meters: at === null ? null : distance(at, localAt),
      isLocal: local !== null && !local.isNull() && chr.equals(local),
    };
    if (isPlayer) {
      const name = nameOf(chr);
      entry.name = name === null ? null : name.text;
      entry.nameAt = name === null ? null : name.at;
      entry.verdict = entry.isLocal ? 'the local player' : classify(name);
      // WHERE THE LOCAL PLAYER'S NAME WAS FOUND, TRIED ON THIS OBJECT TOO.
      //
      // The scan above found `Player_000100` on the local player and nothing on either remote,
      // and two explanations fit that equally: the remotes have no name, or they keep it
      // somewhere the scan does not reach. Reading the SAME offset that worked separates them --
      // a null there is the first answer, a pointer to something else is the second.
      entry.probe = [];
      // `+0x118` is where the scan found `Player_000100` on the local player, so it is the name
      // field rather than a guess. The neighbours are read beside it only to show that a null
      // there is a null field and not a mis-stepped pointer.
      for (const offset of [0x110, 0x118, 0x120]) {
        const slot = readPointer(chr.add(offset));
        let text = null;
        if (slot !== null && !slot.isNull()) {
          try {
            text = slot.readUtf16String(24);
          } catch (_) {
            text = null;
          }
        }
        entry.probe.push({
          at: offset,
          value: slot === null ? null : slot.toString(),
          text: text,
        });
      }
    } else {
      entry.verdict = 'an NPC -- not a PlayerCtrl at all';
    }
    roster.push(entry);
  }

  send({
    base: base.toString(),
    local: local === null ? null : local.toString(),
    localPosition: localAt,
    playerVtable: playerVtable.toString(),
    roster: roster,
  });
}

main();
