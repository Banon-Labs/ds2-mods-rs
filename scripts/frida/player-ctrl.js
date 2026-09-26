// Are the local PlayerCtrl's fields where `darksouls2::game::chr` says they are?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/player-ctrl.js`
//
// Reads once, at load, and writes nothing. The walk is the one `ds2-build-import` takes to the
// covenant setter's receiver: `[[GameManagerImp] + 0xd0]`. From there it reads the fields the
// crates in this repo read through `ds2-rva`:
//
//   +0x00   vtable, compared with PlayerCtrl's (0x010e4bb8) and CharacterCtrl's (0x010df218)
//   +0x90   position, four f32               (CharacterCtrl)
//   +0xb0   phantom block pointer, its +0x3c  (CharacterCtrl)
//   +0x118  name, an MSVC std::wstring        (CharacterCtrl)
//   +0x378  ChrAsmCtrl pointer                (CharacterCtrl)
//           its +0x28 equip pointer, and that object's +0x10 i32 grip state
//
// A `null` answer at any hop is printed rather than thrown, so the title screen reports where the
// chain stops instead of ending the session with nothing.

'use strict';

const PLAYER_CTRL_VTABLE = 0x010e4bb8;
const CHARACTER_CTRL_VTABLE = 0x010df218;

const image = Process.getModuleByName('DarkSoulsII.exe');
const say = (text) => console.log('[player-ctrl] ' + text);

function readWString(at) {
  const len = at.add(0x10).readU64().toNumber();
  const cap = at.add(0x18).readU64().toNumber();
  const chars = cap > 7 ? at.readPointer() : at;
  return { len, cap, text: len > 0 && len < 256 ? chars.readUtf16String(len) : '' };
}

const manager = image.base.add(0x16148f0).readPointer();
if (manager.isNull()) {
  say('GameManagerImp is null');
} else {
  const player = manager.add(0xd0).readPointer();
  if (player.isNull()) {
    say('manager=' + manager + ' PlayerCtrl (+0xd0) is null');
  } else {
    const vtable = player.readPointer();
    const kind = vtable.equals(image.base.add(PLAYER_CTRL_VTABLE))
      ? 'PlayerCtrl'
      : vtable.equals(image.base.add(CHARACTER_CTRL_VTABLE))
        ? 'CharacterCtrl'
        : 'unknown';
    say('manager=' + manager + ' player=' + player + ' vtable=' + vtable + ' rva=0x' +
      vtable.sub(image.base).toString(16) + ' (' + kind + ')');
    const p = player.add(0x90);
    say('position +0x90 = ' + [0, 4, 8, 12].map((o) => p.add(o).readFloat().toFixed(3)).join(' '));
    const phantom = player.add(0xb0).readPointer();
    say('phantom block +0xb0 = ' + phantom +
      (phantom.isNull() ? '' : ' phantom param id +0x3c = 0x' + phantom.add(0x3c).readU8().toString(16)));
    const name = readWString(player.add(0x118));
    say('name +0x118 = "' + name.text + '" len=' + name.len + ' cap=' + name.cap);
    const asm = player.add(0x378).readPointer();
    say('chr asm ctrl +0x378 = ' + asm +
      (asm.isNull() ? '' : ' vtable rva=0x' + asm.readPointer().sub(image.base).toString(16)));
    if (!asm.isNull()) {
      const equip = asm.add(0x28).readPointer();
      say('equip +0x28 = ' + equip +
        (equip.isNull() ? '' : ' grip +0x10 = ' + equip.add(0x10).readS32()));
    }
  }
}
