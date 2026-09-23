// Does a timer fire at all in this process right now?
//
// Kept because the question recurs and the answer is not guessable. The arrow agent loads,
// installs both D3D hooks, logs three lines, and then never emits its five-second heartbeat. Two
// explanations fit that exactly and they lead opposite ways: the JS thread is saturated by the
// hooks, or timer output is not reaching the watcher at all. This agent installs no hooks, so a
// `[tick] 1` means the plumbing is fine and the hooks are the problem, and silence means the
// plumbing is broken and every conclusion drawn from a missing heartbeat is worthless.

'use strict';

let ticks = 0;
console.log('[tick] loaded; nothing hooked.');
setInterval(function () {
  ticks += 1;
  console.log('[tick] ' + ticks);
}, 2000);
