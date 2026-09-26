// Log PadDevice's key test (vtable slot 27, 0x140f04d40) whenever it answers "down", with the
// device, the key asked, and the device's +0x198 / +0x19c / +0x314 / +0x2f8 at that moment. Shows
// which object and field a harness `buttons` press actually reaches. Twenty seconds. Read-only.

'use strict';

const KEY_TEST = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f04d40);
const t0 = Date.now();
const seen = {};

const l = Interceptor.attach(KEY_TEST, {
  onEnter(args) { this.self = args[0]; this.key = args[1].toInt32(); },
  onLeave(ret) {
    if (Date.now() - t0 > 20000) { l.detach(); return; }
    if ((ret.toInt32() & 0xff) === 0) return;
    const s = this.self;
    const line = 'dev=' + s + ' key=' + this.key + ' 198=0x' + s.add(0x198).readU16().toString(16) +
      ' 19c=' + s.add(0x19c).readS32() + ' 314=' + s.add(0x314).readS32() + ' 2f8=0x' +
      s.add(0x2f8).readU32().toString(16);
    if (seen[line]) return;
    seen[line] = true;
    console.log('[key-test] t=' + (Date.now() - t0) + ' ' + line);
  },
});
console.log('[key-test] attached ' + KEY_TEST);
