import { u } from "./jsxRuntime.module.js";
import { Z as t, aw as d, av as y, ax as A, d as sendRpc, bG as modelVersionScore, ay as S, aM as R } from "./theme.js";
import { c as connectWs, e as eventListeners, s as subscribeEvents, t as targetValue, w as prepareCreationOptions, x as detectPasskeyName, g as get, r as refresh, E as EmojiPicker, n as validateIdentityFields, u as updateIdentity, d as completeProviderOAuth, i as saveProviderKey, v as validateProviderKey, p as providerApiKeyHelp, j as testModel, k as isModelServiceNotConfigured, m as humanizeProbeError, h as startProviderOAuth } from "./webauthn-helpers.js";
let wsStarted = false;
function ensureWsConnected() {
  if (wsStarted) return;
  wsStarted = true;
  connectWs({
    backoff: { factor: 2, max: 1e4 },
    onConnected: () => {
      subscribeEvents(["channel"]);
    },
    onFrame: (frame) => {
      if (frame.type !== "event") return;
      const listeners = eventListeners[frame.event || ""] || [];
      listeners.forEach((h) => {
        h(frame.payload || {});
      });
    }
  });
}
function ErrorPanel({ message }) {
  return /* @__PURE__ */ u("div", { role: "alert", className: "alert-error-text whitespace-pre-line", children: [
    /* @__PURE__ */ u("span", { className: "text-[var(--error)] font-medium", children: t("onboarding:errorPrefix") }),
    " ",
    message
  ] });
}
function preferredChatPath() {
  const key = localStorage.getItem("moltis-session") || "main";
  return `/chats/${key.replace(/:/g, "/")}`;
}
function detectBrowserTimezone() {
  try {
    const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return typeof timezone === "string" ? timezone.trim() : "";
  } catch {
    return "";
  }
}
function bufferToBase64(buf) {
  const bytes = new Uint8Array(buf);
  let str = "";
  for (const b of bytes) str += String.fromCharCode(b);
  return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function AuthStep({ onNext, skippable }) {
  const [method, setMethod] = d(null);
  const [password, setPassword] = d("");
  const [confirm, setConfirm] = d("");
  const [setupCode, setSetupCode] = d("");
  const [passkeyName, setPasskeyName] = d("");
  const [codeRequired, setCodeRequired] = d(false);
  const [localhostOnly, setLocalhostOnly] = d(false);
  const [webauthnAvailable, setWebauthnAvailable] = d(false);
  const [error, setError] = d(null);
  const [saving, setSaving] = d(false);
  const [loading, setLoading] = d(true);
  const [passkeyOrigins, setPasskeyOrigins] = d([]);
  const [passkeyDone, setPasskeyDone] = d(false);
  const [optPw, setOptPw] = d("");
  const [optPwConfirm, setOptPwConfirm] = d("");
  const [optPwSaving, setOptPwSaving] = d(false);
  const [recoveryKey, setRecoveryKey] = d(null);
  const [recoveryCopied, setRecoveryCopied] = d(false);
  const isIpAddress = /^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[");
  const browserSupportsWebauthn = !!window.PublicKeyCredential;
  const passkeyEnabled = webauthnAvailable && browserSupportsWebauthn && !isIpAddress;
  const [setupComplete, setSetupComplete] = d(false);
  y(() => {
    fetch("/api/auth/status").then((r) => r.json()).then(
      (data) => {
        if (data.setup_code_required) setCodeRequired(true);
        if (data.localhost_only) setLocalhostOnly(true);
        if (data.webauthn_available) setWebauthnAvailable(true);
        if (data.passkey_origins) setPasskeyOrigins(data.passkey_origins);
        if (data.setup_complete) setSetupComplete(true);
        setLoading(false);
      }
    ).catch(() => setLoading(false));
  }, []);
  y(() => {
    if (passkeyEnabled && method === null) setMethod("passkey");
  }, [passkeyEnabled]);
  function onPasswordSubmit(e) {
    e.preventDefault();
    setError(null);
    if (password.length > 0 || !localhostOnly) {
      if (password.length < 12) {
        setError("Password must be at least 12 characters.");
        return;
      }
      if (password !== confirm) {
        setError("Passwords do not match.");
        return;
      }
    }
    if (codeRequired && setupCode.trim().length === 0) {
      setError("Enter the setup code shown in the process log (stdout).");
      return;
    }
    setSaving(true);
    const body = password ? { password } : {};
    if (codeRequired) body.setup_code = setupCode.trim();
    fetch("/api/auth/setup", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    }).then((r) => {
      if (r.ok) {
        ensureWsConnected();
        return r.json().then((data) => {
          if (data.recovery_key) {
            setRecoveryKey(data.recovery_key);
            setSaving(false);
          } else {
            onNext();
          }
        }).catch(() => onNext());
      } else {
        return r.text().then((txt) => {
          setError(txt || "Setup failed");
          setSaving(false);
        });
      }
    }).catch((err) => {
      setError(err.message);
      setSaving(false);
    });
  }
  function onPasskeyRegister() {
    setError(null);
    if (codeRequired && setupCode.trim().length === 0) {
      setError("Enter the setup code shown in the process log (stdout).");
      return;
    }
    setSaving(true);
    const codeBody = codeRequired ? { setup_code: setupCode.trim() } : {};
    let requestedRpId = null;
    fetch("/api/auth/setup/passkey/register/begin", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(codeBody)
    }).then((r) => {
      if (!r.ok)
        return r.text().then((txt) => Promise.reject(new Error(txt || "Failed to start passkey registration")));
      return r.json();
    }).then((data) => {
      var _a;
      const pk = data.options.publicKey;
      requestedRpId = ((_a = pk.rp) == null ? void 0 : _a.id) || null;
      const publicKey = prepareCreationOptions(pk);
      return navigator.credentials.create({ publicKey }).then((cred) => ({ cred, challengeId: data.challenge_id }));
    }).then(({ cred, challengeId }) => {
      const attestation = cred.response;
      const body = {
        challenge_id: challengeId,
        name: passkeyName.trim() || detectPasskeyName(cred),
        credential: {
          id: cred.id,
          rawId: bufferToBase64(cred.rawId),
          type: cred.type,
          response: {
            attestationObject: bufferToBase64(attestation.attestationObject),
            clientDataJSON: bufferToBase64(attestation.clientDataJSON)
          }
        }
      };
      if (codeRequired) body.setup_code = setupCode.trim();
      return fetch("/api/auth/setup/passkey/register/finish", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
      });
    }).then((r) => {
      if (r.ok) {
        ensureWsConnected();
        setSaving(false);
        setPasskeyDone(true);
      } else {
        return r.text().then((txt) => {
          setError(txt || "Passkey registration failed");
          setSaving(false);
        });
      }
    }).catch((err) => {
      if (err.name === "NotAllowedError") {
        setError("Passkey registration was cancelled.");
      } else {
        let msg = err.message || "Passkey registration failed";
        if (requestedRpId) {
          msg += ` (RPID: "${requestedRpId}", current origin: "${location.origin}")`;
        }
        setError(msg);
      }
      setSaving(false);
    });
  }
  function onOptionalPassword(e) {
    e.preventDefault();
    setError(null);
    if (optPw.length < 12) {
      setError("Password must be at least 12 characters.");
      return;
    }
    if (optPw !== optPwConfirm) {
      setError("Passwords do not match.");
      return;
    }
    setOptPwSaving(true);
    fetch("/api/auth/password/change", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ new_password: optPw })
    }).then((r) => {
      if (r.ok) {
        ensureWsConnected();
        onNext();
      } else {
        return r.text().then((txt) => {
          setError(txt || "Failed to set password");
          setOptPwSaving(false);
        });
      }
    }).catch((err) => {
      setError(err.message);
      setOptPwSaving(false);
    });
  }
  if (loading) {
    return /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: [
      "Checking authentication",
      "…"
    ] });
  }
  if (setupComplete) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:auth.secureYourInstance") }),
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 text-sm text-[var(--accent)]", children: [
        /* @__PURE__ */ u("span", { className: "icon icon-checkmark" }),
        "Authentication is already configured."
      ] }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn",
          onClick: () => {
            ensureWsConnected();
            onNext();
          },
          children: "Next"
        },
        `auth-${saving}`
      ) })
    ] });
  }
  if (recoveryKey) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Secure your instance" }),
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 text-sm text-[var(--accent)]", children: [
        /* @__PURE__ */ u("span", { className: "icon icon-checkmark" }),
        "Password set and vault initialized"
      ] }),
      /* @__PURE__ */ u(
        "div",
        {
          style: {
            maxWidth: "600px",
            padding: "12px 16px",
            borderRadius: "6px",
            border: "1px solid var(--border)",
            background: "var(--bg)"
          },
          children: [
            /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", style: { marginBottom: "8px" }, children: "Recovery key" }),
            /* @__PURE__ */ u(
              "code",
              {
                className: "select-all break-all",
                style: {
                  fontFamily: "var(--font-mono)",
                  fontSize: ".8rem",
                  color: "var(--text-strong)",
                  display: "block",
                  lineHeight: "1.5"
                },
                children: recoveryKey
              }
            ),
            /* @__PURE__ */ u("div", { style: { display: "flex", alignItems: "center", gap: "8px", marginTop: "10px" }, children: /* @__PURE__ */ u(
              "button",
              {
                type: "button",
                className: "provider-btn provider-btn-secondary",
                onClick: () => {
                  navigator.clipboard.writeText(recoveryKey).then(() => {
                    setRecoveryCopied(true);
                    setTimeout(() => setRecoveryCopied(false), 2e3);
                  });
                },
                children: recoveryCopied ? "Copied!" : "Copy"
              }
            ) })
          ]
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs", style: { color: "var(--error)", maxWidth: "600px" }, children: "Save this recovery key in a safe place. It will not be shown again. You need it to unlock the vault if you forget your password." }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: "Continue" }) })
    ] });
  }
  const passkeyDisabledReason = webauthnAvailable ? browserSupportsWebauthn ? isIpAddress ? "Requires domain name" : null : "Browser not supported" : "Not available on this server";
  const originsHint = passkeyOrigins.length > 1 ? passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ") : null;
  if (passkeyDone) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:auth.secureYourInstance") }),
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 text-sm text-[var(--accent)]", children: [
        /* @__PURE__ */ u("span", { className: "icon icon-checkmark" }),
        "Passkey registered successfully!"
      ] }),
      /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Optionally set a password as a fallback for when passkeys aren't available." }),
      /* @__PURE__ */ u("form", { onSubmit: onOptionalPassword, className: "flex flex-col gap-3", children: [
        /* @__PURE__ */ u("div", { children: [
          /* @__PURE__ */ u("label", { htmlFor: "onboarding-passkey-password", className: "text-xs text-[var(--muted)] mb-1 block", children: "Password" }),
          /* @__PURE__ */ u(
            "input",
            {
              id: "onboarding-passkey-password",
              type: "password",
              name: "password",
              autoComplete: "new-password",
              className: "provider-key-input w-full",
              value: optPw,
              onInput: (e) => setOptPw(targetValue(e)),
              placeholder: "At least 12 characters",
              autofocus: true
            }
          )
        ] }),
        /* @__PURE__ */ u("div", { children: [
          /* @__PURE__ */ u("label", { htmlFor: "onboarding-passkey-password-confirm", className: "text-xs text-[var(--muted)] mb-1 block", children: "Confirm password" }),
          /* @__PURE__ */ u(
            "input",
            {
              id: "onboarding-passkey-password-confirm",
              type: "password",
              name: "confirm_password",
              autoComplete: "new-password",
              className: "provider-key-input w-full",
              value: optPwConfirm,
              onInput: (e) => setOptPwConfirm(targetValue(e)),
              placeholder: "Repeat password"
            }
          )
        ] }),
        error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
        /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
          /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: optPwSaving, children: optPwSaving ? "Setting…" : "Set password & continue" }),
          /* @__PURE__ */ u(
            "button",
            {
              type: "button",
              className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
              onClick: () => {
                ensureWsConnected();
                onNext();
              },
              children: "Skip"
            }
          )
        ] })
      ] })
    ] });
  }
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:auth.secureYourInstance") }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: localhostOnly ? "Choose how to secure your instance, or skip for now. Setting a password also enables the encryption vault, which protects API keys and secrets stored in the database." : "Choose how to secure your instance." }),
    codeRequired && /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Setup code" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          inputMode: "numeric",
          pattern: "[0-9]*",
          value: setupCode,
          onInput: (e) => setSetupCode(targetValue(e)),
          placeholder: "6-digit code from terminal"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "Find this code in the moltis process log (stdout)." })
    ] }),
    /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: [
      /* @__PURE__ */ u(
        "div",
        {
          className: `backend-card ${method === "passkey" ? "selected" : ""} ${passkeyEnabled ? "" : "disabled"}`,
          onClick: passkeyEnabled ? () => setMethod("passkey") : void 0,
          children: [
            /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center justify-between gap-2", children: [
              /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text)]", children: "Passkey" }),
              /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2 justify-end", children: [
                passkeyEnabled ? /* @__PURE__ */ u("span", { className: "recommended-badge", children: "Recommended" }) : null,
                passkeyDisabledReason ? /* @__PURE__ */ u("span", { className: "tier-badge", children: passkeyDisabledReason }) : null
              ] })
            ] }),
            /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "Use Touch ID, Face ID, or a security key" })
          ]
        }
      ),
      /* @__PURE__ */ u(
        "div",
        {
          className: `backend-card ${method === "password" ? "selected" : ""}`,
          onClick: () => setMethod("password"),
          children: [
            /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center justify-between gap-2", children: /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text)]", children: "Password" }) }),
            /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "Set a password and enable the encryption vault for stored secrets" })
          ]
        }
      )
    ] }),
    method === "passkey" && /* @__PURE__ */ u("div", { className: "flex flex-col gap-3", children: [
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Passkey name" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            value: passkeyName,
            onInput: (e) => setPasskeyName(targetValue(e)),
            placeholder: "e.g. MacBook Touch ID (optional)"
          }
        )
      ] }),
      originsHint && /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
        "Passkeys will work when visiting: ",
        originsHint
      ] }),
      error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
        /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn",
            disabled: saving,
            onClick: onPasskeyRegister,
            children: saving ? "Registering…" : "Register passkey"
          },
          `pk-${saving}`
        ),
        skippable ? /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
            onClick: onNext,
            children: t("common:actions.skip")
          }
        ) : null
      ] })
    ] }),
    method === "password" && /* @__PURE__ */ u("form", { onSubmit: onPasswordSubmit, className: "flex flex-col gap-3", children: [
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { htmlFor: "onboarding-password", className: "text-xs text-[var(--muted)] mb-1 block", children: [
          "Password",
          localhostOnly ? "" : " *"
        ] }),
        /* @__PURE__ */ u(
          "input",
          {
            id: "onboarding-password",
            type: "password",
            name: "password",
            autoComplete: "new-password",
            className: "provider-key-input w-full",
            value: password,
            onInput: (e) => setPassword(targetValue(e)),
            placeholder: localhostOnly ? "Optional on localhost" : "At least 12 characters",
            autofocus: true
          }
        )
      ] }),
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { htmlFor: "onboarding-password-confirm", className: "text-xs text-[var(--muted)] mb-1 block", children: "Confirm password" }),
        /* @__PURE__ */ u(
          "input",
          {
            id: "onboarding-password-confirm",
            type: "password",
            name: "confirm_password",
            autoComplete: "new-password",
            className: "provider-key-input w-full",
            value: confirm,
            onInput: (e) => setConfirm(targetValue(e)),
            placeholder: "Repeat password"
          }
        )
      ] }),
      error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
        /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Setting up…" : localhostOnly && !password ? "Skip" : "Set password" }, `pw-${saving}`),
        skippable ? /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
            onClick: onNext,
            children: t("common:actions.skip")
          }
        ) : null
      ] })
    ] }),
    method === null && /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: skippable ? /* @__PURE__ */ u(
      "button",
      {
        type: "button",
        className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
        onClick: onNext,
        children: t("common:actions.skip")
      }
    ) : null })
  ] });
}
function IdentityStep({ onNext, onBack }) {
  const identityData = get("identity") || {};
  const [userName, setUserName] = d(identityData.user_name || "");
  const [name, setName] = d(identityData.name || "Moltis");
  const [emoji, setEmoji] = d(identityData.emoji || "🤖");
  const [theme, setTheme] = d(identityData.theme || "");
  const [saving, setSaving] = d(false);
  const [error, setError] = d(null);
  y(() => {
    let cancelled = false;
    refresh().then(() => {
      if (cancelled) return;
      const refreshed = get("identity") || {};
      if (refreshed.user_name) setUserName((prev) => prev || refreshed.user_name || "");
      if (refreshed.name) setName((prev) => prev && prev !== "Moltis" ? prev : refreshed.name || "");
      if (refreshed.emoji) setEmoji((prev) => prev && prev !== "🤖" ? prev : refreshed.emoji || "");
      if (refreshed.theme) setTheme((prev) => prev || refreshed.theme || "");
    });
    return () => {
      cancelled = true;
    };
  }, []);
  function onSubmit(e) {
    e.preventDefault();
    const v = validateIdentityFields(name, userName);
    if (!v.valid) {
      setError(v.error);
      return;
    }
    setError(null);
    setSaving(true);
    const userTimezone = detectBrowserTimezone();
    updateIdentity({
      name: name.trim(),
      emoji: emoji.trim() || "",
      theme: theme.trim() || "",
      user_name: userName.trim(),
      user_timezone: userTimezone || ""
    }).then((res) => {
      var _a;
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        refresh();
        onNext();
      } else {
        setError(((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to save");
      }
    });
  }
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:identity.title") }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Tell us about yourself and customise your agent." }),
    /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mb-1", children: "Your name *" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            value: userName,
            onInput: (e) => setUserName(targetValue(e)),
            placeholder: "e.g. Alice",
            autofocus: true
          }
        )
      ] }),
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-3", children: [
        /* @__PURE__ */ u("div", { className: "grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:gap-x-4", children: [
          /* @__PURE__ */ u("div", { className: "min-w-0", children: [
            /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mb-1", children: "Agent name *" }),
            /* @__PURE__ */ u(
              "input",
              {
                type: "text",
                className: "provider-key-input w-full",
                value: name,
                onInput: (e) => setName(targetValue(e)),
                placeholder: "e.g. Rex"
              }
            )
          ] }),
          /* @__PURE__ */ u("div", { children: [
            /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mb-1", children: "Emoji" }),
            /* @__PURE__ */ u(EmojiPicker, { value: emoji, onChange: setEmoji })
          ] })
        ] }),
        /* @__PURE__ */ u("div", { children: [
          /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mb-1", children: "Theme" }),
          /* @__PURE__ */ u(
            "input",
            {
              type: "text",
              className: "provider-key-input w-full",
              value: theme,
              onInput: (e) => setTheme(targetValue(e)),
              placeholder: "wise owl, chill fox, witty robot{'\\u2026'}"
            }
          )
        ] })
      ] }),
      error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
        onBack ? /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }) : null,
        /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Saving…" : "Continue" }, `id-${saving}`)
      ] })
    ] })
  ] });
}
const OPENAI_COMPATIBLE = ["openai", "mistral", "openrouter", "cerebras", "minimax", "moonshot", "venice", "ollama"];
const BYOM_PROVIDERS = ["venice"];
const RECOMMENDED_PROVIDERS = /* @__PURE__ */ new Set([
  "anthropic",
  "openai",
  "gemini",
  "deepseek",
  "minimax",
  "zai",
  "ollama",
  "local-llm",
  "lmstudio"
]);
const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;
function sortProviders(list) {
  list.sort((a, b) => {
    const aOrder = Number.isFinite(a.uiOrder) ? a.uiOrder : Number.MAX_SAFE_INTEGER;
    const bOrder = Number.isFinite(b.uiOrder) ? b.uiOrder : Number.MAX_SAFE_INTEGER;
    if (aOrder !== bOrder) return aOrder - bOrder;
    return a.displayName.localeCompare(b.displayName);
  });
  return list;
}
function normalizeProviderToken(value) {
  return String(value || "").toLowerCase().replace(/[^a-z0-9]/g, "");
}
function normalizeModelToken(value) {
  return String(value || "").trim().toLowerCase();
}
function stripModelNamespace(modelId) {
  if (!modelId || typeof modelId !== "string") return "";
  const sep = modelId.lastIndexOf("::");
  return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}
function resolveSavedModelSelection(savedModels, availableModels) {
  const selected = /* @__PURE__ */ new Set();
  if (!((savedModels == null ? void 0 : savedModels.length) && savedModels.length > 0) || availableModels.length === 0) return selected;
  const exactIdLookup = /* @__PURE__ */ new Map();
  const rawIdLookup = /* @__PURE__ */ new Map();
  for (const mdl of availableModels) {
    const id = String((mdl == null ? void 0 : mdl.id) || "").trim();
    if (!id) continue;
    exactIdLookup.set(normalizeModelToken(id), id);
    const rawId = normalizeModelToken(stripModelNamespace(id));
    if (rawId && !rawIdLookup.has(rawId)) rawIdLookup.set(rawId, id);
  }
  for (const savedModel of savedModels) {
    const savedNorm = normalizeModelToken(savedModel);
    if (!savedNorm) continue;
    const exact = exactIdLookup.get(savedNorm);
    if (exact) {
      selected.add(exact);
      continue;
    }
    const raw = normalizeModelToken(stripModelNamespace(savedModel));
    const mapped = rawIdLookup.get(raw);
    if (mapped) selected.add(mapped);
  }
  return selected;
}
function modelBelongsToProvider(providerName, mdl) {
  const needle = normalizeProviderToken(providerName);
  if (!needle) return false;
  const modelProvider = normalizeProviderToken(mdl == null ? void 0 : mdl.provider);
  if (modelProvider == null ? void 0 : modelProvider.includes(needle)) return true;
  const modelId = String((mdl == null ? void 0 : mdl.id) || "");
  const modelPrefix = normalizeProviderToken(modelId.split("::")[0]);
  return modelPrefix === needle;
}
function toModelSelectorRow(modelRow) {
  return {
    id: modelRow.id,
    displayName: modelRow.displayName || modelRow.id,
    provider: modelRow.provider,
    supportsTools: modelRow.supportsTools,
    createdAt: modelRow.createdAt || 0
  };
}
function ModelSelectCard({
  model,
  selected,
  probe,
  onToggle
}) {
  const probeError = probe && probe !== "ok" && probe !== "probing" ? probe.error || "" : "";
  return /* @__PURE__ */ u("div", { className: `model-card ${selected ? "selected" : ""}`, onClick: onToggle, children: [
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center justify-between gap-2", children: [
      /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text)]", children: model.displayName }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2 justify-end", children: [
        model.supportsTools ? /* @__PURE__ */ u("span", { className: "recommended-badge", children: "Tools" }) : null,
        probe === "probing" ? /* @__PURE__ */ u("span", { className: "tier-badge", children: [
          "Probing",
          "…"
        ] }) : null,
        probeError ? /* @__PURE__ */ u("span", { className: "provider-item-badge warning", children: "Unsupported" }) : null
      ] })
    ] }),
    /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1 font-mono", children: model.id }),
    probeError ? /* @__PURE__ */ u("div", { className: "text-xs font-medium text-[var(--danger,#ef4444)] mt-0.5", children: probeError }) : null,
    model.createdAt ? /* @__PURE__ */ u(
      "time",
      {
        className: "text-xs text-[var(--muted)] mt-0.5 opacity-60 block",
        "data-epoch-ms": model.createdAt * 1e3,
        "data-format": "year-month"
      }
    ) : null
  ] });
}
function OnboardingProviderRow(props) {
  const {
    provider,
    configuring,
    phase,
    providerModels,
    selectedModels,
    probeResults,
    modelSearch,
    setModelSearch,
    oauthProvider,
    oauthInfo,
    oauthCallbackInput,
    setOauthCallbackInput,
    oauthSubmitting,
    localProvider,
    sysInfo,
    localModels,
    selectedBackend,
    setSelectedBackend,
    apiKey,
    setApiKey,
    endpoint,
    setEndpoint,
    model,
    setModel,
    saving,
    savingModels,
    error,
    validationResult,
    onStartConfigure,
    onCancelConfigure,
    onSaveKey,
    onToggleModel,
    onSaveModels,
    onSubmitOAuthCallback,
    onCancelOAuth,
    onConfigureLocalModel,
    onCancelLocal
  } = props;
  const isApiKeyForm = configuring === provider.name && (phase === "form" || phase === "validating");
  const isModelSelect = configuring === provider.name && phase === "selectModel";
  const isOAuth = oauthProvider === provider.name;
  const isLocal = localProvider === provider.name;
  const isExpanded = isApiKeyForm || isModelSelect || isOAuth || isLocal;
  const keyInputRef = A(null);
  const rowRef = A(null);
  y(() => {
    if (isApiKeyForm && keyInputRef.current) keyInputRef.current.focus();
  }, [isApiKeyForm]);
  y(() => {
    if (isExpanded && rowRef.current) rowRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }, [isExpanded]);
  const supportsEndpoint = OPENAI_COMPATIBLE.includes(provider.name);
  const needsModel = BYOM_PROVIDERS.includes(provider.name);
  const keyHelp = providerApiKeyHelp(provider);
  const [showAllModels, setShowAllModels] = d(false);
  const DEFAULT_VISIBLE = 3;
  const sortedModels = (providerModels || []).slice().sort((a, b) => {
    const aRec = a.recommended ? 1 : 0;
    const bRec = b.recommended ? 1 : 0;
    if (aRec !== bRec) return bRec - aRec;
    const aTime = a.createdAt || 0;
    const bTime = b.createdAt || 0;
    if (aTime !== bTime) return bTime - aTime;
    const aVer = modelVersionScore(a.id);
    const bVer = modelVersionScore(b.id);
    if (aVer !== bVer) return bVer - aVer;
    return (a.displayName || a.id).localeCompare(b.displayName || b.id);
  });
  const filteredModels = sortedModels.filter(
    (m) => !modelSearch || m.displayName.toLowerCase().includes(modelSearch.toLowerCase()) || m.id.toLowerCase().includes(modelSearch.toLowerCase())
  );
  const hasMoreModels = filteredModels.length > DEFAULT_VISIBLE && !modelSearch;
  const visibleModels = showAllModels || modelSearch ? filteredModels : filteredModels.slice(0, DEFAULT_VISIBLE);
  const hiddenModelCount = filteredModels.length - DEFAULT_VISIBLE;
  return /* @__PURE__ */ u("div", { ref: rowRef, className: "rounded-md border border-[var(--border)] bg-[var(--surface)] p-3", children: [
    /* @__PURE__ */ u("div", { className: "flex items-center gap-3", children: [
      /* @__PURE__ */ u("div", { className: "flex-1 min-w-0 flex flex-col gap-0.5", children: /* @__PURE__ */ u("div", { className: "flex items-center gap-2 flex-wrap", children: [
        /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text-strong)]", children: provider.displayName }),
        provider.configured ? /* @__PURE__ */ u("span", { className: "provider-item-badge configured", children: "configured" }) : null,
        (validationResult == null ? void 0 : validationResult.ok) === true ? /* @__PURE__ */ u("span", { className: "icon icon-md icon-check-circle inline-block", style: { color: "var(--ok)" } }) : null,
        /* @__PURE__ */ u("span", { className: `provider-item-badge ${provider.authType}`, children: provider.authType === "oauth" ? "OAuth" : provider.authType === "local" ? "Local" : "API Key" })
      ] }) }),
      /* @__PURE__ */ u("div", { className: "shrink-0", children: isExpanded ? null : /* @__PURE__ */ u(
        "button",
        {
          className: "provider-btn provider-btn-secondary provider-btn-sm",
          onClick: () => onStartConfigure(provider.name),
          children: provider.configured ? "Choose Model" : "Configure"
        }
      ) })
    ] }),
    (validationResult == null ? void 0 : validationResult.ok) === false && !isExpanded ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--warning)] mt-1", children: validationResult.message }) : null,
    isApiKeyForm ? /* @__PURE__ */ u("form", { onSubmit: onSaveKey, className: "flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3", children: [
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "API Key" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "password",
            className: "provider-key-input w-full",
            ref: keyInputRef,
            value: apiKey,
            onInput: (e) => setApiKey(targetValue(e)),
            placeholder: provider.keyOptional ? "(optional)" : "sk-..."
          }
        ),
        keyHelp ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: keyHelp.url ? /* @__PURE__ */ u(S, { children: [
          keyHelp.text,
          " ",
          /* @__PURE__ */ u(
            "a",
            {
              href: keyHelp.url,
              target: "_blank",
              rel: "noopener noreferrer",
              className: "text-[var(--accent)] underline",
              children: keyHelp.label || keyHelp.url
            }
          )
        ] }) : keyHelp.text }) : null
      ] }),
      supportsEndpoint ? /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Endpoint (optional)" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            value: endpoint,
            onInput: (e) => setEndpoint(targetValue(e)),
            placeholder: provider.defaultBaseUrl || "https://api.example.com/v1"
          }
        ),
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "Leave empty to use the default endpoint." })
      ] }) : null,
      needsModel ? /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Model ID" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            value: model,
            onInput: (e) => setModel(targetValue(e)),
            placeholder: "model-id"
          }
        )
      ] }) : null,
      error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 mt-1", children: [
        /* @__PURE__ */ u(
          "button",
          {
            type: "submit",
            className: "provider-btn provider-btn-sm",
            disabled: phase === "validating",
            children: phase === "validating" ? "Saving…" : "Save"
          },
          `prov-${phase}`
        ),
        /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-secondary provider-btn-sm",
            onClick: onCancelConfigure,
            disabled: phase === "validating",
            children: "Cancel"
          }
        )
      ] }),
      phase === "validating" ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: [
        "Discovering available models",
        "…"
      ] }) : null
    ] }) : null,
    isModelSelect ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3", children: [
      /* @__PURE__ */ u("div", { className: "text-xs font-medium text-[var(--text-strong)]", children: "Select preferred models" }),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Selected models appear first in the session model selector." }),
      (providerModels || []).length > 5 ? /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full text-xs",
          placeholder: "Search models…",
          value: modelSearch,
          onInput: (e) => setModelSearch(targetValue(e))
        }
      ) : null,
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: [
        visibleModels.length === 0 ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] py-4 text-center", children: "No models match your search." }) : visibleModels.map((m) => /* @__PURE__ */ u(
          ModelSelectCard,
          {
            model: m,
            selected: selectedModels.has(m.id),
            probe: probeResults.get(m.id),
            onToggle: () => onToggleModel(m.id)
          },
          m.id
        )),
        hasMoreModels ? /* @__PURE__ */ u(
          "button",
          {
            className: "text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none py-1 text-left hover:underline",
            onClick: () => setShowAllModels(!showAllModels),
            children: showAllModels ? t("providers:showFewerModels") : t("providers:showAllModels", { count: hiddenModelCount })
          }
        ) : null
      ] }),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: selectedModels.size === 0 ? "No models selected" : `${selectedModels.size} model${selectedModels.size > 1 ? "s" : ""} selected` }),
      error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 mt-1", children: [
        /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-sm",
            disabled: selectedModels.size === 0 || savingModels,
            onClick: onSaveModels,
            children: savingModels ? "Saving…" : "Save"
          }
        ),
        /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-secondary provider-btn-sm",
            onClick: onCancelConfigure,
            disabled: savingModels,
            children: "Cancel"
          }
        )
      ] }),
      savingModels ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: [
        "Saving credentials and validating selected models",
        "…"
      ] }) : null
    ] }) : null,
    isOAuth ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3", children: [
      (oauthInfo == null ? void 0 : oauthInfo.status) === "device" ? /* @__PURE__ */ u("div", { className: "text-sm text-[var(--text)]", children: [
        "Open",
        " ",
        /* @__PURE__ */ u("a", { href: oauthInfo.uri, target: "_blank", className: "text-[var(--accent)] underline", children: oauthInfo.uri }),
        " ",
        "and enter code:",
        /* @__PURE__ */ u("strong", { className: "font-mono ml-1", children: oauthInfo.code })
      ] }) : /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: [
        "Waiting for authentication",
        "…"
      ] }),
      (oauthInfo == null ? void 0 : oauthInfo.status) === "device" ? null : /* @__PURE__ */ u(S, { children: [
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "If localhost callback fails, paste the redirect URL (or code#state) below." }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            placeholder: "http://localhost:1455/auth/callback?code=...&state=...",
            value: oauthCallbackInput,
            onInput: (event) => setOauthCallbackInput(event.target.value),
            disabled: oauthSubmitting
          }
        ),
        /* @__PURE__ */ u(
          "button",
          {
            className: "provider-btn provider-btn-secondary provider-btn-sm self-start",
            onClick: () => onSubmitOAuthCallback(provider.name),
            disabled: oauthSubmitting,
            children: oauthSubmitting ? "Submitting..." : "Submit Callback"
          }
        )
      ] }),
      error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary provider-btn-sm self-start", onClick: onCancelOAuth, children: "Cancel" })
    ] }) : null,
    isLocal ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3", children: [
      sysInfo ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-3", children: [
        /* @__PURE__ */ u("div", { className: "flex gap-3 text-xs text-[var(--muted)]", children: [
          /* @__PURE__ */ u("span", { children: [
            "RAM: ",
            sysInfo.totalRamGb,
            "GB"
          ] }),
          /* @__PURE__ */ u("span", { children: [
            "Tier: ",
            sysInfo.memoryTier
          ] }),
          sysInfo.hasGpu ? /* @__PURE__ */ u("span", { className: "text-[var(--ok)]", children: "GPU available" }) : null
        ] }),
        sysInfo.isAppleSilicon && (sysInfo.availableBackends || []).length > 0 ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: [
          /* @__PURE__ */ u("div", { className: "text-xs font-medium text-[var(--text-strong)]", children: "Backend" }),
          /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: (sysInfo.availableBackends || []).map((b) => /* @__PURE__ */ u(
            "div",
            {
              className: `backend-card ${b.id === selectedBackend ? "selected" : ""} ${b.available ? "" : "disabled"}`,
              onClick: () => {
                if (b.available) setSelectedBackend(b.id);
              },
              children: [
                /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center justify-between gap-2", children: [
                  /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text)]", children: b.name }),
                  /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2 justify-end", children: [
                    b.id === sysInfo.recommendedBackend && b.available ? /* @__PURE__ */ u("span", { className: "recommended-badge", children: "Recommended" }) : null,
                    b.available ? null : /* @__PURE__ */ u("span", { className: "tier-badge", children: "Not installed" })
                  ] })
                ] }),
                /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: b.description })
              ]
            },
            b.id
          )) })
        ] }) : null,
        /* @__PURE__ */ u("div", { className: "text-xs font-medium text-[var(--text-strong)]", children: "Select a model" }),
        /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: localModels.filter((m) => m.backend === selectedBackend).length === 0 ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] py-4 text-center", children: [
          "No models available for ",
          selectedBackend
        ] }) : localModels.filter((m) => m.backend === selectedBackend).map((mdl) => /* @__PURE__ */ u("div", { className: "model-card", onClick: () => onConfigureLocalModel(mdl), children: [
          /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center justify-between gap-2", children: [
            /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text)]", children: mdl.displayName }),
            /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2 justify-end", children: [
              /* @__PURE__ */ u("span", { className: "tier-badge", children: [
                mdl.minRamGb,
                "GB"
              ] }),
              mdl.suggested ? /* @__PURE__ */ u("span", { className: "recommended-badge", children: "Recommended" }) : null
            ] })
          ] }),
          /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: [
            "Context: ",
            (mdl.contextWindow / 1e3).toFixed(0),
            "k tokens"
          ] })
        ] }, mdl.id)) }),
        saving ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
          "Configuring",
          "…"
        ] }) : null
      ] }) : /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
        "Loading system info",
        "…"
      ] }),
      error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary provider-btn-sm self-start", onClick: onCancelLocal, children: "Cancel" })
    ] }) : null
  ] });
}
function ProviderStep({ onNext, onBack }) {
  const [providers, setProviders] = d([]);
  const [loading, setLoading] = d(true);
  const [error, setError] = d(null);
  const [showAllProviders, setShowAllProviders] = d(false);
  const [configuring, setConfiguring] = d(null);
  const [oauthProvider, setOauthProvider] = d(null);
  const [localProvider, setLocalProvider] = d(null);
  const [phase, setPhase] = d("form");
  const [providerModels, setProviderModels] = d([]);
  const [selectedModels, setSelectedModels] = d(/* @__PURE__ */ new Set());
  const [probeResults, setProbeResults] = d(/* @__PURE__ */ new Map());
  const [modelSearch, setModelSearch] = d("");
  const [savingModels, setSavingModels] = d(false);
  const [modelSelectProvider, setModelSelectProvider] = d(null);
  const [apiKey, setApiKey] = d("");
  const [endpoint, setEndpoint] = d("");
  const [model, setModel] = d("");
  const [saving, setSaving] = d(false);
  const [validationResults, setValidationResults] = d({});
  const [oauthInfo, setOauthInfo] = d(null);
  const [oauthCallbackInput, setOauthCallbackInput] = d("");
  const [oauthSubmitting, setOauthSubmitting] = d(false);
  const oauthTimerRef = A(null);
  const [sysInfo, setSysInfo] = d(null);
  const [localModels, setLocalModels] = d([]);
  const [selectedBackend, setSelectedBackend] = d(null);
  function refreshProviders() {
    return sendRpc("providers.available", {}).then((res) => {
      if (res == null ? void 0 : res.ok) setProviders(sortProviders(res.payload || []));
      return res;
    });
  }
  y(() => {
    let cancelled = false;
    let attempts = 0;
    function loadProviders() {
      if (cancelled) return;
      sendRpc("providers.available", {}).then((res) => {
        var _a, _b;
        if (cancelled) return;
        if (res == null ? void 0 : res.ok) {
          setProviders(sortProviders(res.payload || []));
          setLoading(false);
          return;
        }
        if ((((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.code) === "UNAVAILABLE" || ((_b = res == null ? void 0 : res.error) == null ? void 0 : _b.message) === "WebSocket not connected") && attempts < WS_RETRY_LIMIT) {
          attempts += 1;
          window.setTimeout(loadProviders, WS_RETRY_DELAY_MS);
          return;
        }
        setLoading(false);
      });
    }
    loadProviders();
    return () => {
      cancelled = true;
    };
  }, []);
  y(() => {
    return () => {
      if (oauthTimerRef.current) {
        clearInterval(oauthTimerRef.current);
        oauthTimerRef.current = null;
      }
    };
  }, []);
  function closeAll() {
    setConfiguring(null);
    setOauthProvider(null);
    setLocalProvider(null);
    setModelSelectProvider(null);
    setPhase("form");
    setProviderModels([]);
    setSelectedModels(/* @__PURE__ */ new Set());
    setProbeResults(/* @__PURE__ */ new Map());
    setModelSearch("");
    setSavingModels(false);
    setApiKey("");
    setEndpoint("");
    setModel("");
    setError(null);
    setOauthInfo(null);
    setOauthCallbackInput("");
    setOauthSubmitting(false);
    setSysInfo(null);
    setLocalModels([]);
    if (oauthTimerRef.current) {
      clearInterval(oauthTimerRef.current);
      oauthTimerRef.current = null;
    }
  }
  async function loadModelsForProvider(providerName) {
    const modelsRes = await sendRpc("models.list", {});
    const allModels = (modelsRes == null ? void 0 : modelsRes.ok) ? modelsRes.payload || [] : [];
    return allModels.filter((m) => modelBelongsToProvider(providerName, toModelSelectorRow(m))).map(toModelSelectorRow);
  }
  async function openModelSelectForConfiguredApiProvider(provider) {
    if (provider.authType !== "api-key" || !provider.configured) return false;
    const existingModels = await loadModelsForProvider(provider.name);
    if (existingModels.length === 0) return false;
    const saved = resolveSavedModelSelection(provider.models, existingModels);
    setModelSelectProvider(provider.name);
    setConfiguring(provider.name);
    setProviderModels(existingModels);
    setSelectedModels(saved);
    setPhase("selectModel");
    return true;
  }
  async function onStartConfigure(name) {
    closeAll();
    const p = providers.find((pr) => pr.name === name);
    if (!p) return;
    if (p.authType === "api-key") {
      setEndpoint(p.baseUrl || "");
      setModel(p.model || "");
      if (await openModelSelectForConfiguredApiProvider(p)) return;
      setConfiguring(name);
      setPhase("form");
    } else if (p.authType === "oauth") {
      startOAuth(p);
    } else if (p.authType === "local") {
      startLocal(p);
    }
  }
  function onSaveKey(e) {
    e.preventDefault();
    const p = providers.find((pr) => pr.name === configuring);
    if (!p) return;
    if (!(apiKey.trim() || p.keyOptional)) {
      setError("API key is required.");
      return;
    }
    if (BYOM_PROVIDERS.includes(p.name) && !model.trim()) {
      setError("Model ID is required.");
      return;
    }
    setError(null);
    setPhase("validating");
    const keyVal = apiKey.trim() || p.name;
    const endpointVal = endpoint.trim() || null;
    const modelVal = model.trim() || null;
    validateProviderKey(p.name, keyVal, endpointVal, modelVal).then(async (result) => {
      var _a;
      if (!result.valid) {
        setPhase("form");
        setError(result.error || "Validation failed.");
        return;
      }
      if (BYOM_PROVIDERS.includes(p.name)) {
        saveAndFinishByom(p.name, keyVal, endpointVal, modelVal);
        return;
      }
      const saveRes = await saveProviderKey(p.name, keyVal, endpointVal, modelVal);
      if (!(saveRes == null ? void 0 : saveRes.ok)) {
        setPhase("form");
        setError(((_a = saveRes == null ? void 0 : saveRes.error) == null ? void 0 : _a.message) || "Failed to save credentials.");
        return;
      }
      setProviderModels(result.models || []);
      setPhase("selectModel");
    }).catch((err) => {
      setPhase("form");
      setError((err == null ? void 0 : err.message) || "Validation failed.");
    });
  }
  function probeModelAsync(modelId) {
    setProbeResults((prev) => {
      const next = new Map(prev);
      next.set(modelId, "probing");
      return next;
    });
    testModel(modelId).then((result) => {
      setProbeResults((prev) => {
        const next = new Map(prev);
        if (isModelServiceNotConfigured(result.error || "")) next.delete(modelId);
        else
          next.set(
            modelId,
            result.ok ? "ok" : { error: humanizeProbeError(result.error || "Unsupported") }
          );
        return next;
      });
    });
  }
  function onToggleModel(modelId) {
    setSelectedModels((prev) => {
      const next = new Set(prev);
      if (next.has(modelId)) next.delete(modelId);
      else {
        next.add(modelId);
        probeModelAsync(modelId);
      }
      return next;
    });
  }
  async function onSaveSelectedModels() {
    var _a, _b;
    const providerName = modelSelectProvider || configuring;
    if (!providerName) return false;
    const modelIds = Array.from(selectedModels);
    setSavingModels(true);
    setError(null);
    try {
      if (!modelSelectProvider) {
        const p = providers.find((pr) => pr.name === providerName);
        const keyVal = apiKey.trim() || (p == null ? void 0 : p.name) || "";
        const endpointVal = endpoint.trim() || null;
        const modelVal = model.trim() || ((p == null ? void 0 : p.keyOptional) && modelIds.length > 0 ? modelIds[0] : null);
        const res2 = await saveProviderKey(providerName, keyVal, endpointVal, modelVal);
        if (!(res2 == null ? void 0 : res2.ok)) {
          setSavingModels(false);
          setError(((_a = res2 == null ? void 0 : res2.error) == null ? void 0 : _a.message) || "Failed to save credentials.");
          return false;
        }
      }
      const res = await sendRpc("providers.save_models", { provider: providerName, models: modelIds });
      if (!(res == null ? void 0 : res.ok)) {
        setSavingModels(false);
        setError(((_b = res == null ? void 0 : res.error) == null ? void 0 : _b.message) || "Failed to save model preferences.");
        return false;
      }
      if (modelIds.length > 0) localStorage.setItem("moltis-model", modelIds[0]);
      setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
      closeAll();
      refreshProviders();
      return true;
    } catch (err) {
      setSavingModels(false);
      setError((err == null ? void 0 : err.message) || "Failed to save credentials.");
      return false;
    }
  }
  async function onContinue() {
    const hasPendingModelSelection = phase === "selectModel" && (configuring || modelSelectProvider) && selectedModels.size > 0;
    if (hasPendingModelSelection) {
      const saved = await onSaveSelectedModels();
      if (!saved) return;
    }
    onNext();
  }
  function saveAndFinishByom(providerName, keyVal, endpointVal, modelVal) {
    saveProviderKey(providerName, keyVal, endpointVal, modelVal).then(async (res) => {
      var _a;
      if (!(res == null ? void 0 : res.ok)) {
        setPhase("form");
        setError(((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to save credentials.");
        return;
      }
      if (modelVal) {
        const testResult = await testModel(modelVal);
        const modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
        if (!(testResult.ok || modelServiceUnavailable)) {
          setPhase("form");
          setError(testResult.error || "Model test failed.");
          return;
        }
        await sendRpc("providers.save_models", { provider: providerName, models: [modelVal] });
        localStorage.setItem("moltis-model", modelVal);
      }
      setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
      setConfiguring(null);
      setPhase("form");
      setProviderModels([]);
      setSelectedModels(/* @__PURE__ */ new Set());
      setProbeResults(/* @__PURE__ */ new Map());
      setModelSearch("");
      setApiKey("");
      setEndpoint("");
      setModel("");
      setError(null);
      refreshProviders();
    }).catch((err) => {
      setPhase("form");
      setError((err == null ? void 0 : err.message) || "Failed to save credentials.");
    });
  }
  function startOAuth(p) {
    setOauthProvider(p.name);
    setOauthInfo({ status: "starting" });
    setOauthCallbackInput("");
    setOauthSubmitting(false);
    startProviderOAuth(p.name).then(
      (result) => {
        if (result.status === "already") onOAuthAuthenticated(p.name);
        else if (result.status === "browser") {
          window.open(result.authUrl, "_blank");
          setOauthInfo({ status: "waiting" });
          pollOAuth(p);
        } else if (result.status === "device") {
          setOauthInfo({ status: "device", uri: result.verificationUrl, code: result.userCode });
          pollOAuth(p);
        } else {
          setError(result.error || "Failed to start OAuth");
          setOauthProvider(null);
          setOauthInfo(null);
          setOauthCallbackInput("");
          setOauthSubmitting(false);
        }
      }
    );
  }
  async function onOAuthAuthenticated(providerName) {
    const provModels = await loadModelsForProvider(providerName);
    setOauthProvider(null);
    setOauthInfo(null);
    setOauthCallbackInput("");
    setOauthSubmitting(false);
    if (provModels.length > 0) {
      setModelSelectProvider(providerName);
      setConfiguring(providerName);
      setProviderModels(provModels);
      setSelectedModels(/* @__PURE__ */ new Set());
      setPhase("selectModel");
    } else setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
    refreshProviders();
  }
  function pollOAuth(p) {
    let attempts = 0;
    if (oauthTimerRef.current) clearInterval(oauthTimerRef.current);
    oauthTimerRef.current = setInterval(() => {
      attempts++;
      if (attempts > 60) {
        clearInterval(oauthTimerRef.current);
        oauthTimerRef.current = null;
        setError("OAuth timed out.");
        setOauthProvider(null);
        setOauthInfo(null);
        setOauthCallbackInput("");
        setOauthSubmitting(false);
        return;
      }
      sendRpc("providers.oauth.status", { provider: p.name }).then((res) => {
        var _a;
        if ((res == null ? void 0 : res.ok) && ((_a = res.payload) == null ? void 0 : _a.authenticated)) {
          clearInterval(oauthTimerRef.current);
          oauthTimerRef.current = null;
          onOAuthAuthenticated(p.name);
        }
      });
    }, 2e3);
  }
  function cancelOAuth() {
    if (oauthTimerRef.current) {
      clearInterval(oauthTimerRef.current);
      oauthTimerRef.current = null;
    }
    setOauthProvider(null);
    setOauthInfo(null);
    setOauthCallbackInput("");
    setOauthSubmitting(false);
    setError(null);
  }
  function submitOAuthCallback(providerName) {
    const callback = oauthCallbackInput.trim();
    if (!callback) {
      setError("Paste the callback URL (or code#state) to continue.");
      return;
    }
    setOauthSubmitting(true);
    setError(null);
    completeProviderOAuth(providerName, callback).then((res) => {
      var _a;
      if (res == null ? void 0 : res.ok) {
        if (oauthTimerRef.current) {
          clearInterval(oauthTimerRef.current);
          oauthTimerRef.current = null;
        }
        onOAuthAuthenticated(providerName);
        return;
      }
      setError(((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to complete OAuth callback.");
    }).catch((err) => {
      setError((err == null ? void 0 : err.message) || "Failed to complete OAuth callback.");
    }).finally(() => {
      setOauthSubmitting(false);
    });
  }
  function startLocal(p) {
    setLocalProvider(p.name);
    sendRpc("providers.local.system_info", {}).then((sysRes) => {
      var _a, _b;
      if (!(sysRes == null ? void 0 : sysRes.ok)) {
        setError(((_a = sysRes == null ? void 0 : sysRes.error) == null ? void 0 : _a.message) || "Failed to get system info");
        setLocalProvider(null);
        return;
      }
      setSysInfo(sysRes.payload);
      setSelectedBackend(((_b = sysRes.payload) == null ? void 0 : _b.recommendedBackend) || "GGUF");
      sendRpc("providers.local.models", {}).then((modelsRes) => {
        var _a2;
        if (modelsRes == null ? void 0 : modelsRes.ok) setLocalModels(((_a2 = modelsRes.payload) == null ? void 0 : _a2.recommended) || []);
      });
    });
  }
  function configureLocalModel(mdl) {
    const provName = localProvider;
    setSaving(true);
    setError(null);
    sendRpc("providers.local.configure", { modelId: mdl.id, backend: selectedBackend }).then((res) => {
      var _a;
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        setLocalProvider(null);
        setSysInfo(null);
        setLocalModels([]);
        setValidationResults((prev) => ({ ...prev, [provName]: { ok: true, message: null } }));
        refreshProviders();
      } else setError(((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.message) || "Failed to configure model");
    });
  }
  function cancelLocal() {
    setLocalProvider(null);
    setSysInfo(null);
    setLocalModels([]);
    setError(null);
  }
  if (loading) return /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: t("onboarding:provider.loadingLlms") });
  const configuredProviders = providers.filter((p) => p.configured);
  const recommendedProviders = providers.filter((p) => RECOMMENDED_PROVIDERS.has(p.name));
  const otherProviders = providers.filter((p) => !RECOMMENDED_PROVIDERS.has(p.name));
  const otherIsActive = otherProviders.some(
    (p) => configuring === p.name || oauthProvider === p.name || localProvider === p.name
  );
  const showOther = showAllProviders || otherIsActive;
  function renderProviderRow(p) {
    return /* @__PURE__ */ u(
      OnboardingProviderRow,
      {
        provider: p,
        configuring,
        phase: configuring === p.name ? phase : "form",
        providerModels: configuring === p.name ? providerModels : [],
        selectedModels: configuring === p.name ? selectedModels : /* @__PURE__ */ new Set(),
        probeResults: configuring === p.name ? probeResults : /* @__PURE__ */ new Map(),
        modelSearch: configuring === p.name ? modelSearch : "",
        setModelSearch,
        oauthProvider,
        oauthInfo,
        oauthCallbackInput,
        setOauthCallbackInput,
        oauthSubmitting,
        localProvider,
        sysInfo,
        localModels,
        selectedBackend,
        setSelectedBackend,
        apiKey,
        setApiKey,
        endpoint,
        setEndpoint,
        model,
        setModel,
        saving,
        savingModels,
        error: configuring === p.name || oauthProvider === p.name || localProvider === p.name ? error : null,
        validationResult: validationResults[p.name] || null,
        onStartConfigure,
        onCancelConfigure: closeAll,
        onSaveKey,
        onToggleModel,
        onSaveModels: onSaveSelectedModels,
        onSubmitOAuthCallback: submitOAuthCallback,
        onCancelOAuth: cancelOAuth,
        onConfigureLocalModel: configureLocalModel,
        onCancelLocal: cancelLocal
      },
      p.name
    );
  }
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("div", { className: "flex items-baseline justify-between gap-2", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:provider.addLlms") }),
      /* @__PURE__ */ u(
        "a",
        {
          href: "https://docs.moltis.org/choosing-a-provider.html",
          target: "_blank",
          rel: "noopener noreferrer",
          className: "text-xs text-[var(--accent)] hover:underline shrink-0",
          children: "Help me choose"
        }
      )
    ] }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Configure one or more LLM providers to power your agent. You can add more later in Settings." }),
    configuredProviders.length > 0 ? /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2", children: [
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Detected LLM providers" }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2", children: configuredProviders.map((p) => /* @__PURE__ */ u("span", { className: "provider-item-badge configured", children: p.displayName }, p.name)) })
    ] }) : null,
    /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: [
      /* @__PURE__ */ u("div", { className: "text-xs font-medium text-[var(--text)] uppercase tracking-wide", children: "Recommended" }),
      recommendedProviders.map(renderProviderRow)
    ] }),
    otherProviders.length > 0 ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: [
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "text-xs text-[var(--muted)] hover:text-[var(--text)] cursor-pointer bg-transparent border-none text-left flex items-center gap-1",
          onClick: () => setShowAllProviders((v) => !v),
          children: [
            /* @__PURE__ */ u("span", { className: `inline-block transition-transform ${showOther ? "rotate-90" : ""}`, children: "▶" }),
            "All providers (",
            otherProviders.length,
            " more)"
          ]
        }
      ),
      showOther ? otherProviders.map(renderProviderRow) : null
    ] }) : null,
    error && !configuring && !oauthProvider && !localProvider ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack || void 0, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onContinue, disabled: phase === "validating" || savingModels, children: t("common:actions.continue") }),
      /* @__PURE__ */ u(
        "button",
        {
          className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
          onClick: onNext,
          children: t("common:actions.skip")
        }
      )
    ] })
  ] });
}
function StepIndicator({ steps, current }) {
  const ref = A(null);
  y(() => {
    if (!ref.current) return;
    const active = ref.current.querySelector(".onboarding-step.active");
    if (active) active.scrollIntoView({ inline: "center", block: "nearest", behavior: "smooth" });
  }, [current]);
  return /* @__PURE__ */ u("div", { className: "onboarding-steps", ref, children: steps.map((label, i) => {
    const state = i < current ? "completed" : i === current ? "active" : "";
    const isLast = i === steps.length - 1;
    return /* @__PURE__ */ u(S, { children: [
      /* @__PURE__ */ u(StepDot, { index: i, label, state }, i),
      !isLast && /* @__PURE__ */ u("div", { className: `onboarding-step-line ${i < current ? "completed" : ""}` })
    ] });
  }) });
}
function StepDot({ index, label, state }) {
  return /* @__PURE__ */ u("div", { className: `onboarding-step ${state}`, children: [
    /* @__PURE__ */ u("div", { className: `onboarding-step-dot ${state}`, children: state === "completed" ? /* @__PURE__ */ u("span", { className: "icon icon-md icon-checkmark" }) : index + 1 }),
    /* @__PURE__ */ u("div", { className: "onboarding-step-label", children: label })
  ] });
}
function VoiceStep({ onNext, onBack }) {
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Voice (optional)" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Voice configuration step — full TSX conversion pending." }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onNext, children: t("common:actions.continue") })
    ] })
  ] });
}
function RemoteAccessStep({ onNext, onBack }) {
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Remote Access" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Remote access configuration step — full TSX conversion pending." }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onNext, children: t("common:actions.continue") })
    ] })
  ] });
}
function ChannelStep({ onNext, onBack }) {
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Connect a Channel" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Channel configuration step — full TSX conversion pending." }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onNext, children: t("common:actions.continue") }),
      /* @__PURE__ */ u(
        "button",
        {
          className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
          onClick: onNext,
          children: t("common:actions.skip")
        }
      )
    ] })
  ] });
}
function OpenClawImportStep({ onNext, onBack }) {
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Import from OpenClaw" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Import step — full TSX conversion pending." }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      onBack ? /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack, children: "Back" }) : null,
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onNext, children: "Skip" })
    ] })
  ] });
}
function SummaryStep({ onBack, onFinish }) {
  const identity = get("identity") || {};
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:summary.title") }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Overview of your configuration. You can change any of these later in Settings." }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("div", { className: "flex-1" }),
      /* @__PURE__ */ u("button", { className: "provider-btn", onClick: onFinish, children: [
        identity.emoji || "",
        " ",
        identity.name || "Your agent",
        ", reporting for duty"
      ] })
    ] })
  ] });
}
function OnboardingPage() {
  const [step, setStep] = d(-1);
  const [authNeeded, setAuthNeeded] = d(false);
  const [authSkippable, setAuthSkippable] = d(false);
  const [voiceAvailable] = d(() => get("voice_enabled") === true);
  const headerRef = A(null);
  const navRef = A(null);
  const sessionsPanelRef = A(null);
  y(() => {
    const header = document.querySelector("header");
    const nav = document.getElementById("navPanel");
    const sessions = document.getElementById("sessionsPanel");
    const burger = document.getElementById("burgerBtn");
    const toggle = document.getElementById("sessionsToggle");
    const authBanner = document.getElementById("authDisabledBanner");
    headerRef.current = header;
    navRef.current = nav;
    sessionsPanelRef.current = sessions;
    if (header) header.style.display = "none";
    if (nav) nav.style.display = "none";
    if (sessions) sessions.style.display = "none";
    if (burger) burger.style.display = "none";
    if (toggle) toggle.style.display = "none";
    if (authBanner) authBanner.style.display = "none";
    return () => {
      if (header) header.style.display = "";
      if (nav) nav.style.display = "";
      if (sessions) sessions.style.display = "";
      if (burger) burger.style.display = "";
      if (toggle) toggle.style.display = "";
    };
  }, []);
  y(() => {
    fetch("/api/auth/status").then((r) => r.ok ? r.json() : null).then((auth) => {
      if ((auth == null ? void 0 : auth.setup_required) || (auth == null ? void 0 : auth.auth_disabled) && !(auth == null ? void 0 : auth.localhost_only)) {
        setAuthNeeded(true);
        setAuthSkippable(!auth.setup_required);
        setStep(0);
      } else {
        setAuthNeeded(false);
        ensureWsConnected();
        setStep(1);
      }
    }).catch(() => {
      setAuthNeeded(false);
      ensureWsConnected();
      setStep(1);
    });
  }, []);
  if (step === -1) {
    return /* @__PURE__ */ u("div", { className: "onboarding-card", children: /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: t("common:status.loading") }) });
  }
  const openclawDetected = get("openclaw_detected") === true;
  const allLabels = [t("onboarding:steps.security")];
  if (openclawDetected) allLabels.push(t("onboarding:steps.import"));
  allLabels.push(t("onboarding:steps.llm"));
  if (voiceAvailable) allLabels.push(t("onboarding:steps.voice"));
  allLabels.push(
    t("onboarding:steps.remoteAccess"),
    t("onboarding:steps.channel"),
    t("onboarding:steps.identity"),
    t("onboarding:steps.summary")
  );
  const steps = authNeeded ? allLabels : allLabels.slice(1);
  const stepIndex = authNeeded ? step : step - 1;
  let nextIdx = 1;
  const importStep = openclawDetected ? nextIdx++ : -1;
  const llmStep = nextIdx++;
  const voiceStep = voiceAvailable ? nextIdx++ : -1;
  const remoteAccessStep = nextIdx++;
  const channelStep = nextIdx++;
  const identityStep = nextIdx++;
  const summaryStep = nextIdx;
  const lastStep = summaryStep;
  function goNext() {
    if (step === lastStep) window.location.assign(preferredChatPath());
    else setStep(step + 1);
  }
  function goFinish() {
    window.location.assign(preferredChatPath());
  }
  function goBack() {
    if (authNeeded) setStep(Math.max(0, step - 1));
    else setStep(Math.max(1, step - 1));
  }
  const startedAt = get("started_at");
  const version = String(get("version") || "").trim();
  return /* @__PURE__ */ u("div", { className: "onboarding-card", children: [
    /* @__PURE__ */ u(StepIndicator, { steps, current: stepIndex }),
    /* @__PURE__ */ u("div", { className: "mt-6", children: [
      step === 0 && /* @__PURE__ */ u(AuthStep, { onNext: goNext, skippable: authSkippable }),
      step === importStep && /* @__PURE__ */ u(OpenClawImportStep, { onNext: goNext, onBack: authNeeded ? goBack : null }),
      step === llmStep && /* @__PURE__ */ u(ProviderStep, { onNext: goNext, onBack: authNeeded || openclawDetected ? goBack : null }),
      step === voiceStep && /* @__PURE__ */ u(VoiceStep, { onNext: goNext, onBack: goBack }),
      step === remoteAccessStep && /* @__PURE__ */ u(RemoteAccessStep, { onNext: goNext, onBack: goBack }),
      step === channelStep && /* @__PURE__ */ u(ChannelStep, { onNext: goNext, onBack: goBack }),
      step === identityStep && /* @__PURE__ */ u(IdentityStep, { onNext: goNext, onBack: goBack }),
      step === summaryStep && /* @__PURE__ */ u(SummaryStep, { onBack: goBack, onFinish: goFinish })
    ] }),
    startedAt || version ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] text-center mt-4 pt-3 border-t border-[var(--border)]", children: [
      startedAt ? /* @__PURE__ */ u("span", { children: [
        "Server started ",
        /* @__PURE__ */ u("time", { "data-epoch-ms": startedAt })
      ] }) : null,
      startedAt && version ? /* @__PURE__ */ u("span", { children: [
        " ",
        "·",
        " "
      ] }) : null,
      version ? /* @__PURE__ */ u("span", { children: [
        t("onboarding:summary.versionLabel"),
        " v",
        version
      ] }) : null
    ] }) : null
  ] });
}
let containerRef = null;
function mountOnboarding(container) {
  containerRef = container;
  container.style.cssText = "display:flex;align-items:flex-start;justify-content:center;min-height:100vh;padding:max(0.75rem, env(safe-area-inset-top)) max(0.75rem, env(safe-area-inset-right)) max(0.75rem, env(safe-area-inset-bottom)) max(0.75rem, env(safe-area-inset-left));box-sizing:border-box;width:100%;max-width:100vw;overflow-x:hidden;overflow-y:auto;";
  R(/* @__PURE__ */ u(OnboardingPage, {}), container);
}
function unmountOnboarding() {
  if (containerRef) R(null, containerRef);
  containerRef = null;
}
export {
  mountOnboarding,
  unmountOnboarding
};
