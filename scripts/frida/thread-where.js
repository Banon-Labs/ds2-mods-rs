// Say where one thread of the game is: its instruction pointer and a stack scan resolved to
// module+offset, once.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/thread-where.js \
//   --config-json '{"thread":328}'`
//
// Thread 328 is the game's main thread (Present, the nav update and the menus all run there). For
// a dialog that stops answering: is its thread in the dialog's message loop, or somewhere else?
// Read-only; Frida suspends the thread only to read its context.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const want = config.thread === undefined ? 328 : config.thread;

function name(address) {
  const m = Process.findModuleByAddress(address);
  return m ? m.name + '+0x' + address.sub(m.base).toString(16) : address.toString();
}

const thread = Process.enumerateThreads().find((t) => t.id === want);
if (config.thread === 'all') {
  // Every thread with a DarkSoulsII.exe frame in its top eight, for finding the game thread when
  // its id is not known in this process.
  for (const t of Process.enumerateThreads()) {
    const frames = [name(t.context.pc)].concat(
      Thread.backtrace(t.context, Backtracer.FUZZY).slice(0, 8).map(name));
    if (frames.some((f) => f.startsWith('DarkSoulsII.exe'))) {
      console.log('[thread-where] thread ' + t.id + ' state=' + t.state + ' ' + frames.join(' < '));
    }
  }
} else if (!thread) {
  console.log('[thread-where] no thread ' + want);
} else {
  console.log('[thread-where] thread ' + want + ' state=' + thread.state + ' pc=' +
    name(thread.context.pc));
  const frames = Thread.backtrace(thread.context, Backtracer.FUZZY).slice(0, 24).map(name);
  frames.forEach((f, i) => console.log('[thread-where]   #' + i + ' ' + f));
}
