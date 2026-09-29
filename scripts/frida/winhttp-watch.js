// Log every WinHTTP connection and request the game process makes: host, port, verb, path, the
// status code, and how many body bytes were read. Read-only.
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/winhttp-watch.js`
//
// For `ds2-build-import`'s fetch (`ds2_game_base::http::get`), which is WinHTTP underneath: it
// shows whether a submitted link reached the network at all, which host it asked, and what came
// back -- none of which the loader log says past "fetching build N". `winhttp.dll` is loaded on the
// first fetch, not at startup, so the hooks go in when the module appears if it is not there yet.

'use strict';

const WINHTTP_QUERY_STATUS_CODE = 19;
const WINHTTP_QUERY_FLAG_NUMBER = 0x20000000;
const readBytes = new Map();
let hooked = false;

function hook(module) {
  if (hooked) return;
  hooked = true;
  const at = (name) => module.getExportByName(name);
  const queryHeaders = new NativeFunction(at('WinHttpQueryHeaders'), 'int',
    ['pointer', 'uint', 'pointer', 'pointer', 'pointer', 'pointer']);

  Interceptor.attach(at('WinHttpConnect'), {
    onEnter(args) {
      this.host = args[1].isNull() ? '' : args[1].readUtf16String();
      this.port = args[2].toInt32() & 0xffff;
    },
    onLeave(ret) {
      console.log('[winhttp] connect host=' + this.host + ' port=' + this.port + ' handle=' + ret);
    },
  });
  Interceptor.attach(at('WinHttpOpenRequest'), {
    onEnter(args) {
      this.verb = args[1].isNull() ? 'GET' : args[1].readUtf16String();
      this.path = args[2].isNull() ? '/' : args[2].readUtf16String();
    },
    onLeave(ret) {
      readBytes.set(ret.toString(), 0);
      console.log('[winhttp] request ' + this.verb + ' ' + this.path + ' handle=' + ret);
    },
  });
  Interceptor.attach(at('WinHttpReceiveResponse'), {
    onEnter(args) { this.request = args[0]; },
    onLeave(ret) {
      const status = Memory.alloc(4);
      const size = Memory.alloc(4);
      size.writeU32(4);
      const ok = queryHeaders(this.request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        ptr(0), status, size, ptr(0));
      console.log('[winhttp] response ok=' + ret.toInt32() + ' status=' +
        (ok ? status.readU32() : 'unreadable') + ' handle=' + this.request);
    },
  });
  Interceptor.attach(at('WinHttpReadData'), {
    onEnter(args) { this.request = args[0].toString(); this.read = args[3]; },
    onLeave(ret) {
      if (!ret.toInt32() || this.read.isNull()) return;
      const got = this.read.readU32();
      const total = (readBytes.get(this.request) || 0) + got;
      readBytes.set(this.request, total);
      if (got === 0) console.log('[winhttp] body done bytes=' + total + ' handle=' + this.request);
    },
  });
  console.log('[winhttp] hooked ' + module.name + ' at ' + module.base);
}

const loaded = Process.findModuleByName('winhttp.dll');
if (loaded) {
  hook(loaded);
} else {
  console.log('[winhttp] winhttp.dll not loaded yet -- hooking when it appears');
  Process.attachModuleObserver({
    onAdded(module) {
      if (module.name.toLowerCase() === 'winhttp.dll') hook(module);
    },
  });
}
