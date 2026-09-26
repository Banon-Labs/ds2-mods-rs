// Print the local player's PlayerCtrl, its PlayerParam and the pointer at PlayerParam+0, with the
// floats at +0x168/+0x16c/+0x170 and byte +0x54 of each -- the fields the HP-cap function
// 0x14038be90 reads through that first pointer. Read-only.
'use strict';
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const gmi = exe.add(0x016148f0).readPointer();
const pc = gmi.add(0xd0).readPointer();
const param = pc.add(0x490).readPointer();
const first = param.readPointer();
function show(name, p) {
  let s = '[hp-probe] ' + name + '=' + p;
  try {
    s += ' b54=' + p.add(0x54).readU8() + ' f168=' + p.add(0x168).readFloat() + ' f16c=' +
      p.add(0x16c).readFloat() + ' f170=' + p.add(0x170).readFloat() + ' i168=' + p.add(0x168).readS32();
  } catch (e) { s += ' unreadable'; }
  console.log(s);
}
show('playerctrl', pc);
show('param', param);
show('param[0]', first);
