import { c as connectWs, s as subscribeEvents, u } from "./ws-connect.js";
import { Z as t, aw as d, av as y, ax as A, b as sendRpc, ay as S, bq as modelVersionScore, v as activeSessionKey, aM as R } from "./theme.js";
import { H as eventListeners, t as targetValue, U as prepareCreationOptions, V as detectPasskeyName, w as channelStorageNote, v as validateChannelFields, p as parseChannelConfigPatch, b as addChannel, T as TabBar, g as get, u as defaultTeamsBaseUrl, M as MATRIX_DEFAULT_HOMESERVER, c as MATRIX_ENCRYPTION_GUIDANCE, n as normalizeMatrixAuthMode, m as matrixAuthModeGuidance, d as targetChecked, e as normalizeMatrixOwnershipMode, f as matrixOwnershipModeGuidance, h as matrixCredentialLabel, i as matrixCredentialPlaceholder, j as MATRIX_DOCS_URL, o as onEvent, s as generateWebhookSecretHex, r as buildTeamsEndpoint, k as deriveMatrixAccountId, l as normalizeMatrixOtpCooldown, I as refresh, O as EmojiPicker, P as validateIdentityFields, Q as updateIdentity, z as completeProviderOAuth, B as saveProviderKey, y as validateProviderKey, x as providerApiKeyHelp, D as testModel, E as isModelServiceNotConfigured, G as humanizeProbeError, A as startProviderOAuth, K as CATEGORY_META, L as categoryLabel, W as fetchVoiceProviders, $ as toggleVoiceProvider, a0 as saveVoiceKey, a1 as saveVoiceSettings, a4 as VOICE_COUNTERPART_IDS, X as fetchPhrase, Y as testTts, Z as decodeBase64Safe, _ as transcribeAudio, q as fetchChannelStatus } from "./voice-utils.js";
var WsEventName = /* @__PURE__ */ ((WsEventName2) => {
  WsEventName2["Chat"] = "chat";
  WsEventName2["Error"] = "error";
  WsEventName2["AuthCredentialsChanged"] = "auth.credentials_changed";
  WsEventName2["ExecApprovalRequested"] = "exec.approval.requested";
  WsEventName2["LogsEntry"] = "logs.entry";
  WsEventName2["SandboxPrepare"] = "sandbox.prepare";
  WsEventName2["SandboxImageBuild"] = "sandbox.image.build";
  WsEventName2["SandboxImageProvision"] = "sandbox.image.provision";
  WsEventName2["SandboxHostProvision"] = "sandbox.host.provision";
  WsEventName2["BrowserImagePull"] = "browser.image.pull";
  WsEventName2["LocalLlmDownload"] = "local-llm.download";
  WsEventName2["ModelsUpdated"] = "models.updated";
  WsEventName2["LocationRequest"] = "location.request";
  WsEventName2["NetworkAuditEntry"] = "network.audit.entry";
  WsEventName2["Tick"] = "tick";
  WsEventName2["Session"] = "session";
  WsEventName2["Channel"] = "channel";
  WsEventName2["Presence"] = "presence";
  WsEventName2["UpdateAvailable"] = "update.available";
  WsEventName2["McpStatus"] = "mcp.status";
  WsEventName2["HooksStatus"] = "hooks.status";
  WsEventName2["MetricsUpdate"] = "metrics.update";
  WsEventName2["SkillsInstallProgress"] = "skills.install.progress";
  WsEventName2["PushSubscriptions"] = "push.subscriptions";
  WsEventName2["NodePairRequested"] = "node.pair.requested";
  WsEventName2["NodePairResolved"] = "node.pair.resolved";
  WsEventName2["DevicePairResolved"] = "device.pair.resolved";
  WsEventName2["NodeTelemetry"] = "node.telemetry";
  return WsEventName2;
})(WsEventName || {});
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
function ChannelStorageNotice() {
  return /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]", children: [
    /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Storage note." }),
    " ",
    channelStorageNote()
  ] });
}
function AdvancedConfigPatchField({ value, onInput }) {
  return /* @__PURE__ */ u("details", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3", children: [
    /* @__PURE__ */ u("summary", { className: "cursor-pointer text-xs font-medium text-[var(--text-strong)]", children: "Advanced Config JSON" }),
    /* @__PURE__ */ u("div", { className: "mt-3 flex flex-col gap-2", children: [
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not have dedicated fields yet." }),
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Advanced config JSON patch (optional)" }),
        /* @__PURE__ */ u(
          "textarea",
          {
            name: "channel_advanced_config",
            className: "provider-key-input w-full min-h-[140px] font-mono text-xs",
            value,
            onInput: (e) => onInput(targetValue(e)),
            placeholder: '{"reply_to_message": true}'
          }
        )
      ] })
    ] })
  ] });
}
function ChannelTypeSelector({ onSelect, offered }) {
  const channelOptions = [
    ["telegram", "icon-telegram", "Telegram"],
    ["whatsapp", "icon-whatsapp", "WhatsApp"],
    ["msteams", "icon-msteams", "Microsoft Teams"],
    ["discord", "icon-discord", "Discord"],
    ["slack", "icon-slack", "Slack"],
    ["matrix", "icon-matrix", "Matrix"],
    ["nostr", "icon-nostr", "Nostr"]
  ].filter(([type]) => offered.has(type));
  return /* @__PURE__ */ u("div", { className: "grid grid-cols-2 gap-3 md:grid-cols-3", "data-testid": "channel-type-selector", children: channelOptions.map(([type, iconClass, label]) => /* @__PURE__ */ u(
    "button",
    {
      type: "button",
      className: "backend-card w-full min-h-[120px] items-center justify-center gap-4 px-4 py-8 text-center",
      onClick: () => onSelect(type),
      children: [
        /* @__PURE__ */ u("span", { className: `icon icon-xl ${iconClass}` }),
        /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text-strong)]", children: label })
      ]
    },
    type
  )) });
}
function channelDisplayLabel(type) {
  if (type === "msteams") return "Microsoft Teams";
  if (type === "discord") return "Discord";
  if (type === "slack") return "Slack";
  if (type === "whatsapp") return "WhatsApp";
  if (type === "matrix") return "Matrix";
  if (type === "nostr") return "Nostr";
  return "Telegram";
}
function ChannelSuccess({
  channelName,
  channelType: type,
  onAnother
}) {
  const label = channelDisplayLabel(type);
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--ok)] bg-[var(--surface)] p-4 flex gap-3 items-center", children: [
      /* @__PURE__ */ u("span", { className: "icon icon-lg icon-check-circle shrink-0", style: "color:var(--ok)" }),
      /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("div", { className: "text-sm font-medium text-[var(--text-strong)]", children: "Channel connected" }),
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-0.5", children: [
          channelName,
          " (",
          label,
          ") is now linked to your agent."
        ] })
      ] })
    ] }),
    type === "discord" && /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1.5", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Next steps" }),
      /* @__PURE__ */ u("span", { children: [
        "• ",
        /* @__PURE__ */ u("strong", { children: "Invite to a server:" }),
        " the invite link was shown on the previous screen. You can also generate one in the",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: "https://discord.com/developers/applications",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline",
            children: "Developer Portal"
          }
        ),
        " ",
        "→ OAuth2 → URL Generator (scope: bot, permissions: Send Messages, Attach Files, Read Message History)."
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "• ",
        /* @__PURE__ */ u("strong", { children: "DM the bot:" }),
        " search for the bot’s username in Discord and click Message. Make sure your username is in the DM allowlist."
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "• ",
        /* @__PURE__ */ u("strong", { children: "In a server:" }),
        " @mention the bot to get a response."
      ] })
    ] }),
    /* @__PURE__ */ u(
      "button",
      {
        type: "button",
        className: "text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none underline self-start",
        onClick: onAnother,
        children: "Connect another channel"
      }
    )
  ] });
}
function TelegramForm({ onConnected, error, setError }) {
  const [accountId, setAccountId] = d("");
  const [token, setToken] = d("");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [allowlist, setAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  function onSubmit(e) {
    e.preventDefault();
    const v = validateChannelFields("telegram", accountId, token);
    if (!v.valid) {
      setError(v.error);
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const allowlistEntries = allowlist.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
    const config = {
      token: token.trim(),
      dm_policy: dmPolicy,
      mention_mode: "mention",
      allowlist: allowlistEntries
    };
    Object.assign(config, advancedPatch.value);
    addChannel("telegram", accountId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(accountId.trim(), "telegram");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect bot.");
      }
    });
  }
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "How to create a Telegram bot" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Open",
        " ",
        /* @__PURE__ */ u("a", { href: "https://t.me/BotFather", target: "_blank", rel: "noopener", className: "text-[var(--accent)] underline", children: "@BotFather" }),
        " ",
        "in Telegram"
      ] }),
      /* @__PURE__ */ u("span", { children: "2. Send /newbot and follow the prompts" }),
      /* @__PURE__ */ u("span", { children: "3. Copy the bot token and paste it below" })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Bot username" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: accountId,
          onInput: (e) => setAccountId(targetValue(e)),
          placeholder: "e.g. my_assistant_bot",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "telegram_bot_username",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Bot token (from @BotFather)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: token,
          onInput: (e) => setToken(targetValue(e)),
          placeholder: "123456:ABC-DEF...",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "telegram_bot_token"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: dmPolicy,
          onChange: (e) => setDmPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only (recommended)" }),
            /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Your Telegram username(s)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: allowlist,
          onInput: (e) => setAllowlist(targetValue(e)),
          placeholder: "your_username",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "One username per line, without the @ sign. These users can DM your bot." })
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Connecting…" : "Connect Bot" })
  ] });
}
function discordInviteUrl(token) {
  if (!token) return "";
  const parts = token.split(".");
  if (parts.length < 3) return "";
  try {
    const id = atob(parts[0]);
    if (!/^\d+$/.test(id)) return "";
    return `https://discord.com/oauth2/authorize?client_id=${id}&scope=bot&permissions=100352`;
  } catch {
    return "";
  }
}
function DiscordForm({ onConnected, error, setError }) {
  const [accountId, setAccountId] = d("");
  const [token, setToken] = d("");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [allowlist, setAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  function onSubmit(e) {
    e.preventDefault();
    const v = validateChannelFields("discord", accountId, token);
    if (!v.valid) {
      setError(v.error);
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const allowlistEntries = allowlist.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
    const config = {
      token: token.trim(),
      dm_policy: dmPolicy,
      mention_mode: "mention",
      allowlist: allowlistEntries
    };
    Object.assign(config, advancedPatch.value);
    addChannel("discord", accountId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(accountId.trim(), "discord");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect bot.");
      }
    });
  }
  const inviteUrl = discordInviteUrl(token);
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "How to set up a Discord bot" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Go to the",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: "https://discord.com/developers/applications",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline",
            children: "Discord Developer Portal"
          }
        )
      ] }),
      /* @__PURE__ */ u("span", { children: "2. Create a new Application → Bot tab → copy the bot token" }),
      /* @__PURE__ */ u("span", { children: [
        "3. Enable ",
        /* @__PURE__ */ u("strong", { children: "Message Content Intent" }),
        " under Privileged Gateway Intents"
      ] }),
      /* @__PURE__ */ u("span", { children: "4. Paste the token below — an invite link will be generated automatically" }),
      /* @__PURE__ */ u("span", { children: "5. You can also DM the bot directly without adding it to a server" })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Account ID" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: accountId,
          onInput: (e) => setAccountId(targetValue(e)),
          placeholder: "e.g. my_discord_bot",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "discord_account_id",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Bot token" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: token,
          onInput: (e) => setToken(targetValue(e)),
          placeholder: "Bot token from Developer Portal",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "discord_bot_token"
        }
      )
    ] }),
    inviteUrl && /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 text-xs flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Invite bot to a server" }),
      /* @__PURE__ */ u("span", { className: "text-[var(--muted)]", children: "Open this link to add the bot (Send Messages, Attach Files, Read Message History):" }),
      /* @__PURE__ */ u("a", { href: inviteUrl, target: "_blank", rel: "noopener", className: "text-[var(--accent)] underline break-all", children: inviteUrl })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: dmPolicy,
          onChange: (e) => setDmPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only (recommended)" }),
            /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Allowed Discord username(s)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: allowlist,
          onInput: (e) => setAllowlist(targetValue(e)),
          placeholder: "your_username",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "One username per line. These users can DM your bot." })
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Connecting…" : "Connect Bot" })
  ] });
}
function NostrForm({ onConnected, error, setError }) {
  const [accountId, setAccountId] = d("");
  const [secretKey, setSecretKey] = d("");
  const [relays, setRelays] = d("wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [allowlist, setAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  function onSubmit(e) {
    e.preventDefault();
    if (!accountId.trim()) {
      setError("Account ID is required.");
      return;
    }
    if (!secretKey.trim()) {
      setError("Secret key is required.");
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const relayList = relays.split(",").map((r) => r.trim()).filter(Boolean);
    const allowlistEntries = allowlist.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
    const config = {
      secret_key: secretKey.trim(),
      relays: relayList,
      dm_policy: dmPolicy,
      allowed_pubkeys: allowlistEntries
    };
    Object.assign(config, advancedPatch.value);
    addChannel("nostr", accountId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(accountId.trim(), "nostr");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect channel.");
      }
    });
  }
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "How to set up Nostr DMs" }),
      /* @__PURE__ */ u("span", { children: "1. Generate or use an existing Nostr secret key (nsec1... or hex)" }),
      /* @__PURE__ */ u("span", { children: "2. Configure relay URLs (defaults are provided)" }),
      /* @__PURE__ */ u("span", { children: "3. Add allowed public keys (npub1... or hex) to the allowlist" }),
      /* @__PURE__ */ u("span", { children: "4. Send a DM to the bot's public key from any Nostr client" })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Account ID" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: accountId,
          onInput: (e) => setAccountId(targetValue(e)),
          placeholder: "e.g. my-nostr-bot",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "nostr_account_id",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Secret Key" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: secretKey,
          onInput: (e) => setSecretKey(targetValue(e)),
          placeholder: "nsec1... or 64-char hex",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "nostr_secret_key"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Relays (comma-separated)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: relays,
          onInput: (e) => setRelays(targetValue(e)),
          placeholder: "wss://relay.damus.io, wss://nos.lol",
          name: "nostr_relays"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u("select", { className: "channel-select w-full", value: dmPolicy, onChange: (e) => setDmPolicy(targetValue(e)), children: [
        /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only" }),
        /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
        /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
      ] })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Allowed Public Keys (one per line, npub1 or hex)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 3,
          value: allowlist,
          onInput: (e) => setAllowlist(targetValue(e)),
          placeholder: "npub1abc123...\nnpub1def456...",
          name: "nostr_allowed_pubkeys"
        }
      )
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u("div", { className: "text-xs text-[var(--error)]", children: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn self-start", disabled: saving, children: saving ? "Connecting…" : "Connect Nostr" })
  ] });
}
function fetchRemoteAccessStatus(path, featureDisabledMessage) {
  return fetch(path).then((response) => {
    const contentType = response.headers.get("content-type") || "";
    if (response.status === 404 || !contentType.includes("application/json")) {
      return {
        error: featureDisabledMessage,
        feature_disabled: true
      };
    }
    return response.json();
  }).catch((err) => ({
    error: err.message
  }));
}
function preferredPublicBaseUrl({
  ngrokStatus,
  tailscaleStatus
}) {
  const ngrokUrl = typeof (ngrokStatus == null ? void 0 : ngrokStatus.public_url) === "string" ? ngrokStatus.public_url.trim() : "";
  if (ngrokUrl) return ngrokUrl;
  const tailscaleUrl = typeof (tailscaleStatus == null ? void 0 : tailscaleStatus.url) === "string" ? tailscaleStatus.url.trim() : "";
  if ((tailscaleStatus == null ? void 0 : tailscaleStatus.mode) === "funnel" && tailscaleUrl) return tailscaleUrl;
  return "";
}
function RemoteAccessStep({ onNext, onBack }) {
  const [remoteTab, setRemoteTab] = d("tailscale");
  const [authReady, setAuthReady] = d(false);
  const [tsStatus, setTsStatus] = d(null);
  const [tsError, setTsError] = d(null);
  const [tsWarning, setTsWarning] = d(null);
  const [tsLoading, setTsLoading] = d(true);
  const [configuringTailscale, setConfiguringTailscale] = d(false);
  const [ngStatus, setNgStatus] = d(null);
  const [ngError, setNgError] = d(null);
  const [ngLoading, setNgLoading] = d(true);
  const [ngSaving, setNgSaving] = d(false);
  const [ngMsg, setNgMsg] = d(null);
  const [ngForm, setNgForm] = d({
    enabled: false,
    authtoken: "",
    domain: ""
  });
  function loadAuthStatus() {
    return fetch("/api/auth/status").then((response) => response.ok ? response.json() : null).then((data) => {
      const ready = (data == null ? void 0 : data.auth_disabled) ? false : (data == null ? void 0 : data.has_password) === true;
      setAuthReady(ready);
    }).catch(() => {
      setAuthReady(false);
    });
  }
  function loadTailscaleStatus() {
    setTsLoading(true);
    return fetchRemoteAccessStatus("/api/tailscale/status", "Tailscale feature is not enabled in this build.").then((data) => {
      setTsStatus((data == null ? void 0 : data.feature_disabled) ? null : data);
      setTsError((data == null ? void 0 : data.error) || null);
      setTsWarning((data == null ? void 0 : data.passkey_warning) || null);
      setTsLoading(false);
    }).catch((err) => {
      setTsError(err.message);
      setTsLoading(false);
    });
  }
  function loadNgrokStatus() {
    setNgLoading(true);
    return fetchRemoteAccessStatus("/api/ngrok/status", "ngrok feature is not enabled in this build.").then((data) => {
      setNgStatus((data == null ? void 0 : data.feature_disabled) ? null : data);
      setNgError((data == null ? void 0 : data.error) || null);
      setNgLoading(false);
      setNgForm((current) => ({
        enabled: Boolean(data == null ? void 0 : data.enabled),
        authtoken: current.authtoken,
        domain: current.domain || (data == null ? void 0 : data.domain) || ""
      }));
    }).catch((err) => {
      setNgError(err.message);
      setNgLoading(false);
    });
  }
  y(() => {
    loadAuthStatus();
    loadTailscaleStatus();
    loadNgrokStatus();
  }, []);
  function setTailscaleMode(mode) {
    setConfiguringTailscale(true);
    setTsError(null);
    setTsWarning(null);
    fetch("/api/tailscale/configure", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ mode })
    }).then(
      (response) => response.json().catch(() => ({})).then((data) => ({ ok: response.ok, data }))
    ).then(({ ok, data }) => {
      if (!ok || data.error) {
        setTsError(data.error || "Failed to configure Tailscale.");
      } else {
        setTsWarning(data.passkey_warning || null);
        loadTailscaleStatus();
      }
      setConfiguringTailscale(false);
    }).catch((err) => {
      setTsError(err.message);
      setConfiguringTailscale(false);
    });
  }
  function toggleTailscaleFunnel() {
    const nextMode = (tsStatus == null ? void 0 : tsStatus.mode) === "funnel" ? "off" : "funnel";
    setTailscaleMode(nextMode);
  }
  function applyNgrokConfig(nextForm, successMessage) {
    setNgSaving(true);
    setNgError(null);
    setNgMsg(null);
    fetch("/api/ngrok/config", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        enabled: nextForm.enabled,
        authtoken: nextForm.authtoken,
        clear_authtoken: false,
        domain: nextForm.domain
      })
    }).then(
      (response) => response.json().catch(() => ({})).then((data) => ({ ok: response.ok, data }))
    ).then(({ ok, data }) => {
      setNgSaving(false);
      if (!ok || data.error) {
        setNgError(data.error || "Failed to apply ngrok settings.");
        return;
      }
      const status = data.status || null;
      setNgMsg(successMessage);
      setNgStatus(status);
      setNgForm({
        enabled: Boolean(status == null ? void 0 : status.enabled),
        authtoken: "",
        domain: (status == null ? void 0 : status.domain) || nextForm.domain || ""
      });
    }).catch((err) => {
      setNgSaving(false);
      setNgError(err.message);
    });
  }
  function toggleNgrokEnabled() {
    const nextForm = {
      ...ngForm,
      enabled: !ngForm.enabled
    };
    setNgForm(nextForm);
    applyNgrokConfig(nextForm, `ngrok ${nextForm.enabled ? "enabled" : "disabled"}.`);
  }
  const tailscaleAvailable = tsStatus !== null;
  const tailscaleFunnelEnabled = (tsStatus == null ? void 0 : tsStatus.mode) === "funnel";
  const tailscaleInstalled = (tsStatus == null ? void 0 : tsStatus.installed) !== false;
  const tailscaleBlocked = !(tailscaleAvailable && tailscaleInstalled) || (tsStatus == null ? void 0 : tsStatus.tailscale_up) === false;
  const ngrokAvailable = ngStatus !== null;
  const activePublicUrl = preferredPublicBaseUrl({
    ngrokStatus: ngStatus,
    tailscaleStatus: tsStatus
  });
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Remote Access" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Public endpoints are optional for most channels, but Microsoft Teams needs one. Enable Tailscale Funnel, ngrok, or both before connecting team channels." }),
    activePublicUrl ? /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Active public URL" }),
      /* @__PURE__ */ u("a", { href: activePublicUrl, target: "_blank", rel: "noopener", className: "text-[var(--accent)] underline break-all", children: activePublicUrl }),
      /* @__PURE__ */ u("span", { children: "The Teams webhook step will prefill this URL." })
    ] }) : /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]", children: "Teams webhooks need a public URL. If you skip this step, you can still configure remote access later in Settings." }),
    /* @__PURE__ */ u(
      TabBar,
      {
        tabs: [
          {
            id: "tailscale",
            label: "Tailscale",
            badge: tsLoading ? void 0 : tailscaleFunnelEnabled ? "funnel" : void 0
          },
          { id: "ngrok", label: "ngrok", badge: ngLoading ? void 0 : ngForm.enabled ? "on" : void 0 }
        ],
        active: remoteTab,
        onChange: setRemoteTab
      }
    ),
    remoteTab === "tailscale" && /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Public HTTPS through Tailscale. Tailscale Serve is tailnet-only, so Teams webhooks need Funnel instead." }),
      tsLoading ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Loading Tailscale status…" }) : /* @__PURE__ */ u("div", { className: "text-sm text-[var(--text-strong)]", children: [
        "Tailscale Funnel is ",
        tailscaleFunnelEnabled ? "enabled" : "disabled",
        "."
      ] }),
      (tsStatus == null ? void 0 : tsStatus.url) && tailscaleFunnelEnabled ? /* @__PURE__ */ u(
        "a",
        {
          href: tsStatus.url,
          target: "_blank",
          rel: "noopener",
          className: "text-sm text-[var(--accent)] underline break-all",
          children: tsStatus.url
        }
      ) : null,
      tsError ? /* @__PURE__ */ u(ErrorPanel, { message: tsError }) : null,
      tsWarning ? /* @__PURE__ */ u("div", { className: "alert-warning-text max-w-form", children: tsWarning }) : null,
      (tsStatus == null ? void 0 : tsStatus.installed) === false ? /* @__PURE__ */ u(
        "a",
        {
          href: "https://tailscale.com/download",
          target: "_blank",
          rel: "noopener",
          className: "provider-btn self-start no-underline",
          children: "Install Tailscale"
        }
      ) : null,
      (tsStatus == null ? void 0 : tsStatus.tailscale_up) === false ? /* @__PURE__ */ u("div", { className: "alert-warning-text max-w-form", children: [
        /* @__PURE__ */ u("span", { className: "alert-label-warn", children: "Warning:" }),
        " Start Tailscale before enabling Funnel."
      ] }) : null,
      authReady ? null : /* @__PURE__ */ u("div", { className: "alert-warning-text max-w-form", children: [
        /* @__PURE__ */ u("span", { className: "alert-label-warn", children: "Warning:" }),
        " Funnel can be enabled now, but remote visitors will see the setup-required page until authentication is configured."
      ] }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn self-start",
          disabled: tsLoading || configuringTailscale || tailscaleBlocked,
          onClick: toggleTailscaleFunnel,
          children: configuringTailscale ? "Applying…" : tailscaleFunnelEnabled ? "Disable Funnel" : "Enable Funnel"
        }
      )
    ] }),
    remoteTab === "ngrok" && /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Public HTTPS without installing an external binary. This is useful for demos, shared testing, and Teams." }),
      ngLoading ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Loading ngrok status…" }) : /* @__PURE__ */ u("div", { className: "text-sm text-[var(--text-strong)]", children: [
        "ngrok is ",
        ngForm.enabled ? "enabled" : "disabled",
        "."
      ] }),
      (ngStatus == null ? void 0 : ngStatus.public_url) ? /* @__PURE__ */ u(
        "a",
        {
          href: ngStatus.public_url,
          target: "_blank",
          rel: "noopener",
          className: "text-sm text-[var(--accent)] underline break-all",
          children: ngStatus.public_url
        }
      ) : null,
      ngError ? /* @__PURE__ */ u(ErrorPanel, { message: ngError }) : null,
      (ngStatus == null ? void 0 : ngStatus.passkey_warning) ? /* @__PURE__ */ u("div", { className: "alert-warning-text max-w-form", children: ngStatus.passkey_warning }) : null,
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)]", htmlFor: "onboarding-ngrok-authtoken", children: "Authtoken" }),
        /* @__PURE__ */ u(
          "input",
          {
            id: "onboarding-ngrok-authtoken",
            type: "password",
            className: "provider-key-input w-full",
            placeholder: (ngStatus == null ? void 0 : ngStatus.authtoken_source) ? "Leave blank to keep the current token" : "Paste your ngrok authtoken",
            value: ngForm.authtoken,
            onInput: (e) => setNgForm({ ...ngForm, authtoken: targetValue(e) })
          }
        ),
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
          "Create or copy an authtoken from",
          " ",
          /* @__PURE__ */ u(
            "a",
            {
              href: "https://dashboard.ngrok.com/get-started/your-authtoken",
              target: "_blank",
              rel: "noopener",
              className: "text-[var(--accent)] underline",
              children: "ngrok dashboard"
            }
          ),
          "."
        ] })
      ] }),
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)]", htmlFor: "onboarding-ngrok-domain", children: "Reserved domain (optional)" }),
        /* @__PURE__ */ u(
          "input",
          {
            id: "onboarding-ngrok-domain",
            type: "text",
            className: "provider-key-input w-full",
            placeholder: "team-gateway.ngrok.app",
            value: ngForm.domain,
            onInput: (e) => setNgForm({ ...ngForm, domain: targetValue(e) })
          }
        ),
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Use a reserved domain if you want a stable public hostname." })
      ] }),
      ngMsg ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--ok)]", children: ngMsg }) : null,
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn self-start",
          disabled: !ngrokAvailable || ngLoading || ngSaving,
          onClick: toggleNgrokEnabled,
          children: ngSaving ? "Applying…" : ngForm.enabled ? "Disable ngrok" : "Enable ngrok"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: t("common:actions.continue") }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
          onClick: onNext,
          children: "Skip for now"
        }
      )
    ] })
  ] });
}
function MatrixForm({ onConnected, error, setError }) {
  const [homeserver, setHomeserver] = d(MATRIX_DEFAULT_HOMESERVER);
  const [authMode, setAuthMode] = d("password");
  const [userId, setUserId] = d("");
  const [credential, setCredential] = d("");
  const [deviceDisplayName, setDeviceDisplayName] = d("");
  const [ownershipMode, setOwnershipMode] = d("moltis_owned");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [roomPolicy, setRoomPolicy] = d("allowlist");
  const [mentionMode, setMentionMode] = d("mention");
  const [autoJoin, setAutoJoin] = d("always");
  const [otpSelfApproval, setOtpSelfApproval] = d(true);
  const [otpCooldown, setOtpCooldown] = d("300");
  const [userAllowlist, setUserAllowlist] = d("");
  const [roomAllowlist, setRoomAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  function splitLines(value) {
    return value.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
  }
  function onSubmit(e) {
    e.preventDefault();
    const accountId = deriveMatrixAccountId({ userId, homeserver });
    const v = validateChannelFields("matrix", accountId, credential, {
      matrixAuthMode: authMode,
      matrixUserId: userId
    });
    if (!v.valid) {
      setError(v.error);
      return;
    }
    if (!homeserver.trim()) {
      setError("Homeserver URL is required.");
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const config = {
      homeserver: homeserver.trim(),
      ownership_mode: normalizeMatrixAuthMode(authMode) === "password" ? normalizeMatrixOwnershipMode(ownershipMode) : "user_managed",
      dm_policy: dmPolicy,
      room_policy: roomPolicy,
      mention_mode: mentionMode,
      auto_join: autoJoin,
      otp_self_approval: otpSelfApproval,
      otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldown),
      user_allowlist: splitLines(userAllowlist),
      room_allowlist: splitLines(roomAllowlist)
    };
    if (normalizeMatrixAuthMode(authMode) === "password") {
      config.password = credential.trim();
    } else {
      config.access_token = credential.trim();
    }
    if (userId.trim()) config.user_id = userId.trim();
    if (deviceDisplayName.trim()) config.device_display_name = deviceDisplayName.trim();
    Object.assign(config, advancedPatch.value);
    addChannel("matrix", accountId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(accountId.trim(), "matrix");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect Matrix.");
      }
    });
  }
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Connect a Matrix bot user" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Leave the homeserver as ",
        /* @__PURE__ */ u("span", { className: "font-mono", children: MATRIX_DEFAULT_HOMESERVER }),
        " for matrix.org accounts"
      ] }),
      /* @__PURE__ */ u("span", { children: "2. Password is the default because it supports encrypted Matrix chats. Access token auth is only for plain Matrix traffic" }),
      /* @__PURE__ */ u("span", { children: "3. Moltis generates the local account ID automatically from the Matrix user or homeserver" })
    ] }),
    /* @__PURE__ */ u("div", { className: "rounded-md border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs text-emerald-100 flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-emerald-50", children: "Encrypted chats require password auth" }),
      /* @__PURE__ */ u("span", { children: MATRIX_ENCRYPTION_GUIDANCE })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Homeserver URL" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: homeserver,
          onInput: (e) => setHomeserver(targetValue(e)),
          placeholder: MATRIX_DEFAULT_HOMESERVER,
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "matrix_homeserver",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Authentication" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: authMode,
          onChange: (e) => setAuthMode(normalizeMatrixAuthMode(targetValue(e))),
          children: [
            /* @__PURE__ */ u("option", { value: "password", children: "Password" }),
            /* @__PURE__ */ u("option", { value: "access_token", children: "Access token" })
          ]
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: matrixAuthModeGuidance(authMode) })
    ] }),
    authMode === "password" ? /* @__PURE__ */ u("label", { className: "flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3", children: [
      /* @__PURE__ */ u(
        "input",
        {
          type: "checkbox",
          "aria-label": "Let Moltis own this Matrix account",
          checked: normalizeMatrixOwnershipMode(ownershipMode) === "moltis_owned",
          onChange: (e) => setOwnershipMode(targetChecked(e) ? "moltis_owned" : "user_managed")
        }
      ),
      /* @__PURE__ */ u("span", { className: "flex flex-col gap-1", children: [
        /* @__PURE__ */ u("span", { className: "text-xs font-medium text-[var(--text-strong)]", children: "Let Moltis own this Matrix account" }),
        /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: matrixOwnershipModeGuidance(authMode, ownershipMode) })
      ] })
    ] }) : /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: matrixOwnershipModeGuidance(authMode, "user_managed") }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: [
        "Matrix User ID",
        authMode === "password" ? " (required)" : " (optional)"
      ] }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: userId,
          onInput: (e) => setUserId(targetValue(e)),
          placeholder: "@bot:example.com",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "matrix_user_id"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: matrixCredentialLabel(authMode) }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: credential,
          onInput: (e) => setCredential(targetValue(e)),
          placeholder: matrixCredentialPlaceholder(authMode),
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "matrix_credential"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: [
        authMode === "password" ? /* @__PURE__ */ u(S, { children: "Use the password for the dedicated Matrix bot account. This is the required mode for encrypted Matrix chats because Moltis needs to create and persist its own Matrix device keys." }) : /* @__PURE__ */ u(S, { children: [
          "Get the access token in Element:",
          " ",
          /* @__PURE__ */ u("span", { className: "font-mono", children: "Settings -> Help & About -> Advanced -> Access Token" }),
          ". Access token mode does ",
          /* @__PURE__ */ u("span", { className: "font-medium", children: "not" }),
          " support encrypted Matrix chats because Moltis cannot import that existing device's private encryption keys."
        ] }),
        " ",
        /* @__PURE__ */ u("a", { href: MATRIX_DOCS_URL, target: "_blank", rel: "noreferrer", className: "text-[var(--accent)] underline", children: "Matrix setup docs" })
      ] })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Device Display Name (optional)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: deviceDisplayName,
          onInput: (e) => setDeviceDisplayName(targetValue(e)),
          placeholder: "Moltis Matrix Bot",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "matrix_device_display_name"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: dmPolicy,
          onChange: (e) => setDmPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only (recommended)" }),
            /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Room Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: roomPolicy,
          onChange: (e) => setRoomPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Room allowlist only (recommended)" }),
            /* @__PURE__ */ u("option", { value: "open", children: "Open (any joined room)" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Room Mention Mode" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: mentionMode,
          onChange: (e) => setMentionMode(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "mention", children: "Must mention bot" }),
            /* @__PURE__ */ u("option", { value: "always", children: "Always respond" }),
            /* @__PURE__ */ u("option", { value: "none", children: "Never respond in rooms" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Invite Auto-Join" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: autoJoin,
          onChange: (e) => setAutoJoin(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "always", children: "Always join invites" }),
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Only when inviter or room is allowlisted" }),
            /* @__PURE__ */ u("option", { value: "off", children: "Do not auto-join" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Unknown DM Approval" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: otpSelfApproval ? "on" : "off",
          onChange: (e) => setOtpSelfApproval(targetValue(e) !== "off"),
          children: [
            /* @__PURE__ */ u("option", { value: "on", children: "PIN challenge enabled (recommended)" }),
            /* @__PURE__ */ u("option", { value: "off", children: "Reject unknown DMs without a PIN" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "PIN Cooldown Seconds" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "number",
          min: "1",
          step: "1",
          className: "provider-key-input w-full",
          value: otpCooldown,
          onInput: (e) => setOtpCooldown(targetValue(e)),
          name: "matrix_otp_cooldown_secs"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default." })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Allowlist (Matrix user IDs)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: userAllowlist,
          onInput: (e) => setUserAllowlist(targetValue(e)),
          placeholder: "@alice:example.com",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Room Allowlist (room IDs or aliases)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: roomAllowlist,
          onInput: (e) => setRoomAllowlist(targetValue(e)),
          placeholder: "!room:example.com",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      )
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Connecting…" : "Connect Matrix" })
  ] });
}
function WhatsAppForm({ onConnected, error, setError }) {
  const [accountId, setAccountId] = d("");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [allowlist, setAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  const [pairingStarted, setPairingStarted] = d(false);
  const [qrData, setQrData] = d(null);
  const [qrSvg, setQrSvg] = d(null);
  const [qrSvgUrl, setQrSvgUrl] = d(null);
  const [pairingError, setPairingError] = d(null);
  const unsubRef = A(null);
  const hadQrRef = A(false);
  y(() => {
    return () => {
      if (unsubRef.current) unsubRef.current();
    };
  }, []);
  y(() => {
    if (!pairingStarted) return void 0;
    const id = accountId.trim() || "main";
    const timer = setInterval(async () => {
      var _a, _b, _c;
      try {
        const res = await sendRpc("channels.status", {});
        if (!(res == null ? void 0 : res.ok)) return;
        const ch = (((_a = res.payload) == null ? void 0 : _a.channels) || []).find((c) => c.type === "whatsapp" && c.account_id === id);
        if (!ch) return;
        if (ch.status === "connected") {
          onConnected(id, "whatsapp");
          return;
        }
        if (hadQrRef.current && !((_b = ch.extra) == null ? void 0 : _b.qr_data)) {
          onConnected(id, "whatsapp");
          return;
        }
        if ((_c = ch.extra) == null ? void 0 : _c.qr_data) {
          hadQrRef.current = true;
          setQrData(ch.extra.qr_data);
          if (ch.extra.qr_svg) setQrSvg(ch.extra.qr_svg);
        }
      } catch (_e) {
      }
    }, 2e3);
    return () => clearInterval(timer);
  }, [pairingStarted]);
  y(() => {
    if (!qrSvg) {
      setQrSvgUrl(null);
      return void 0;
    }
    let nextUrl = null;
    try {
      nextUrl = URL.createObjectURL(new Blob([qrSvg], { type: "image/svg+xml" }));
      setQrSvgUrl(nextUrl);
    } catch (_err) {
      setQrSvgUrl(null);
    }
    return () => {
      if (nextUrl) URL.revokeObjectURL(nextUrl);
    };
  }, [qrSvg]);
  function onStartPairing(e) {
    e.preventDefault();
    const id = accountId.trim() || "main";
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    setQrData(null);
    setQrSvg(null);
    setPairingError(null);
    if (unsubRef.current) unsubRef.current();
    unsubRef.current = onEvent(WsEventName.Channel, (p) => {
      if (p.account_id !== id) return;
      if (p.kind === "pairing_qr_code") {
        setQrData(p.qr_data);
        setQrSvg(p.qr_svg || null);
      }
      if (p.kind === "pairing_complete") onConnected(id, "whatsapp");
      if (p.kind === "pairing_failed") setPairingError(p.reason || "Pairing failed");
    });
    const allowlistEntries = allowlist.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
    const config = { dm_policy: dmPolicy, allowlist: allowlistEntries };
    Object.assign(config, advancedPatch.value);
    addChannel("whatsapp", id, config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        setPairingStarted(true);
      } else {
        if (unsubRef.current) {
          unsubRef.current();
          unsubRef.current = null;
        }
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to start pairing.");
      }
    });
  }
  if (pairingStarted) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4 items-center", children: [
      pairingError ? /* @__PURE__ */ u(ErrorPanel, { message: pairingError }) : qrData ? /* @__PURE__ */ u(
        "div",
        {
          className: "rounded-lg bg-white p-3",
          style: "width:200px;height:200px;display:flex;align-items:center;justify-content:center;",
          children: qrSvgUrl ? /* @__PURE__ */ u("img", { src: qrSvgUrl, alt: "WhatsApp pairing QR code", style: "width:100%;height:100%;display:block;" }) : /* @__PURE__ */ u("div", { className: "text-center text-xs text-gray-600", children: /* @__PURE__ */ u("div", { style: "font-family:monospace;font-size:9px;word-break:break-all;max-height:180px;overflow:hidden;", children: qrData.substring(0, 200) }) })
        }
      ) : /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: "Waiting for QR code..." }),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] text-center", children: "Scan the QR code from your terminal, or open WhatsApp > Settings > Linked Devices > Link a Device." }),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] text-center italic", children: "Only new messages will be processed. Past conversations are not synced." })
    ] });
  }
  return /* @__PURE__ */ u("form", { onSubmit: onStartPairing, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Link your WhatsApp" }),
      /* @__PURE__ */ u("span", { children: '1. Click "Start Pairing" to generate a QR code' }),
      /* @__PURE__ */ u("span", { children: "2. Open WhatsApp > Settings > Linked Devices > Link a Device" }),
      /* @__PURE__ */ u("span", { children: "3. Scan the QR code to connect" }),
      /* @__PURE__ */ u("span", { className: "mt-1 italic", children: "Only new messages will be processed — past conversations are not synced." })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Account ID (optional)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: accountId,
          onInput: (e) => setAccountId(targetValue(e)),
          placeholder: "main",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "whatsapp_account_id"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: dmPolicy,
          onChange: (e) => setDmPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Allowlist (optional)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: allowlist,
          onInput: (e) => setAllowlist(targetValue(e)),
          placeholder: "phone number or identifier",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: 'One per line. Only needed if DM policy is "Allowlist only".' })
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Starting…" : "Start Pairing" })
  ] });
}
function SlackForm({ onConnected, error, setError }) {
  const [accountId, setAccountId] = d("");
  const [botToken, setBotToken] = d("");
  const [connectionMode, setConnectionMode] = d("socket_mode");
  const [appToken, setAppToken] = d("");
  const [signingSecret, setSigningSecret] = d("");
  const [dmPolicy, setDmPolicy] = d("allowlist");
  const [allowlist, setAllowlist] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  function onSubmit(e) {
    e.preventDefault();
    if (!accountId.trim()) {
      setError("Account ID is required.");
      return;
    }
    if (!botToken.trim()) {
      setError("Bot Token is required.");
      return;
    }
    if (connectionMode === "socket_mode" && !appToken.trim()) {
      setError("App Token is required for Socket Mode.");
      return;
    }
    if (connectionMode === "events_api" && !signingSecret.trim()) {
      setError("Signing Secret is required for Events API mode.");
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const allowlistEntries = allowlist.trim().split(/\n/).map((s) => s.trim()).filter(Boolean);
    const config = {
      bot_token: botToken.trim(),
      connection_mode: connectionMode,
      dm_policy: dmPolicy,
      mention_mode: "mention",
      allowlist: allowlistEntries
    };
    if (connectionMode === "socket_mode") config.app_token = appToken.trim();
    if (connectionMode === "events_api") config.signing_secret = signingSecret.trim();
    Object.assign(config, advancedPatch.value);
    addChannel("slack", accountId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(accountId.trim(), "slack");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect Slack.");
      }
    });
  }
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "How to set up a Slack bot" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Go to",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: "https://api.slack.com/apps",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline",
            children: "api.slack.com/apps"
          }
        ),
        " ",
        "and create a new app"
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "2. Under OAuth & Permissions, add bot scopes: ",
        /* @__PURE__ */ u("code", { className: "text-[var(--accent)]", children: "chat:write" }),
        ",",
        " ",
        /* @__PURE__ */ u("code", { className: "text-[var(--accent)]", children: "channels:history" }),
        ",",
        " ",
        /* @__PURE__ */ u("code", { className: "text-[var(--accent)]", children: "im:history" }),
        ",",
        " ",
        /* @__PURE__ */ u("code", { className: "text-[var(--accent)]", children: "app_mentions:read" })
      ] }),
      /* @__PURE__ */ u("span", { children: "3. Install the app to your workspace and copy the Bot User OAuth Token" }),
      /* @__PURE__ */ u("span", { children: [
        "4. For Socket Mode: enable it and generate an App-Level Token with",
        " ",
        /* @__PURE__ */ u("code", { className: "text-[var(--accent)]", children: "connections:write" }),
        " scope"
      ] }),
      /* @__PURE__ */ u("span", { children: "5. For Events API: set the Request URL to your server’s webhook endpoint" })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Account ID" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: accountId,
          onInput: (e) => setAccountId(targetValue(e)),
          placeholder: "e.g. my-slack-bot",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "slack_account_id",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Bot Token (xoxb-...)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: botToken,
          onInput: (e) => setBotToken(targetValue(e)),
          placeholder: "xoxb-...",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "slack_bot_token"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Connection Mode" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: connectionMode,
          onChange: (e) => setConnectionMode(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "socket_mode", children: "Socket Mode (recommended)" }),
            /* @__PURE__ */ u("option", { value: "events_api", children: "Events API (HTTP webhook)" })
          ]
        }
      )
    ] }),
    connectionMode === "socket_mode" && /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "App Token (xapp-...)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: appToken,
          onInput: (e) => setAppToken(targetValue(e)),
          placeholder: "xapp-...",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "slack_app_token"
        }
      )
    ] }),
    connectionMode === "events_api" && /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Signing Secret" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: signingSecret,
          onInput: (e) => setSigningSecret(targetValue(e)),
          placeholder: "Signing secret from Basic Information",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "slack_signing_secret"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "DM Policy" }),
      /* @__PURE__ */ u(
        "select",
        {
          className: "provider-key-input w-full cursor-pointer",
          value: dmPolicy,
          onChange: (e) => setDmPolicy(targetValue(e)),
          children: [
            /* @__PURE__ */ u("option", { value: "allowlist", children: "Allowlist only (recommended)" }),
            /* @__PURE__ */ u("option", { value: "open", children: "Open (anyone)" }),
            /* @__PURE__ */ u("option", { value: "disabled", children: "Disabled" })
          ]
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Allowed Slack user(s)" }),
      /* @__PURE__ */ u(
        "textarea",
        {
          className: "provider-key-input w-full",
          rows: 2,
          value: allowlist,
          onInput: (e) => setAllowlist(targetValue(e)),
          placeholder: "slack_username",
          style: "resize:vertical;font-family:var(--font-body);"
        }
      ),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "One per line. These users can DM your bot." })
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Connecting…" : "Connect Slack" })
  ] });
}
function TeamsForm({ onConnected, error, setError }) {
  const [appId, setAppId] = d("");
  const [appPassword, setAppPassword] = d("");
  const [webhookSecret, setWebhookSecret] = d("");
  const [baseUrl, setBaseUrl] = d(defaultTeamsBaseUrl());
  const [bootstrapEndpoint, setBootstrapEndpoint] = d("");
  const [advancedConfig, setAdvancedConfig] = d("");
  const [saving, setSaving] = d(false);
  y(() => {
    let cancelled = false;
    const currentDefault = defaultTeamsBaseUrl();
    if (baseUrl !== currentDefault) return void 0;
    Promise.all([
      fetchRemoteAccessStatus("/api/ngrok/status", "ngrok feature is not enabled in this build."),
      fetchRemoteAccessStatus("/api/tailscale/status", "Tailscale feature is not enabled in this build.")
    ]).then(([nextNgrokStatus, nextTailscaleStatus]) => {
      if (cancelled) return;
      const nextPublicBaseUrl = preferredPublicBaseUrl({
        ngrokStatus: nextNgrokStatus,
        tailscaleStatus: nextTailscaleStatus
      });
      if (nextPublicBaseUrl) setBaseUrl(nextPublicBaseUrl);
    });
    return () => {
      cancelled = true;
    };
  }, [baseUrl]);
  function onBootstrap() {
    const id = appId.trim();
    if (!id) {
      setError("Enter App ID first.");
      return;
    }
    let secret = webhookSecret.trim();
    if (!secret) {
      secret = generateWebhookSecretHex();
      setWebhookSecret(secret);
    }
    const endpoint = buildTeamsEndpoint(baseUrl, id, secret);
    if (!endpoint) {
      setError("Enter a valid public base URL (e.g. https://bot.example.com).");
      return;
    }
    setBootstrapEndpoint(endpoint);
    setError(null);
  }
  function onCopyEndpoint() {
    var _a;
    if (!bootstrapEndpoint) return;
    if (typeof navigator !== "undefined" && ((_a = navigator.clipboard) == null ? void 0 : _a.writeText))
      navigator.clipboard.writeText(bootstrapEndpoint);
  }
  function onSubmit(e) {
    e.preventDefault();
    const v = validateChannelFields("msteams", appId, appPassword);
    if (!v.valid) {
      setError(v.error);
      return;
    }
    const advancedPatch = parseChannelConfigPatch(advancedConfig);
    if (!advancedPatch.ok) {
      setError(advancedPatch.error);
      return;
    }
    setError(null);
    setSaving(true);
    const config = {
      app_id: appId.trim(),
      app_password: appPassword.trim(),
      dm_policy: "allowlist",
      mention_mode: "mention",
      allowlist: []
    };
    if (webhookSecret.trim()) config.webhook_secret = webhookSecret.trim();
    Object.assign(config, advancedPatch.value);
    addChannel("msteams", appId.trim(), config).then((res) => {
      setSaving(false);
      if (res == null ? void 0 : res.ok) {
        onConnected(appId.trim(), "msteams");
      } else {
        setError((res == null ? void 0 : res.error) && (res.error.message || res.error.detail) || "Failed to connect channel.");
      }
    });
  }
  const isLocalUrl = !baseUrl || /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\])/i.test(baseUrl) || baseUrl === defaultTeamsBaseUrl();
  return /* @__PURE__ */ u("form", { onSubmit, className: "flex flex-col gap-3", children: [
    isLocalUrl && /* @__PURE__ */ u("div", { className: "rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs flex flex-col gap-1", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "Public URL required" }),
      /* @__PURE__ */ u("span", { className: "text-[var(--muted)]", children: [
        "Teams sends messages via webhook — your server must be reachable over HTTPS. Set up a tunnel in the previous ",
        /* @__PURE__ */ u("strong", { children: "Remote Access" }),
        " step, or enter a public URL below."
      ] })
    ] }),
    /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-2", children: [
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)]", children: "How to create a Teams bot" }),
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)] text-[10px] opacity-70", children: "Option A: Teams Developer Portal (easiest)" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Open",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: "https://dev.teams.microsoft.com/bots",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline",
            children: "Teams Developer Portal → Bot Management"
          }
        )
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "2. Click ",
        /* @__PURE__ */ u("strong", { children: "+ New Bot" }),
        ", give it a name, and click ",
        /* @__PURE__ */ u("strong", { children: "Add" })
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "3. Go to ",
        /* @__PURE__ */ u("strong", { children: "Configure" }),
        " — copy the ",
        /* @__PURE__ */ u("strong", { children: "Bot ID" }),
        " (this is your App ID)"
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "4. Under ",
        /* @__PURE__ */ u("strong", { children: "Client secrets" }),
        ", click ",
        /* @__PURE__ */ u("strong", { children: "Add a client secret" }),
        " and copy the value"
      ] }),
      /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text-strong)] text-[10px] opacity-70 mt-1", children: "Option B: Azure Portal" }),
      /* @__PURE__ */ u("span", { children: [
        "1. Go to",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: "https://portal.azure.com/#create/Microsoft.AzureBot",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline",
            children: "Azure Portal → Create Azure Bot"
          }
        )
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "2. Create the bot, then go to ",
        /* @__PURE__ */ u("strong", { children: "Configuration" }),
        " to find the App ID"
      ] }),
      /* @__PURE__ */ u("span", { children: [
        "3. Click ",
        /* @__PURE__ */ u("strong", { children: "Manage Password" }),
        " → ",
        /* @__PURE__ */ u("strong", { children: "New client secret" }),
        " to get the App Password"
      ] }),
      /* @__PURE__ */ u("span", { className: "mt-1", children: [
        "After creating the bot, generate the endpoint below and paste it as the ",
        /* @__PURE__ */ u("strong", { children: "Messaging endpoint" }),
        " in your bot settings."
      ] })
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "App ID (Bot ID from Azure)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: appId,
          onInput: (e) => setAppId(targetValue(e)),
          placeholder: "e.g. 12345678-abcd-efgh-ijkl-000000000000",
          autoComplete: "off",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "teams_app_id",
          autoFocus: true
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "App Password (client secret from Azure)" }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "password",
          className: "provider-key-input w-full",
          value: appPassword,
          onInput: (e) => setAppPassword(targetValue(e)),
          placeholder: "Client secret value",
          autoComplete: "new-password",
          autoCapitalize: "none",
          autoCorrect: "off",
          spellcheck: false,
          name: "teams_app_password"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: [
        "Webhook Secret ",
        /* @__PURE__ */ u("span", { className: "opacity-60", children: "(optional — auto-generated if blank)" })
      ] }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: webhookSecret,
          onInput: (e) => setWebhookSecret(targetValue(e)),
          placeholder: "Leave blank to auto-generate"
        }
      )
    ] }),
    /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: [
        "Public Base URL ",
        /* @__PURE__ */ u("span", { className: "opacity-60", children: "(your server’s HTTPS address)" })
      ] }),
      /* @__PURE__ */ u(
        "input",
        {
          type: "text",
          className: "provider-key-input w-full",
          value: baseUrl,
          onInput: (e) => setBaseUrl(targetValue(e)),
          placeholder: "https://bot.example.com"
        }
      ),
      isLocalUrl && /* @__PURE__ */ u("div", { className: "text-[10px] text-amber-600 mt-1", children: "This looks like a local address. Teams webhooks need a publicly reachable HTTPS URL." })
    ] }),
    /* @__PURE__ */ u("div", { className: "flex gap-2", children: [
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-sm provider-btn-secondary", onClick: onBootstrap, children: "Generate Endpoint" }),
      bootstrapEndpoint && /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn provider-btn-sm provider-btn-secondary",
          onClick: onCopyEndpoint,
          children: "Copy"
        }
      )
    ] }),
    bootstrapEndpoint && /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2", children: [
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mb-1", children: "Messaging endpoint — paste this into your bot’s configuration:" }),
      /* @__PURE__ */ u("code", { className: "text-xs block break-all select-all", children: bootstrapEndpoint })
    ] }),
    /* @__PURE__ */ u(AdvancedConfigPatchField, { value: advancedConfig, onInput: setAdvancedConfig }),
    error && /* @__PURE__ */ u(ErrorPanel, { message: error }),
    /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn", disabled: saving, children: saving ? "Connecting…" : "Connect Teams" })
  ] });
}
function ChannelStep({ onNext, onBack }) {
  const offeredList = get("channels_offered") || [
    "telegram",
    "whatsapp",
    "discord",
    "slack",
    "matrix"
  ];
  const offered = new Set(offeredList);
  const singleType = offeredList.length === 1 ? offeredList[0] : null;
  const [phase, setPhase] = d(singleType ? "form" : "select");
  const [selectedType, setSelectedType] = d(singleType);
  const [connectedName, setConnectedName] = d("");
  const [connectedType, setConnectedType] = d(null);
  const [channelError, setChannelError] = d(null);
  function onSelectType(type) {
    setSelectedType(type);
    setPhase("form");
    setChannelError(null);
  }
  function onConnected(name, type) {
    setConnectedName(name);
    setConnectedType(type);
    setPhase("success");
    setChannelError(null);
  }
  function onAnother() {
    if (singleType) {
      setPhase("form");
      setChannelError(null);
    } else {
      setPhase("select");
      setSelectedType(null);
      setChannelError(null);
    }
  }
  const showBackSelector = phase === "form" && !singleType;
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Connect a Channel" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Connect a messaging channel so you can chat from your phone or team workspace. You can set this up later in Channels." }),
    /* @__PURE__ */ u(ChannelStorageNotice, {}),
    phase === "select" && /* @__PURE__ */ u(ChannelTypeSelector, { onSelect: onSelectType, offered }),
    phase === "form" && selectedType === "telegram" && /* @__PURE__ */ u(TelegramForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "whatsapp" && /* @__PURE__ */ u(WhatsAppForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "msteams" && /* @__PURE__ */ u(TeamsForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "discord" && /* @__PURE__ */ u(DiscordForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "slack" && /* @__PURE__ */ u(SlackForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "matrix" && /* @__PURE__ */ u(MatrixForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "form" && selectedType === "nostr" && /* @__PURE__ */ u(NostrForm, { onConnected, error: channelError, setError: setChannelError }),
    phase === "success" && connectedType && /* @__PURE__ */ u(ChannelSuccess, { channelName: connectedName, channelType: connectedType, onAnother }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn provider-btn-secondary",
          onClick: showBackSelector ? () => setPhase("select") : onBack,
          children: t("common:actions.back")
        }
      ),
      phase === "success" && /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: t("common:actions.continue") }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
          onClick: onNext,
          children: t("common:actions.skip")
        }
      )
    ] })
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
const WS_RETRY_LIMIT$2 = 75;
const WS_RETRY_DELAY_MS$2 = 200;
const IMPORT_CATEGORY_ICONS = {
  identity: "👤",
  providers: "🔑",
  skills: "✨",
  memory: "🧠",
  channels: "💬",
  sessions: "💾",
  workspace_files: "📁"
};
function OpenClawImportStep({ onNext, onBack }) {
  var _a, _b, _c, _d, _e;
  const [loading, setLoading] = d(true);
  const [scan, setScan] = d(null);
  const [importing, setImporting] = d(false);
  const [done, setDone] = d(false);
  const [result, setResult] = d(null);
  const [error, setError] = d(null);
  const [selection, setSelection] = d({
    identity: true,
    providers: true,
    skills: true,
    memory: true,
    channels: true,
    sessions: true,
    workspace_files: true
  });
  y(() => {
    let cancelled = false;
    let attempts = 0;
    let retryTimer = null;
    function loadScan() {
      if (cancelled) return;
      sendRpc("openclaw.scan", {}).then((res) => {
        var _a2, _b2, _c2;
        if (cancelled) return;
        if (res == null ? void 0 : res.ok) {
          setScan(res.payload || null);
          setLoading(false);
          return;
        }
        if ((((_a2 = res == null ? void 0 : res.error) == null ? void 0 : _a2.code) === "UNAVAILABLE" || ((_b2 = res == null ? void 0 : res.error) == null ? void 0 : _b2.message) === "WebSocket not connected") && attempts < WS_RETRY_LIMIT$2) {
          attempts += 1;
          ensureWsConnected();
          retryTimer = setTimeout(loadScan, WS_RETRY_DELAY_MS$2);
          return;
        }
        setError(((_c2 = res == null ? void 0 : res.error) == null ? void 0 : _c2.message) || "Failed to scan OpenClaw installation");
        setLoading(false);
      });
    }
    ensureWsConnected();
    loadScan();
    return () => {
      cancelled = true;
      if (retryTimer) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
    };
  }, []);
  function toggleCategory(key) {
    setSelection((prev) => {
      const next = { ...prev };
      next[key] = !prev[key];
      return next;
    });
  }
  async function doImport() {
    var _a2;
    setImporting(true);
    setError(null);
    const res = await sendRpc("openclaw.import", selection);
    setImporting(false);
    if (res == null ? void 0 : res.ok) {
      setResult(res.payload || null);
      await refresh();
      setDone(true);
    } else {
      setError(((_a2 = res == null ? void 0 : res.error) == null ? void 0 : _a2.message) || "Import failed");
    }
  }
  if (loading) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col items-center justify-center gap-3 min-h-[200px]", children: [
      /* @__PURE__ */ u("div", { className: "inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" }),
      /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: "Scanning OpenClaw installation…" })
    ] });
  }
  if (done && result) {
    const total = (result.categories || []).reduce((sum, cat) => sum + (Number(cat.items_imported) || 0), 0);
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Import Complete" }),
      /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: [
        total,
        " item(s) imported from OpenClaw."
      ] }),
      result.categories ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: result.categories.map((cat) => /* @__PURE__ */ u("div", { className: "text-xs text-[var(--text)]", children: [
        /* @__PURE__ */ u("span", { className: "font-mono", children: [
          "[",
          cat.status === "success" ? "✓" : cat.status === "partial" ? "~" : cat.status === "skipped" ? "-" : "!",
          "]"
        ] }),
        " ",
        cat.category,
        ": ",
        cat.items_imported,
        " imported, ",
        cat.items_skipped,
        " skipped",
        (cat.warnings || []).map((w) => /* @__PURE__ */ u("div", { className: "text-[var(--warn)] ml-6", children: w }, w))
      ] }, cat.category)) }) : null,
      (((_a = result.todos) == null ? void 0 : _a.length) ?? 0) > 0 ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
        /* @__PURE__ */ u("div", { className: "font-medium", children: "Not yet supported in Moltis:" }),
        (result.todos || []).map((td) => /* @__PURE__ */ u("div", { children: [
          "• ",
          td.feature,
          ": ",
          td.description
        ] }, td.feature))
      ] }) : null,
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: "Continue" }) })
    ] });
  }
  if (!(scan == null ? void 0 : scan.detected)) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
      /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Import from OpenClaw" }),
      /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: "Could not scan OpenClaw installation." }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
        onBack ? /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: "Back" }) : null,
        /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: "Skip" })
      ] })
    ] });
  }
  const telegramAccounts = Number(scan.telegram_accounts) || 0;
  const discordAccounts = Number(scan.discord_accounts) || 0;
  const channelParts = [];
  if (telegramAccounts > 0) channelParts.push(`${telegramAccounts} Telegram account(s)`);
  if (discordAccounts > 0) channelParts.push(`${discordAccounts} Discord account(s)`);
  const channelDetail = channelParts.length > 0 ? channelParts.join(", ") : null;
  const unsupportedChannels = (scan.unsupported_channels || []).filter(
    (channel) => String(channel).toLowerCase() !== "discord"
  );
  const categories = [
    {
      key: "identity",
      label: "Identity",
      available: !!scan.identity_available,
      detail: [scan.identity_agent_name, scan.identity_theme].filter(Boolean).join(", ") || null
    },
    {
      key: "providers",
      label: "Providers",
      available: !!scan.providers_available,
      detail: null
    },
    {
      key: "skills",
      label: "Skills",
      available: (scan.skills_count ?? 0) > 0,
      detail: `${scan.skills_count} skill(s)`
    },
    {
      key: "memory",
      label: "Memory",
      available: !!scan.memory_available,
      detail: `${scan.memory_files_count} memory file(s)`
    },
    {
      key: "channels",
      label: "Channels",
      available: !!scan.channels_available,
      detail: channelDetail
    },
    {
      key: "sessions",
      label: "Sessions",
      available: (scan.sessions_count ?? 0) > 0,
      detail: `${scan.sessions_count} session(s)`
    },
    {
      key: "workspace_files",
      label: "Workspace Files",
      available: !!scan.workspace_files_available,
      detail: (((_b = scan.workspace_files_found) == null ? void 0 : _b.length) ?? 0) > 0 ? ((_c = scan.workspace_files_found) == null ? void 0 : _c.join(", ")) || null : null
    }
  ];
  const anySelected = categories.some((c) => c.available && selection[c.key]);
  const workspaceMissing = !scan.memory_available && (scan.skills_count ?? 0) === 0 && !scan.identity_theme;
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Import from OpenClaw" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: [
      "We detected an OpenClaw installation at ",
      /* @__PURE__ */ u("code", { className: "text-[var(--text)]", children: scan.home_dir }),
      ". Select the data you'd like to bring into Moltis."
    ] }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "This is a read-only copy — your OpenClaw installation will not be modified or removed. You can keep using OpenClaw alongside Moltis, and re-import at any time from Settings." }),
    workspaceMissing ? /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: [
      "If OpenClaw ran on another machine, copy its workspace directory (e.g. ",
      /* @__PURE__ */ u("code", { children: "clawd/" }),
      ") into",
      " ",
      /* @__PURE__ */ u("code", { children: [
        scan.home_dir,
        "/"
      ] }),
      " or ",
      /* @__PURE__ */ u("code", { children: "~/" }),
      " for a full import including identity, memory, and skills."
    ] }) : null,
    error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
    /* @__PURE__ */ u("div", { className: "grid grid-cols-1 sm:grid-cols-2 gap-2", children: categories.map((cat) => {
      const checked = selection[cat.key] && cat.available;
      return /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          onClick: () => cat.available && !importing && toggleCategory(cat.key),
          disabled: !cat.available || importing,
          className: `flex items-center gap-3 p-3 rounded-md border text-left cursor-pointer transition-colors ${cat.available ? checked ? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]" : "border-[var(--border)] bg-[var(--surface)] opacity-60" : "border-[var(--border)] bg-[var(--surface)] opacity-40 cursor-not-allowed"}`,
          children: [
            /* @__PURE__ */ u("span", { className: "text-lg shrink-0", children: IMPORT_CATEGORY_ICONS[cat.key] || "📦" }),
            /* @__PURE__ */ u("div", { className: "flex-1 min-w-0", children: [
              /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text-strong)]", children: cat.label }),
              cat.detail && cat.available ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-0.5", children: cat.detail }) : null,
              cat.available ? null : /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-0.5", children: "not found" })
            ] }),
            /* @__PURE__ */ u("div", { className: "shrink-0", children: checked ? /* @__PURE__ */ u("span", { className: "icon icon-check-circle text-[var(--accent)]" }) : /* @__PURE__ */ u("span", { className: "w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block" }) })
          ]
        },
        cat.key
      );
    }) }),
    (((_d = scan.agents) == null ? void 0 : _d.length) ?? 0) > 1 ? /* @__PURE__ */ u(
      "div",
      {
        className: "text-xs text-[var(--muted)] leading-relaxed border border-[var(--border)] rounded p-2",
        style: "max-width:400px;",
        children: [
          /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: [
            (_e = scan.agents) == null ? void 0 : _e.length,
            " agents detected"
          ] }),
          /* @__PURE__ */ u("span", { className: "ml-1", children: "— non-default agents will be created as separate personas:" }),
          /* @__PURE__ */ u("ul", { className: "mt-1 ml-4 list-disc", children: (scan.agents || []).map((a) => /* @__PURE__ */ u("li", { children: [
            /* @__PURE__ */ u("span", { className: "text-[var(--text)]", children: a.name || a.openclaw_id }),
            a.is_default ? /* @__PURE__ */ u("span", { className: "ml-1 text-[var(--muted)]", children: "(default)" }) : null,
            a.theme ? /* @__PURE__ */ u("span", { className: "ml-1 text-[var(--muted)]", children: [
              "— ",
              a.theme
            ] }) : null
          ] }, a.openclaw_id)) })
        ]
      }
    ) : null,
    unsupportedChannels.length > 0 ? /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)]", children: [
      "Unsupported channels (coming soon): ",
      unsupportedChannels.join(", ")
    ] }) : null,
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      onBack ? /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, disabled: importing, children: "Back" }) : null,
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: doImport, disabled: !anySelected || importing, children: importing ? "Importing…" : "Import Selected" }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline",
          onClick: onNext,
          disabled: importing,
          children: "Skip for now"
        }
      )
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
const WS_RETRY_LIMIT$1 = 75;
const WS_RETRY_DELAY_MS$1 = 200;
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
        if ((((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.code) === "UNAVAILABLE" || ((_b = res == null ? void 0 : res.error) == null ? void 0 : _b.message) === "WebSocket not connected") && attempts < WS_RETRY_LIMIT$1) {
          attempts += 1;
          window.setTimeout(loadProviders, WS_RETRY_DELAY_MS$1);
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
function SkillsStep({ onNext, onBack }) {
  const [categories, setCategories] = d([]);
  const [totalSkills, setTotalSkills] = d(0);
  const [loading, setLoading] = d(true);
  const [busy, setBusy] = d(false);
  y(() => {
    sendRpc("skills.bundled.categories", {}).then((res) => {
      if (res == null ? void 0 : res.ok) {
        const payload = res.payload;
        setCategories(payload.categories || []);
        setTotalSkills(payload.total_skills || 0);
      }
      setLoading(false);
    });
  }, []);
  function toggle(cat) {
    if (busy) return;
    const newEnabled = !cat.enabled;
    setBusy(true);
    sendRpc("skills.bundled.toggle_category", { category: cat.name, enabled: newEnabled }).then((res) => {
      setBusy(false);
      if (res == null ? void 0 : res.ok) {
        setCategories((prev) => prev.map((c) => c.name === cat.name ? { ...c, enabled: newEnabled } : c));
      }
    });
  }
  function bulkToggle(enabled) {
    const targets = categories.filter((c) => c.enabled !== enabled);
    if (!targets.length || busy) return;
    setBusy(true);
    Promise.all(
      targets.map(
        (c) => sendRpc("skills.bundled.toggle_category", { category: c.name, enabled }).then((res) => ({
          name: c.name,
          ok: !!(res == null ? void 0 : res.ok)
        }))
      )
    ).then((results) => {
      setBusy(false);
      const succeeded = new Set(results.filter((r) => r.ok).map((r) => r.name));
      if (succeeded.size > 0) {
        setCategories((prev) => prev.map((c) => succeeded.has(c.name) ? { ...c, enabled } : c));
      }
    });
  }
  const enabledCount = categories.filter((c) => c.enabled).length;
  const enabledSkillCount = categories.filter((c) => c.enabled).reduce((sum, c) => sum + c.count, 0);
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:skills.title") }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: t("onboarding:skills.description") }),
    loading ? /* @__PURE__ */ u("div", { className: "flex items-center justify-center gap-2 py-8", children: [
      /* @__PURE__ */ u("div", { className: "inline-block w-5 h-5 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" }),
      /* @__PURE__ */ u("span", { className: "text-sm text-[var(--muted)]", children: t("common:status.loading") })
    ] }) : /* @__PURE__ */ u(S, { children: [
      /* @__PURE__ */ u("div", { className: "flex items-center justify-between", children: [
        /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: [
          enabledCount,
          " of ",
          categories.length,
          " categories (",
          enabledSkillCount,
          " of ",
          totalSkills,
          " skills)"
        ] }),
        /* @__PURE__ */ u("div", { className: "flex gap-2", children: [
          /* @__PURE__ */ u(
            "button",
            {
              type: "button",
              className: "text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0",
              disabled: busy,
              onClick: () => bulkToggle(true),
              children: t("onboarding:skills.enableAll")
            }
          ),
          /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: "/" }),
          /* @__PURE__ */ u(
            "button",
            {
              type: "button",
              className: "text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0",
              disabled: busy,
              onClick: () => bulkToggle(false),
              children: t("onboarding:skills.disableAll")
            }
          )
        ] })
      ] }),
      /* @__PURE__ */ u("div", { className: "grid grid-cols-1 sm:grid-cols-2 gap-2", children: categories.map((cat) => {
        const meta = CATEGORY_META[cat.name];
        const icon = (meta == null ? void 0 : meta.icon) || "📦";
        const desc = (meta == null ? void 0 : meta.desc) || "";
        return /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            onClick: () => toggle(cat),
            disabled: busy,
            className: `flex items-start gap-3 p-3 rounded-md border text-left cursor-pointer transition-colors ${cat.enabled ? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]" : "border-[var(--border)] bg-[var(--surface)] opacity-60"}`,
            children: [
              /* @__PURE__ */ u("span", { className: "text-lg shrink-0 mt-0.5", children: icon }),
              /* @__PURE__ */ u("div", { className: "flex-1 min-w-0", children: [
                /* @__PURE__ */ u("div", { className: "flex items-center gap-2", children: [
                  /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text-strong)]", children: categoryLabel(cat.name) }),
                  /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: [
                    "(",
                    cat.count,
                    ")"
                  ] })
                ] }),
                desc && /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-0.5", children: desc })
              ] }),
              /* @__PURE__ */ u("div", { className: "shrink-0 mt-1", children: cat.enabled ? /* @__PURE__ */ u("span", { className: "icon icon-check-circle text-[var(--accent)]" }) : /* @__PURE__ */ u("span", { className: "w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block" }) })
            ]
          },
          cat.name
        );
      }) })
    ] }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      onBack && /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("div", { className: "flex-1" }),
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: t("common:actions.continue") })
    ] })
  ] });
}
const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;
function OnboardingVoiceRow({
  provider,
  type,
  configuring,
  apiKey,
  setApiKey,
  baseUrl,
  setBaseUrl,
  saving,
  error,
  onSaveKey,
  onStartConfigure,
  onCancelConfigure,
  onTest,
  voiceTesting,
  voiceTestResult
}) {
  var _a;
  const isConfiguring = configuring === provider.id;
  const keyInputRef = A(null);
  y(() => {
    if (isConfiguring && keyInputRef.current) {
      keyInputRef.current.focus();
    }
  }, [isConfiguring]);
  const supportsBaseUrl = ((_a = provider.capabilities) == null ? void 0 : _a.baseUrl) === true;
  const keySourceLabel = provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";
  const testState = (voiceTesting == null ? void 0 : voiceTesting.id) === provider.id && (voiceTesting == null ? void 0 : voiceTesting.type) === type ? voiceTesting : null;
  const showTestBtn = provider.available;
  let testBtnText = "Test";
  let testBtnDisabled = false;
  if (testState) {
    if (testState.phase === "recording") {
      testBtnText = "Stop";
    } else {
      testBtnText = "Testing…";
      testBtnDisabled = true;
    }
  }
  return /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface)] p-3", children: [
    /* @__PURE__ */ u("div", { className: "flex items-center gap-3", children: [
      /* @__PURE__ */ u("div", { className: "flex-1 min-w-0 flex flex-col gap-0.5", children: [
        /* @__PURE__ */ u("div", { className: "flex items-center gap-2 flex-wrap", children: [
          /* @__PURE__ */ u("span", { className: "text-sm font-medium text-[var(--text-strong)]", children: provider.name }),
          provider.available ? /* @__PURE__ */ u("span", { className: "provider-item-badge configured", children: "configured" }) : /* @__PURE__ */ u("span", { className: "provider-item-badge needs-key", children: "needs key" }),
          keySourceLabel ? /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: keySourceLabel }) : null
        ] }),
        provider.description ? /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)]", children: [
          provider.description,
          !isConfiguring && provider.keyUrl ? /* @__PURE__ */ u(S, { children: [
            " — ",
            "get your key at",
            " ",
            /* @__PURE__ */ u("a", { href: provider.keyUrl, target: "_blank", rel: "noopener", className: "text-[var(--accent)] underline", children: provider.keyUrlLabel || provider.keyUrl })
          ] }) : null
        ] }) : null
      ] }),
      /* @__PURE__ */ u("div", { className: "shrink-0 flex items-center gap-2", children: [
        isConfiguring ? null : /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-secondary provider-btn-sm",
            onClick: () => onStartConfigure(provider.id),
            children: "Configure"
          }
        ),
        showTestBtn ? /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-secondary provider-btn-sm",
            onClick: onTest,
            disabled: testBtnDisabled,
            title: type === "tts" ? "Test voice output" : "Test voice input",
            children: testBtnText
          }
        ) : null
      ] })
    ] }),
    (testState == null ? void 0 : testState.phase) === "recording" ? /* @__PURE__ */ u("div", { className: "voice-recording-hint mt-2", children: [
      /* @__PURE__ */ u("span", { className: "voice-recording-dot" }),
      /* @__PURE__ */ u("span", { children: "Speak now, then click Stop when finished" })
    ] }) : null,
    (testState == null ? void 0 : testState.phase) === "transcribing" ? /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)] mt-1 block", children: "Transcribing…" }) : null,
    (testState == null ? void 0 : testState.phase) === "testing" && type === "tts" ? /* @__PURE__ */ u("span", { className: "text-xs text-[var(--muted)] mt-1 block", children: "Playing audio…" }) : null,
    (voiceTestResult == null ? void 0 : voiceTestResult.text) ? /* @__PURE__ */ u("div", { className: "voice-transcription-result mt-2", children: [
      /* @__PURE__ */ u("span", { className: "voice-transcription-label", children: "Transcribed:" }),
      /* @__PURE__ */ u("span", { className: "voice-transcription-text", children: [
        '"',
        voiceTestResult.text,
        '"'
      ] })
    ] }) : null,
    (voiceTestResult == null ? void 0 : voiceTestResult.success) === true ? /* @__PURE__ */ u("div", { className: "voice-success-result mt-2", children: [
      /* @__PURE__ */ u("span", { className: "icon icon-md icon-check-circle" }),
      /* @__PURE__ */ u("span", { children: "Audio played successfully" })
    ] }) : null,
    (voiceTestResult == null ? void 0 : voiceTestResult.error) ? /* @__PURE__ */ u("div", { className: "voice-error-result", children: [
      /* @__PURE__ */ u("span", { className: "icon icon-md icon-x-circle" }),
      /* @__PURE__ */ u("span", { children: voiceTestResult.error })
    ] }) : null,
    isConfiguring ? /* @__PURE__ */ u("form", { onSubmit: onSaveKey, className: "flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3", children: [
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
            placeholder: provider.keyPlaceholder || "API key"
          }
        )
      ] }),
      supportsBaseUrl ? /* @__PURE__ */ u("div", { children: [
        /* @__PURE__ */ u("label", { className: "text-xs text-[var(--muted)] mb-1 block", children: "Base URL" }),
        /* @__PURE__ */ u(
          "input",
          {
            type: "text",
            className: "provider-key-input w-full",
            "data-field": "baseUrl",
            value: baseUrl,
            onInput: (e) => setBaseUrl(targetValue(e)),
            placeholder: "http://localhost:8000/v1"
          }
        ),
        /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children: "Use this for a local or OpenAI-compatible server. Leave the API key blank if the endpoint does not require one." })
      ] }) : null,
      provider.keyUrl ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: [
        "Get your key at",
        " ",
        /* @__PURE__ */ u("a", { href: provider.keyUrl, target: "_blank", rel: "noopener", className: "text-[var(--accent)] underline", children: provider.keyUrlLabel || provider.keyUrl })
      ] }) : null,
      provider.hint ? /* @__PURE__ */ u("div", { className: "text-xs text-[var(--accent)]", children: provider.hint }) : null,
      error ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
      /* @__PURE__ */ u("div", { className: "flex items-center gap-2 mt-1", children: [
        /* @__PURE__ */ u("button", { type: "submit", className: "provider-btn provider-btn-sm", disabled: saving, children: saving ? "Saving…" : "Save" }),
        /* @__PURE__ */ u(
          "button",
          {
            type: "button",
            className: "provider-btn provider-btn-secondary provider-btn-sm",
            onClick: onCancelConfigure,
            children: "Cancel"
          }
        )
      ] })
    ] }) : null
  ] });
}
function VoiceStep({ onNext, onBack }) {
  const [loading, setLoading] = d(true);
  const [allProviders, setAllProviders] = d({ tts: [], stt: [] });
  const [configuring, setConfiguring] = d(null);
  const [apiKey, setApiKey] = d("");
  const [baseUrl, setBaseUrl] = d("");
  const [saving, setSaving] = d(false);
  const [error, setError] = d(null);
  const [voiceTesting, setVoiceTesting] = d(null);
  const [voiceTestResults, setVoiceTestResults] = d({});
  const [activeRecorder, setActiveRecorder] = d(null);
  const [enableSaving, setEnableSaving] = d(false);
  function fetchProviders() {
    return fetchVoiceProviders().then((res) => {
      if (res == null ? void 0 : res.ok) {
        setAllProviders(res.payload || { tts: [], stt: [] });
      }
      return res;
    });
  }
  y(() => {
    let cancelled = false;
    let attempts = 0;
    function load() {
      if (cancelled) return;
      fetchVoiceProviders().then((res) => {
        var _a, _b;
        if (cancelled) return;
        if (res == null ? void 0 : res.ok) {
          setAllProviders(res.payload || { tts: [], stt: [] });
          setLoading(false);
          return;
        }
        if ((((_a = res == null ? void 0 : res.error) == null ? void 0 : _a.code) === "UNAVAILABLE" || ((_b = res == null ? void 0 : res.error) == null ? void 0 : _b.message) === "WebSocket not connected") && attempts < WS_RETRY_LIMIT) {
          attempts += 1;
          ensureWsConnected();
          window.setTimeout(load, WS_RETRY_DELAY_MS);
          return;
        }
        onNext();
      });
    }
    load();
    return () => {
      cancelled = true;
    };
  }, []);
  const cloudStt = allProviders.stt.filter((p) => p.category === "cloud");
  const cloudTts = allProviders.tts.filter((p) => p.category === "cloud");
  const autoDetected = [...allProviders.stt, ...allProviders.tts].filter(
    (p) => p.available && p.keySource === "llm_provider" && !p.enabled && p.category === "cloud"
  );
  const hasAutoDetected = autoDetected.length > 0;
  function enableAutoDetected() {
    setEnableSaving(true);
    setError(null);
    const firstStt = allProviders.stt.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
    const firstTts = allProviders.tts.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
    const toggles = [];
    if (firstStt) toggles.push(toggleVoiceProvider(firstStt.id, true, "stt"));
    if (firstTts) toggles.push(toggleVoiceProvider(firstTts.id, true, "tts"));
    if (toggles.length === 0) {
      setEnableSaving(false);
      return;
    }
    Promise.all(toggles).then((results) => {
      var _a;
      setEnableSaving(false);
      const failed = results.find((r) => !(r == null ? void 0 : r.ok));
      if (failed) {
        setError(((_a = failed == null ? void 0 : failed.error) == null ? void 0 : _a.message) || "Failed to enable voice provider");
        return;
      }
      fetchProviders();
    });
  }
  function onStartConfigure(providerId) {
    var _a;
    const provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === providerId);
    setConfiguring(providerId);
    setApiKey("");
    setBaseUrl(((_a = provider == null ? void 0 : provider.settings) == null ? void 0 : _a.baseUrl) || "");
    setError(null);
  }
  function onCancelConfigure() {
    setConfiguring(null);
    setApiKey("");
    setBaseUrl("");
    setError(null);
  }
  function onSaveKey(e) {
    var _a, _b;
    e.preventDefault();
    const provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === configuring);
    const trimmedApiKey = apiKey.trim();
    const trimmedBaseUrl = baseUrl.trim();
    const hadBaseUrl = typeof ((_a = provider == null ? void 0 : provider.settings) == null ? void 0 : _a.baseUrl) === "string" && provider.settings.baseUrl.trim().length > 0;
    const shouldSaveBaseUrl = ((_b = provider == null ? void 0 : provider.capabilities) == null ? void 0 : _b.baseUrl) === true && (trimmedBaseUrl.length > 0 || hadBaseUrl);
    if (!(trimmedApiKey || shouldSaveBaseUrl)) {
      setError("API key or base URL is required.");
      return;
    }
    setError(null);
    setSaving(true);
    const providerId = configuring;
    const req = trimmedApiKey ? saveVoiceKey(providerId, trimmedApiKey, {
      baseUrl: shouldSaveBaseUrl ? trimmedBaseUrl : void 0
    }) : saveVoiceSettings(providerId, shouldSaveBaseUrl ? { baseUrl: trimmedBaseUrl } : void 0);
    req.then(async (res) => {
      var _a2;
      if (res == null ? void 0 : res.ok) {
        const counterId = VOICE_COUNTERPART_IDS[providerId];
        const toggles = [];
        const sttMatch = allProviders.stt.find((p) => p.id === providerId) || counterId && allProviders.stt.find((p) => p.id === counterId);
        const ttsMatch = allProviders.tts.find((p) => p.id === providerId) || counterId && allProviders.tts.find((p) => p.id === counterId);
        if (sttMatch) {
          toggles.push(toggleVoiceProvider(sttMatch.id, true, "stt"));
        }
        if (ttsMatch) {
          toggles.push(toggleVoiceProvider(ttsMatch.id, true, "tts"));
        }
        await Promise.all(toggles);
        setSaving(false);
        setConfiguring(null);
        setApiKey("");
        setBaseUrl("");
        fetchProviders();
      } else {
        setSaving(false);
        setError(((_a2 = res == null ? void 0 : res.error) == null ? void 0 : _a2.message) || "Failed to save");
      }
    });
  }
  function stopSttRecording() {
    if (activeRecorder) {
      activeRecorder.stop();
    }
  }
  async function testVoiceProvider(providerId, type) {
    var _a;
    if ((voiceTesting == null ? void 0 : voiceTesting.id) === providerId && (voiceTesting == null ? void 0 : voiceTesting.type) === "stt" && (voiceTesting == null ? void 0 : voiceTesting.phase) === "recording") {
      stopSttRecording();
      return;
    }
    setError(null);
    setVoiceTesting({ id: providerId, type, phase: "testing" });
    const prov = (type === "stt" ? allProviders.stt : allProviders.tts).find((p) => p.id === providerId);
    if ((prov == null ? void 0 : prov.available) && !(prov == null ? void 0 : prov.enabled)) {
      const toggleRes = await toggleVoiceProvider(providerId, true, type);
      if (!(toggleRes == null ? void 0 : toggleRes.ok)) {
        setVoiceTestResults((prev) => {
          var _a2;
          return {
            ...prev,
            [providerId]: {
              success: false,
              error: ((_a2 = toggleRes == null ? void 0 : toggleRes.error) == null ? void 0 : _a2.message) || "Failed to enable provider"
            }
          };
        });
        setVoiceTesting(null);
        return;
      }
      const counterType = type === "stt" ? "tts" : "stt";
      const counterList = type === "stt" ? allProviders.tts : allProviders.stt;
      const counterId = VOICE_COUNTERPART_IDS[providerId] || providerId;
      const counterProv = counterList.find((p) => p.id === counterId);
      if ((counterProv == null ? void 0 : counterProv.available) && !(counterProv == null ? void 0 : counterProv.enabled)) {
        await toggleVoiceProvider(counterId, true, counterType);
      }
      fetchProviders();
    }
    if (type === "tts") {
      try {
        const identity = get("identity");
        const user = (identity == null ? void 0 : identity.user_name) || "friend";
        const bot = (identity == null ? void 0 : identity.name) || "Moltis";
        const ttsText = await fetchPhrase("onboarding", user, bot);
        const res = await testTts(ttsText, providerId);
        if ((res == null ? void 0 : res.ok) && ((_a = res.payload) == null ? void 0 : _a.audio)) {
          const bytes = decodeBase64Safe(res.payload.audio);
          const audioMime = res.payload.mimeType || res.payload.content_type || "audio/mpeg";
          const blob = new Blob([bytes.buffer], { type: audioMime });
          const url = URL.createObjectURL(blob);
          const audio = new Audio(url);
          audio.onerror = (e) => {
            var _a2;
            console.error("[TTS] audio element error:", ((_a2 = audio.error) == null ? void 0 : _a2.message) || e);
            URL.revokeObjectURL(url);
          };
          audio.onended = () => URL.revokeObjectURL(url);
          audio.play().catch((e) => console.error("[TTS] play() failed:", e));
          setVoiceTestResults((prev) => ({
            ...prev,
            [providerId]: { success: true, error: null }
          }));
        } else {
          setVoiceTestResults((prev) => {
            var _a2;
            return {
              ...prev,
              [providerId]: {
                success: false,
                error: ((_a2 = res == null ? void 0 : res.error) == null ? void 0 : _a2.message) || "TTS test failed"
              }
            };
          });
        }
      } catch (err) {
        setVoiceTestResults((prev) => ({
          ...prev,
          [providerId]: {
            success: false,
            error: err.message || "TTS test failed"
          }
        }));
      }
      setVoiceTesting(null);
    } else {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus") ? "audio/webm;codecs=opus" : "audio/webm";
        const mediaRecorder = new MediaRecorder(stream, { mimeType });
        const audioChunks = [];
        mediaRecorder.ondataavailable = (e) => {
          if (e.data.size > 0) audioChunks.push(e.data);
        };
        mediaRecorder.start();
        setActiveRecorder(mediaRecorder);
        setVoiceTesting({ id: providerId, type, phase: "recording" });
        mediaRecorder.onstop = async () => {
          var _a2, _b;
          setActiveRecorder(null);
          for (const track of stream.getTracks()) track.stop();
          setVoiceTesting({ id: providerId, type, phase: "transcribing" });
          const audioBlob = new Blob(audioChunks, {
            type: mediaRecorder.mimeType || mimeType
          });
          try {
            const resp = await transcribeAudio(activeSessionKey, providerId, audioBlob);
            if (resp.ok) {
              const sttRes = await resp.json();
              if (sttRes.ok && typeof ((_a2 = sttRes.transcription) == null ? void 0 : _a2.text) === "string") {
                const transcriptText = sttRes.transcription.text.trim();
                setVoiceTestResults((prev) => ({
                  ...prev,
                  [providerId]: {
                    text: transcriptText || null,
                    error: transcriptText ? null : "No speech detected"
                  }
                }));
              } else {
                setVoiceTestResults((prev) => ({
                  ...prev,
                  [providerId]: {
                    text: null,
                    error: sttRes.transcriptionError || sttRes.error || "STT test failed"
                  }
                }));
              }
            } else {
              const errBody = await resp.text();
              console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
              let errMsg = "STT test failed";
              try {
                errMsg = ((_b = JSON.parse(errBody)) == null ? void 0 : _b.error) || errMsg;
              } catch (_e) {
              }
              setVoiceTestResults((prev) => ({
                ...prev,
                [providerId]: {
                  text: null,
                  error: `${errMsg} (HTTP ${resp.status})`
                }
              }));
            }
          } catch (fetchErr) {
            setVoiceTestResults((prev) => ({
              ...prev,
              [providerId]: {
                text: null,
                error: fetchErr.message || "STT test failed"
              }
            }));
          }
          setVoiceTesting(null);
        };
      } catch (err) {
        const domErr = err;
        if (domErr.name === "NotAllowedError") {
          setError("Microphone permission denied");
        } else if (domErr.name === "NotFoundError") {
          setError("No microphone found");
        } else {
          setError(domErr.message || "STT test failed");
        }
        setVoiceTesting(null);
      }
    }
  }
  if (loading) {
    return /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: "Checking voice providers…" });
  }
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: "Voice (optional)" }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Enable voice input (speech-to-text) and output (text-to-speech) for your agent. You can configure this later in Settings." }),
    hasAutoDetected ? /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2", children: [
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)]", children: "Auto-detected from your LLM provider" }),
      /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-2", children: autoDetected.map((p) => /* @__PURE__ */ u("span", { className: "provider-item-badge configured", children: p.name }, p.id)) }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
          className: "provider-btn self-start",
          disabled: enableSaving,
          onClick: enableAutoDetected,
          children: enableSaving ? "Enabling…" : "Enable voice"
        }
      )
    ] }) : null,
    cloudStt.length > 0 ? /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("h3", { className: "text-sm font-medium text-[var(--text-strong)] mb-2", children: "Speech-to-Text" }),
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: cloudStt.map((prov) => /* @__PURE__ */ u(
        OnboardingVoiceRow,
        {
          provider: prov,
          type: "stt",
          configuring,
          apiKey,
          setApiKey,
          baseUrl,
          setBaseUrl,
          saving,
          error: configuring === prov.id ? error : null,
          onSaveKey,
          onStartConfigure,
          onCancelConfigure,
          onTest: () => testVoiceProvider(prov.id, "stt"),
          voiceTesting,
          voiceTestResult: voiceTestResults[prov.id] || null
        },
        prov.id
      )) })
    ] }) : null,
    cloudTts.length > 0 ? /* @__PURE__ */ u("div", { children: [
      /* @__PURE__ */ u("h3", { className: "text-sm font-medium text-[var(--text-strong)] mb-2", children: "Text-to-Speech" }),
      /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: cloudTts.map((prov) => /* @__PURE__ */ u(
        OnboardingVoiceRow,
        {
          provider: prov,
          type: "tts",
          configuring,
          apiKey,
          setApiKey,
          baseUrl,
          setBaseUrl,
          saving,
          error: configuring === prov.id ? error : null,
          onSaveKey,
          onStartConfigure,
          onCancelConfigure,
          onTest: () => testVoiceProvider(prov.id, "tts"),
          voiceTesting,
          voiceTestResult: voiceTestResults[prov.id] || null
        },
        prov.id
      )) })
    ] }) : null,
    error && !configuring ? /* @__PURE__ */ u(ErrorPanel, { message: error }) : null,
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onNext, children: t("common:actions.continue") }),
      /* @__PURE__ */ u(
        "button",
        {
          type: "button",
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
const LOW_MEMORY_THRESHOLD = 2 * 1024 * 1024 * 1024;
function formatMemBytes(bytes) {
  if (bytes == null) return "?";
  const gb = bytes / (1024 * 1024 * 1024);
  return `${gb.toFixed(1)} GB`;
}
function CheckIcon() {
  return /* @__PURE__ */ u("span", { className: "icon icon-check-circle shrink-0", style: "color:var(--ok)" });
}
function WarnIcon() {
  return /* @__PURE__ */ u("span", { className: "icon icon-warn-triangle shrink-0", style: "color:var(--warn)" });
}
function ErrorIcon() {
  return /* @__PURE__ */ u("span", { className: "icon icon-x-circle shrink-0", style: "color:var(--error)" });
}
function InfoIcon() {
  return /* @__PURE__ */ u("span", { className: "icon icon-info-circle shrink-0", style: "color:var(--muted)" });
}
function SummaryRow({
  icon,
  label,
  children
}) {
  return /* @__PURE__ */ u("div", { className: "rounded-md border border-[var(--border)] bg-[var(--surface)] p-3 flex gap-3 items-start", children: [
    /* @__PURE__ */ u("div", { className: "mt-0.5", children: icon }),
    /* @__PURE__ */ u("div", { className: "flex-1 min-w-0", children: [
      /* @__PURE__ */ u("div", { className: "text-sm font-medium text-[var(--text-strong)]", children: label }),
      /* @__PURE__ */ u("div", { className: "text-xs text-[var(--muted)] mt-1", children })
    ] })
  ] });
}
function SummaryStep({ onBack, onFinish }) {
  var _a, _b, _c, _d, _e, _f, _g, _h, _i, _j, _k, _l, _m, _n, _o;
  const [loading, setLoading] = d(true);
  const [data, setData] = d(null);
  y(() => {
    let cancelled = false;
    async function load() {
      var _a2, _b2, _c2;
      await refresh();
      const identity = get("identity");
      const mem = get("mem");
      const update = get("update");
      const voiceEnabled = get("voice_enabled") === true;
      const [providersRes, channelsRes, tailscaleRes, voiceRes, bootstrapRes, skillsRes] = await Promise.all([
        sendRpc("providers.available", {}).catch(() => null),
        fetchChannelStatus().catch(() => null),
        fetch("/api/tailscale/status").then(
          (r) => r.ok ? r.json() : null
        ).catch(() => null),
        voiceEnabled ? fetchVoiceProviders().catch(() => null) : Promise.resolve(null),
        fetch(
          "/api/bootstrap?include_channels=false&include_sessions=false&include_models=false&include_projects=false&include_counts=false&include_identity=false"
        ).then(
          (r) => r.ok ? r.json() : null
        ).catch(() => null),
        sendRpc("skills.bundled.categories", {}).catch(() => null)
      ]);
      if (cancelled) return;
      const skillsCats = (skillsRes == null ? void 0 : skillsRes.ok) ? ((_a2 = skillsRes.payload) == null ? void 0 : _a2.categories) || [] : [];
      const skillsTotal = (skillsRes == null ? void 0 : skillsRes.ok) ? ((_b2 = skillsRes.payload) == null ? void 0 : _b2.total_skills) || 0 : 0;
      const skillsEnabledCats = skillsCats.filter((c) => c.enabled);
      setData({
        identity,
        mem,
        update,
        voiceEnabled,
        providers: (providersRes == null ? void 0 : providersRes.ok) ? providersRes.payload || [] : [],
        channels: (channelsRes == null ? void 0 : channelsRes.ok) ? ((_c2 = channelsRes.payload) == null ? void 0 : _c2.channels) || [] : [],
        tailscale: tailscaleRes,
        voice: (voiceRes == null ? void 0 : voiceRes.ok) ? voiceRes.payload || { tts: [], stt: [] } : null,
        sandbox: (bootstrapRes == null ? void 0 : bootstrapRes.sandbox) || null,
        skills: skillsCats.length ? {
          enabledCategories: skillsEnabledCats.length,
          totalCategories: skillsCats.length,
          enabledSkills: skillsEnabledCats.reduce((sum, c) => sum + c.count, 0),
          totalSkills: skillsTotal
        } : null
      });
      setLoading(false);
    }
    load();
    return () => {
      cancelled = true;
    };
  }, []);
  if (loading || !data) {
    return /* @__PURE__ */ u("div", { className: "flex flex-col items-center justify-center gap-3 min-h-[200px]", children: [
      /* @__PURE__ */ u("div", { className: "inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" }),
      /* @__PURE__ */ u("div", { className: "text-sm text-[var(--muted)]", children: t("onboarding:summary.loadingSummary") })
    ] });
  }
  const activeModel = localStorage.getItem("moltis-model");
  const configuredProviders = data.providers.filter((p) => p.configured);
  return /* @__PURE__ */ u("div", { className: "flex flex-col gap-4", children: [
    /* @__PURE__ */ u("h2", { className: "text-lg font-medium text-[var(--text-strong)]", children: t("onboarding:summary.title") }),
    /* @__PURE__ */ u("p", { className: "text-xs text-[var(--muted)] leading-relaxed", children: "Overview of your configuration. You can change any of these later in Settings." }),
    /* @__PURE__ */ u("div", { className: "flex flex-col gap-2", children: [
      /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: ((_a = data.identity) == null ? void 0 : _a.user_name) && ((_b = data.identity) == null ? void 0 : _b.name) ? /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(WarnIcon, {}),
          label: "Identity",
          children: ((_c = data.identity) == null ? void 0 : _c.user_name) && ((_d = data.identity) == null ? void 0 : _d.name) ? /* @__PURE__ */ u(S, { children: [
            "You: ",
            /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: data.identity.user_name }),
            " Agent:",
            " ",
            /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: [
              data.identity.emoji || "",
              " ",
              data.identity.name
            ] })
          ] }) : /* @__PURE__ */ u("span", { className: "text-[var(--warn)]", children: "Identity not fully configured" })
        }
      ),
      /* @__PURE__ */ u(SummaryRow, { icon: configuredProviders.length > 0 ? /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(ErrorIcon, {}), label: "LLMs", children: configuredProviders.length > 0 ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: [
        /* @__PURE__ */ u("div", { className: "flex flex-wrap gap-1", children: configuredProviders.map((p) => /* @__PURE__ */ u("span", { className: "provider-item-badge configured", children: p.displayName }, p.name)) }),
        activeModel ? /* @__PURE__ */ u("div", { children: [
          "Active model: ",
          /* @__PURE__ */ u("span", { className: "font-mono font-medium text-[var(--text)]", children: activeModel })
        ] }) : null
      ] }) : /* @__PURE__ */ u("span", { className: "text-[var(--error)]", children: "No LLM providers configured" }) }),
      /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: data.channels.length > 0 ? data.channels.some((c) => c.status === "error") ? /* @__PURE__ */ u(ErrorIcon, {}) : data.channels.some((c) => c.status === "disconnected") ? /* @__PURE__ */ u(WarnIcon, {}) : /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(InfoIcon, {}),
          label: "Channels",
          children: data.channels.length > 0 ? /* @__PURE__ */ u("div", { className: "flex flex-col gap-1", children: data.channels.map((ch) => {
            const statusColor = ch.status === "connected" ? "var(--ok)" : ch.status === "error" ? "var(--error)" : "var(--warn)";
            return /* @__PURE__ */ u("div", { className: "flex items-center gap-1", children: [
              /* @__PURE__ */ u("span", { style: `color:${statusColor}`, children: "●" }),
              /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: ch.type }),
              ": ",
              ch.name || ch.account_id,
              /* @__PURE__ */ u("span", { children: [
                "(",
                ch.status,
                ")"
              ] })
            ] }, ch.account_id);
          }) }) : /* @__PURE__ */ u(S, { children: "No channels configured" })
        }
      ),
      data.skills && /* @__PURE__ */ u(SummaryRow, { icon: data.skills.enabledCategories > 0 ? /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(InfoIcon, {}), label: "Skills", children: [
        /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: data.skills.enabledSkills }),
        " skills enabled across",
        " ",
        /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: [
          data.skills.enabledCategories,
          "/",
          data.skills.totalCategories
        ] }),
        " ",
        "categories"
      ] }),
      /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: ((_e = data.mem) == null ? void 0 : _e.total) && data.mem.total < LOW_MEMORY_THRESHOLD ? /* @__PURE__ */ u(WarnIcon, {}) : /* @__PURE__ */ u(CheckIcon, {}),
          label: "System Memory",
          children: data.mem ? /* @__PURE__ */ u(S, { children: [
            "Total: ",
            /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: formatMemBytes(data.mem.total) }),
            " Available:",
            " ",
            /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: formatMemBytes(data.mem.available) }),
            data.mem.total && data.mem.total < LOW_MEMORY_THRESHOLD ? /* @__PURE__ */ u("div", { className: "text-[var(--warn)] mt-1", children: "Low memory detected. Consider upgrading to an instance with more RAM." }) : null
          ] }) : /* @__PURE__ */ u(S, { children: "Memory info unavailable" })
        }
      ),
      /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: ((_f = data.sandbox) == null ? void 0 : _f.backend) && data.sandbox.backend !== "none" ? /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(InfoIcon, {}),
          label: "Sandbox",
          children: ((_g = data.sandbox) == null ? void 0 : _g.backend) && data.sandbox.backend !== "none" ? /* @__PURE__ */ u(S, { children: [
            "Backend: ",
            /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: data.sandbox.backend })
          ] }) : /* @__PURE__ */ u(S, { children: "No container runtime detected" })
        }
      ),
      /* @__PURE__ */ u(SummaryRow, { icon: ((_h = data.update) == null ? void 0 : _h.available) ? /* @__PURE__ */ u(WarnIcon, {}) : /* @__PURE__ */ u(CheckIcon, {}), label: "Version", children: ((_i = data.update) == null ? void 0 : _i.available) ? /* @__PURE__ */ u(S, { children: [
        "Update available:",
        " ",
        /* @__PURE__ */ u(
          "a",
          {
            href: data.update.release_url || "#",
            target: "_blank",
            rel: "noopener",
            className: "text-[var(--accent)] underline font-medium",
            children: data.update.latest_version
          }
        )
      ] }) : /* @__PURE__ */ u(S, { children: "You are running the latest version." }) }),
      data.tailscale !== null ? /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: ((_j = data.tailscale) == null ? void 0 : _j.tailscale_up) ? /* @__PURE__ */ u(CheckIcon, {}) : ((_k = data.tailscale) == null ? void 0 : _k.installed) ? /* @__PURE__ */ u(WarnIcon, {}) : /* @__PURE__ */ u(InfoIcon, {}),
          label: "Tailscale",
          children: ((_l = data.tailscale) == null ? void 0 : _l.tailscale_up) ? /* @__PURE__ */ u(S, { children: "Connected" }) : ((_m = data.tailscale) == null ? void 0 : _m.installed) ? /* @__PURE__ */ u(S, { children: [
            "Installed but not connected —",
            " ",
            /* @__PURE__ */ u("a", { href: "/settings/remote-access", className: "text-[var(--accent)] underline", children: "Configure in Settings" })
          ] }) : /* @__PURE__ */ u(S, { children: "Not installed. Install Tailscale for secure remote access." })
        }
      ) : null,
      data.voiceEnabled ? /* @__PURE__ */ u(
        SummaryRow,
        {
          icon: data.voice && [...data.voice.tts, ...data.voice.stt].some((p) => p.enabled) ? /* @__PURE__ */ u(CheckIcon, {}) : /* @__PURE__ */ u(InfoIcon, {}),
          label: "Voice",
          children: (() => {
            if (!data.voice) return /* @__PURE__ */ u(S, { children: "Voice providers unavailable" });
            const enabledStt = data.voice.stt.filter((p) => p.enabled).map((p) => p.name);
            const enabledTts = data.voice.tts.filter((p) => p.enabled).map((p) => p.name);
            if (enabledStt.length === 0 && enabledTts.length === 0) return /* @__PURE__ */ u(S, { children: "No voice providers enabled" });
            return /* @__PURE__ */ u("div", { className: "flex flex-col gap-0.5", children: [
              enabledStt.length > 0 ? /* @__PURE__ */ u("div", { children: [
                "STT: ",
                /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: enabledStt.join(", ") })
              ] }) : null,
              enabledTts.length > 0 ? /* @__PURE__ */ u("div", { children: [
                "TTS: ",
                /* @__PURE__ */ u("span", { className: "font-medium text-[var(--text)]", children: enabledTts.join(", ") })
              ] }) : null
            ] });
          })()
        }
      ) : null
    ] }),
    /* @__PURE__ */ u("div", { className: "flex flex-wrap items-center gap-3 mt-1", children: [
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn provider-btn-secondary", onClick: onBack, children: t("common:actions.back") }),
      /* @__PURE__ */ u("div", { className: "flex-1" }),
      /* @__PURE__ */ u("button", { type: "button", className: "provider-btn", onClick: onFinish, children: [
        ((_n = data.identity) == null ? void 0 : _n.emoji) || "",
        " ",
        ((_o = data.identity) == null ? void 0 : _o.name) || "Your agent",
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
    t("onboarding:steps.skills"),
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
  const skillsStep = nextIdx++;
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
      step === skillsStep && /* @__PURE__ */ u(SkillsStep, { onNext: goNext, onBack: goBack }),
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
