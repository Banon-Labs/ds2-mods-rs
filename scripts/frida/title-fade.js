// Is the screen fade the title waits on laid out the way the static reading says?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/title-fade.js`
//
// `FeOperatorTitle::v4` (0x1400ef390) leaves its state 2 by starting a 2.0 s fade through
// 0x14039a4d0 and then, in state 3, waits until the fade object's remaining time reaches zero
// before it starts the title flow. The fade object is `[[GameManagerImp]+0x1160]`, read as
// `+0x00` current opacity, `+0x04` target opacity, `+0x08` remaining seconds. This reads those
// three floats once, at load, wherever the game is. In the world after a finished fade the
// remaining time should be 0 and current should equal target. It writes nothing.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const manager = image.base.add(0x16148f0).readPointer();
if (manager.isNull()) {
  console.log('[title-fade] GameManagerImp is null');
} else {
  const fade = manager.add(0x1160).readPointer();
  if (fade.isNull()) {
    console.log('[title-fade] manager=' + manager + ' fade object is null');
  } else {
    console.log('[title-fade] manager=' + manager + ' fade=' + fade +
      ' current=' + fade.readFloat() + ' target=' + fade.add(4).readFloat() +
      ' remaining=' + fade.add(8).readFloat() + ' +0x0c=' + fade.add(0xc).readFloat() +
      ' +0x10=' + fade.add(0x10).readFloat());
  }
}
