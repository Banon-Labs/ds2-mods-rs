// Does `KatanaSfxSystem`'s stop call actually put a Prism Stone out?
//
// THE QUESTION. `ds2-invasion-path` now tears its whole trail down whenever the route moves off
// it, and each teardown calls `Handle::extinguish` -> `KATANA_SFX_STOP` (RVA 0x00141530) on every
// stone. A live session logged four teardowns of 32, 29, 21 and 14 stones while the player saw a
// growing pile of stones in one place. Two readings fit that exactly and they lead opposite ways:
//
//   - the stop works, and the pile is the trail being re-laid on the same ground -- a routing bug;
//   - the stop is a no-op, and the pile is ~96 orphaned effects nobody put out -- a teardown bug,
//     introduced by the teardown.
//
// HOW IT IS SETTLED, without believing either. The control block carries two `0x30`-byte halves,
// each with an effect-node pointer at `+0x10`. A node is playing while bit 30 of `node + 0x58` is
// set. This reads both halves' nodes and both alive bits IMMEDIATELY BEFORE the engine's own stop
// runs and IMMEDIATELY AFTER it returns. A bit that is set before and clear after is the stop
// working. A bit set on both sides is the stop not stopping anything.
//
// The node pointer is captured on entry and re-read on exit deliberately: if the stop's effect is
// to UNLINK rather than to clear the bit, the pointer goes null and the bit is unreadable, and
// those are different answers that must not collapse into one.
//
// NO TIMER. Frida's JS runtime in this process can wedge permanently (`frida-js-thread-wedges-in-
// ds2`), so there is no heartbeat here to misread. One hook, a hard cap on how many calls it
// reports, and a single message when the cap is reached.

'use strict';

Interceptor.detachAll();

const KATANA_SFX_STOP = 0x00141530; // RVA; ds2_rva::KATANA_SFX_STOP
const CTRL_HALF_BYTES = 0x30; // the block is two halves
const CTRL_NODE_OFFSET = 0x10; // half -> effect node
const NODE_ALIVE_OFFSET = 0x58; // node -> flags
const NODE_ALIVE_BIT = 0x40000000; // bit 30: still playing

//: Enough calls to see a whole teardown and then some; low enough that the JS thread cannot be
//: starved by string building even if the engine stops effects of its own at a high rate.
const SAMPLE_LIMIT = 40;
//: How much of the effect node to photograph either side of the call. 0x80 covers the flags word
//: and the fields around it without turning each sample into a kilobyte of hex.
const NODE_WINDOW = 0x80;

function hex(bytes) {
  const view = new Uint8Array(bytes);
  let out = '';
  for (let i = 0; i < view.length; i += 1) {
    out += (view[i] < 16 ? '0' : '') + view[i].toString(16);
  }
  return out;
}

let seen = 0;
let reported = false;
const samples = [];

function nodesOf(block) {
  const out = [];
  for (const half of [0, CTRL_HALF_BYTES]) {
    let node = null;
    try {
      node = block.add(half + CTRL_NODE_OFFSET).readPointer();
    } catch (_) {
      node = null;
    }
    if (node === null || node.isNull()) {
      out.push({ node: null, alive: null });
      continue;
    }
    let flags = null;
    try {
      flags = node.add(NODE_ALIVE_OFFSET).readU32();
    } catch (_) {
      flags = null;
    }
    out.push({
      node: node.toString(),
      flags: flags === null ? null : '0x' + (flags >>> 0).toString(16),
      alive: flags === null ? null : (flags & NODE_ALIVE_BIT) !== 0,
    });
  }
  return out;
}

const module = Process.findModuleByName('DarkSoulsII.exe');
if (module === null) {
  send({ error: 'DarkSoulsII.exe is not loaded' });
} else {
  const stop = module.base.add(KATANA_SFX_STOP);
  console.log('[stop-works] hooking KATANA_SFX_STOP at ' + stop);
  Interceptor.attach(stop, {
    onEnter: function (args) {
      // A plain integer gate FIRST, before touching args at all -- materialising a NativePointer
      // per call is what starved the JS thread in four earlier agents.
      if (seen >= SAMPLE_LIMIT) {
        this.skip = true;
        return;
      }
      this.skip = false;
      this.block = args[0];
      this.before = nodesOf(this.block);
      // The node addresses themselves, kept, because the block's copies are about to be cleared
      // and an address that has been erased cannot be re-read.
      const block = this.block;
      this.nodes = [0, CTRL_HALF_BYTES].map(function (half) {
        try {
          const node = block.add(half + CTRL_NODE_OFFSET).readPointer();
          return node.isNull() ? null : node;
        } catch (_) {
          return null;
        }
      });
      this.nodeBefore = this.nodes.map(function (node) {
        if (node === null) {
          return null;
        }
        try {
          return { at: node.toString(), bytes: hex(node.readByteArray(NODE_WINDOW)) };
        } catch (_) {
          return { at: node.toString(), bytes: null };
        }
      });
    },
    onLeave: function () {
      if (this.skip) {
        return;
      }
      seen += 1;
      const after = nodesOf(this.block);
      // THE BLOCK GOING NULL IS NOT THE EFFECT STOPPING, and conflating the two is how a
      // teardown that only detaches gets recorded as a teardown that works. `0x140141530` is
      // three calls per half and every one of them is passed the BLOCK -- nothing in its own
      // body touches the node. So the node captured on entry is re-read here at its own address,
      // and its bytes are compared: an effect the engine really stopped has something different
      // in it, and one that was merely detached is byte-for-byte what it was.
      const nodeAfter = this.nodes.map(function (node) {
        if (node === null) {
          return null;
        }
        try {
          return { at: node.toString(), bytes: hex(node.readByteArray(NODE_WINDOW)) };
        } catch (_) {
          return { at: node.toString(), bytes: null };
        }
      });
      samples.push({
        call: seen,
        block: this.block.toString(),
        before: this.before,
        after: after,
        nodeBefore: this.nodeBefore,
        nodeAfter: nodeAfter,
      });
      if (seen >= SAMPLE_LIMIT && !reported) {
        reported = true;
        send({ samples: samples });
      }
    },
  });
  console.log('[stop-works] armed; reporting after ' + SAMPLE_LIMIT + ' stop call(s).');
}

// Report whatever has been collected when the host asks, so a run that never reaches the cap
// still produces an answer rather than silence.
recv('drain', function () {
  send({ samples: samples, partial: seen < SAMPLE_LIMIT, seen: seen });
});
