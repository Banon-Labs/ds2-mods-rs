// Break FUN_1402e47f0's header walk out of the spin on a hollow slot (bd ds2-mods-rs-t46i).
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/load-hang-escape.js`
//
// The walk reads 0x20-byte headers until type 0xD. On an all-zero stream it reaches the end, where
// the stream read (0x14084a8e0) returns -1 and leaves the header as it was, so the walk repeats
// forever. This hooks that read: when a call made from inside the walk comes back -1, it writes
// type 0xFF into the header buffer, and the walk's own `0xe < type` branch returns. Every other
// caller of the read is left alone. Writes only that one header, only on a failed read.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const READ = exe.add(0x84a8e0);
const WALK_LO = exe.add(0x2e47f0);
const WALK_HI = exe.add(0x2e4940);

let fired = 0;
Interceptor.attach(READ, {
  onEnter(args) {
    const ret = this.returnAddress;
    this.inWalk = ret.compare(WALK_LO) >= 0 && ret.compare(WALK_HI) < 0;
    this.buf = args[1];
    this.len = args[2].toUInt32();
  },
  onLeave(retval) {
    if (!this.inWalk || this.len !== 0x20 || retval.toInt32() !== -1) return;
    const was = this.buf.readU32();
    this.buf.writeU32(0xff);
    fired += 1;
    if (fired <= 5) {
      console.log('[load-hang-escape] failed header read in the walk: type ' + was +
        ' -> 0xff, walk exits (fire ' + fired + ')');
    }
  },
});
console.log('[load-hang-escape] armed on read exe+0x84a8e0 for callers in exe+0x2e47f0..0x2e4940');
