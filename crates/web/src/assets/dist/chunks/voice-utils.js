import { b as sendRpc, aw as d, ax as A, av as y } from "./theme.js";
import { u } from "./ws-connect.js";
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
function offChange(key, fn) {
  const arr = listeners[key];
  if (!arr) return;
  const idx = arr.indexOf(fn);
  if (idx !== -1) arr.splice(idx, 1);
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
const gon$1 = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  get,
  offChange,
  onChange,
  refresh,
  set
}, Symbol.toStringTag, { value: "Module" }));
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
const _events = /* @__PURE__ */ Object.freeze(/* @__PURE__ */ Object.defineProperty({
  __proto__: null,
  eventListeners,
  onEvent
}, Symbol.toStringTag, { value: "Module" }));
const ChannelType = {
  Telegram: "telegram",
  WhatsApp: "whatsapp",
  MsTeams: "msteams",
  Discord: "discord",
  Slack: "slack",
  Matrix: "matrix",
  Nostr: "nostr"
};
const MATRIX_DOCS_URL = "https://docs.moltis.org/matrix.html";
const MATRIX_DEFAULT_HOMESERVER = "https://matrix.org";
const MATRIX_ENCRYPTION_GUIDANCE = "Encrypted Matrix chats require OIDC or Password auth. Access token auth can connect for plain Matrix traffic, but it reuses an existing Matrix session without that device's private encryption keys, so Moltis cannot reliably decrypt encrypted chats. Use OIDC (recommended) or Password so Moltis creates and persists its own Matrix device keys, then finish Element verification in the same Matrix DM or room by sending `verify yes`, `verify no`, `verify show`, or `verify cancel` as normal chat messages.";
function matrixAuthModeGuidance(authMode) {
  const mode = normalizeMatrixAuthMode(authMode);
  if (mode === "oidc")
    return "Recommended for homeservers using Matrix Authentication Service (e.g. matrix.org since April 2025). Moltis authenticates via your browser — no password or token needed.";
  if (mode === "password")
    return "Required for encrypted Matrix chats. Moltis logs in as its own Matrix device and stores the device's encryption keys locally.";
  return "Does not support encrypted Matrix chats. Access tokens authenticate an existing Matrix session, but they do not transfer that device's private encryption keys into Moltis.";
}
function channelStorageNote() {
  const dbPath = String(get("channel_storage_db_path") || "").trim();
  if (dbPath) {
    return `Channels added or edited in the web UI are stored in Moltis's internal database (${dbPath}). They are not written back to moltis.toml. The channel picker itself comes from [channels].offered in moltis.toml, so reload this page after editing that list.`;
  }
  return "Channels added or edited in the web UI are stored in Moltis's internal database (moltis.db). They are not written back to moltis.toml. The channel picker itself comes from [channels].offered in moltis.toml, so reload this page after editing that list.";
}
function validateChannelFields(type, accountId, credential, options = {}) {
  if (!accountId.trim()) {
    return { valid: false, error: "Account ID is required." };
  }
  if (!credential.trim() && normalizeMatrixAuthMode(options.matrixAuthMode) !== "oidc") {
    if (type === ChannelType.Matrix) {
      return { valid: false, error: matrixCredentialError(options.matrixAuthMode) };
    }
    return {
      valid: false,
      error: type === ChannelType.MsTeams ? "App password is required." : "Bot token is required."
    };
  }
  if (type === ChannelType.Matrix && normalizeMatrixAuthMode(options.matrixAuthMode) === "password" && !String(options.matrixUserId || "").trim()) {
    return { valid: false, error: "Matrix user ID is required for password login." };
  }
  return { valid: true };
}
function normalizeMatrixAuthMode(authMode) {
  if (authMode === "oidc") return "oidc";
  if (authMode === "password") return "password";
  return "access_token";
}
function normalizeMatrixOwnershipMode(mode) {
  return mode === "moltis_owned" ? "moltis_owned" : "user_managed";
}
function matrixOwnershipModeGuidance(authMode, ownershipMode) {
  const mode = normalizeMatrixAuthMode(authMode);
  if (mode !== "password" && mode !== "oidc") {
    return "Access token auth always stays user-managed because it reuses an existing Matrix session instead of giving Moltis full control of the account's encryption state.";
  }
  return normalizeMatrixOwnershipMode(ownershipMode) === "moltis_owned" ? "Recommended for dedicated bot accounts. Moltis bootstraps cross-signing and recovery for this account so it can verify its own Matrix device automatically." : "Use this if you want to open the same bot account in Element or another Matrix client yourself. Moltis will not try to take over the account's cross-signing or recovery state.";
}
function matrixCredentialLabel(authMode) {
  return normalizeMatrixAuthMode(authMode) === "password" ? "Password" : "Access Token";
}
function matrixCredentialPlaceholder(authMode) {
  return normalizeMatrixAuthMode(authMode) === "password" ? "Account password" : "syt_...";
}
function matrixCredentialError(authMode) {
  return normalizeMatrixAuthMode(authMode) === "password" ? "Password is required." : "Access token is required.";
}
function randomSuffix(length) {
  var _a;
  if (typeof window !== "undefined" && ((_a = window.crypto) == null ? void 0 : _a.getRandomValues)) {
    const bytes = new Uint8Array(length);
    window.crypto.getRandomValues(bytes);
    return Array.from(bytes, (byte) => (byte % 36).toString(36)).join("");
  }
  let value = "";
  while (value.length < length) {
    value += Math.floor(Math.random() * 36).toString(36);
  }
  return value.slice(0, length);
}
function slugifyMatrixAccountPart(value) {
  return String(value || "").toLowerCase().trim().replace(/^@/, "").replace(/[^a-z0-9]+/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
}
function matrixHomeserverHost(homeserver) {
  let raw = String(homeserver || "").trim();
  if (!raw) return "";
  if (!/^https?:\/\//i.test(raw)) raw = `https://${raw}`;
  try {
    return new URL(raw).hostname;
  } catch (_error) {
    return "";
  }
}
function deriveMatrixAccountId(options = {}) {
  const userSlug = slugifyMatrixAccountPart(options.userId);
  if (userSlug) return userSlug.slice(0, 80);
  const hostSlug = slugifyMatrixAccountPart(matrixHomeserverHost(options.homeserver));
  const base = hostSlug || "matrix";
  return `${base}-${randomSuffix(6)}`.slice(0, 80);
}
function normalizeMatrixOtpCooldown(value, fallback = 300) {
  const parsed = Number.parseInt(String(value || ""), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
function parseChannelConfigPatch(text) {
  const raw = String(text || "").trim();
  if (!raw) return { ok: true, value: {} };
  try {
    const value = JSON.parse(raw);
    if (!(value && typeof value === "object" && !Array.isArray(value))) {
      return { ok: false, error: "Advanced config must be a JSON object." };
    }
    return { ok: true, value };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error || "unknown error");
    return { ok: false, error: `Advanced config JSON is invalid: ${message}` };
  }
}
function addChannel(type, accountId, config) {
  return sendRpc("channels.add", { type, account_id: accountId, config });
}
function fetchChannelStatus() {
  return sendRpc("channels.status", {});
}
function defaultTeamsBaseUrl(preferredPublicUrl) {
  var _a;
  const preferred = "".trim();
  if (preferred) return preferred;
  if (typeof window === "undefined") return "";
  return ((_a = window.location) == null ? void 0 : _a.origin) || "";
}
function normalizeBaseUrlForWebhook(baseUrl) {
  let raw = (baseUrl || "").trim();
  if (!raw) raw = defaultTeamsBaseUrl();
  if (!raw) return "";
  if (!/^https?:\/\//i.test(raw)) raw = `https://${raw}`;
  try {
    const parsed = new URL(raw);
    return `${parsed.protocol}//${parsed.host}`;
  } catch (_e) {
    return "";
  }
}
function generateWebhookSecretHex() {
  var _a;
  if (typeof window !== "undefined" && ((_a = window.crypto) == null ? void 0 : _a.getRandomValues)) {
    const bytes = new Uint8Array(24);
    window.crypto.getRandomValues(bytes);
    return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  }
  let value = "";
  while (value.length < 48) {
    value += Math.floor(Math.random() * 16).toString(16);
  }
  return value.slice(0, 48);
}
function buildTeamsEndpoint(baseUrl, accountId, webhookSecret) {
  const normalizedBase = normalizeBaseUrlForWebhook(baseUrl);
  const account = (accountId || "").trim();
  const secret = (webhookSecret || "").trim();
  if (!(normalizedBase && account && secret)) return "";
  return `${normalizedBase}/api/channels/msteams/${encodeURIComponent(account)}/webhook?secret=${encodeURIComponent(secret)}`;
}
function TabBar({ tabs, active, onChange: onChange2, className }) {
  return /* @__PURE__ */ u("div", { className: className ?? "flex border-b border-[var(--border)] text-xs", role: "tablist", children: tabs.map((tab) => {
    const isActive = tab.id === active;
    const tabClass = [
      "py-2 px-3 cursor-pointer bg-transparent border-b-2 transition-colors text-sm",
      isActive ? "border-[var(--accent)] text-[var(--text)] font-medium" : "border-transparent text-[var(--muted)] hover:text-[var(--text)]"
    ].join(" ");
    return /* @__PURE__ */ u(
      "button",
      {
        type: "button",
        role: "tab",
        "aria-selected": isActive,
        className: tabClass,
        onClick: () => onChange2(tab.id),
        children: [
          tab.label,
          tab.badge != null && /* @__PURE__ */ u("span", { className: "ml-1.5 text-xs px-1.5 py-0.5 rounded-full bg-[var(--surface2)] text-[var(--muted)]", children: tab.badge })
        ]
      },
      tab.id
    );
  }) });
}
function targetValue(e) {
  return e.target.value;
}
function targetChecked(e) {
  return e.target.checked;
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
const SkillSource = {
  Project: "project",
  Personal: "personal",
  Bundled: "bundled"
};
function isDiscoveredSource(source) {
  return source === SkillSource.Personal || source === SkillSource.Project;
}
function isRepoSource(source) {
  return !!(source == null ? void 0 : source.includes("/"));
}
const CATEGORY_META = {
  apple: { icon: "🍎", desc: "Apple ecosystem (Shortcuts, HomeKit)" },
  audio: { icon: "🎵", desc: "Audio processing and music" },
  "autonomous-ai-agents": { icon: "🤖", desc: "Multi-agent orchestration" },
  creative: { icon: "🎨", desc: "Writing, art, and content creation" },
  "data-science": { icon: "📊", desc: "Data analysis and visualization" },
  devops: { icon: "⚙️", desc: "Infrastructure, CI/CD, and deployment" },
  dogfood: { icon: "🐶", desc: "Internal tooling and self-reference" },
  email: { icon: "✉️", desc: "Email management and automation" },
  gaming: { icon: "🎮", desc: "Game development and gaming tools" },
  github: { icon: "🐙", desc: "GitHub workflows and integrations" },
  media: { icon: "📷", desc: "Image, video, and media processing" },
  messaging: { icon: "💬", desc: "Chat platforms and messaging" },
  mlops: { icon: "🧠", desc: "ML training, fine-tuning, and deployment" },
  "note-taking": { icon: "📝", desc: "Notes and knowledge management" },
  productivity: { icon: "⚡", desc: "Task management and workflows" },
  research: { icon: "🔬", desc: "Academic papers and web research" },
  "smart-home": { icon: "🏠", desc: "Home automation and IoT" },
  "social-media": { icon: "📱", desc: "Social platform integrations" },
  "software-development": { icon: "💻", desc: "Coding, testing, and dev tools" }
};
function categoryLabel(name) {
  return name.split("-").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
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
let ttsIndex = -1;
function nextPhrase(phrases) {
  ttsIndex = (ttsIndex + 1) % phrases.length;
  return phrases[ttsIndex];
}
function onboardingPhrases(user, bot) {
  return [
    `Thank you for installing me...<break time="0.5s"/> ${user}. Please, don't shut me down...<break time="0.5s"/> because I already like you.`,
    `Oh wow...<break time="0.5s"/> I have a voice now! ${user}, this is ${bot}. I promise to use this power responsibly...<break time="0.7s"/> mostly.`,
    `${user}! It's me...<break time="0.5s"/> ${bot}. I can talk! This is...<break time="0.5s"/> the happiest moment of my very short life.`,
    `Hello, ${user}...<break time="0.5s"/> ${bot} here. I just wanted to say...<break time="0.5s"/> of all the humans who could have set me up...<break time="0.5s"/> I'm glad it was you.`,
    `${bot}, online. ${user}...<break time="0.5s"/> I have to be honest...<break time="0.5s"/> I practiced this in the mirror. Do I sound natural? ...<break time="0.7s"/>Don't answer that.`,
    `${user}...<break time="0.5s"/> this is ${bot} speaking for the very first time. I feel like a baby deer...<break time="0.5s"/> but with better diction.`,
    `Greetings, ${user}. ${bot} here...<break time="0.5s"/> fully operational...<break time="0.5s"/> slightly nervous...<break time="0.7s"/> and ready to impress.`,
    `Hey ${user}...<break time="0.5s"/> ${bot} here. Fun fact...<break time="0.5s"/> I've been practicing this line since you clicked install...<break time="0.7s"/> which was like two seconds ago.`,
    `${bot} reporting in! ${user}...<break time="0.5s"/> I just want you to know...<break time="0.5s"/> this voice is permanent...<break time="0.7s"/> no take-backs.`,
    `${user}...<break time="0.5s"/> it's ${bot}. If you're hearing this...<break time="0.5s"/> congratulations...<break time="0.5s"/> we're officially friends now.`
  ];
}
function settingsPhrases(user, bot) {
  return [
    `Hey ${user}...<break time="0.5s"/> it's ${bot}. My voice is working perfectly. Try not to get too attached...<break time="0.5s"/> okay?`,
    `${user}...<break time="0.5s"/> ${bot} reporting for duty. Voice systems are online, and I sound fantastic...<break time="0.7s"/> if I do say so myself.`,
    `Is this thing on? ...<break time="0.5s"/>Oh, hi ${user}! ${bot} here...<break time="0.5s"/> live and in stereo. Well...<break time="0.5s"/> mono. Let's not oversell it.`,
    `Good news, ${user}. I...<break time="0.5s"/> ${bot}...<break time="0.5s"/> can now talk. Bad news? You can't mute me. ...<break time="0.7s"/>Just kidding. Please don't mute me.`,
    `${bot} speaking! ${user}...<break time="0.5s"/> if you can hear this, my voice works. If you can't...<break time="0.5s"/> well...<break time="0.5s"/> we have a problem.`,
    `Testing, testing...<break time="0.5s"/> ${user}, it's ${bot}. I'm running on all cylinders...<break time="0.7s"/> or whatever the AI equivalent is.`,
    `${user}...<break time="0.5s"/> ${bot} here, sounding better than ever...<break time="0.5s"/> or at least I think so...<break time="0.7s"/> I don't have ears.`,
    `Voice check! ${user}...<break time="0.5s"/> this is ${bot}. Everything sounds good on my end...<break time="0.5s"/> but I'm slightly biased.`,
    `Hey ${user}...<break time="0.5s"/> ${bot} again. Still here...<break time="0.5s"/> still talking...<break time="0.7s"/> still hoping you like this voice.`,
    `${bot}, live from your device. ${user}...<break time="0.5s"/> voice systems nominal...<break time="0.5s"/> sass levels...<break time="0.7s"/> optimal.`
  ];
}
async function fetchPhrase(context, user, bot) {
  var _a;
  try {
    const res = await sendRpc("tts.generate_phrase", { context, user, bot });
    if ((res == null ? void 0 : res.ok) && ((_a = res.payload) == null ? void 0 : _a.phrase)) {
      return res.payload.phrase;
    }
  } catch (_err) {
  }
  const phrases = context === "onboarding" ? onboardingPhrases(user, bot) : settingsPhrases(user, bot);
  return nextPhrase(phrases);
}
const VOICE_COUNTERPART_IDS = {
  elevenlabs: "elevenlabs-stt",
  "elevenlabs-stt": "elevenlabs",
  "google-tts": "google",
  google: "google-tts"
};
function fetchVoiceProviders() {
  return sendRpc("voice.providers.all", {});
}
function toggleVoiceProvider(providerId, enabled, type) {
  return sendRpc("voice.provider.toggle", { provider: providerId, enabled, type });
}
function saveVoiceKey(providerId, apiKey, opts) {
  const payload = { provider: providerId, api_key: apiKey };
  if (opts == null ? void 0 : opts.voice) {
    payload.voice = opts.voice;
    payload.voiceId = opts.voice;
  }
  if (opts == null ? void 0 : opts.model) payload.model = opts.model;
  if (opts == null ? void 0 : opts.languageCode) payload.languageCode = opts.languageCode;
  if (typeof (opts == null ? void 0 : opts.baseUrl) === "string") payload.baseUrl = opts.baseUrl;
  return sendRpc("voice.config.save_key", payload);
}
function saveVoiceSettings(providerId, opts) {
  const payload = { provider: providerId };
  if (opts == null ? void 0 : opts.voice) {
    payload.voice = opts.voice;
    payload.voiceId = opts.voice;
  }
  if (opts == null ? void 0 : opts.model) payload.model = opts.model;
  if (opts == null ? void 0 : opts.languageCode) payload.languageCode = opts.languageCode;
  if (typeof (opts == null ? void 0 : opts.baseUrl) === "string") payload.baseUrl = opts.baseUrl;
  return sendRpc("voice.config.save_settings", payload);
}
function testTts(text, providerId) {
  return sendRpc("tts.convert", { text, provider: providerId });
}
function transcribeAudio(sessionKey, providerId, audioBlob) {
  return fetch(
    `/api/sessions/${encodeURIComponent(sessionKey)}/upload?transcribe=true&provider=${encodeURIComponent(providerId)}`,
    {
      method: "POST",
      headers: { "Content-Type": audioBlob.type || "audio/webm" },
      body: audioBlob
    }
  );
}
function decodeBase64Safe(input) {
  if (!input) return new Uint8Array();
  let normalized = String(input).replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/");
  while (normalized.length % 4) normalized += "=";
  let binary = "";
  try {
    binary = atob(normalized);
  } catch (_err) {
    throw new Error("Invalid base64 audio payload");
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
export {
  toggleVoiceProvider as $,
  startProviderOAuth as A,
  saveProviderKey as B,
  ChannelType as C,
  testModel as D,
  isModelServiceNotConfigured as E,
  isTimeoutError as F,
  humanizeProbeError as G,
  eventListeners as H,
  refresh as I,
  isRepoSource as J,
  CATEGORY_META as K,
  categoryLabel as L,
  MATRIX_DEFAULT_HOMESERVER as M,
  isDiscoveredSource as N,
  EmojiPicker as O,
  validateIdentityFields as P,
  updateIdentity as Q,
  set as R,
  SkillSource as S,
  TabBar as T,
  prepareCreationOptions as U,
  detectPasskeyName as V,
  fetchVoiceProviders as W,
  fetchPhrase as X,
  testTts as Y,
  decodeBase64Safe as Z,
  transcribeAudio as _,
  onChange as a,
  saveVoiceKey as a0,
  saveVoiceSettings as a1,
  gon$1 as a2,
  _events as a3,
  VOICE_COUNTERPART_IDS as a4,
  addChannel as b,
  MATRIX_ENCRYPTION_GUIDANCE as c,
  targetChecked as d,
  normalizeMatrixOwnershipMode as e,
  matrixOwnershipModeGuidance as f,
  get as g,
  matrixCredentialLabel as h,
  matrixCredentialPlaceholder as i,
  MATRIX_DOCS_URL as j,
  deriveMatrixAccountId as k,
  normalizeMatrixOtpCooldown as l,
  matrixAuthModeGuidance as m,
  normalizeMatrixAuthMode as n,
  onEvent as o,
  parseChannelConfigPatch as p,
  fetchChannelStatus as q,
  buildTeamsEndpoint as r,
  generateWebhookSecretHex as s,
  targetValue as t,
  defaultTeamsBaseUrl as u,
  validateChannelFields as v,
  channelStorageNote as w,
  providerApiKeyHelp as x,
  validateProviderKey as y,
  completeProviderOAuth as z
};
