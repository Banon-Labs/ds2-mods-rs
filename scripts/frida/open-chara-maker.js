// Open the full character creator (class, gift, appearance, name) wherever the player stands, by
// calling the game's own `openCharaMakerWindow` (0x1401986c0) once, on the game thread. It is the
// twin of open-name-window.js; confirming it may rewrite the current character, not only the name.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/open-chara-maker.js`
//
// Read 2026-09-26 (SOTFS 9527516), scripts/pe-disasm.py:
//
// * Talk-script command 130451 (0x1fd93) in EventEzStateCtrl's switch (0x140461f20) calls
//   openCharaMakerWindow; 130452 calls openNameWindow (0x140198f70). Both bodies are
//   `rcx = [GameManagerImp+0x22e0]; if (rcx) jmp <frontend body>`, ignoring their argument.
// * The chara-maker body (0x1404fffd0) is the name body (0x1405000c0) byte for byte except its
//   tail: it stores FeOperatorTestCharaMaking+0x28 = 0, so the operator's update (0x1400e3060)
//   pushes the full creator group (0x1400e35e0) instead of the name group (0x1400e3620).
//
// Called from inside the per-frame nav update (0x140baeb20), as open-name-window.js does, so it
// waits out a load and fires once the character is in. Seen on screen by the user 2026-09-26.
// PlayerGameData's first 0x400 bytes are diffed every 250 ms; walking alone churns +0x6c..+0xc4
// (position and facing), so read past those for what the creator wrote.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const openCharaMakerWindow = new NativeFunction(exe.add(0x001986c0), 'void', ['pointer']);
const WATCH_BYTES = 0x400;

function playerGameData(gmi) {
  const gdm = gmi.add(0xa8).readPointer();
  if (gdm.isNull()) return null;
  const pgd = gdm.add(0xc0).readPointer();
  return pgd.isNull() ? null : pgd;
}

let done = false;
const hook = Interceptor.attach(exe.add(0x00baeb20), {
  onEnter() {
    if (done) return;
    const gmi = exe.add(0x016148f0).readPointer();
    if (gmi.isNull()) return;
    const events = gmi.add(0x70).readPointer();
    const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
    const pgd = playerGameData(gmi);
    // Attached during a load: keep waiting on the nav update until the character is in.
    if (gmi.add(0x22e0).readPointer().isNull() || pgd === null) return;
    done = true;
    {
      console.log('[chara-maker] name before: ' + JSON.stringify(pgd.add(0x24).readUtf16String(0x20)));
      const before = new Uint32Array(pgd.readByteArray(WATCH_BYTES));
      openCharaMakerWindow(wm);
      console.log('[chara-maker] openCharaMakerWindow(' + wm + ') called on thread ' +
        Process.getCurrentThreadId());
      watch(pgd, before);
    }
    setTimeout(() => hook.detach(), 0);
  },
});

function watch(pgd, last) {
  setInterval(() => {
    const now = new Uint32Array(pgd.readByteArray(WATCH_BYTES));
    for (let i = 0; i < now.length; i++) {
      if (now[i] !== last[i]) {
        console.log('[chara-maker] pgd+0x' + (i * 4).toString(16) + ': 0x' +
          last[i].toString(16) + ' -> 0x' + now[i].toString(16));
      }
    }
    last = now;
  }, 250);
}

console.log('[chara-maker] armed');
