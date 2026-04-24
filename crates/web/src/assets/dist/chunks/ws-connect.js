import { c3 as l, r as connected, c4 as localizeRpcError, c5 as pending, c6 as setConnected, c7 as nextId, c8 as getPreferredLocale, c9 as setReconnectDelay, ca as reconnectDelay, b as sendRpc, cb as setWs } from "./theme.js";
var f = 0;
function u(e, t, n, o, i, u2) {
  t || (t = {});
  var a, c, p = t;
  if ("ref" in p) for (c in p = {}, t) "ref" == c ? a = t[c] : p[c] = t[c];
  var l$1 = { type: e, props: p, key: n, ref: a, __k: null, __: null, __b: 0, __e: null, __c: null, constructor: void 0, __v: --f, __i: -1, __u: 0, __source: i, __self: u2 };
  if ("function" == typeof e && (a = e.defaultProps)) for (c in a) void 0 === p[c] && (p[c] = a[c]);
  return l.vnode && l.vnode(l$1), l$1;
}
let reconnectTimer = null;
let lastOpts = null;
let authRedirectPending = false;
const serverRequestHandlers = {};
function resolveLocale() {
  return getPreferredLocale();
}
function resetAuthRedirectGuard() {
  authRedirectPending = false;
}
window.addEventListener("moltis:auth-status-sync-complete", resetAuthRedirectGuard);
function onServerRequest(method, handler) {
  serverRequestHandlers[method] = handler;
  return function off() {
    delete serverRequestHandlers[method];
  };
}
function connectWs(opts) {
  lastOpts = opts;
  const backoff = Object.assign({ factor: 1.5, max: 5e3 }, opts.backoff);
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${proto}//${location.host}/ws/chat`);
  setWs(ws);
  ws.onopen = () => {
    const id = nextId();
    pending[id] = (res) => {
      if (res.ok && res.payload) {
        const hello = res.payload;
        if (hello.type === "hello-ok") {
          setConnected(true);
          setReconnectDelay(1e3);
          if (opts.onConnected) opts.onConnected(hello);
          return;
        }
      }
      setConnected(false);
      if (opts.onHandshakeFailed) {
        opts.onHandshakeFailed({
          type: "res",
          ok: res.ok,
          payload: res.payload,
          error: res.error
        });
      } else {
        ws.close();
      }
    };
    ws.send(
      JSON.stringify({
        type: "req",
        id,
        method: "connect",
        params: {
          protocol: { min: 3, max: 4 },
          client: {
            id: "web-chat-ui",
            version: "0.1.0",
            platform: "browser",
            mode: "operator"
          },
          locale: resolveLocale(),
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone
        }
      })
    );
  };
  ws.onmessage = (evt) => {
    var _a;
    let frame;
    try {
      frame = JSON.parse(evt.data);
    } catch {
      return;
    }
    if ((frame == null ? void 0 : frame.type) === "res" && frame.error) {
      frame.error = localizeRpcError(frame.error);
      if (((_a = frame.error) == null ? void 0 : _a.code) === "UNAUTHORIZED" && !authRedirectPending) {
        authRedirectPending = true;
        window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
      }
    }
    if (frame.type === "res" && frame.id && Object.hasOwn(pending, frame.id)) {
      pending[frame.id]({
        ok: frame.ok ?? false,
        payload: frame.payload,
        error: frame.error
      });
      delete pending[frame.id];
      return;
    }
    if (frame.type === "req" && frame.id && frame.method) {
      handleServerRequest(ws, frame);
      return;
    }
    if (opts.onFrame) opts.onFrame(frame);
  };
  ws.onclose = () => {
    const wasConnected = connected;
    setConnected(false);
    for (const id in pending) {
      pending[id]({ ok: false, error: { code: "DISCONNECTED", message: "WebSocket disconnected" } });
      delete pending[id];
    }
    if (opts.onDisconnected) opts.onDisconnected(wasConnected);
    if (wasConnected) {
      scheduleReconnect(() => connectWs(opts), backoff);
    } else {
      checkAuthOrReconnect(opts, backoff);
    }
  };
  ws.onerror = () => {
  };
}
function handleServerRequest(ws, frame) {
  const method = frame.method ?? "";
  if (!Object.hasOwn(serverRequestHandlers, method)) {
    ws.send(
      JSON.stringify({
        type: "res",
        id: frame.id,
        ok: false,
        error: { code: "UNKNOWN_METHOD", message: `no handler for ${method}` }
      })
    );
    return;
  }
  const handler = serverRequestHandlers[method];
  Promise.resolve().then(() => handler(frame.params || {})).then((result) => {
    ws.send(JSON.stringify({ type: "res", id: frame.id, ok: true, payload: result || {} }));
  }).catch((err) => {
    ws.send(
      JSON.stringify({
        type: "res",
        id: frame.id,
        ok: false,
        error: { code: "INTERNAL", message: String((err == null ? void 0 : err.message) || err) }
      })
    );
  });
}
function subscribeEvents(events) {
  return sendRpc("subscribe", { events });
}
function checkAuthOrReconnect(opts, backoff) {
  fetch("/api/auth/status").then((r) => r.ok ? r.json() : null).then((auth) => {
    if (auth == null ? void 0 : auth.setup_required) {
      window.location.assign("/onboarding");
    } else if (auth && !auth.authenticated) {
      window.location.assign("/login");
    } else {
      scheduleReconnect(() => connectWs(opts), backoff);
    }
  }).catch(() => {
    scheduleReconnect(() => connectWs(opts), backoff);
  });
}
function scheduleReconnect(reconnect, backoff) {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    setReconnectDelay(Math.min(reconnectDelay * backoff.factor, backoff.max));
    reconnect();
  }, reconnectDelay);
}
function forceReconnect(opts) {
  const resolved = opts || lastOpts;
  if (!resolved || connected) return;
  if (reconnectTimer) clearTimeout(reconnectTimer);
  reconnectTimer = null;
  setReconnectDelay(1e3);
  connectWs(resolved);
}
const _wsConnect = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  connectWs,
  forceReconnect,
  onServerRequest,
  subscribeEvents
}, Symbol.toStringTag, { value: "Module" }));
export {
  _wsConnect as _,
  connectWs as c,
  forceReconnect as f,
  subscribeEvents as s,
  u
};
