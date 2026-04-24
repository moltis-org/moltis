import { _ as _wsConnect, u } from "./chunks/ws-connect.js";
import { bT as S, bX as initTheme, bZ as init, aQ as R, ay as d, ax as y, a0 as t } from "./chunks/theme.js";
import { b as formatLoginTitle, a as applyIdentityFavicon } from "./chunks/branding.js";
window.__moltis_state = S;
window.__moltis_modules = { ...window.__moltis_modules || {}, "ws-connect": _wsConnect };
initTheme();
const i18nReady = init().catch((err) => {
  console.warn("[i18n] login init failed", err);
});
const gonData = window.__MOLTIS__ || {};
const identity = gonData.identity || null;
document.title = formatLoginTitle(identity);
applyIdentityFavicon(identity);
showVaultBanner(gonData.vault_status || null);
function showVaultBanner(status) {
  const el = document.getElementById("vaultBanner");
  if (!el) return;
  el.style.display = status === "sealed" ? "" : "none";
}
async function parseLoginFailure(response) {
  if (response.status === 429) {
    let retryAfter = 0;
    try {
      const data = await response.json();
      if (data && Number.isFinite(data.retry_after_seconds)) {
        retryAfter = Math.max(1, Math.ceil(data.retry_after_seconds));
      }
    } catch {
    }
    if (retryAfter <= 0) {
      const retryAfterHeader = Number.parseInt(response.headers.get("Retry-After") || "0", 10);
      if (Number.isFinite(retryAfterHeader) && retryAfterHeader > 0) {
        retryAfter = retryAfterHeader;
      }
    }
    return { type: "retry", retryAfter: Math.max(1, retryAfter) };
  }
  if (response.status === 401) {
    return { type: "invalid_password" };
  }
  const bodyText = await response.text();
  return { type: "error", message: bodyText || t("login:loginFailed") };
}
function startPasskeyLogin(setError, setLoading) {
  setError(null);
  if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
    setError(t("login:passkeyRequiresDomain", { hostname: location.hostname }));
    return;
  }
  setLoading(true);
  fetch("/api/auth/passkey/auth/begin", { method: "POST" }).then((r) => r.json()).then(
    (data) => {
      const options = data.options;
      options.publicKey.challenge = base64ToBuffer(
        options.publicKey.challenge
      );
      if (options.publicKey.allowCredentials) {
        for (const c of options.publicKey.allowCredentials) {
          c.id = base64ToBuffer(c.id);
        }
      }
      return navigator.credentials.get({ publicKey: options.publicKey }).then((cred) => ({ cred, challengeId: data.challenge_id }));
    }
  ).then(({ cred, challengeId }) => {
    const assertionResponse = cred.response;
    const body = {
      challenge_id: challengeId,
      credential: {
        id: cred.id,
        rawId: bufferToBase64(cred.rawId),
        type: cred.type,
        response: {
          authenticatorData: bufferToBase64(assertionResponse.authenticatorData),
          clientDataJSON: bufferToBase64(assertionResponse.clientDataJSON),
          signature: bufferToBase64(assertionResponse.signature),
          userHandle: assertionResponse.userHandle ? bufferToBase64(assertionResponse.userHandle) : null
        }
      }
    };
    return fetch("/api/auth/passkey/auth/finish", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
  }).then((r) => {
    if (r.ok) {
      location.href = "/";
    } else {
      return r.text().then((msg) => {
        setError(msg || t("login:passkeyAuthFailed"));
        setLoading(false);
      });
    }
  }).catch((err) => {
    setError(err.message || t("login:passkeyAuthFailed"));
    setLoading(false);
  });
}
function renderLoginCard({
  title,
  showPassword,
  showPasskeys,
  showDivider,
  password,
  setPassword,
  onPasswordLogin,
  onPasskeyLogin,
  loading,
  retrySecondsLeft,
  error
}) {
  return /* @__PURE__ */ u("div", { className: "auth-card", children: [
    /* @__PURE__ */ u("h1", { className: "auth-title", children: title }),
    /* @__PURE__ */ u("p", { className: "auth-subtitle", children: t("login:signInToContinue") }),
    showPassword ? /* @__PURE__ */ u("form", { onSubmit: onPasswordLogin, className: "flex flex-col gap-3", children: [
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: t("login:password") }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "password",
            className: "provider-key-input w-full",
            value: password,
            onInput: (e) => setPassword(e.target.value),
            placeholder: t("login:enterPassword"),
            autofocus: true
          }
        )
      ] }),
      /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn w-full mt-1", disabled: loading || retrySecondsLeft > 0, children: loading ? t("login:signingIn") : retrySecondsLeft > 0 ? t("login:retryIn", { seconds: retrySecondsLeft }) : t("login:signIn") })
    ] }) : null,
    showDivider ? /* @__PURE__ */ u("div", { className: "auth-divider", children: /* @__PURE__ */ u("span", { children: t("login:or") }) }) : null,
    showPasskeys ? /* @__PURE__ */ u(
      "button",
      {
        type: "button",
        className: `provider-btn ${showPassword ? "provider-btn-secondary" : ""} w-full`,
        onClick: onPasskeyLogin,
        disabled: loading,
        children: t("login:signInWithPasskey")
      }
    ) : null,
    error ? /* @__PURE__ */ u("p", { className: "auth-error mt-2", children: error }) : null
  ] });
}
function LoginApp() {
  const [password, setPassword] = d("");
  const [error, setError] = d(null);
  const [loading, setLoading] = d(false);
  const [retrySecondsLeft, setRetrySecondsLeft] = d(0);
  const [hasPasskeys, setHasPasskeys] = d(false);
  const [hasPassword, setHasPassword] = d(false);
  const [ready, setReady] = d(false);
  y(() => {
    fetch("/api/auth/status").then((r) => r.ok ? r.json() : null).then((data) => {
      if (!data) return;
      if (data.authenticated) {
        location.href = "/";
        return;
      }
      setHasPasskeys(!!data.has_passkeys);
      setHasPassword(!!data.has_password);
      setReady(true);
    }).catch(() => setReady(true));
  }, []);
  y(() => {
    if (retrySecondsLeft <= 0) return void 0;
    const timer = setInterval(() => {
      setRetrySecondsLeft((value) => value > 1 ? value - 1 : 0);
    }, 1e3);
    return () => clearInterval(timer);
  }, [retrySecondsLeft]);
  function onPasswordLogin(e) {
    e.preventDefault();
    if (retrySecondsLeft > 0) return;
    setError(null);
    setLoading(true);
    fetch("/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password })
    }).then(async (r) => {
      if (r.ok) {
        location.href = "/";
        return;
      }
      const failure = await parseLoginFailure(r);
      if (failure.type === "retry") {
        setRetrySecondsLeft(failure.retryAfter || 1);
        setError(t("login:wrongPassword"));
      } else if (failure.type === "invalid_password") {
        setError(t("login:invalidPassword"));
      } else {
        setError(failure.message || null);
      }
      setLoading(false);
    }).catch((err) => {
      setError(err.message);
      setLoading(false);
    });
  }
  function onPasskeyLogin() {
    startPasskeyLogin(setError, setLoading);
  }
  if (!ready) {
    return /* @__PURE__ */ u("div", { className: "auth-card", children: /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: t("common:status.loading") }) });
  }
  const title = formatLoginTitle(identity);
  const showPassword = hasPassword || !hasPasskeys;
  const showPasskeys = hasPasskeys;
  const showDivider = showPassword && showPasskeys;
  return renderLoginCard({
    title,
    showPassword,
    showPasskeys,
    showDivider,
    password,
    setPassword,
    onPasswordLogin,
    onPasskeyLogin,
    loading,
    retrySecondsLeft,
    error
  });
}
function base64ToBuffer(b64) {
  let str = b64.replace(/-/g, "+").replace(/_/g, "/");
  while (str.length % 4) str += "=";
  const bin = atob(str);
  const buf = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
  return buf.buffer;
}
function bufferToBase64(buf) {
  const bytes = new Uint8Array(buf);
  let str = "";
  for (const b of bytes) str += String.fromCharCode(b);
  return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
const root = document.getElementById("loginRoot");
if (root) {
  i18nReady.finally(() => {
    R(/* @__PURE__ */ u(LoginApp, {}), root);
  });
}
