// Apply one SpEffect to the local player the way the bonfire does, from the game thread, and
// report the player's HP around it.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/apply-speffect.js \
//   --config-json '{"id":110000010,"drop_hp":true}'`
//
// Chain (docs/DS2-SPEFFECT.md): [exe+0x16148f0] GameManagerImp -> +0xd0 PlayerCtrl -> +0x3e0
// ChrSpEffectCtrl. applySpEffect 0x14014bec0(ctrl, request) with the 0x10-byte request the bonfire
// builds: id, 1, -1.0f, slot 25, 1, 0, 0. HP is the i32 at PlayerCtrl +0x168 (max +0x170),
// measured live (2461 on the test character); PlayerParam+0 points back at PlayerCtrl, as
// the HP-cap function 0x14038be90 reads them. With drop_hp the current HP is first set to a tenth
// of max, so a healing effect (110000010, bonfire rest) shows as HP coming back. The call is made
// once, from inside NvNavigationSystem::Update (0x140baeb20), which runs on the game thread.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const id = config.id === undefined ? 110000010 : config.id;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const apply = new NativeFunction(exe.add(0x0014bec0), 'pointer', ['pointer', 'pointer']);
const NAV = exe.add(0x00baeb20);

function player() {
  const gmi = exe.add(0x016148f0).readPointer();
  if (gmi.isNull()) return null;
  const pc = gmi.add(0xd0).readPointer();
  return pc.isNull() ? null : pc;
}

const request = Memory.alloc(0x10);
let stage = 0;
let t0 = 0;
const listener = Interceptor.attach(NAV, {
  onEnter() {
    const pc = player();
    if (pc === null) return;
    const owner = pc;
    const hp = () => owner.add(0x168).readS32() + '/' + owner.add(0x170).readS32();
    if (stage === 0) {
      console.log('[apply-speffect] player=' + pc + ' hp=' + hp());
      if (config.drop_hp) {
        owner.add(0x168).writeS32(Math.max(1, Math.floor(owner.add(0x170).readS32() / 10)));
        console.log('[apply-speffect] hp dropped to ' + hp());
      }
      stage = 1;
      t0 = Date.now();
    } else if (stage === 1 && Date.now() - t0 > 1000) {
      const ctrl = pc.add(0x3e0).readPointer();
      if (ctrl.isNull()) {
        console.log('[apply-speffect] no ChrSpEffectCtrl');
        stage = 3;
        return;
      }
      request.writeS32(id);
      request.add(4).writeS32(1);
      request.add(8).writeFloat(-1.0);
      request.add(0xc).writeU8(25);
      request.add(0xd).writeU8(1);
      request.add(0xe).writeU8(0);
      request.add(0xf).writeU8(0);
      const before = hp();
      const ret = apply(ctrl, request);
      console.log('[apply-speffect] applied id=' + id + ' ctrl=' + ctrl + ' ret=' + ret +
        ' hp before=' + before + ' right after=' + hp());
      stage = 2;
      t0 = Date.now();
    } else if (stage === 2) {
      const age = Date.now() - t0;
      if (age % 500 < 20) console.log('[apply-speffect] +' + age + 'ms hp=' + hp());
      if (age > 4000) stage = 3;
    } else if (stage === 3) {
      console.log('[apply-speffect] done hp=' + hp());
      stage = 4;
      setTimeout(() => listener.detach(), 0);
    }
  },
});
console.log('[apply-speffect] attached, id=' + id);
