// Does [GameManagerImp]+0x104 advance once per frame, and what does the game pass to 0x140505ce0?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/frame-counter.js`
//
// Two questions, both read-only:
//
// 1. The frame counter found statically (docs/DS2-FRAME-COUNTER.md): `inc dword [rcx+0x104]` once
//    per call of GameManagerImp's per-frame update. Sampled once a second with the wall clock, so
//    the rate it prints is frames per second if the reading is right, and anything else if not.
//    The GameManagerImp pointer is printed each time too, so a replaced object would show.
// 2. `0x140505ce0`, FeGroupBase::v1 -- `mov rcx,[rcx+8]; test; xorps xmm3,xmm3; jmp 0x140afdb80`
//    -- which the title-flow notes say is the bare "play sequence id on a group" that
//    ds2-dialog-skip meant to call. This logs the first calls the game itself makes: the group,
//    the sequence id in edx and the pose in r8, and the return address, so the argument order is
//    measured rather than assumed. Capped, because it may be called every frame.

'use strict';

const base = Process.getModuleByName('DarkSoulsII.exe').base;
const GAME_MANAGER_IMP = base.add(0x016148f0);
const COUNTER_OFFSET = 0x104;
const GROUP_PLAY = base.add(0x00505ce0);

function readCounter() {
  try {
    const gmi = GAME_MANAGER_IMP.readPointer();
    if (gmi.isNull()) {
      return { gmi: gmi, count: null };
    }
    return { gmi: gmi, count: gmi.add(COUNTER_OFFSET).readU32() };
  } catch (e) {
    return { gmi: null, count: null };
  }
}

let last = readCounter();
let lastAt = Date.now();
console.log('[frame] load gmi=' + last.gmi + ' count=' + last.count);
setInterval(function () {
  const now = readCounter();
  const at = Date.now();
  let rate = 'n/a';
  if (now.count !== null && last.count !== null) {
    rate = ((now.count - last.count) * 1000 / (at - lastAt)).toFixed(1);
  }
  console.log('[frame] gmi=' + now.gmi + ' count=' + now.count + ' per_second=' + rate);
  last = now;
  lastAt = at;
}, 1000);

let calls = 0;
Interceptor.attach(GROUP_PLAY, {
  onEnter(args) {
    calls += 1;
    if (calls > 12) {
      return;
    }
    const group = this.context.rcx;
    const id = this.context.rdx.and(0xffffffff);
    const pose = this.context.r8;
    let inner = 'unreadable';
    try {
      inner = group.add(8).readPointer();
    } catch (e) {}
    const ret = this.returnAddress;
    console.log(
      '[group-play] call=' + calls + ' group=' + group + ' [group+8]=' + inner + ' id=0x' +
        id.toString(16) + ' pose=' + pose + ' ret_rva=0x' + ret.sub(base).toString(16)
    );
  },
});
console.log('[group-play] hooked 0x140505ce0');
