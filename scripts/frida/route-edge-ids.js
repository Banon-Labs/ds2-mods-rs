// Read a finished route's segment ids against its NvNaviGraph's triangle and edge tables, once.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/route-edge-ids.js`
// with PLANNER set to the `route planner 0x... linked` address a self-check run logged.
//
// Question (ds2-mods-rs-4yd): is a route id with bit 15 an edge index into `graph+0x58`
// (stride 0x10, i16 triangles at +0xc/+0xe), and does indexing `graph+0x48` with it read past
// the per-triangle attribute array? Prints, per id: the graph's triangle and edge counts, what
// the crate reads today, and the attribute word of each triangle the edge record names.

'use strict';

const PLANNER = ptr('0x7fffe833b6f0');
const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);

const manager = at(0x16148f0).readPointer();
const worldFrom = new NativeFunction(at(0x39a9f0), 'pointer', ['pointer']);
const graphFor = new NativeFunction(at(0xbb2620), 'pointer', ['pointer', 'uint32']);
const world = worldFrom(manager);
const table = world.add(0x88).readPointer();

const route = PLANNER.add(0x48);
const segments = route.add(0x10).readPointer();
const count = route.add(0x18).readS32();
console.log('[route-edge-ids] world=' + world + ' table=' + table + ' segments=' + count);

const hex = (v) => '0x' + (v >>> 0).toString(16).padStart(8, '0');
for (let i = 0; i < count; i++) {
  const segment = segments.add(i * 0x60);
  for (const off of [0x8, 0xc]) {
    const id = segment.add(off).readU32();
    if (id === 0xffffffff || (id & 0x7fff) === 0x7fff) continue;
    const graph = graphFor(table, (id | 0x1ffff) >>> 0);
    if (graph.isNull()) {
      console.log('[route-edge-ids] seg ' + i + ' id ' + hex(id) + ' no graph');
      continue;
    }
    const tris = graph.add(0x2c).readS16();
    const edges = graph.add(0x2e).readS16();
    const attrs = graph.add(0x48).readPointer();
    const index = id & 0x7fff;
    const edgeFlag = (id & 0x8000) !== 0;
    let line = 'seg ' + i + ' id ' + hex(id) + ' edge=' + edgeFlag + ' index=' + index +
      ' tris=' + tris + ' edges=' + edges + ' crate-reads ' + hex(attrs.add(index * 4).readU32());
    if (edgeFlag && index < edges) {
      const rec = graph.add(0x58).readPointer().add(index * 0x10);
      for (const side of [0xc, 0xe]) {
        const tri = rec.add(side).readS16();
        line += ' side+' + side.toString(16) + ' tri=' + tri;
        if (tri >= 0 && tri !== 0x7fff && tri < tris) {
          line += ' attrs=' + hex(attrs.add(tri * 4).readU32());
        }
      }
    }
    console.log('[route-edge-ids] ' + line);
  }
}
