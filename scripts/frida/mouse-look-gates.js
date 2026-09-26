// Are the gates on DARK SOULS II's mouse-look open, and what is the camera being fed?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/mouse-look-gates.js`
//
// From the static reading in docs/DS2-MOUSE-LOOK.md: the camera's input stage (FUN_14049cbf0)
// reads only `cursorObj+0x18/+0x1c`, where `cursorObj = [[BaseP1+0x60]+8]` and BaseP1 is the
// KatanaMainApp at `[0x1416751f8]`. Those two floats are filled from the DirectInput mouse device
// at `cursorObj+0xf0`, from its `+0x108/+0x10c`, and only while `BaseP1+0x30` (window active),
// `BaseP1+0x133` (keyboard/mouse enabled) and the device's `+0xe8` (its DirectInput interface)
// are all non-zero. This prints every one of those once, at load. It writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');

function readPtr(p) {
  try {
    return p.readPointer();
  } catch (e) {
    return NULL;
  }
}

const app = readPtr(image.base.add(0x16751f8));
if (app.isNull()) {
  console.log('[mouse-look-gates] KatanaMainApp is null');
} else {
  const input = readPtr(app.add(0x60));
  const cursor = input.isNull() ? NULL : readPtr(input.add(8));
  const mouse = cursor.isNull() ? NULL : readPtr(cursor.add(0xf0));
  let line = 'app=' + app + ' app+0x30=' + app.add(0x30).readU8() + ' app+0x133=' + app.add(0x133).readU8();
  if (!cursor.isNull()) {
    line += ' cursor=' + cursor + ' cursor+0x18=' + cursor.add(0x18).readFloat() +
      ' cursor+0x1c=' + cursor.add(0x1c).readFloat();
  }
  if (!mouse.isNull()) {
    line += ' mouse=' + mouse + ' mouse+0xe8=' + readPtr(mouse.add(0xe8)) +
      ' mouse+0x108=' + mouse.add(0x108).readFloat() + ' mouse+0x10c=' + mouse.add(0x10c).readFloat();
  } else {
    line += ' mouse=null';
  }
  console.log('[mouse-look-gates] ' + line);
}
