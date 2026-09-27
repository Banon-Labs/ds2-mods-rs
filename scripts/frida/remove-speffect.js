// Remove one SpEffect from the local player with the game's own removeSpEffect, on the game
// thread, and report the action list around the call.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/remove-speffect.js \
//   --pid <windows pid> --config-json '{"id":140001010,"times":2}'`
//
// removeSpEffect is 0x14014c0e0(ChrSpEffectCtrl*, i32 id) -> bool, read statically:
//   entry `e9` -> Arxan stub 0x141b898c6 `mov rcx,[rcx+0x10]` -> 0x14022f7c0(worker, id), which
//   checks the id (0x14022fb20), removes every action with that id through 0x140230370
//   ([worker+0x30], id, 0) -> 0x1402204c0 (filter mode 2: action slot 0x40 == id), and, when
//   something was removed, flushes the removal notices with 0x1402277c0([worker+0x10]).
//   The game's callers (0x140462b90, 0x140426730) reach the controller exactly as applySpEffect's
//   callers do: character vtable slot 0x130, then call it with the id in edx.
//
// It needs the game thread, and this agent has no frame clock of its own, so it borrows the one
// call our DLL makes on that thread: ds2-net-effects' re-apply of `id` through applySpEffect
// (0x14014bec0). On leaving that call it reads the list, calls removeSpEffect, reads it again.
// No other hook, and it detaches after `times` removals.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const id = config.id === undefined ? 140001010 : config.id;
const times = config.times || 2;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const APPLY = exe.add(0x0014bec0);
const remove = new NativeFunction(exe.add(0x0014c0e0), 'uint8', ['pointer', 'int']);
const say = (text) => console.log('[remove-speffect] ' + text);

function ctrlOfPlayer() {
  const gmi = exe.add(0x016148f0).readPointer();
  if (gmi.isNull()) return null;
  const pc = gmi.add(0xd0).readPointer();
  if (pc.isNull()) return null;
  const ctrl = pc.add(0x3e0).readPointer();
  return ctrl.isNull() ? null : ctrl;
}

function ids(ctrl) {
  const list = ctrl.add(0x10).readPointer().add(0x20).readPointer();
  const count = list.add(0x418).readU64().toNumber();
  const out = [];
  for (let i = 0; i < Math.min(count, 128); i++) {
    const a = list.add(0x10 + i * 8).readPointer();
    if (!a.isNull()) out.push(a.add(0x10).readS32());
  }
  return out;
}

let done = 0;
const listener = Interceptor.attach(APPLY, {
  onEnter(args) {
    this.mine = args[1].readS32() === id;
    this.ctrl = args[0];
  },
  onLeave() {
    if (!this.mine || done >= times) return;
    const ctrl = ctrlOfPlayer();
    if (ctrl === null || !ctrl.equals(this.ctrl)) {
      say('apply of ' + id + ' was not on the local player controller, skipped');
      return;
    }
    const before = ids(ctrl);
    const ret = remove(ctrl, id);
    const after = ids(ctrl);
    done += 1;
    const n = (l) => l.filter((x) => x === id).length;
    const line = 'removal #' + done + ' id=' + id + ' ctrl=' + ctrl + ' ret=' + ret +
      ' actions-with-id before=' + n(before) + ' after=' + n(after) +
      ' list before=' + before.length + ' after=' + after.length;
    say(line);
    send({ event: 'removed', done, id, ret, before: n(before), after: n(after), listBefore: before.length, listAfter: after.length });
    if (done >= times) setTimeout(() => { listener.detach(); say('detached'); send({ event: 'detached' }); }, 0);
  },
});
say('attached, waiting for the re-apply of ' + id);
send({ event: 'attached', id });
