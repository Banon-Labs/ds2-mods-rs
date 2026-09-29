// Who closes each frontend group, and from where? Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/group-close-trace.js`
//
// Hooks FeGroupClose (ds2-rva FE_GROUP_CLOSE, 0x1400f18b0) and FeGroupOpen (0x1400f1cb0), and
// logs each call: the group's vtable RVA, whether it was open ([group+0x30]), and a fuzzy
// backtrace of DarkSoulsII.exe frames as RVAs. The loading screen's close names the code that
// decided the load was done.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe');
const inExe = (p) => p.compare(exe.base) >= 0 && p.compare(exe.base.add(exe.size)) < 0;
const rva = (p) => '0x' + p.sub(exe.base).toString(16);

for (const [name, off] of [['close', 0xf18b0], ['open', 0xf1cb0]]) {
  Interceptor.attach(exe.base.add(off), {
    onEnter(args) {
      const g = args[0];
      let vt = '?';
      let open = '?';
      try { vt = rva(g.readPointer()); open = g.add(0x30).readU8(); } catch (e) { /* unreadable */ }
      const bt = Thread.backtrace(this.context, Backtracer.FUZZY).filter(inExe).slice(0, 12).map(rva).join(' ');
      console.log('[group] ' + name + ' group=' + g + ' vtable=' + vt + ' open=' + open + ' bt=' + bt);
    },
  });
}
console.log('[group] tracing');
