// Re-open the Fire Keepers' hut "name your character" screen wherever the player stands, by calling
// the game's own `openNameWindow` (0x140198f70) once, on the game thread. Confirming the name
// overwrites the current character's name; the next save writes it to disk.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/open-name-window.js`
//
// Read in Ghidra 2026-09-26 (SOTFS 9527516):
//
// * The hut's talk script reaches it through EventEzStateCtrl's command switch
//   (0x140461f20): command 130451 (0x1fd93) -> openCharaMakerWindow (0x1401986c0), the full
//   creator; command 130452 (0x1fd94) -> openNameWindow. Both are called as
//   `open*([[GameManagerImp+0x70]+0x50])`, the event window manager, which neither body reads.
// * openNameWindow is `rcx = [GameManagerImp+0x22e0]; if (rcx) jmp 0x1405000c0`. That body puts a
//   FeOperatorTestCharaMaking on the frontend root and sets operator+0x28 = 1; the operator's
//   update (0x1400e3060) then pushes only the name group (0x1400e3620) instead of the full
//   creator (0x1400e35e0, taken when +0x28 == 0).
// * FeGroupCreateNameEntry's confirm (0x1400ecd80) asks "is this OK", and its commit
//   (0x1400ed1e0) is a bounded wide copy of the entered string, 0x20 chars, into
//   [[GameManagerImp+0xa8]+0xc0]+0x24. Nothing else is written: class, stats and level are not
//   touched on this path.
//
// The call is made from inside the per-frame nav update (0x140baeb20), as open-bonfire-menu.js
// does, so it runs on the game thread rather than on Frida's.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const openNameWindow = new NativeFunction(exe.add(0x00198f70), 'void', ['pointer']);
const NAME_CHARS = 0x20;

function nameField(gmi) {
  const gdm = gmi.add(0xa8).readPointer();
  if (gdm.isNull()) return null;
  const pgd = gdm.add(0xc0).readPointer();
  return pgd.isNull() ? null : pgd.add(0x24);
}

function readName(gmi) {
  const field = nameField(gmi);
  return field === null ? null : field.readUtf16String(NAME_CHARS);
}

let done = false;
const hook = Interceptor.attach(exe.add(0x00baeb20), {
  onEnter() {
    if (done) return;
    done = true;
    const gmi = exe.add(0x016148f0).readPointer();
    const events = gmi.add(0x70).readPointer();
    const wm = events.isNull() ? ptr(0) : events.add(0x50).readPointer();
    if (gmi.add(0x22e0).readPointer().isNull() || nameField(gmi) === null) {
      console.log('[name-window] not opened: frontend root or player game data is null');
    } else {
      console.log('[name-window] current name: ' + JSON.stringify(readName(gmi)));
      openNameWindow(wm);
      console.log('[name-window] openNameWindow(' + wm + ') called on thread ' +
        Process.getCurrentThreadId());
      watchName(gmi, readName(gmi));
    }
    setTimeout(() => hook.detach(), 0);
  },
});

// Report the name the moment the commit lands, so a run proves the write without a screenshot.
function watchName(gmi, before) {
  const timer = setInterval(() => {
    const now = readName(gmi);
    if (now !== before) {
      console.log('[name-window] name changed: ' + JSON.stringify(before) + ' -> ' +
        JSON.stringify(now));
      clearInterval(timer);
    }
  }, 250);
}

console.log('[name-window] armed');
