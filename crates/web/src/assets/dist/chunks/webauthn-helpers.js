import { c as connected, bW as localizeRpcError, bX as pending, bY as setConnected, bZ as nextId, b_ as getPreferredLocale, b$ as setReconnectDelay, c0 as reconnectDelay, d as sendRpc, c1 as setWs, aw as d, ax as A, av as y } from "./theme.js";
import { u } from "./jsxRuntime.module.js";
const gon = window.__MOLTIS__ || {};
const listeners = {};
function get(key) {
  return gon[key] ?? null;
}
function set(key, value) {
  gon[key] = value;
  notify(key, value);
}
function onChange(key, fn) {
  if (!listeners[key]) listeners[key] = [];
  listeners[key].push(fn);
}
function refresh() {
  return fetch(`/api/gon?_=${Date.now()}`, {
    cache: "no-store",
    headers: {
      "Cache-Control": "no-cache",
      Pragma: "no-cache"
    }
  }).then((r) => r.ok ? r.json() : null).then((data) => {
    if (!data) return;
    for (const key of Object.keys(data)) {
      gon[key] = data[key];
      notify(key, data[key]);
    }
  });
}
function notify(key, value) {
  for (const fn of listeners[key] || []) fn(value);
}
const eventListeners = {};
function onEvent(eventName, handler) {
  (eventListeners[eventName] = eventListeners[eventName] || []).push(handler);
  return function off() {
    const arr = eventListeners[eventName];
    if (arr) {
      const idx = arr.indexOf(handler);
      if (idx !== -1) arr.splice(idx, 1);
    }
  };
}
function targetValue(e) {
  return e.target.value;
}
function targetChecked(e) {
  return e.target.checked;
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
const EMOJI_LIST = [
  "🐶",
  "🐱",
  "🐰",
  "🐹",
  "🐻",
  "🐺",
  "🦁",
  "🦅",
  "🦉",
  "🐧",
  "🐢",
  "🐍",
  "🐉",
  "🦄",
  "🐙",
  "🦀",
  "🦞",
  "🐝",
  "🦊",
  "🐿️",
  "🦔",
  "🦇",
  "🐊",
  "🐳",
  "🐬",
  "🦝",
  "🦭",
  "🦜",
  "🦩",
  "🐦",
  "🐎",
  "🦌",
  "🐘",
  "🦛",
  "🐼",
  "🐨",
  "🤖",
  "👾",
  "👻",
  "🎃",
  "⭐",
  "🔥",
  "⚡",
  "🌈",
  "🌟",
  "💡",
  "🧠",
  "🧭",
  "🔮",
  "🚀",
  "🌍",
  "🌵",
  "🌻",
  "🍀",
  "🍄",
  "❄️"
];
function EmojiPicker({ value, onChange: onChange2, onSelect }) {
  const [open, setOpen] = d(false);
  const wrapRef = A(null);
  y(() => {
    if (!open) return;
    function onClick(e) {
      if (wrapRef.current && !wrapRef.current.contains(e.target)) setOpen(false);
    }
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [open]);
  return /* @__PURE__ */ u("div", { class: "settings-emoji-field", ref: wrapRef, children: [
    /* @__PURE__ */ u(
      "input",
      {
        type: "text",
        class: "provider-key-input w-12 px-1 py-1 text-center text-xl",
        value: value || "",
        onInput: (e) => onChange2(e.target.value),
        placeholder: "🐾"
      }
    ),
    /* @__PURE__ */ u("button", { type: "button", class: "provider-btn provider-btn-sm", onClick: () => setOpen(!open), children: open ? "Close" : "Pick" }),
    open ? /* @__PURE__ */ u("div", { class: "settings-emoji-picker", children: EMOJI_LIST.map((em) => /* @__PURE__ */ u(
      "button",
      {
        type: "button",
        class: `settings-emoji-btn ${value === em ? "active" : ""}`,
        onClick: () => {
          onChange2(em);
          if (onSelect) onSelect(em);
          setOpen(false);
        },
        children: em
      }
    )) }) : null
  ] });
}
const MODEL_SERVICE_NOT_CONFIGURED = "model service not configured";
const MODEL_TEST_RETRY_ATTEMPTS = 40;
const MODEL_TEST_RETRY_DELAY_MS = 250;
function humanizeProbeError(error) {
  if (!error || typeof error !== "string") return error;
  const lower = error.toLowerCase();
  if (lower.includes("401") || lower.includes("unauthorized") || lower.includes("invalid api key") || lower.includes("invalid x-api-key")) {
    return "Invalid API key. Please double-check and try again.";
  }
  if (lower.includes("403") || lower.includes("forbidden")) {
    return "Your API key doesn't have access. Check your account permissions.";
  }
  if (lower.includes("permission")) {
    return error;
  }
  if (lower.includes("429") || lower.includes("rate limit") || lower.includes("too many requests")) {
    return "Rate limited by the provider. Wait a moment and try again.";
  }
  if (lower.includes("timeout") || lower.includes("timed out")) {
    return "Connection timed out. Check your endpoint URL and try again.";
  }
  if (lower.includes("connection refused") || lower.includes("econnrefused")) {
    return "Connection refused. Make sure the provider endpoint is running and reachable.";
  }
  if (lower.includes("dns") || lower.includes("getaddrinfo") || lower.includes("name or service not known")) {
    return "Could not resolve the endpoint address. Check the URL and try again.";
  }
  if (lower.includes("ollama pull")) {
    return error;
  }
  if (lower.includes("404") || lower.includes("not found")) {
    return "Model not found at this endpoint. Make sure it is installed and try again.";
  }
  return error;
}
function isModelServiceNotConfigured(error) {
  if (!error || typeof error !== "string") return false;
  return error.toLowerCase().includes(MODEL_SERVICE_NOT_CONFIGURED);
}
function isTimeoutError(error) {
  if (!error || typeof error !== "string") return false;
  const lower = error.toLowerCase();
  return lower.includes("timeout") || lower.includes("timed out");
}
async function validateProviderKey(provider, apiKey, baseUrl, model, requestId) {
  var _a;
  const payload = { provider, apiKey };
  if (baseUrl) payload.baseUrl = baseUrl;
  if (model) payload.model = model;
  if (requestId) payload.requestId = requestId;
  const res = await sendRpc("providers.validate_key", payload);
  if (!(res == null ? void 0 : res.ok)) {
    return {
      valid: false,
      error: humanizeProbeError(((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to validate credentials.")
    };
  }
  const data = res.payload || {};
  if (data.valid) {
    return { valid: true, models: data.models || [] };
  }
  return {
    valid: false,
    error: humanizeProbeError(data.error || "Validation failed.")
  };
}
async function testModel(modelId) {
  var _a;
  for (let attempt = 0; attempt < MODEL_TEST_RETRY_ATTEMPTS; attempt++) {
    const res = await sendRpc("models.test", { modelId });
    if (res == null ? void 0 : res.ok) {
      return { ok: true };
    }
    const message = ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Model test failed.";
    const lower = String(message).toLowerCase();
    const shouldRetry = lower.includes(MODEL_SERVICE_NOT_CONFIGURED) && attempt < MODEL_TEST_RETRY_ATTEMPTS - 1;
    if (!shouldRetry) {
      return {
        ok: false,
        error: humanizeProbeError(message)
      };
    }
    await new Promise((resolve) => {
      window.setTimeout(resolve, MODEL_TEST_RETRY_DELAY_MS);
    });
  }
  return {
    ok: false,
    error: humanizeProbeError("Model test failed.")
  };
}
function buildSaveKeyPayload(providerName, apiKey, baseUrl, model) {
  const payload = { provider: providerName, apiKey };
  if (baseUrl) payload.baseUrl = baseUrl;
  if (model) payload.model = model;
  return payload;
}
function saveProviderKey(providerName, apiKey, baseUrl, model) {
  const payload = buildSaveKeyPayload(providerName, apiKey, baseUrl, model);
  return sendRpc("providers.save_key", payload);
}
const KEY_SOURCE_BY_PROVIDER = {
  anthropic: {
    url: "https://console.anthropic.com/settings/keys",
    label: "Anthropic Console"
  },
  openai: {
    url: "https://platform.openai.com/api-keys",
    label: "OpenAI Platform"
  },
  gemini: {
    url: "https://aistudio.google.com/app/apikey",
    label: "Google AI Studio"
  },
  groq: {
    url: "https://console.groq.com/keys",
    label: "Groq Console"
  },
  xai: {
    url: "https://console.x.ai/",
    label: "xAI Console"
  },
  deepseek: {
    url: "https://platform.deepseek.com/api_keys",
    label: "DeepSeek Platform"
  },
  mistral: {
    url: "https://console.mistral.ai/api-keys/",
    label: "Mistral Console"
  },
  openrouter: {
    url: "https://openrouter.ai/settings/keys",
    label: "OpenRouter Settings"
  },
  cerebras: {
    url: "https://cloud.cerebras.ai/",
    label: "Cerebras Cloud"
  },
  minimax: {
    url: "https://www.minimax.io/platform",
    label: "MiniMax Platform"
  },
  moonshot: {
    url: "https://platform.moonshot.ai/console/api-keys",
    label: "Moonshot Platform"
  },
  "kimi-code": {
    url: "https://www.kimi.com/code/console",
    label: "Kimi Code Console"
  },
  venice: {
    url: "https://venice.ai/settings/api-keys",
    label: "Venice Settings"
  }
};
function providerApiKeyHelp(provider) {
  if (!provider || provider.authType !== "api-key") return null;
  if (provider.keyOptional) {
    return {
      text: `API key is optional for ${provider.displayName}. Leave blank unless your gateway requires one.`
    };
  }
  const source = KEY_SOURCE_BY_PROVIDER[provider.name];
  if (source) {
    return {
      text: "Get your key at",
      url: source.url,
      label: source.label
    };
  }
  return {
    text: `Get your API key from the ${provider.displayName} dashboard.`
  };
}
function normalizeOAuthStartResponse(res) {
  var _a;
  const payload = res == null ? void 0 : res.payload;
  if ((res == null ? void 0 : res.ok) && (payload == null ? void 0 : payload.alreadyAuthenticated)) {
    return {
      status: "already"
    };
  }
  if ((res == null ? void 0 : res.ok) && (payload == null ? void 0 : payload.authUrl)) {
    return {
      status: "browser",
      authUrl: payload.authUrl
    };
  }
  if ((res == null ? void 0 : res.ok) && (payload == null ? void 0 : payload.deviceFlow)) {
    const verificationUrl = payload.verificationUriComplete || payload.verificationUri;
    if (!(verificationUrl && payload.userCode)) {
      return {
        status: "error",
        error: "OAuth device flow response is missing verification data."
      };
    }
    return {
      status: "device",
      verificationUrl,
      userCode: payload.userCode
    };
  }
  return {
    status: "error",
    error: ((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to start OAuth"
  };
}
function startProviderOAuth(providerName) {
  return sendRpc("providers.oauth.start", {
    provider: providerName,
    redirectUri: `${window.location.origin}/auth/callback`
  }).then((res) => normalizeOAuthStartResponse(res));
}
function completeProviderOAuth(providerName, callback) {
  return sendRpc("providers.oauth.complete", {
    provider: providerName,
    callback
  });
}
function validateIdentityFields(name, userName) {
  if (!(name.trim() || userName.trim())) {
    return { valid: false, error: "Agent name and your name are required." };
  }
  if (!name.trim()) {
    return { valid: false, error: "Agent name is required." };
  }
  if (!userName.trim()) {
    return { valid: false, error: "Your name is required." };
  }
  return { valid: true };
}
function isMissingMethodError(res) {
  var _a;
  const message = (_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message;
  if (typeof message !== "string") return false;
  const lower = message.toLowerCase();
  return lower.includes("method") && (lower.includes("not found") || lower.includes("unknown"));
}
function updateIdentity(fields, options = {}) {
  const agentId = options.agentId;
  if (!agentId) {
    return sendRpc("agent.identity.update", fields);
  }
  const params = { ...fields, agent_id: agentId };
  return sendRpc("agents.identity.update", params).then((res) => {
    if ((res == null ? void 0 : res.ok) || !isMissingMethodError(res)) return res;
    return sendRpc("agent.identity.update", fields);
  });
}
const AAGUID_NAMES = {
  "fbfc3007-154e-4ecc-8c0b-6e020557d7bd": "Apple Passwords",
  "dd4ec289-e01d-41c9-bb89-70fa845d4bf2": "iCloud Keychain (Managed)",
  "adce0002-35bc-c60a-648b-0b25f1f05503": "Chrome on Mac",
  "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4": "Google Password Manager",
  "08987058-cadc-4b81-b6e1-30de50dcbe96": "Windows Hello",
  "9ddd1817-af5a-4672-a2b9-3e3dd95000a9": "Windows Hello",
  "6028b017-b1d4-4c02-b4b3-afcdafc96bb2": "Windows Hello",
  "bada5566-a7aa-401f-bd96-45619a55120d": "1Password",
  "d548826e-79b4-db40-a3d8-11116f7e8349": "Bitwarden",
  "531126d6-e717-415c-9320-3d9aa6981239": "Dashlane",
  "b84e4048-15dc-4dd0-8640-f4f60813c8af": "NordPass",
  "0ea242b4-43c4-4a1b-8b17-dd6d0b6baec6": "Keeper",
  "f3809540-7f14-49c1-a8b3-8f813b225541": "Enpass",
  "53414d53-554e-4700-0000-000000000000": "Samsung Pass",
  "b5397666-4885-aa6b-cebf-e52262a439a2": "Chromium Browser",
  "771b48fd-d3d4-4f74-9232-fc157ab0507a": "Edge on Mac",
  "891494da-2c90-4d31-a9cd-4eab0aed1309": "Sesame"
};
function detectPasskeyName(cred) {
  try {
    const response = cred.response;
    const authData = new Uint8Array(response.getAuthenticatorData());
    if (authData.length >= 53) {
      let hex = "";
      for (let i = 37; i < 53; i++) hex += authData[i].toString(16).padStart(2, "0");
      const uuid = hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-" + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20);
      if (uuid !== "00000000-0000-0000-0000-000000000000") {
        const name = AAGUID_NAMES[uuid];
        if (name) return name;
      }
    }
  } catch (_e) {
  }
  if (cred.authenticatorAttachment === "platform") return "This device";
  if (cred.authenticatorAttachment === "cross-platform") return "Security key";
  return "Passkey";
}
function base64ToArrayBuffer(b64) {
  let str = b64.replace(/-/g, "+").replace(/_/g, "/");
  while (str.length % 4) str += "=";
  const bin = atob(str);
  const buf = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
  return buf.buffer;
}
function prepareCreationOptions(serverPk) {
  serverPk.challenge = base64ToArrayBuffer(serverPk.challenge);
  const user = serverPk.user;
  user.id = base64ToArrayBuffer(user.id);
  if (serverPk.excludeCredentials) {
    for (const c of serverPk.excludeCredentials) {
      c.id = base64ToArrayBuffer(c.id);
    }
  }
  return serverPk;
}
export {
  EmojiPicker as E,
  onChange as a,
  targetChecked as b,
  connectWs as c,
  completeProviderOAuth as d,
  eventListeners as e,
  forceReconnect as f,
  get as g,
  startProviderOAuth as h,
  saveProviderKey as i,
  testModel as j,
  isModelServiceNotConfigured as k,
  isTimeoutError as l,
  humanizeProbeError as m,
  validateIdentityFields as n,
  onEvent as o,
  providerApiKeyHelp as p,
  set as q,
  refresh as r,
  subscribeEvents as s,
  targetValue as t,
  updateIdentity as u,
  validateProviderKey as v,
  prepareCreationOptions as w,
  detectPasskeyName as x
};
