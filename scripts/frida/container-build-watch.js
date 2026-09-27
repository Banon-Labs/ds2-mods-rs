// Which `.flo` containers go through the container builder `FUN_140b50f20`, and from where.
// For ds2-item-warn: the badge is only added to infusion containers built through that function.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/container-build-watch.js \
//   [--config-json '{"open_bonfire":true}']`
//
// Read-only apart from the optional bonfire open, which calls the game's own `bonfireWindow`
// (0x140198680) once from the game thread exactly as scripts/frida/open-bonfire-menu.js does.
// Logs every definition with nine children whose first record id is 0x5f5c3e9 (the infusion
// container) and, when ds2-item-warn substituted it, the ten-child replacement it passes on.
'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (t) => {
  console.log('[container-build] ' + t);
  send({ container_build: t });
};

let calls = 0;
let lines = 0;
Interceptor.attach(at(0x00b50f20), {
  onEnter(args) {
    calls++;
    const def = args[2];
    if (def.isNull()) return;
    let count, first;
    try {
      count = def.add(2).readU16();
      const records = def.add(8).readPointer();
      if (records.isNull() || count === 0) return;
      first = records.add(0x1c).readU32(); // FLO_RECORD_ID_OFFSET
    } catch (e) {
      return;
    }
    if (config.all === true && calls <= 150) {
      say('call ' + calls + ' def=' + def + ' key=0x' + def.readU16().toString(16) + ' children=' +
          count + ' first-id=0x' + first.toString(16) + ' doc=' + args[1] +
          ' ret=+0x' + this.returnAddress.sub(exe).toString(16));
    }
    if (first !== 0x5f5c3e9) return;
    if (lines++ >= 60) return;
    say('container def=' + def + ' children=' + count + ' doc=' + args[1] + ' parent=' + args[3] +
        ' ret=DarkSoulsII.exe+0x' + this.returnAddress.sub(exe).toString(16));
  },
});

setInterval(() => say('builder calls so far=' + calls + ' infusion containers=' + lines), 10000);

if (config.open_bonfire === true) {
  const bonfireWindow = new NativeFunction(at(0x00198680), 'void', ['pointer', 'uint16']);
  let done = false;
  const hook = Interceptor.attach(at(0x00baeb20), {
    onEnter() {
      if (done) return;
      done = true;
      const gmi = at(0x016148f0).readPointer();
      const events = gmi.add(0x70).readPointer();
      const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
      if (wm.isNull() || gmi.add(0x22e0).readPointer().isNull()) {
        say('bonfire not opened: event window manager or frontend root is null');
      } else {
        bonfireWindow(wm, 0);
        say('bonfireWindow(' + wm + ', 0) called');
      }
      setTimeout(() => hook.detach(), 0);
    },
  });
}
say('armed');
