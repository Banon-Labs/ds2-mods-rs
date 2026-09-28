// Which arm does the player's pad take, and does XInputGetState see it? Read-only.
//
// Answers the question the Build Recommender panel's controller support rests on: the panel reads
// the pad around the game (XInputGetState on pad 0, like ds2-save-file's picker), so it only works
// if the player's pad is an XInput pad at user index 0.
//
// For 240 PadDevice polls (0x140f05540) it records every distinct `this` with its XInput port
// (+0x19c, negative = no XInput pad) and third-backend index (+0x314, >= 0 = the HID-shaped arm),
// then detaches. For five seconds it also counts XInputGetState calls by user index and return code
// (0 = connected, 0x48f = ERROR_DEVICE_NOT_CONNECTED), and reports the last button word per index.

'use strict';

const POLL = Process.getModuleByName('DarkSoulsII.exe').base.add(0x00f05540);
const POLLS = 240;
const XINPUT_MS = 5000;

const pads = {};
let polls = 0;
const pollHook = Interceptor.attach(POLL, {
  onEnter(args) {
    polls += 1;
    const pad = args[0];
    const key = pad.toString();
    if (!pads[key]) {
      pads[key] = {
        xinput_port: pad.add(0x19c).readS32(),
        third_backend: pad.add(0x314).readS32(),
        polls: 0,
      };
    }
    pads[key].polls += 1;
    if (polls >= POLLS) {
      pollHook.detach();
      send({ kind: 'pad-devices', polls, pads });
    }
  },
});

const xinput = Process.findModuleByName('XINPUT1_3.dll');
if (xinput === null) {
  send({ kind: 'xinput', error: 'XINPUT1_3.dll not loaded' });
} else {
  const getState = xinput.getExportByName('XInputGetState');
  const calls = {};
  const hook = Interceptor.attach(getState, {
    onEnter(args) {
      this.index = args[0].toUInt32();
      this.state = args[1];
    },
    onLeave(ret) {
      const key = 'user' + this.index + ':ret=0x' + ret.toUInt32().toString(16);
      const entry = calls[key] || { calls: 0, buttons: 0 };
      entry.calls += 1;
      if (ret.toUInt32() === 0) entry.buttons = this.state.add(4).readU16();
      calls[key] = entry;
    },
  });
  setTimeout(() => {
    hook.detach();
    send({ kind: 'xinput', module: xinput.path, calls });
  }, XINPUT_MS);
}
