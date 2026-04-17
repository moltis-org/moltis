// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Identity → Provider → Voice (conditional) →
// Remote Access → Channel → Summary
// No new Rust code — all existing RPC methods and REST endpoints.

import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import {
	addChannel,
	buildTeamsEndpoint,
	channelStorageNote,
	defaultTeamsBaseUrl,
	deriveMatrixAccountId,
	fetchChannelStatus,
	generateWebhookSecretHex,
	MATRIX_DEFAULT_HOMESERVER,
	MATRIX_DOCS_URL,
	MATRIX_ENCRYPTION_GUIDANCE,
	matrixAuthModeGuidance,
	matrixCredentialLabel,
	matrixCredentialPlaceholder,
	matrixOwnershipModeGuidance,
	normalizeMatrixAuthMode,
	normalizeMatrixOtpCooldown,
	normalizeMatrixOwnershipMode,
	parseChannelConfigPatch,
	validateChannelFields,
} from "./channel-utils.js";
import { EmojiPicker } from "./emoji-picker.js";
import { eventListeners, onEvent } from "./events.js";
import { get as getGon, refresh as refreshGon } from "./gon.js";
import { modelVersionScore, sendRpc } from "./helpers.js";
import { t } from "./i18n.js";
import { updateIdentity, validateIdentityFields } from "./identity-utils.js";
import { detectPasskeyName } from "./passkey-detect.js";
import { providerApiKeyHelp } from "./provider-key-help.js";
import { completeProviderOAuth, startProviderOAuth } from "./provider-oauth.js";
import {
	humanizeProbeError,
	isModelServiceNotConfigured,
	saveProviderKey,
	testModel,
	validateProviderKey,
} from "./provider-validation.js";
import * as S from "./state.js";
import { fetchPhrase } from "./tts-phrases.js";
import {
	decodeBase64Safe,
	fetchVoiceProviders,
	saveVoiceKey,
	saveVoiceSettings,
	testTts,
	toggleVoiceProvider,
	transcribeAudio,
	VOICE_COUNTERPART_IDS,
} from "./voice-utils.js";
import { connectWs, subscribeEvents } from "./ws-connect.js";

var wsStarted = false;
function ensureWsConnected() {
	if (wsStarted) return;
	wsStarted = true;
	connectWs({
		backoff: { factor: 2, max: 10000 },
		onConnected: () => {
			subscribeEvents(["channel"]);
		},
		onFrame: (frame) => {
			if (frame.type !== "event") return;
			var listeners = eventListeners[frame.event] || [];
			listeners.forEach((h) => {
				h(frame.payload || {});
			});
		},
	});
}

var WS_RETRY_LIMIT = 75;
var WS_RETRY_DELAY_MS = 200;

// ── Step indicator ──────────────────────────────────────────

function preferredChatPath() {
	var key = localStorage.getItem("moltis-session") || "main";
	return `/chats/${key.replace(/:/g, "/")}`;
}

function detectBrowserTimezone() {
	try {
		var timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
		return typeof timezone === "string" ? timezone.trim() : "";
	} catch {
		return "";
	}
}

function ErrorPanel({ message }) {
	return html`<div role="alert" class="alert-error-text whitespace-pre-line">
		<span class="text-[var(--error)] font-medium">${t("onboarding:errorPrefix")}</span> ${message}
	</div>`;
}

function ChannelStorageNotice() {
	return html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]">
		<span class="font-medium text-[var(--text-strong)]">Storage note.</span> ${channelStorageNote()}
	</div>`;
}

function AdvancedConfigPatchField({ value, onInput }) {
	return html`<details class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3">
		<summary class="cursor-pointer text-xs font-medium text-[var(--text-strong)]">Advanced Config JSON</summary>
		<div class="mt-3 flex flex-col gap-2">
			<div class="text-xs text-[var(--muted)]">
				Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not have dedicated fields yet.
			</div>
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Advanced config JSON patch (optional)</label>
				<textarea
					name="channel_advanced_config"
					class="provider-key-input w-full min-h-[140px] font-mono text-xs"
					value=${value}
					onInput=${(e) => onInput(e.target.value)}
					placeholder='{"reply_to_message": true}'></textarea>
			</div>
		</div>
	</details>`;
}

function StepIndicator({ steps, current }) {
	var ref = useRef(null);
	useEffect(() => {
		if (!ref.current) return;
		var active = ref.current.querySelector(".onboarding-step.active");
		if (active) active.scrollIntoView({ inline: "center", block: "nearest", behavior: "smooth" });
	}, [current]);
	return html`<div class="onboarding-steps" ref=${ref}>
		${steps.map((label, i) => {
			var state = i < current ? "completed" : i === current ? "active" : "";
			var isLast = i === steps.length - 1;
			return html`<${StepDot} key=${i} index=${i} label=${label} state=${state} />
				${!isLast && html`<div class="onboarding-step-line ${i < current ? "completed" : ""}" />`}`;
		})}
	</div>`;
}

function StepDot({ index, label, state }) {
	return html`<div class="onboarding-step ${state}">
		<div class="onboarding-step-dot ${state}">
			${state === "completed" ? html`<span class="icon icon-md icon-checkmark"></span>` : index + 1}
		</div>
		<div class="onboarding-step-label">${label}</div>
	</div>`;
}

// ── Base64url helpers for WebAuthn ───────────────────────────

function base64ToBuffer(b64) {
	var str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	var bin = atob(str);
	var buf = new Uint8Array(bin.length);
	for (var i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer;
}

function bufferToBase64(buf) {
	var bytes = new Uint8Array(buf);
	var str = "";
	for (var b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ── Auth step ───────────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: auth step handles passkey+password+code flows
function AuthStep({ onNext, skippable }) {
	var [method, setMethod] = useState(null); // null | "passkey" | "password"
	var [password, setPassword] = useState("");
	var [confirm, setConfirm] = useState("");
	var [setupCode, setSetupCode] = useState("");
	var [passkeyName, setPasskeyName] = useState("");
	var [codeRequired, setCodeRequired] = useState(false);
	var [localhostOnly, setLocalhostOnly] = useState(false);
	var [webauthnAvailable, setWebauthnAvailable] = useState(false);
	var [error, setError] = useState(null);
	var [saving, setSaving] = useState(false);
	var [loading, setLoading] = useState(true);
	var [passkeyOrigins, setPasskeyOrigins] = useState([]);
	var [passkeyDone, setPasskeyDone] = useState(false);
	var [optPw, setOptPw] = useState("");
	var [optPwConfirm, setOptPwConfirm] = useState("");
	var [optPwSaving, setOptPwSaving] = useState(false);
	var [recoveryKey, setRecoveryKey] = useState(null);
	var [recoveryCopied, setRecoveryCopied] = useState(false);

	var isIpAddress = /^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[");
	var browserSupportsWebauthn = !!window.PublicKeyCredential;
	var passkeyEnabled = webauthnAvailable && browserSupportsWebauthn && !isIpAddress;

	var [setupComplete, setSetupComplete] = useState(false);

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => r.json())
			.then((data) => {
				if (data.setup_code_required) setCodeRequired(true);
				if (data.localhost_only) setLocalhostOnly(true);
				if (data.webauthn_available) setWebauthnAvailable(true);
				if (data.passkey_origins) setPasskeyOrigins(data.passkey_origins);
				if (data.setup_complete) setSetupComplete(true);
				setLoading(false);
			})
			.catch(() => setLoading(false));
	}, []);

	// Pre-select passkey when available (easier than passwords)
	useEffect(() => {
		if (passkeyEnabled && method === null) setMethod("passkey");
	}, [passkeyEnabled]);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: password+code validation
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
		var body = password ? { password } : {};
		if (codeRequired) body.setup_code = setupCode.trim();
		fetch("/api/auth/setup", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					return r
						.json()
						.then((data) => {
							if (data.recovery_key) {
								setRecoveryKey(data.recovery_key);
								setSaving(false);
							} else {
								onNext();
							}
						})
						.catch(() => onNext());
				} else {
					return r.text().then((t) => {
						setError(t || "Setup failed");
						setSaving(false);
					});
				}
			})
			.catch((err) => {
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
		var codeBody = codeRequired ? { setup_code: setupCode.trim() } : {};
		var requestedRpId = null;
		fetch("/api/auth/setup/passkey/register/begin", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(codeBody),
		})
			.then((r) => {
				if (!r.ok) return r.text().then((t) => Promise.reject(new Error(t || "Failed to start passkey registration")));
				return r.json();
			})
			.then((data) => {
				var options = data.options;
				requestedRpId = options.publicKey.rp?.id || null;
				options.publicKey.challenge = base64ToBuffer(options.publicKey.challenge);
				options.publicKey.user.id = base64ToBuffer(options.publicKey.user.id);
				if (options.publicKey.excludeCredentials) {
					for (var c of options.publicKey.excludeCredentials) {
						c.id = base64ToBuffer(c.id);
					}
				}
				return navigator.credentials
					.create({ publicKey: options.publicKey })
					.then((cred) => ({ cred, challengeId: data.challenge_id }));
			})
			.then(({ cred, challengeId }) => {
				var body = {
					challenge_id: challengeId,
					name: passkeyName.trim() || detectPasskeyName(cred),
					credential: {
						id: cred.id,
						rawId: bufferToBase64(cred.rawId),
						type: cred.type,
						response: {
							attestationObject: bufferToBase64(cred.response.attestationObject),
							clientDataJSON: bufferToBase64(cred.response.clientDataJSON),
						},
					},
				};
				if (codeRequired) body.setup_code = setupCode.trim();
				return fetch("/api/auth/setup/passkey/register/finish", {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(body),
				});
			})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					setSaving(false);
					setPasskeyDone(true);
				} else {
					return r.text().then((t) => {
						setError(t || "Passkey registration failed");
						setSaving(false);
					});
				}
			})
			.catch((err) => {
				if (err.name === "NotAllowedError") {
					setError("Passkey registration was cancelled.");
				} else {
					var msg = err.message || "Passkey registration failed";
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
			body: JSON.stringify({ new_password: optPw }),
		})
			.then((r) => {
				if (r.ok) {
					ensureWsConnected();
					onNext();
				} else {
					return r.text().then((t) => {
						setError(t || "Failed to set password");
						setOptPwSaving(false);
					});
				}
			})
			.catch((err) => {
				setError(err.message);
				setOptPwSaving(false);
			});
	}

	if (loading) {
		return html`<div class="text-sm text-[var(--muted)]">Checking authentication\u2026</div>`;
	}

	// Setup already complete (passkeys/password configured) — let user proceed.
	if (setupComplete) {
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:auth.secureYourInstance")}</h2>
			<div class="flex items-center gap-2 text-sm text-[var(--accent)]">
				<span class="icon icon-checkmark"></span>
				Authentication is already configured.
			</div>
			<div class="flex flex-wrap items-center gap-3 mt-1">
				<button type="button" class="provider-btn" onClick=${() => {
					ensureWsConnected();
					onNext();
				}}>Next</button>
			</div>
		</div>`;
	}

	// ── Recovery key display after vault initialization ────
	if (recoveryKey) {
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Secure your instance</h2>

			<div class="flex items-center gap-2 text-sm text-[var(--accent)]">
				<span class="icon icon-checkmark"></span>
				Password set and vault initialized
			</div>

			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--bg);">
				<div class="text-xs text-[var(--muted)]" style="margin-bottom:8px;">Recovery key</div>
				<code class="select-all break-all" style="font-family:var(--font-mono);font-size:.8rem;color:var(--text-strong);display:block;line-height:1.5;">${recoveryKey}</code>
				<div style="display:flex;align-items:center;gap:8px;margin-top:10px;">
					<button type="button" class="provider-btn provider-btn-secondary" onClick=${() => {
						navigator.clipboard.writeText(recoveryKey).then(() => {
							setRecoveryCopied(true);
							setTimeout(() => setRecoveryCopied(false), 2000);
						});
					}}>${recoveryCopied ? "Copied!" : "Copy"}</button>
				</div>
			</div>

			<div class="text-xs" style="color:var(--error);max-width:600px;">
				Save this recovery key in a safe place. It will not be shown again. You need it to unlock the vault if you forget your password.
			</div>

			<div class="flex flex-wrap items-center gap-3 mt-1">
				<button type="button" class="provider-btn" onClick=${onNext}>Continue</button>
			</div>
		</div>`;
	}

	var passkeyDisabledReason = webauthnAvailable
		? browserSupportsWebauthn
			? isIpAddress
				? "Requires domain name"
				: null
			: "Browser not supported"
		: "Not available on this server";

	var originsHint =
		passkeyOrigins.length > 1 ? passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ") : null;

	// ── After passkey registration: optional password ────────
	if (passkeyDone) {
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:auth.secureYourInstance")}</h2>

			<div class="flex items-center gap-2 text-sm text-[var(--accent)]">
				<span class="icon icon-checkmark"></span>
				Passkey registered successfully!
			</div>

			<p class="text-xs text-[var(--muted)] leading-relaxed">
				Optionally set a password as a fallback for when passkeys aren't available.
			</p>

			<form onSubmit=${onOptionalPassword} class="flex flex-col gap-3">
				<div>
					<label for="onboarding-passkey-password" class="text-xs text-[var(--muted)] mb-1 block">Password</label>
					<input
						id="onboarding-passkey-password"
						type="password"
						name="password"
						autocomplete="new-password"
						class="provider-key-input w-full"
						value=${optPw}
						onInput=${(e) => setOptPw(e.target.value)}
						placeholder="At least 12 characters"
						autofocus
					/>
				</div>
				<div>
					<label for="onboarding-passkey-password-confirm" class="text-xs text-[var(--muted)] mb-1 block">Confirm password</label>
					<input
						id="onboarding-passkey-password-confirm"
						type="password"
						name="confirm_password"
						autocomplete="new-password"
						class="provider-key-input w-full"
						value=${optPwConfirm}
						onInput=${(e) => setOptPwConfirm(e.target.value)}
						placeholder="Repeat password"
					/>
				</div>
				${error && html`<${ErrorPanel} message=${error} />`}
				<div class="flex flex-wrap items-center gap-3 mt-1">
					<button key=${`pw-${optPwSaving}`} type="submit" class="provider-btn" disabled=${optPwSaving}>${optPwSaving ? "Setting\u2026" : "Set password & continue"}</button>
					<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${() => {
						ensureWsConnected();
						onNext();
					}}>Skip</button>
				</div>
			</form>
		</div>`;
	}

	// ── Method selection ─────────────────────────────────────
	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:auth.secureYourInstance")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			${
				localhostOnly
					? "Choose how to secure your instance, or skip for now. Setting a password also enables the encryption vault, which protects API keys and secrets stored in the database."
					: "Choose how to secure your instance."
			}
		</p>

		${
			codeRequired &&
			html`<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Setup code</label>
			<input type="text" class="provider-key-input w-full" inputmode="numeric" pattern="[0-9]*"
				value=${setupCode} onInput=${(e) => setSetupCode(e.target.value)}
				placeholder="6-digit code from terminal" />
			<div class="text-xs text-[var(--muted)] mt-1">Find this code in the moltis process log (stdout).</div>
		</div>`
		}

		<div class="flex flex-col gap-2">
			<div class=${`backend-card ${method === "passkey" ? "selected" : ""} ${passkeyEnabled ? "" : "disabled"}`}
				onClick=${passkeyEnabled ? () => setMethod("passkey") : null}>
				<div class="flex flex-wrap items-center justify-between gap-2">
					<span class="text-sm font-medium text-[var(--text)]">Passkey</span>
					<div class="flex flex-wrap gap-2 justify-end">
						${passkeyEnabled ? html`<span class="recommended-badge">Recommended</span>` : null}
						${passkeyDisabledReason ? html`<span class="tier-badge">${passkeyDisabledReason}</span>` : null}
					</div>
				</div>
				<div class="text-xs text-[var(--muted)] mt-1">Use Touch ID, Face ID, or a security key</div>
			</div>
			<div class=${`backend-card ${method === "password" ? "selected" : ""}`}
				onClick=${() => setMethod("password")}>
				<div class="flex flex-wrap items-center justify-between gap-2">
					<span class="text-sm font-medium text-[var(--text)]">Password</span>
				</div>
				<div class="text-xs text-[var(--muted)] mt-1">Set a password and enable the encryption vault for stored secrets</div>
			</div>
		</div>

		${
			method === "passkey" &&
			html`<div class="flex flex-col gap-3">
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Passkey name</label>
				<input type="text" class="provider-key-input w-full"
					value=${passkeyName} onInput=${(e) => setPasskeyName(e.target.value)}
					placeholder="e.g. MacBook Touch ID (optional)" />
			</div>
			${originsHint && html`<div class="text-xs text-[var(--muted)]">Passkeys will work when visiting: ${originsHint}</div>`}
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex flex-wrap items-center gap-3 mt-1">
				<button key=${`pk-${saving}`} type="button" class="provider-btn" disabled=${saving} onClick=${onPasskeyRegister}>${saving ? "Registering\u2026" : "Register passkey"}</button>
				${
					skippable
						? html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>`
						: null
				}
			</div>
		</div>`
		}

		${
			method === "password" &&
			html`<form onSubmit=${onPasswordSubmit} class="flex flex-col gap-3">
				<div>
					<label for="onboarding-password" class="text-xs text-[var(--muted)] mb-1 block">Password${localhostOnly ? "" : " *"}</label>
					<input
						id="onboarding-password"
						type="password"
						name="password"
						autocomplete="new-password"
						class="provider-key-input w-full"
						value=${password}
						onInput=${(e) => setPassword(e.target.value)}
						placeholder=${localhostOnly ? "Optional on localhost" : "At least 12 characters"}
						autofocus
					/>
				</div>
				<div>
					<label for="onboarding-password-confirm" class="text-xs text-[var(--muted)] mb-1 block">Confirm password</label>
					<input
						id="onboarding-password-confirm"
						type="password"
						name="confirm_password"
						autocomplete="new-password"
						class="provider-key-input w-full"
						value=${confirm}
						onInput=${(e) => setConfirm(e.target.value)}
						placeholder="Repeat password"
					/>
				</div>
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex flex-wrap items-center gap-3 mt-1">
				<button key=${`auth-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Setting up\u2026" : localhostOnly && !password ? "Skip" : "Set password"}</button>
				${
					skippable
						? html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>`
						: null
				}
			</div>
		</form>`
		}

		${
			method === null &&
			html`<div class="flex flex-wrap items-center gap-3 mt-1">
			${
				skippable
					? html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>`
					: null
			}
		</div>`
		}
	</div>`;
}

// ── Identity step ───────────────────────────────────────────

function IdentityStep({ onNext, onBack }) {
	var identity = getGon("identity") || {};
	var [userName, setUserName] = useState(identity.user_name || "");
	var [name, setName] = useState(identity.name || "Moltis");
	var [emoji, setEmoji] = useState(identity.emoji || "\u{1f916}");
	var [theme, setTheme] = useState(identity.theme || "");
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);

	useEffect(() => {
		var cancelled = false;
		refreshGon().then(() => {
			if (cancelled) return;
			var refreshed = getGon("identity") || {};
			if (refreshed.user_name) setUserName((prev) => prev || refreshed.user_name);
			if (refreshed.name) setName((prev) => (prev && prev !== "Moltis" ? prev : refreshed.name));
			if (refreshed.emoji) setEmoji((prev) => (prev && prev !== "\u{1f916}" ? prev : refreshed.emoji));
			if (refreshed.theme) setTheme((prev) => prev || refreshed.theme);
		});
		return () => {
			cancelled = true;
		};
	}, []);

	function onSubmit(e) {
		e.preventDefault();
		var v = validateIdentityFields(name, userName);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		setError(null);
		setSaving(true);
		var userTimezone = detectBrowserTimezone();
		updateIdentity({
			name: name.trim(),
			emoji: emoji.trim() || "",
			theme: theme.trim() || "",
			user_name: userName.trim(),
			user_timezone: userTimezone || "",
		}).then((res) => {
			setSaving(false);
			if (res?.ok) {
				refreshGon();
				onNext();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
		});
	}

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:identity.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Tell us about yourself and customise your agent.</p>
		<form onSubmit=${onSubmit} class="flex flex-col gap-4">
			<!-- User section -->
			<div>
				<div class="text-xs text-[var(--muted)] mb-1">Your name *</div>
				<input type="text" class="provider-key-input w-full"
					value=${userName} onInput=${(e) => setUserName(e.target.value)}
					placeholder="e.g. Alice" autofocus />
			</div>
			<!-- Agent section -->
			<div class="flex flex-col gap-3">
				<div class="grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_auto] md:gap-x-4">
					<div class="min-w-0">
						<div class="text-xs text-[var(--muted)] mb-1">Agent name *</div>
						<input type="text" class="provider-key-input w-full"
							value=${name} onInput=${(e) => setName(e.target.value)}
							placeholder="e.g. Rex" />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)] mb-1">Emoji</div>
						<${EmojiPicker} value=${emoji} onChange=${setEmoji} />
					</div>
				</div>
				<div>
					<div class="text-xs text-[var(--muted)] mb-1">Theme</div>
					<input type="text" class="provider-key-input w-full"
						value=${theme} onInput=${(e) => setTheme(e.target.value)}
						placeholder="wise owl, chill fox, witty robot\u2026" />
				</div>
			</div>
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex flex-wrap items-center gap-3 mt-1">
				${
					onBack
						? html`<button type="button" class="provider-btn provider-btn-secondary" onClick=${onBack}>${t("common:actions.back")}</button>`
						: null
				}
				<button key=${`id-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Saving\u2026" : "Continue"}</button>
			</div>
		</form>
	</div>`;
}

// ── Provider step ───────────────────────────────────────────

var OPENAI_COMPATIBLE = ["openai", "mistral", "openrouter", "cerebras", "minimax", "moonshot", "venice", "ollama"];
var BYOM_PROVIDERS = ["venice"];
var RECOMMENDED_PROVIDERS = new Set([
	"anthropic",
	"openai",
	"gemini",
	"deepseek",
	"minimax",
	"zai",
	"ollama",
	"local-llm",
	"lmstudio",
]);

function ModelSelectCard({ model, selected, probe, onToggle }) {
	var probeError = probe && probe !== "ok" && probe !== "probing" ? probe.error || "" : "";
	return html`<div class="model-card ${selected ? "selected" : ""}" onClick=${onToggle}>
		<div class="flex flex-wrap items-center justify-between gap-2">
			<span class="text-sm font-medium text-[var(--text)]">${model.displayName}</span>
			<div class="flex flex-wrap gap-2 justify-end">
				${model.supportsTools ? html`<span class="recommended-badge">Tools</span>` : null}
				${probe === "probing" ? html`<span class="tier-badge">Probing\u2026</span>` : null}
				${probeError ? html`<span class="provider-item-badge warning">Unsupported</span>` : null}
			</div>
		</div>
		<div class="text-xs text-[var(--muted)] mt-1 font-mono">${model.id}</div>
		${probeError ? html`<div class="text-xs font-medium text-[var(--danger,#ef4444)] mt-0.5">${probeError}</div>` : null}
		${model.createdAt ? html`<time class="text-xs text-[var(--muted)] mt-0.5 opacity-60 block" data-epoch-ms=${model.createdAt * 1000} data-format="year-month"></time>` : null}
	</div>`;
}

// ── Provider row for multi-provider onboarding ──────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config forms for api-key, oauth, and local flows
function OnboardingProviderRow({
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
	onCancelLocal,
}) {
	var isApiKeyForm = configuring === provider.name && (phase === "form" || phase === "validating");
	var isModelSelect = configuring === provider.name && phase === "selectModel";
	var isOAuth = oauthProvider === provider.name;
	var isLocal = localProvider === provider.name;
	var isExpanded = isApiKeyForm || isModelSelect || isOAuth || isLocal;
	var keyInputRef = useRef(null);
	var rowRef = useRef(null);

	useEffect(() => {
		if (isApiKeyForm && keyInputRef.current) {
			keyInputRef.current.focus();
		}
	}, [isApiKeyForm]);

	useEffect(() => {
		if (isExpanded && rowRef.current) {
			rowRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
		}
	}, [isExpanded]);

	var supportsEndpoint = OPENAI_COMPATIBLE.includes(provider.name);
	var needsModel = BYOM_PROVIDERS.includes(provider.name);
	var keyHelp = providerApiKeyHelp(provider);

	var [showAllModels, setShowAllModels] = useState(false);
	var DEFAULT_VISIBLE = 3;

	// Sort models: recommended > newest date > highest version > alpha.
	var sortedModels = (providerModels || []).slice().sort((a, b) => {
		var aRec = a.recommended ? 1 : 0;
		var bRec = b.recommended ? 1 : 0;
		if (aRec !== bRec) return bRec - aRec;
		var aTime = a.createdAt || 0;
		var bTime = b.createdAt || 0;
		if (aTime !== bTime) return bTime - aTime;
		var aVer = modelVersionScore(a.id);
		var bVer = modelVersionScore(b.id);
		if (aVer !== bVer) return bVer - aVer;
		return (a.displayName || a.id).localeCompare(b.displayName || b.id);
	});

	// Filter models for the model selector.
	var filteredModels = sortedModels.filter(
		(m) =>
			!modelSearch ||
			m.displayName.toLowerCase().includes(modelSearch.toLowerCase()) ||
			m.id.toLowerCase().includes(modelSearch.toLowerCase()),
	);

	var hasMoreModels = filteredModels.length > DEFAULT_VISIBLE && !modelSearch;
	var visibleModels = showAllModels || modelSearch ? filteredModels : filteredModels.slice(0, DEFAULT_VISIBLE);
	var hiddenModelCount = filteredModels.length - DEFAULT_VISIBLE;

	return html`<div ref=${rowRef} class="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3">
		<div class="flex items-center gap-3">
			<div class="flex-1 min-w-0 flex flex-col gap-0.5">
				<div class="flex items-center gap-2 flex-wrap">
					<span class="text-sm font-medium text-[var(--text-strong)]">${provider.displayName}</span>
					${provider.configured ? html`<span class="provider-item-badge configured">configured</span>` : null}
					${
						validationResult?.ok === true
							? html`<span class="icon icon-md icon-check-circle inline-block" style="color:var(--ok)"></span>`
							: null
					}
					<span class="provider-item-badge ${provider.authType}">
						${provider.authType === "oauth" ? "OAuth" : provider.authType === "local" ? "Local" : "API Key"}
					</span>
				</div>
			</div>
			<div class="shrink-0">
				${
					isExpanded
						? null
						: html`<button class="provider-btn provider-btn-secondary provider-btn-sm"
							onClick=${() => onStartConfigure(provider.name)}>${provider.configured ? "Choose Model" : "Configure"}</button>`
				}
			</div>
		</div>
		${
			validationResult?.ok === false && !isExpanded
				? html`<div class="text-xs text-[var(--warning)] mt-1">${validationResult.message}</div>`
				: null
		}
		${
			isApiKeyForm
				? html`<form onSubmit=${onSaveKey} class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
				<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">API Key</label>
					<input type="password" class="provider-key-input w-full"
						ref=${keyInputRef}
						value=${apiKey} onInput=${(e) => setApiKey(e.target.value)}
						placeholder=${provider.keyOptional ? "(optional)" : "sk-..."} />
					${
						keyHelp
							? html`<div class="text-xs text-[var(--muted)] mt-1">
							${
								keyHelp.url
									? html`${keyHelp.text} <a href=${keyHelp.url} target="_blank" rel="noopener noreferrer" class="text-[var(--accent)] underline">${keyHelp.label || keyHelp.url}</a>`
									: keyHelp.text
							}
						</div>`
							: null
					}
				</div>
				${
					supportsEndpoint
						? html`<div>
						<label class="text-xs text-[var(--muted)] mb-1 block">Endpoint (optional)</label>
						<input type="text" class="provider-key-input w-full"
							value=${endpoint} onInput=${(e) => setEndpoint(e.target.value)}
							placeholder=${provider.defaultBaseUrl || "https://api.example.com/v1"} />
						<div class="text-xs text-[var(--muted)] mt-1">Leave empty to use the default endpoint.</div>
					</div>`
						: null
				}
				${
					needsModel
						? html`<div>
						<label class="text-xs text-[var(--muted)] mb-1 block">Model ID</label>
						<input type="text" class="provider-key-input w-full"
							value=${model} onInput=${(e) => setModel(e.target.value)}
							placeholder="model-id" />
					</div>`
						: null
				}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<div class="flex items-center gap-2 mt-1">
					<button key=${`prov-${phase}`} type="submit" class="provider-btn provider-btn-sm" disabled=${phase === "validating"}>${phase === "validating" ? "Saving\u2026" : "Save"}</button>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onCancelConfigure} disabled=${phase === "validating"}>Cancel</button>
				</div>
				${phase === "validating" ? html`<div class="text-xs text-[var(--muted)] mt-1">Discovering available models\u2026</div>` : null}
			</form>`
				: null
		}
		${
			isModelSelect
				? html`<div class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
				<div class="text-xs font-medium text-[var(--text-strong)]">Select preferred models</div>
				<div class="text-xs text-[var(--muted)]">Selected models appear first in the session model selector.</div>
				${
					(providerModels || []).length > 5
						? html`<input type="text" class="provider-key-input w-full text-xs"
							placeholder="Search models\u2026"
							value=${modelSearch}
							onInput=${(e) => setModelSearch(e.target.value)} />`
						: null
				}
				<div class="flex flex-col gap-1">
					${
						visibleModels.length === 0
							? html`<div class="text-xs text-[var(--muted)] py-4 text-center">No models match your search.</div>`
							: visibleModels.map(
									(m) => html`<${ModelSelectCard} key=${m.id} model=${m}
										selected=${selectedModels.has(m.id)}
										probe=${probeResults.get(m.id)}
										onToggle=${() => onToggleModel(m.id)} />`,
								)
					}
					${
						hasMoreModels
							? html`<button
						class="text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none py-1 text-left hover:underline"
						onClick=${() => setShowAllModels(!showAllModels)}
					>${showAllModels ? t("providers:showFewerModels") : t("providers:showAllModels", { count: hiddenModelCount })}</button>`
							: null
					}
				</div>
				<div class="text-xs text-[var(--muted)]">${selectedModels.size === 0 ? "No models selected" : `${selectedModels.size} model${selectedModels.size > 1 ? "s" : ""} selected`}</div>
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<div class="flex items-center gap-2 mt-1">
					<button key=${`models-${savingModels}`} type="button" class="provider-btn provider-btn-sm" disabled=${selectedModels.size === 0 || savingModels} onClick=${onSaveModels}>${savingModels ? "Saving\u2026" : "Save"}</button>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onCancelConfigure} disabled=${savingModels}>Cancel</button>
				</div>
				${savingModels ? html`<div class="text-xs text-[var(--muted)] mt-1">Saving credentials and validating selected models\u2026</div>` : null}
			</div>`
				: null
		}
			${
				isOAuth
					? html`<div class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					${
						oauthInfo?.status === "device"
							? html`<div class="text-sm text-[var(--text)]">
						Open <a href=${oauthInfo.uri} target="_blank" class="text-[var(--accent)] underline">${oauthInfo.uri}</a> and enter code:<strong class="font-mono ml-1">${oauthInfo.code}</strong>
					</div>`
							: html`<div class="text-sm text-[var(--muted)]">Waiting for authentication\u2026</div>`
					}
					${
						oauthInfo?.status === "device"
							? null
							: html`<div class="text-xs text-[var(--muted)]">If localhost callback fails, paste the redirect URL (or code#state) below.</div>
							<input
								type="text"
								class="provider-key-input w-full"
								placeholder="http://localhost:1455/auth/callback?code=...&state=..."
								value=${oauthCallbackInput}
								onInput=${(event) => setOauthCallbackInput(event.target.value)}
								disabled=${oauthSubmitting}
							/>
							<button
								class="provider-btn provider-btn-secondary provider-btn-sm self-start"
								onClick=${() => onSubmitOAuthCallback(provider.name)}
								disabled=${oauthSubmitting}
							>
								${oauthSubmitting ? "Submitting..." : "Submit Callback"}
							</button>`
					}
					${error ? html`<${ErrorPanel} message=${error} />` : null}
					<button class="provider-btn provider-btn-secondary provider-btn-sm self-start" onClick=${onCancelOAuth}>Cancel</button>
				</div>`
					: null
			}
		${
			isLocal
				? html`<div class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
				${
					sysInfo
						? html`<div class="flex flex-col gap-3">
						<div class="flex gap-3 text-xs text-[var(--muted)]">
							<span>RAM: ${sysInfo.totalRamGb}GB</span>
							<span>Tier: ${sysInfo.memoryTier}</span>
							${sysInfo.hasGpu ? html`<span class="text-[var(--ok)]">GPU available</span>` : null}
						</div>
						${
							sysInfo.isAppleSilicon && (sysInfo.availableBackends || []).length > 0
								? html`<div class="flex flex-col gap-2">
								<div class="text-xs font-medium text-[var(--text-strong)]">Backend</div>
								<div class="flex flex-col gap-2">
									${(sysInfo.availableBackends || []).map(
										// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: backend card renders conditional badges inline
										(b) => html`<div key=${b.id}
										class="backend-card ${b.id === selectedBackend ? "selected" : ""} ${b.available ? "" : "disabled"}"
										onClick=${() => {
											if (b.available) setSelectedBackend(b.id);
										}}>
										<div class="flex flex-wrap items-center justify-between gap-2">
											<span class="text-sm font-medium text-[var(--text)]">${b.name}</span>
											<div class="flex flex-wrap gap-2 justify-end">
												${b.id === sysInfo.recommendedBackend && b.available ? html`<span class="recommended-badge">Recommended</span>` : null}
												${b.available ? null : html`<span class="tier-badge">Not installed</span>`}
											</div>
										</div>
										<div class="text-xs text-[var(--muted)] mt-1">${b.description}</div>
									</div>`,
									)}
								</div>
							</div>`
								: null
						}
						<div class="text-xs font-medium text-[var(--text-strong)]">Select a model</div>
						<div class="flex flex-col gap-2">
							${
								localModels.filter((m) => m.backend === selectedBackend).length === 0
									? html`<div class="text-xs text-[var(--muted)] py-4 text-center">No models available for ${selectedBackend}</div>`
									: localModels
											.filter((m) => m.backend === selectedBackend)
											.map(
												(mdl) => html`<div key=${mdl.id} class="model-card" onClick=${() => onConfigureLocalModel(mdl)}>
											<div class="flex flex-wrap items-center justify-between gap-2">
												<span class="text-sm font-medium text-[var(--text)]">${mdl.displayName}</span>
												<div class="flex flex-wrap gap-2 justify-end">
													<span class="tier-badge">${mdl.minRamGb}GB</span>
													${mdl.suggested ? html`<span class="recommended-badge">Recommended</span>` : null}
												</div>
											</div>
											<div class="text-xs text-[var(--muted)] mt-1">Context: ${(mdl.contextWindow / 1000).toFixed(0)}k tokens</div>
										</div>`,
											)
							}
						</div>
						${saving ? html`<div class="text-xs text-[var(--muted)]">Configuring\u2026</div>` : null}
					</div>`
						: html`<div class="text-xs text-[var(--muted)]">Loading system info\u2026</div>`
				}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<button class="provider-btn provider-btn-secondary provider-btn-sm self-start" onClick=${onCancelLocal}>Cancel</button>
			</div>`
				: null
		}
	</div>`;
}

function sortProviders(list) {
	list.sort((a, b) => {
		var aOrder = Number.isFinite(a.uiOrder) ? a.uiOrder : Number.MAX_SAFE_INTEGER;
		var bOrder = Number.isFinite(b.uiOrder) ? b.uiOrder : Number.MAX_SAFE_INTEGER;
		if (aOrder !== bOrder) return aOrder - bOrder;
		return a.displayName.localeCompare(b.displayName);
	});
	return list;
}

function normalizeProviderToken(value) {
	return String(value || "")
		.toLowerCase()
		.replace(/[^a-z0-9]/g, "");
}

function normalizeModelToken(value) {
	return String(value || "")
		.trim()
		.toLowerCase();
}

function stripModelNamespace(modelId) {
	if (!modelId || typeof modelId !== "string") return "";
	var sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

function resolveSavedModelSelection(savedModels, availableModels) {
	var selected = new Set();
	if (!(savedModels?.length > 0) || availableModels.length === 0) return selected;

	var exactIdLookup = new Map();
	var rawIdLookup = new Map();
	for (var model of availableModels) {
		var id = String(model?.id || "").trim();
		if (!id) continue;
		exactIdLookup.set(normalizeModelToken(id), id);
		var rawId = normalizeModelToken(stripModelNamespace(id));
		if (rawId && !rawIdLookup.has(rawId)) {
			rawIdLookup.set(rawId, id);
		}
	}

	for (var savedModel of savedModels) {
		var savedNorm = normalizeModelToken(savedModel);
		if (!savedNorm) continue;
		var exact = exactIdLookup.get(savedNorm);
		if (exact) {
			selected.add(exact);
			continue;
		}
		var raw = normalizeModelToken(stripModelNamespace(savedModel));
		var mapped = rawIdLookup.get(raw);
		if (mapped) {
			selected.add(mapped);
		}
	}
	return selected;
}

function modelBelongsToProvider(providerName, model) {
	var needle = normalizeProviderToken(providerName);
	if (!needle) return false;
	var modelProvider = normalizeProviderToken(model?.provider);
	if (modelProvider?.includes(needle)) {
		return true;
	}
	var modelId = String(model?.id || "");
	var modelPrefix = normalizeProviderToken(modelId.split("::")[0]);
	return modelPrefix === needle;
}

function toModelSelectorRow(modelRow) {
	return {
		id: modelRow.id,
		displayName: modelRow.displayName || modelRow.id,
		provider: modelRow.provider,
		supportsTools: modelRow.supportsTools,
		createdAt: modelRow.createdAt || 0,
	};
}

function ProviderStep({ onNext, onBack }) {
	var [providers, setProviders] = useState([]);
	var [loading, setLoading] = useState(true);
	var [error, setError] = useState(null);
	var [showAllProviders, setShowAllProviders] = useState(false);

	// Which provider has an open inline form (by name), or null
	var [configuring, setConfiguring] = useState(null);
	var [oauthProvider, setOauthProvider] = useState(null);
	var [localProvider, setLocalProvider] = useState(null);

	// Phase: "form" | "validating" | "selectModel"
	var [phase, setPhase] = useState("form");
	var [providerModels, setProviderModels] = useState([]);
	var [selectedModels, setSelectedModels] = useState(new Set());
	var [probeResults, setProbeResults] = useState(new Map());
	var [modelSearch, setModelSearch] = useState("");
	var [savingModels, setSavingModels] = useState(false);

	// Track provider whose credentials already exist and only model selection is needed.
	var [modelSelectProvider, setModelSelectProvider] = useState(null);

	// API key form state
	var [apiKey, setApiKey] = useState("");
	var [endpoint, setEndpoint] = useState("");
	var [model, setModel] = useState("");
	var [saving, setSaving] = useState(false);

	// Validation results: { [providerName]: { ok, message } }
	var [validationResults, setValidationResults] = useState({});

	// OAuth state
	var [oauthInfo, setOauthInfo] = useState(null);
	var [oauthCallbackInput, setOauthCallbackInput] = useState("");
	var [oauthSubmitting, setOauthSubmitting] = useState(false);
	var oauthTimerRef = useRef(null);

	// Local state
	var [sysInfo, setSysInfo] = useState(null);
	var [localModels, setLocalModels] = useState([]);
	var [selectedBackend, setSelectedBackend] = useState(null);

	function refreshProviders() {
		return sendRpc("providers.available", {}).then((res) => {
			if (res?.ok) {
				var list = sortProviders(res.payload || []);
				setProviders(list);
			}
			return res;
		});
	}

	useEffect(() => {
		var cancelled = false;
		var attempts = 0;

		function loadProviders() {
			if (cancelled) return;
			sendRpc("providers.available", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setProviders(sortProviders(res.payload || []));
					setLoading(false);
					return;
				}

				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
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

	// Cleanup OAuth timer on unmount
	useEffect(() => {
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
		setSelectedModels(new Set());
		setProbeResults(new Map());
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
		var modelsRes = await sendRpc("models.list", {});
		var allModels = modelsRes?.ok ? modelsRes.payload || [] : [];
		return allModels.filter((m) => modelBelongsToProvider(providerName, m)).map(toModelSelectorRow);
	}

	async function openModelSelectForConfiguredApiProvider(provider) {
		if (provider.authType !== "api-key" || !provider.configured) return false;
		var existingModels = await loadModelsForProvider(provider.name);
		if (existingModels.length === 0) return false;

		// Pre-select already-saved preferred models, mapping raw IDs to namespaced IDs.
		var saved = resolveSavedModelSelection(provider.models || [], existingModels);

		setModelSelectProvider(provider.name);
		setConfiguring(provider.name);
		setProviderModels(existingModels);
		setSelectedModels(saved);
		setPhase("selectModel");
		return true;
	}

	async function onStartConfigure(name) {
		closeAll();
		var p = providers.find((pr) => pr.name === name);
		if (!p) return;
		if (p.authType === "api-key") {
			setEndpoint(p.baseUrl || "");
			setModel(p.model || "");
			if (await openModelSelectForConfiguredApiProvider(p)) return;
			setConfiguring(name);
			setPhase("form");
			return;
		} else if (p.authType === "oauth") {
			startOAuth(p);
		} else if (p.authType === "local") {
			startLocal(p);
		}
	}

	// ── API key form ─────────────────────────────────────────

	function onSaveKey(e) {
		e.preventDefault();
		var p = providers.find((pr) => pr.name === configuring);
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

		var keyVal = apiKey.trim() || p.name;
		var endpointVal = endpoint.trim() || null;
		var modelVal = model.trim() || null;

		validateProviderKey(p.name, keyVal, endpointVal, modelVal)
			.then(async (result) => {
				if (!result.valid) {
					// Validation failed — stay on the form.
					setPhase("form");
					setError(result.error || "Validation failed. Please check your credentials.");
					return;
				}

				// BYOM providers: we already tested the specific model during validation,
				// so save immediately without showing the model selector.
				if (BYOM_PROVIDERS.includes(p.name)) {
					return saveAndFinishByom(p.name, keyVal, endpointVal, modelVal);
				}

				// Persist credentials before opening model selection so probes
				// and follow-up actions use an initialized provider registry.
				var saveRes = await saveProviderKey(p.name, keyVal, endpointVal, modelVal);
				if (!saveRes?.ok) {
					setPhase("form");
					setError(saveRes?.error?.message || "Failed to save credentials.");
					return;
				}

				// Regular providers: show the model selector.
				setProviderModels(result.models || []);
				setPhase("selectModel");
			})
			.catch((err) => {
				setPhase("form");
				setError(err?.message || "Validation failed.");
			});
	}

	function probeModelAsync(modelId) {
		setProbeResults((prev) => {
			var next = new Map(prev);
			next.set(modelId, "probing");
			return next;
		});
		testModel(modelId).then((result) => {
			setProbeResults((prev) => {
				var next = new Map(prev);
				if (isModelServiceNotConfigured(result.error || "")) {
					next.delete(modelId);
				} else {
					next.set(modelId, result.ok ? "ok" : { error: humanizeProbeError(result.error || "Unsupported") });
				}
				return next;
			});
		});
	}

	function onToggleModel(modelId) {
		setSelectedModels((prev) => {
			var next = new Set(prev);
			if (next.has(modelId)) {
				next.delete(modelId);
			} else {
				next.add(modelId);
				probeModelAsync(modelId);
			}
			return next;
		});
	}

	function resolveCredentials(providerName, modelIds) {
		var p = providers.find((pr) => pr.name === providerName);
		if (!p) return null;
		var keyVal = apiKey.trim() || p.name;
		var endpointVal = endpoint.trim() || null;
		var modelVal = model.trim() || null;
		var effectiveModelVal = p.keyOptional && modelIds.length > 0 ? modelIds[0] : modelVal;
		return { keyVal, endpointVal, modelVal: effectiveModelVal };
	}

	async function saveProviderKeyIfNeeded(providerName, modelIds) {
		if (modelSelectProvider) return true;
		var creds = resolveCredentials(providerName, modelIds);
		if (!creds) return false;
		var res = await saveProviderKey(providerName, creds.keyVal, creds.endpointVal, creds.modelVal);
		if (!res?.ok) {
			setPhase("form");
			setError(res?.error?.message || "Failed to save credentials.");
			return false;
		}
		return true;
	}

	async function onSaveSelectedModels() {
		var providerName = modelSelectProvider || configuring;
		if (!providerName) return false;
		var modelIds = Array.from(selectedModels);

		setSavingModels(true);
		setError(null);

		try {
			if (!(await saveProviderKeyIfNeeded(providerName, modelIds))) {
				setSavingModels(false);
				return false;
			}
			var res = await sendRpc("providers.save_models", { provider: providerName, models: modelIds });
			if (!res?.ok) {
				setSavingModels(false);
				setError(res?.error?.message || "Failed to save model preferences.");
				return false;
			}
			if (modelIds.length > 0) {
				localStorage.setItem("moltis-model", modelIds[0]);
			}
			setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
			closeAll();
			refreshProviders();
			return true;
		} catch (err) {
			setSavingModels(false);
			setError(err?.message || "Failed to save credentials.");
			return false;
		}
	}

	async function onContinue() {
		var hasPendingModelSelection =
			phase === "selectModel" && (configuring || modelSelectProvider) && selectedModels.size > 0;
		if (hasPendingModelSelection) {
			var saved = await onSaveSelectedModels();
			if (!saved) return;
		}
		onNext();
	}

	// BYOM-only save path (no model selector shown for these providers).
	function saveAndFinishByom(providerName, keyVal, endpointVal, modelVal) {
		saveProviderKey(providerName, keyVal, endpointVal, modelVal)
			.then(async (res) => {
				if (!res?.ok) {
					setPhase("form");
					setError(res?.error?.message || "Failed to save credentials.");
					return;
				}

				// Test the specific model from the live registry.
				if (modelVal) {
					var testResult = await testModel(modelVal);
					var modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
					if (!(testResult.ok || modelServiceUnavailable)) {
						setPhase("form");
						setError(testResult.error || "Model test failed. Check your model ID.");
						return;
					}
					await sendRpc("providers.save_models", { provider: providerName, models: [modelVal] });
					localStorage.setItem("moltis-model", modelVal);
				}

				setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
				setConfiguring(null);
				setPhase("form");
				setProviderModels([]);
				setSelectedModels(new Set());
				setProbeResults(new Map());
				setModelSearch("");
				setApiKey("");
				setEndpoint("");
				setModel("");
				setError(null);
				refreshProviders();
			})
			.catch((err) => {
				setPhase("form");
				setError(err?.message || "Failed to save credentials.");
			});
	}

	// ── OAuth flow ───────────────────────────────────────────

	function startOAuth(p) {
		setOauthProvider(p.name);
		setOauthInfo({ status: "starting" });
		setOauthCallbackInput("");
		setOauthSubmitting(false);
		startProviderOAuth(p.name).then((result) => {
			if (result.status === "already") {
				onOAuthAuthenticated(p.name);
			} else if (result.status === "browser") {
				window.open(result.authUrl, "_blank");
				setOauthInfo({ status: "waiting" });
				pollOAuth(p);
			} else if (result.status === "device") {
				setOauthInfo({
					status: "device",
					uri: result.verificationUrl,
					code: result.userCode,
				});
				pollOAuth(p);
			} else {
				setError(result.error || "Failed to start OAuth");
				setOauthProvider(null);
				setOauthInfo(null);
				setOauthCallbackInput("");
				setOauthSubmitting(false);
			}
		});
	}

	async function onOAuthAuthenticated(providerName) {
		var provModels = await loadModelsForProvider(providerName);

		setOauthProvider(null);
		setOauthInfo(null);
		setOauthCallbackInput("");
		setOauthSubmitting(false);

		if (provModels.length > 0) {
			setModelSelectProvider(providerName);
			setConfiguring(providerName);
			setProviderModels(provModels);
			setSelectedModels(new Set());
			setPhase("selectModel");
		} else {
			setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
		}
		refreshProviders();
	}

	function pollOAuth(p) {
		var attempts = 0;
		if (oauthTimerRef.current) clearInterval(oauthTimerRef.current);
		oauthTimerRef.current = setInterval(() => {
			attempts++;
			if (attempts > 60) {
				clearInterval(oauthTimerRef.current);
				oauthTimerRef.current = null;
				setError("OAuth timed out. Please try again.");
				setOauthProvider(null);
				setOauthInfo(null);
				setOauthCallbackInput("");
				setOauthSubmitting(false);
				return;
			}
			sendRpc("providers.oauth.status", { provider: p.name }).then((res) => {
				if (res?.ok && res.payload?.authenticated) {
					clearInterval(oauthTimerRef.current);
					oauthTimerRef.current = null;
					onOAuthAuthenticated(p.name);
				}
			});
		}, 2000);
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
		var callback = oauthCallbackInput.trim();
		if (!callback) {
			setError("Paste the callback URL (or code#state) to continue.");
			return;
		}

		setOauthSubmitting(true);
		setError(null);
		completeProviderOAuth(providerName, callback)
			.then((res) => {
				if (res?.ok) {
					if (oauthTimerRef.current) {
						clearInterval(oauthTimerRef.current);
						oauthTimerRef.current = null;
					}
					onOAuthAuthenticated(providerName);
					return;
				}
				setError(res?.error?.message || "Failed to complete OAuth callback.");
			})
			.catch((err) => {
				setError(err?.message || "Failed to complete OAuth callback.");
			})
			.finally(() => {
				setOauthSubmitting(false);
			});
	}

	// ── Local model flow ─────────────────────────────────────

	function startLocal(p) {
		setLocalProvider(p.name);
		sendRpc("providers.local.system_info", {}).then((sysRes) => {
			if (!sysRes?.ok) {
				setError(sysRes?.error?.message || "Failed to get system info");
				setLocalProvider(null);
				return;
			}
			setSysInfo(sysRes.payload);
			setSelectedBackend(sysRes.payload.recommendedBackend || "GGUF");
			sendRpc("providers.local.models", {}).then((modelsRes) => {
				if (modelsRes?.ok) {
					setLocalModels(modelsRes.payload?.recommended || []);
				}
			});
		});
	}

	function configureLocalModel(mdl) {
		var provName = localProvider;
		setSaving(true);
		setError(null);
		sendRpc("providers.local.configure", { modelId: mdl.id, backend: selectedBackend }).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setLocalProvider(null);
				setSysInfo(null);
				setLocalModels([]);
				setValidationResults((prev) => ({ ...prev, [provName]: { ok: true, message: null } }));
				refreshProviders();
			} else {
				setError(res?.error?.message || "Failed to configure model");
			}
		});
	}

	function cancelLocal() {
		setLocalProvider(null);
		setSysInfo(null);
		setLocalModels([]);
		setError(null);
	}

	// ── Render ────────────────────────────────────────────────

	if (loading) {
		return html`<div class="text-sm text-[var(--muted)]">${t("onboarding:provider.loadingLlms")}</div>`;
	}

	var configuredProviders = providers.filter((p) => p.configured);

	var recommendedProviders = providers.filter((p) => RECOMMENDED_PROVIDERS.has(p.name));
	var otherProviders = providers.filter((p) => !RECOMMENDED_PROVIDERS.has(p.name));
	// If a non-recommended provider is being configured, expand the section.
	var otherIsActive = otherProviders.some(
		(p) => configuring === p.name || oauthProvider === p.name || localProvider === p.name,
	);
	var showOther = showAllProviders || otherIsActive;

	function renderProviderRow(p) {
		return html`<${OnboardingProviderRow}
			key=${p.name}
			provider=${p}
			configuring=${configuring}
			phase=${configuring === p.name ? phase : "form"}
			providerModels=${configuring === p.name ? providerModels : []}
			selectedModels=${configuring === p.name ? selectedModels : new Set()}
			probeResults=${configuring === p.name ? probeResults : new Map()}
			modelSearch=${configuring === p.name ? modelSearch : ""}
			setModelSearch=${setModelSearch}
			oauthProvider=${oauthProvider}
			oauthInfo=${oauthInfo}
			oauthCallbackInput=${oauthCallbackInput}
			setOauthCallbackInput=${setOauthCallbackInput}
			oauthSubmitting=${oauthSubmitting}
			localProvider=${localProvider}
			sysInfo=${sysInfo}
			localModels=${localModels}
			selectedBackend=${selectedBackend}
			setSelectedBackend=${setSelectedBackend}
			apiKey=${apiKey}
			setApiKey=${setApiKey}
			endpoint=${endpoint}
			setEndpoint=${setEndpoint}
			model=${model}
			setModel=${setModel}
			saving=${saving}
			savingModels=${savingModels}
			error=${configuring === p.name || oauthProvider === p.name || localProvider === p.name ? error : null}
			validationResult=${validationResults[p.name] || null}
			onStartConfigure=${onStartConfigure}
			onCancelConfigure=${closeAll}
			onSaveKey=${onSaveKey}
			onToggleModel=${onToggleModel}
			onSaveModels=${onSaveSelectedModels}
			onSubmitOAuthCallback=${submitOAuthCallback}
			onCancelOAuth=${cancelOAuth}
			onConfigureLocalModel=${configureLocalModel}
			onCancelLocal=${cancelLocal}
		/>`;
	}

	return html`<div class="flex flex-col gap-4">
		<div class="flex items-baseline justify-between gap-2">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:provider.addLlms")}</h2>
			<a href="https://docs.moltis.org/choosing-a-provider.html"
				target="_blank" rel="noopener noreferrer"
				class="text-xs text-[var(--accent)] hover:underline shrink-0">Help me choose</a>
		</div>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Configure one or more LLM providers to power your agent. You can add more later in Settings.</p>
		${
			configuredProviders.length > 0
				? html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2">
				<div class="text-xs text-[var(--muted)]">Detected LLM providers</div>
				<div class="flex flex-wrap gap-2">
					${configuredProviders.map((p) => html`<span key=${p.name} class="provider-item-badge configured">${p.displayName}</span>`)}
				</div>
			</div>`
				: null
		}
		<div class="flex flex-col gap-2">
			<div class="text-xs font-medium text-[var(--text)] uppercase tracking-wide">Recommended</div>
			${recommendedProviders.map(renderProviderRow)}
		</div>
		${
			otherProviders.length > 0
				? html`<div class="flex flex-col gap-2">
				<button type="button"
					class="text-xs text-[var(--muted)] hover:text-[var(--text)] cursor-pointer bg-transparent border-none text-left flex items-center gap-1"
					onClick=${() => setShowAllProviders((v) => !v)}>
					<span class="inline-block transition-transform ${showOther ? "rotate-90" : ""}">\u25B6</span>
					All providers (${otherProviders.length} more)
				</button>
				${showOther ? otherProviders.map(renderProviderRow) : null}
			</div>`
				: null
		}
		${error && !configuring && !oauthProvider && !localProvider ? html`<${ErrorPanel} message=${error} />` : null}
		<div class="flex flex-wrap items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>${t("common:actions.back")}</button>
			<button class="provider-btn" onClick=${onContinue} disabled=${phase === "validating" || savingModels}>${t("common:actions.continue")}</button>
			<button class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>
		</div>
	</div>`;
}

// ── Voice helpers ────────────────────────────────────────────

// ── Voice provider row for onboarding ────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config form and test state
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
	voiceTestResult,
}) {
	var isConfiguring = configuring === provider.id;
	var keyInputRef = useRef(null);

	useEffect(() => {
		if (isConfiguring && keyInputRef.current) {
			keyInputRef.current.focus();
		}
	}, [isConfiguring]);
	var supportsBaseUrl = provider.capabilities?.baseUrl === true;
	var keySourceLabel =
		provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";

	// Test button state
	var testState = voiceTesting?.id === provider.id && voiceTesting?.type === type ? voiceTesting : null;
	var showTestBtn = provider.available;
	var testBtnText = "Test";
	var testBtnDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			testBtnText = "Stop";
		} else {
			testBtnText = "Testing\u2026";
			testBtnDisabled = true;
		}
	}

	return html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3">
		<div class="flex items-center gap-3">
			<div class="flex-1 min-w-0 flex flex-col gap-0.5">
				<div class="flex items-center gap-2 flex-wrap">
					<span class="text-sm font-medium text-[var(--text-strong)]">${provider.name}</span>
						${provider.available ? html`<span class="provider-item-badge configured">configured</span>` : html`<span class="provider-item-badge needs-key">needs key</span>`}
					${keySourceLabel ? html`<span class="text-xs text-[var(--muted)]">${keySourceLabel}</span>` : null}
				</div>
				${provider.description ? html`<span class="text-xs text-[var(--muted)]">${provider.description}${!isConfiguring && provider.keyUrl ? html`${" \u2014 "}get your key at <a href=${provider.keyUrl} target="_blank" class="text-[var(--accent)] underline">${provider.keyUrlLabel || provider.keyUrl}</a>` : null}</span>` : null}
			</div>
			<div class="shrink-0 flex items-center gap-2">
				${
					isConfiguring
						? null
						: html`<button class="provider-btn provider-btn-secondary provider-btn-sm"
						onClick=${() => onStartConfigure(provider.id)}>Configure</button>`
				}
				${
					showTestBtn
						? html`<button class="provider-btn provider-btn-secondary provider-btn-sm"
						onClick=${onTest} disabled=${testBtnDisabled}
						title=${type === "tts" ? "Test voice output" : "Test voice input"}>
						${testBtnText}
					</button>`
						: null
				}
			</div>
		</div>
		${
			testState?.phase === "recording"
				? html`<div class="voice-recording-hint mt-2">
				<span class="voice-recording-dot"></span>
				<span>Speak now, then click Stop when finished</span>
			</div>`
				: null
		}
		${testState?.phase === "transcribing" ? html`<span class="text-xs text-[var(--muted)] mt-1 block">Transcribing\u2026</span>` : null}
		${testState?.phase === "testing" && type === "tts" ? html`<span class="text-xs text-[var(--muted)] mt-1 block">Playing audio\u2026</span>` : null}
		${
			voiceTestResult?.text
				? html`<div class="voice-transcription-result mt-2">
				<span class="voice-transcription-label">Transcribed:</span>
				<span class="voice-transcription-text">"${voiceTestResult.text}"</span>
			</div>`
				: null
		}
		${
			voiceTestResult?.success === true
				? html`<div class="voice-success-result mt-2">
				<span class="icon icon-md icon-check-circle"></span>
				<span>Audio played successfully</span>
			</div>`
				: null
		}
		${
			voiceTestResult?.error
				? html`<div class="voice-error-result">
			<span class="icon icon-md icon-x-circle"></span>
			<span>${voiceTestResult.error}</span>
		</div>`
				: null
		}
		${
			isConfiguring
				? html`<form onSubmit=${onSaveKey} class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
				<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">API Key</label>
					<input type="password" class="provider-key-input w-full"
						ref=${keyInputRef}
						value=${apiKey} onInput=${(e) => setApiKey(e.target.value)}
						placeholder=${provider.keyPlaceholder || "API key"} />
				</div>
				${
					supportsBaseUrl
						? html`<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">Base URL</label>
					<input
						type="text"
						class="provider-key-input w-full"
						data-field="baseUrl"
						value=${baseUrl}
						onInput=${(e) => setBaseUrl(e.target.value)}
						placeholder="http://localhost:8000/v1" />
					<div class="text-xs text-[var(--muted)] mt-1">
						Use this for a local or OpenAI-compatible server. Leave the API key blank if the endpoint does not require one.
					</div>
				</div>`
						: null
				}
				${
					provider.keyUrl
						? html`<div class="text-xs text-[var(--muted)]">
					Get your key at <a href=${provider.keyUrl} target="_blank" class="text-[var(--accent)] underline">${provider.keyUrlLabel || provider.keyUrl}</a>
				</div>`
						: null
				}
				${provider.hint ? html`<div class="text-xs text-[var(--accent)]">${provider.hint}</div>` : null}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<div class="flex items-center gap-2 mt-1">
					<button key=${`voice-${saving}`} type="submit" class="provider-btn provider-btn-sm" disabled=${saving}>${saving ? "Saving\u2026" : "Save"}</button>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onCancelConfigure}>Cancel</button>
				</div>
			</form>`
				: null
		}
	</div>`;
}

// ── Voice step ──────────────────────────────────────────────

function VoiceStep({ onNext, onBack }) {
	var [loading, setLoading] = useState(true);
	var [allProviders, setAllProviders] = useState({ tts: [], stt: [] });
	var [configuring, setConfiguring] = useState(null); // provider id with open key form
	var [apiKey, setApiKey] = useState("");
	var [baseUrl, setBaseUrl] = useState("");
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);
	var [voiceTesting, setVoiceTesting] = useState(null); // { id, type, phase }
	var [voiceTestResults, setVoiceTestResults] = useState({});
	var [activeRecorder, setActiveRecorder] = useState(null);
	var [enableSaving, setEnableSaving] = useState(false);

	function fetchProviders() {
		return fetchVoiceProviders().then((res) => {
			if (res?.ok) {
				setAllProviders(res.payload || { tts: [], stt: [] });
			}
			return res;
		});
	}

	useEffect(() => {
		var cancelled = false;
		var attempts = 0;

		function load() {
			if (cancelled) return;
			fetchVoiceProviders().then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setAllProviders(res.payload || { tts: [], stt: [] });
					setLoading(false);
					return;
				}
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				// Voice not compiled → skip
				onNext();
			});
		}

		load();
		return () => {
			cancelled = true;
		};
	}, []);

	// Cloud providers only (filter out local for onboarding)
	var cloudStt = allProviders.stt.filter((p) => p.category === "cloud");
	var cloudTts = allProviders.tts.filter((p) => p.category === "cloud");

	// Auto-detected: available via LLM provider key, not yet enabled.
	// Only show providers whose key came from an LLM provider (not directly configured).
	var autoDetected = [...allProviders.stt, ...allProviders.tts].filter(
		(p) => p.available && p.keySource === "llm_provider" && !p.enabled && p.category === "cloud",
	);
	var hasAutoDetected = autoDetected.length > 0;

	function enableAutoDetected() {
		setEnableSaving(true);
		setError(null);
		var firstStt = allProviders.stt.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
		var firstTts = allProviders.tts.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
		var toggles = [];
		if (firstStt) toggles.push(toggleVoiceProvider(firstStt.id, true, "stt"));
		if (firstTts) toggles.push(toggleVoiceProvider(firstTts.id, true, "tts"));
		if (toggles.length === 0) {
			setEnableSaving(false);
			return;
		}
		Promise.all(toggles).then((results) => {
			setEnableSaving(false);
			var failed = results.find((r) => !r?.ok);
			if (failed) {
				setError(failed?.error?.message || "Failed to enable voice provider");
				return;
			}
			fetchProviders();
		});
	}

	function onStartConfigure(providerId) {
		var provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === providerId);
		setConfiguring(providerId);
		setApiKey("");
		setBaseUrl(provider?.settings?.baseUrl || "");
		setError(null);
	}

	function onCancelConfigure() {
		setConfiguring(null);
		setApiKey("");
		setBaseUrl("");
		setError(null);
	}

	function onSaveKey(e) {
		e.preventDefault();
		var provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === configuring);
		var trimmedApiKey = apiKey.trim();
		var trimmedBaseUrl = baseUrl.trim();
		var hadBaseUrl = typeof provider?.settings?.baseUrl === "string" && provider.settings.baseUrl.trim().length > 0;
		var shouldSaveBaseUrl = provider?.capabilities?.baseUrl === true && (trimmedBaseUrl.length > 0 || hadBaseUrl);
		if (!(trimmedApiKey || shouldSaveBaseUrl)) {
			setError("API key or base URL is required.");
			return;
		}
		setError(null);
		setSaving(true);
		var providerId = configuring;
		var req = trimmedApiKey
			? saveVoiceKey(providerId, trimmedApiKey, { baseUrl: shouldSaveBaseUrl ? trimmedBaseUrl : undefined })
			: saveVoiceSettings(providerId, shouldSaveBaseUrl ? { baseUrl: trimmedBaseUrl } : undefined);
		req.then(async (res) => {
			if (res?.ok) {
				// Auto-enable in onboarding: toggle on for each type this provider appears in.
				// IDs differ between TTS and STT (e.g. "elevenlabs" vs "elevenlabs-stt"),
				// so also check the counterpart ID.
				var counterId = VOICE_COUNTERPART_IDS[providerId];
				var toggles = [];
				var sttMatch =
					allProviders.stt.find((p) => p.id === providerId) ||
					(counterId && allProviders.stt.find((p) => p.id === counterId));
				var ttsMatch =
					allProviders.tts.find((p) => p.id === providerId) ||
					(counterId && allProviders.tts.find((p) => p.id === counterId));
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
				setError(res?.error?.message || "Failed to save");
			}
		});
	}

	// Stop active STT recording
	function stopSttRecording() {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: test function handles TTS playback and STT mic recording flows
	async function testVoiceProvider(providerId, type) {
		// If already recording for this provider, stop it
		if (voiceTesting?.id === providerId && voiceTesting?.type === "stt" && voiceTesting?.phase === "recording") {
			stopSttRecording();
			return;
		}

		setError(null);
		setVoiceTesting({ id: providerId, type, phase: "testing" });

		// Auto-enable the provider if it's available but not yet enabled
		var prov = (type === "stt" ? allProviders.stt : allProviders.tts).find((p) => p.id === providerId);
		if (prov?.available && !prov?.enabled) {
			var toggleRes = await toggleVoiceProvider(providerId, true, type);
			if (!toggleRes?.ok) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: toggleRes?.error?.message || "Failed to enable provider" },
				}));
				setVoiceTesting(null);
				return;
			}
			// ElevenLabs/Google share API keys — enable the counterpart too.
			var counterType = type === "stt" ? "tts" : "stt";
			var counterList = type === "stt" ? allProviders.tts : allProviders.stt;
			var counterId = VOICE_COUNTERPART_IDS[providerId] || providerId;
			var counterProv = counterList.find((p) => p.id === counterId);
			if (counterProv?.available && !counterProv?.enabled) {
				await toggleVoiceProvider(counterId, true, counterType);
			}
			// Refresh provider list in background
			fetchProviders();
		}

		if (type === "tts") {
			try {
				var identity = getGon("identity");
				var user = identity?.user_name || "friend";
				var bot = identity?.name || "Moltis";
				var ttsText = await fetchPhrase("onboarding", user, bot);
				var res = await testTts(ttsText, providerId);
				if (res?.ok && res.payload?.audio) {
					var bytes = decodeBase64Safe(res.payload.audio);
					var audioMime = res.payload.mimeType || res.payload.content_type || "audio/mpeg";
					console.log(
						"[TTS] audio received: %d bytes, mime=%s, format=%s",
						bytes.length,
						audioMime,
						res.payload.format,
					);
					var blob = new Blob([bytes], { type: audioMime });
					var url = URL.createObjectURL(blob);
					var audio = new Audio(url);
					audio.onerror = (e) => {
						console.error("[TTS] audio element error:", audio.error?.message || e);
						URL.revokeObjectURL(url);
					};
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e) => console.error("[TTS] play() failed:", e));
					setVoiceTestResults((prev) => ({ ...prev, [providerId]: { success: true, error: null } }));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: false, error: res?.error?.message || "TTS test failed" },
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: err.message || "TTS test failed" },
				}));
			}
			setVoiceTesting(null);
		} else {
			// STT: record then transcribe
			try {
				var stream = await navigator.mediaDevices.getUserMedia({ audio: true });
				var mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
					? "audio/webm;codecs=opus"
					: "audio/webm";
				var mediaRecorder = new MediaRecorder(stream, { mimeType });
				var audioChunks = [];

				mediaRecorder.ondataavailable = (e) => {
					if (e.data.size > 0) audioChunks.push(e.data);
				};

				mediaRecorder.start();
				setActiveRecorder(mediaRecorder);
				setVoiceTesting({ id: providerId, type, phase: "recording" });

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (var track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });

					var audioBlob = new Blob(audioChunks, { type: mediaRecorder.mimeType || mimeType });

					try {
						var resp = await transcribeAudio(S.activeSessionKey, providerId, audioBlob);
						console.log("[STT] upload response: status=%d ok=%s", resp.status, resp.ok);
						if (resp.ok) {
							var sttRes = await resp.json();

							if (sttRes.ok && typeof sttRes.transcription?.text === "string") {
								var transcriptText = sttRes.transcription.text.trim();
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: transcriptText || null,
										error: transcriptText ? null : "No speech detected",
									},
								}));
							} else {
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: null,
										error: sttRes.transcriptionError || sttRes.error || "STT test failed",
									},
								}));
							}
						} else {
							var errBody = await resp.text();
							console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
							var errMsg = "STT test failed";
							try {
								errMsg = JSON.parse(errBody)?.error || errMsg;
							} catch (_e) {
								// not JSON
							}
							setVoiceTestResults((prev) => ({
								...prev,
								[providerId]: { text: null, error: `${errMsg} (HTTP ${resp.status})` },
							}));
						}
					} catch (fetchErr) {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: { text: null, error: fetchErr.message || "STT test failed" },
						}));
					}
					setVoiceTesting(null);
				};
			} catch (err) {
				if (err.name === "NotAllowedError") {
					setError("Microphone permission denied");
				} else if (err.name === "NotFoundError") {
					setError("No microphone found");
				} else {
					setError(err.message || "STT test failed");
				}
				setVoiceTesting(null);
			}
		}
	}

	// ── Render ────────────────────────────────────────────────

	if (loading) {
		return html`<div class="text-sm text-[var(--muted)]">Checking voice providers\u2026</div>`;
	}

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Voice (optional)</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			Enable voice input (speech-to-text) and output (text-to-speech) for your agent.
			You can configure this later in Settings.
		</p>

		${
			hasAutoDetected
				? html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2">
				<div class="text-xs text-[var(--muted)]">Auto-detected from your LLM provider</div>
				<div class="flex flex-wrap gap-2">
					${autoDetected.map((p) => html`<span key=${p.id} class="provider-item-badge configured">${p.name}</span>`)}
				</div>
				<button class="provider-btn self-start" disabled=${enableSaving} onClick=${enableAutoDetected}>
					${enableSaving ? "Enabling\u2026" : "Enable voice"}
				</button>
			</div>`
				: null
		}

		${
			cloudStt.length > 0
				? html`<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)] mb-2">Speech-to-Text</h3>
				<div class="flex flex-col gap-2">
					${cloudStt.map(
						(prov) => html`<${OnboardingVoiceRow}
						key=${prov.id}
						provider=${prov}
						type="stt"
						configuring=${configuring}
						apiKey=${apiKey}
						setApiKey=${setApiKey}
						baseUrl=${baseUrl}
						setBaseUrl=${setBaseUrl}
						saving=${saving}
						error=${configuring === prov.id ? error : null}
						onSaveKey=${onSaveKey}
						onStartConfigure=${onStartConfigure}
						onCancelConfigure=${onCancelConfigure}
						onTest=${() => testVoiceProvider(prov.id, "stt")}
						voiceTesting=${voiceTesting}
						voiceTestResult=${voiceTestResults[prov.id] || null}
					/>`,
					)}
				</div>
			</div>`
				: null
		}

		${
			cloudTts.length > 0
				? html`<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)] mb-2">Text-to-Speech</h3>
				<div class="flex flex-col gap-2">
					${cloudTts.map(
						(prov) => html`<${OnboardingVoiceRow}
						key=${prov.id}
						provider=${prov}
						type="tts"
						configuring=${configuring}
						apiKey=${apiKey}
						setApiKey=${setApiKey}
						baseUrl=${baseUrl}
						setBaseUrl=${setBaseUrl}
						saving=${saving}
						error=${configuring === prov.id ? error : null}
						onSaveKey=${onSaveKey}
						onStartConfigure=${onStartConfigure}
						onCancelConfigure=${onCancelConfigure}
						onTest=${() => testVoiceProvider(prov.id, "tts")}
						voiceTesting=${voiceTesting}
						voiceTestResult=${voiceTestResults[prov.id] || null}
					/>`,
					)}
				</div>
			</div>`
				: null
		}

		${error && !configuring ? html`<${ErrorPanel} message=${error} />` : null}
		<div class="flex flex-wrap items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>${t("common:actions.back")}</button>
			<button class="provider-btn" onClick=${onNext}>${t("common:actions.continue")}</button>
			<button class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>
		</div>
	</div>`;
}

// ── Channel step ────────────────────────────────────────────

function ChannelTypeSelector({ onSelect, offered }) {
	var channelOptions = [
		["telegram", "icon-telegram", "Telegram"],
		["whatsapp", "icon-whatsapp", "WhatsApp"],
		["msteams", "icon-msteams", "Microsoft Teams"],
		["discord", "icon-discord", "Discord"],
		["slack", "icon-slack", "Slack"],
		["matrix", "icon-matrix", "Matrix"],
		["nostr", "icon-nostr", "Nostr"],
	].filter(([type]) => offered.has(type));

	return html`<div class="grid grid-cols-2 gap-3 md:grid-cols-3" data-testid="channel-type-selector">
		${channelOptions.map(
			([type, iconClass, label]) => html`<button
					key=${type}
					type="button"
					class="backend-card w-full min-h-[120px] items-center justify-center gap-4 px-4 py-8 text-center"
					onClick=${() => onSelect(type)}>
					<span class=${`icon icon-xl ${iconClass}`}></span>
					<span class="text-sm font-medium text-[var(--text-strong)]">${label}</span>
				</button>`,
		)}
	</div>`;
}

function fetchRemoteAccessStatus(path, featureDisabledMessage) {
	return fetch(path)
		.then((response) => {
			var contentType = response.headers.get("content-type") || "";
			if (response.status === 404 || !contentType.includes("application/json")) {
				return {
					error: featureDisabledMessage,
					feature_disabled: true,
				};
			}
			return response.json();
		})
		.catch((error) => ({
			error: error.message,
		}));
}

function preferredPublicBaseUrl({ ngrokStatus, tailscaleStatus }) {
	var ngrokUrl = typeof ngrokStatus?.public_url === "string" ? ngrokStatus.public_url.trim() : "";
	if (ngrokUrl) return ngrokUrl;

	var tailscaleUrl = typeof tailscaleStatus?.url === "string" ? tailscaleStatus.url.trim() : "";
	if (tailscaleStatus?.mode === "funnel" && tailscaleUrl) return tailscaleUrl;

	return "";
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: onboarding remote access manages two public endpoint integrations
function RemoteAccessStep({ onNext, onBack }) {
	var [authReady, setAuthReady] = useState(false);
	var [tsStatus, setTsStatus] = useState(null);
	var [tsError, setTsError] = useState(null);
	var [tsWarning, setTsWarning] = useState(null);
	var [tsLoading, setTsLoading] = useState(true);
	var [configuringTailscale, setConfiguringTailscale] = useState(false);
	var [ngStatus, setNgStatus] = useState(null);
	var [ngError, setNgError] = useState(null);
	var [ngLoading, setNgLoading] = useState(true);
	var [ngSaving, setNgSaving] = useState(false);
	var [ngMsg, setNgMsg] = useState(null);
	var [ngForm, setNgForm] = useState({
		enabled: false,
		authtoken: "",
		domain: "",
	});

	function loadAuthStatus() {
		return fetch("/api/auth/status")
			.then((response) => (response.ok ? response.json() : null))
			.then((data) => {
				var ready = data?.auth_disabled ? false : data?.has_password === true;
				setAuthReady(ready);
			})
			.catch(() => {
				setAuthReady(false);
			});
	}

	function loadTailscaleStatus() {
		setTsLoading(true);
		return fetchRemoteAccessStatus("/api/tailscale/status", "Tailscale feature is not enabled in this build.")
			.then((data) => {
				setTsStatus(data?.feature_disabled ? null : data);
				setTsError(data?.error || null);
				setTsWarning(data?.passkey_warning || null);
				setTsLoading(false);
			})
			.catch((error) => {
				setTsError(error.message);
				setTsLoading(false);
			});
	}

	function loadNgrokStatus() {
		setNgLoading(true);
		return fetchRemoteAccessStatus("/api/ngrok/status", "ngrok feature is not enabled in this build.")
			.then((data) => {
				setNgStatus(data?.feature_disabled ? null : data);
				setNgError(data?.error || null);
				setNgLoading(false);
				setNgForm((current) => ({
					enabled: Boolean(data?.enabled),
					authtoken: current.authtoken,
					domain: current.domain || data?.domain || "",
				}));
			})
			.catch((error) => {
				setNgError(error.message);
				setNgLoading(false);
			});
	}

	useEffect(() => {
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
			body: JSON.stringify({ mode }),
		})
			.then((response) =>
				response
					.json()
					.catch(() => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				if (!ok || data.error) {
					setTsError(data.error || "Failed to configure Tailscale.");
				} else {
					setTsWarning(data.passkey_warning || null);
					loadTailscaleStatus();
				}
				setConfiguringTailscale(false);
			})
			.catch((error) => {
				setTsError(error.message);
				setConfiguringTailscale(false);
			});
	}

	function toggleTailscaleFunnel() {
		var nextMode = tsStatus?.mode === "funnel" ? "off" : "funnel";
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
				domain: nextForm.domain,
			}),
		})
			.then((response) =>
				response
					.json()
					.catch(() => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				setNgSaving(false);
				if (!ok || data.error) {
					setNgError(data.error || "Failed to apply ngrok settings.");
					return;
				}

				var status = data.status || null;
				setNgMsg(successMessage);
				setNgStatus(status);
				setNgForm({
					enabled: Boolean(status?.enabled),
					authtoken: "",
					domain: status?.domain || nextForm.domain || "",
				});
			})
			.catch((error) => {
				setNgSaving(false);
				setNgError(error.message);
			});
	}

	function toggleNgrokEnabled() {
		var nextForm = {
			...ngForm,
			enabled: !ngForm.enabled,
		};
		setNgForm(nextForm);
		applyNgrokConfig(nextForm, `ngrok ${nextForm.enabled ? "enabled" : "disabled"}.`);
	}

	var tailscaleAvailable = tsStatus !== null;
	var tailscaleFunnelEnabled = tsStatus?.mode === "funnel";
	var tailscaleInstalled = tsStatus?.installed !== false;
	var tailscaleBlocked = !(tailscaleAvailable && tailscaleInstalled) || tsStatus?.tailscale_up === false;
	var ngrokAvailable = ngStatus !== null;
	var activePublicUrl = preferredPublicBaseUrl({
		ngrokStatus: ngStatus,
		tailscaleStatus: tsStatus,
	});

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Remote Access</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			Public endpoints are optional for most channels, but Microsoft Teams needs one. Enable
			Tailscale Funnel, ngrok, or both before connecting team channels.
		</p>
		${
			activePublicUrl
				? html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
					<span class="font-medium text-[var(--text-strong)]">Active public URL</span>
					<a href=${activePublicUrl} target="_blank" rel="noopener" class="text-[var(--accent)] underline break-all">
						${activePublicUrl}
					</a>
					<span>The Teams webhook step will prefill this URL.</span>
				</div>`
				: html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]">
					Teams webhooks need a public URL. If you skip this step, you can still configure remote access later in
					Settings.
				</div>`
		}

		<section class="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
			<div class="flex flex-col gap-1">
				<h3 class="text-base font-medium text-[var(--text-strong)]">Tailscale Funnel</h3>
				<p class="text-xs text-[var(--muted)] leading-relaxed">
					Public HTTPS through Tailscale. Tailscale Serve is tailnet-only, so Teams webhooks need Funnel instead.
				</p>
			</div>
			${
				tsLoading
					? html`<div class="text-xs text-[var(--muted)]">Loading Tailscale status…</div>`
					: html`<div class="text-sm text-[var(--text-strong)]">
						Tailscale Funnel is ${tailscaleFunnelEnabled ? "enabled" : "disabled"}.
					</div>`
			}
			${tsStatus?.url && tailscaleFunnelEnabled ? html`<a href=${tsStatus.url} target="_blank" rel="noopener" class="text-sm text-[var(--accent)] underline break-all">${tsStatus.url}</a>` : null}
			${tsError ? html`<${ErrorPanel} message=${tsError} />` : null}
			${tsWarning ? html`<div class="alert-warning-text max-w-form">${tsWarning}</div>` : null}
			${
				tsStatus?.installed === false
					? html`<a href="https://tailscale.com/download" target="_blank" rel="noopener" class="provider-btn self-start no-underline">
						Install Tailscale
					</a>`
					: null
			}
			${
				tsStatus?.tailscale_up === false
					? html`<div class="alert-warning-text max-w-form">
						<span class="alert-label-warn">Warning:</span> Start Tailscale before enabling Funnel.
					</div>`
					: null
			}
			${
				authReady
					? null
					: html`<div class="alert-warning-text max-w-form">
						<span class="alert-label-warn">Warning:</span> Funnel can be enabled now, but remote visitors will
						see the setup-required page until authentication is configured.
					</div>`
			}
			<button
				type="button"
				class="provider-btn self-start"
				disabled=${tsLoading || configuringTailscale || tailscaleBlocked}
				onClick=${toggleTailscaleFunnel}
			>
				${configuringTailscale ? "Applying…" : tailscaleFunnelEnabled ? "Disable Funnel" : "Enable Funnel"}
			</button>
		</section>

		<section class="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
			<div class="flex flex-col gap-1">
				<h3 class="text-base font-medium text-[var(--text-strong)]">ngrok</h3>
				<p class="text-xs text-[var(--muted)] leading-relaxed">
					Public HTTPS without installing an external binary. This is useful for demos, shared testing, and Teams.
				</p>
			</div>
			${
				ngLoading
					? html`<div class="text-xs text-[var(--muted)]">Loading ngrok status…</div>`
					: html`<div class="text-sm text-[var(--text-strong)]">
						ngrok is ${ngForm.enabled ? "enabled" : "disabled"}.
					</div>`
			}
			${ngStatus?.public_url ? html`<a href=${ngStatus.public_url} target="_blank" rel="noopener" class="text-sm text-[var(--accent)] underline break-all">${ngStatus.public_url}</a>` : null}
			${ngError ? html`<${ErrorPanel} message=${ngError} />` : null}
			${ngStatus?.passkey_warning ? html`<div class="alert-warning-text max-w-form">${ngStatus.passkey_warning}</div>` : null}
			<div class="flex flex-col gap-1">
				<label class="text-xs text-[var(--muted)]" for="onboarding-ngrok-authtoken">Authtoken</label>
				<input
					id="onboarding-ngrok-authtoken"
					type="password"
					class="provider-key-input w-full"
					placeholder=${ngStatus?.authtoken_source ? "Leave blank to keep the current token" : "Paste your ngrok authtoken"}
					value=${ngForm.authtoken}
					onInput=${(e) => setNgForm({ ...ngForm, authtoken: e.currentTarget.value })}
				/>
				<div class="text-xs text-[var(--muted)]">
					Create or copy an authtoken from <a
						href="https://dashboard.ngrok.com/get-started/your-authtoken"
						target="_blank"
						rel="noopener"
						class="text-[var(--accent)] underline"
					>ngrok dashboard</a>.
				</div>
			</div>
			<div class="flex flex-col gap-1">
				<label class="text-xs text-[var(--muted)]" for="onboarding-ngrok-domain">Reserved domain (optional)</label>
				<input
					id="onboarding-ngrok-domain"
					type="text"
					class="provider-key-input w-full"
					placeholder="team-gateway.ngrok.app"
					value=${ngForm.domain}
					onInput=${(e) => setNgForm({ ...ngForm, domain: e.currentTarget.value })}
				/>
				<div class="text-xs text-[var(--muted)]">Use a reserved domain if you want a stable public hostname.</div>
			</div>
			${ngMsg ? html`<div class="text-xs text-[var(--ok)]">${ngMsg}</div>` : null}
			<button
				type="button"
				class="provider-btn self-start"
				disabled=${!ngrokAvailable || ngLoading || ngSaving}
				onClick=${toggleNgrokEnabled}
			>
				${ngSaving ? "Applying…" : ngForm.enabled ? "Disable ngrok" : "Enable ngrok"}
			</button>
		</section>

		<div class="flex flex-wrap items-center gap-3 mt-1">
			<button type="button" class="provider-btn provider-btn-secondary" onClick=${onBack}>${t("common:actions.back")}</button>
			<button type="button" class="provider-btn" onClick=${onNext}>${t("common:actions.continue")}</button>
			<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>
				Skip for now
			</button>
		</div>
	</div>`;
}

function TelegramForm({ onConnected, error, setError }) {
	var [accountId, setAccountId] = useState("");
	var [token, setToken] = useState("");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

	function onSubmit(e) {
		e.preventDefault();
		var v = validateChannelFields("telegram", accountId, token);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		var config = {
			token: token.trim(),
			dm_policy: dmPolicy,
			mention_mode: "mention",
			allowlist: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		addChannel("telegram", accountId.trim(), config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "telegram");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
			<span>1. Open <a href="https://t.me/BotFather" target="_blank" class="text-[var(--accent)] underline">@BotFather</a> in Telegram</span>
			<span>2. Send /newbot and follow the prompts</span>
			<span>3. Copy the bot token and paste it below</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Bot username</label>
			<input type="text" class="provider-key-input w-full"
				value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
				placeholder="e.g. my_assistant_bot"
				autocomplete="off"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="telegram_bot_username"
				autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Bot token (from @BotFather)</label>
			<input type="password" class="provider-key-input w-full"
				value=${token} onInput=${(e) => setToken(e.target.value)}
				placeholder="123456:ABC-DEF..."
				autocomplete="new-password"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="telegram_bot_token" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="allowlist">Allowlist only (recommended)</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Your Telegram username(s)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${allowlist} onInput=${(e) => setAllowlist(e.target.value)}
				placeholder="your_username" style="resize:vertical;font-family:var(--font-body);" />
			<div class="text-xs text-[var(--muted)] mt-1">One username per line, without the @ sign. These users can DM your bot.</div>
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`tg-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Bot"}</button>
	</form>`;
}

function TeamsForm({ onConnected, error, setError }) {
	var [appId, setAppId] = useState("");
	var [appPassword, setAppPassword] = useState("");
	var [webhookSecret, setWebhookSecret] = useState("");
	var [baseUrl, setBaseUrl] = useState(defaultTeamsBaseUrl());
	var [bootstrapEndpoint, setBootstrapEndpoint] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

	// Auto-detect public URL from ngrok or Tailscale Funnel.
	useEffect(() => {
		var cancelled = false;
		var currentDefault = defaultTeamsBaseUrl();
		if (baseUrl !== currentDefault) return undefined;

		Promise.all([
			fetchRemoteAccessStatus("/api/ngrok/status", "ngrok feature is not enabled in this build."),
			fetchRemoteAccessStatus("/api/tailscale/status", "Tailscale feature is not enabled in this build."),
		]).then(([nextNgrokStatus, nextTailscaleStatus]) => {
			if (cancelled) return;
			var nextPublicBaseUrl = preferredPublicBaseUrl({
				ngrokStatus: nextNgrokStatus,
				tailscaleStatus: nextTailscaleStatus,
			});
			if (nextPublicBaseUrl) {
				setBaseUrl(nextPublicBaseUrl);
			}
		});

		return () => {
			cancelled = true;
		};
	}, [baseUrl]);

	function onBootstrap() {
		var id = appId.trim();
		if (!id) {
			setError("Enter App ID first.");
			return;
		}
		var secret = webhookSecret.trim();
		if (!secret) {
			secret = generateWebhookSecretHex();
			setWebhookSecret(secret);
		}
		var endpoint = buildTeamsEndpoint(baseUrl, id, secret);
		if (!endpoint) {
			setError("Enter a valid public base URL (e.g. https://bot.example.com).");
			return;
		}
		setBootstrapEndpoint(endpoint);
		setError(null);
	}

	function onCopyEndpoint() {
		if (!bootstrapEndpoint) return;
		if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
			navigator.clipboard.writeText(bootstrapEndpoint);
		}
	}

	function onSubmit(e) {
		e.preventDefault();
		var v = validateChannelFields("msteams", appId, appPassword);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		var config = {
			app_id: appId.trim(),
			app_password: appPassword.trim(),
			dm_policy: "allowlist",
			mention_mode: "mention",
			allowlist: [],
		};
		if (webhookSecret.trim()) config.webhook_secret = webhookSecret.trim();
		Object.assign(config, advancedPatch.value);
		addChannel("msteams", appId.trim(), config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(appId.trim(), "msteams");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.");
			}
		});
	}

	var isLocalUrl =
		!baseUrl ||
		/^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1?\])/i.test(baseUrl) ||
		baseUrl === defaultTeamsBaseUrl();

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		${
			isLocalUrl &&
			html`<div class="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">Public URL required</span>
			<span class="text-[var(--muted)]">Teams sends messages via webhook \u2014 your server must be reachable over HTTPS. Set up a tunnel in the previous <strong>Remote Access</strong> step, or enter a public URL below.</span>
		</div>`
		}
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-2">
			<span class="font-medium text-[var(--text-strong)]">How to create a Teams bot</span>
			<span class="font-medium text-[var(--text-strong)] text-[10px] opacity-70">Option A: Teams Developer Portal (easiest)</span>
			<span>1. Open <a href="https://dev.teams.microsoft.com/bots" target="_blank" class="text-[var(--accent)] underline">Teams Developer Portal \u2192 Bot Management</a></span>
			<span>2. Click <strong>+ New Bot</strong>, give it a name, and click <strong>Add</strong></span>
			<span>3. Go to <strong>Configure</strong> \u2014 copy the <strong>Bot ID</strong> (this is your App ID)</span>
			<span>4. Under <strong>Client secrets</strong>, click <strong>Add a client secret</strong> and copy the value</span>
			<span class="font-medium text-[var(--text-strong)] text-[10px] opacity-70 mt-1">Option B: Azure Portal</span>
			<span>1. Go to <a href="https://portal.azure.com/#create/Microsoft.AzureBot" target="_blank" class="text-[var(--accent)] underline">Azure Portal \u2192 Create Azure Bot</a></span>
			<span>2. Create the bot, then go to <strong>Configuration</strong> to find the App ID</span>
			<span>3. Click <strong>Manage Password</strong> \u2192 <strong>New client secret</strong> to get the App Password</span>
			<span class="mt-1">After creating the bot, generate the endpoint below and paste it as the <strong>Messaging endpoint</strong> in your bot settings.</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">App ID (Bot ID from Azure)</label>
			<input type="text" class="provider-key-input w-full"
				value=${appId} onInput=${(e) => setAppId(e.target.value)}
				placeholder="e.g. 12345678-abcd-efgh-ijkl-000000000000"
				autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="teams_app_id" autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">App Password (client secret from Azure)</label>
			<input type="password" class="provider-key-input w-full"
				value=${appPassword} onInput=${(e) => setAppPassword(e.target.value)}
				placeholder="Client secret value"
				autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="teams_app_password" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Webhook Secret <span class="opacity-60">(optional \u2014 auto-generated if blank)</span></label>
			<input type="text" class="provider-key-input w-full"
				value=${webhookSecret} onInput=${(e) => setWebhookSecret(e.target.value)}
				placeholder="Leave blank to auto-generate" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Public Base URL <span class="opacity-60">(your server\u2019s HTTPS address)</span></label>
			<input type="text" class="provider-key-input w-full"
				value=${baseUrl} onInput=${(e) => setBaseUrl(e.target.value)}
				placeholder="https://bot.example.com" />
			${
				isLocalUrl &&
				html`<div class="text-[10px] text-amber-600 mt-1">This looks like a local address. Teams webhooks need a publicly reachable HTTPS URL.</div>`
			}
		</div>
		<div class="flex gap-2">
			<button type="button" class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${onBootstrap}>Generate Endpoint</button>
			${bootstrapEndpoint && html`<button type="button" class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${onCopyEndpoint}>Copy</button>`}
		</div>
		${
			bootstrapEndpoint &&
			html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2">
			<div class="text-xs text-[var(--muted)] mb-1">Messaging endpoint \u2014 paste this into your bot\u2019s configuration:</div>
			<code class="text-xs block break-all select-all">${bootstrapEndpoint}</code>
		</div>`
		}
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`teams-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Teams"}</button>
	</form>`;
}

function discordInviteUrl(token) {
	if (!token) return "";
	var parts = token.split(".");
	if (parts.length < 3) return "";
	try {
		var id = atob(parts[0]);
		if (!/^\d+$/.test(id)) return "";
		return `https://discord.com/oauth2/authorize?client_id=${id}&scope=bot&permissions=100352`;
	} catch {
		return "";
	}
}

function DiscordForm({ onConnected, error, setError }) {
	var [accountId, setAccountId] = useState("");
	var [token, setToken] = useState("");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

	function onSubmit(e) {
		e.preventDefault();
		var v = validateChannelFields("discord", accountId, token);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		var config = {
			token: token.trim(),
			dm_policy: dmPolicy,
			mention_mode: "mention",
			allowlist: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		addChannel("discord", accountId.trim(), config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "discord");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	var inviteUrl = discordInviteUrl(token);

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">How to set up a Discord bot</span>
			<span>1. Go to the <a href="https://discord.com/developers/applications" target="_blank" class="text-[var(--accent)] underline">Discord Developer Portal</a></span>
			<span>2. Create a new Application \u2192 Bot tab \u2192 copy the bot token</span>
			<span>3. Enable <strong>Message Content Intent</strong> under Privileged Gateway Intents</span>
			<span>4. Paste the token below \u2014 an invite link will be generated automatically</span>
			<span>5. You can also DM the bot directly without adding it to a server</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Account ID</label>
			<input type="text" class="provider-key-input w-full"
				value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
				placeholder="e.g. my_discord_bot"
				autocomplete="off"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="discord_account_id"
				autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Bot token</label>
			<input type="password" class="provider-key-input w-full"
				value=${token} onInput=${(e) => setToken(e.target.value)}
				placeholder="Bot token from Developer Portal"
				autocomplete="new-password"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="discord_bot_token" />
		</div>
		${
			inviteUrl &&
			html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 text-xs flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">Invite bot to a server</span>
			<span class="text-[var(--muted)]">Open this link to add the bot (Send Messages, Attach Files, Read Message History):</span>
			<a href=${inviteUrl} target="_blank" class="text-[var(--accent)] underline break-all">${inviteUrl}</a>
		</div>`
		}
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="allowlist">Allowlist only (recommended)</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Allowed Discord username(s)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${allowlist} onInput=${(e) => setAllowlist(e.target.value)}
				placeholder="your_username" style="resize:vertical;font-family:var(--font-body);" />
			<div class="text-xs text-[var(--muted)] mt-1">One username per line. These users can DM your bot.</div>
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`disc-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Bot"}</button>
	</form>`;
}

function matrixSubmitLabel(saving, oidcWaiting, authMode) {
	if (saving) return "Connecting\u2026";
	if (oidcWaiting) return "Waiting for OIDC\u2026";
	if (authMode === "oidc") return "Authenticate with OIDC";
	return "Connect Matrix";
}

function MatrixForm({ onConnected, error, setError }) {
	var [homeserver, setHomeserver] = useState(MATRIX_DEFAULT_HOMESERVER);
	var [authMode, setAuthMode] = useState("oidc");
	var [oidcWaiting, setOidcWaiting] = useState(false);
	var oidcPollRef = useRef(null);
	var [userId, setUserId] = useState("");
	var [credential, setCredential] = useState("");
	var [deviceDisplayName, setDeviceDisplayName] = useState("");
	var [ownershipMode, setOwnershipMode] = useState("moltis_owned");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [roomPolicy, setRoomPolicy] = useState("allowlist");
	var [mentionMode, setMentionMode] = useState("mention");
	var [autoJoin, setAutoJoin] = useState("always");
	var [otpSelfApproval, setOtpSelfApproval] = useState(true);
	var [otpCooldown, setOtpCooldown] = useState("300");
	var [userAllowlist, setUserAllowlist] = useState("");
	var [roomAllowlist, setRoomAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

	function splitLines(value) {
		return value
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
	}

	function onSubmit(e) {
		e.preventDefault();
		var accountId = deriveMatrixAccountId({ userId, homeserver });
		var v = validateChannelFields("matrix", accountId, credential, {
			matrixAuthMode: authMode,
			matrixUserId: userId,
		});
		if (!v.valid) {
			setError(v.error);
			return;
		}
		if (!homeserver.trim()) {
			setError("Homeserver URL is required.");
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);

		// OIDC flow: start browser-based auth.
		if (normalizeMatrixAuthMode(authMode) === "oidc") {
			var redirectUri = window.location.origin + "/auth/callback";
			var oidcConfig = {
				homeserver: homeserver.trim(),
				ownership_mode: normalizeMatrixOwnershipMode(ownershipMode),
				dm_policy: dmPolicy,
				room_policy: roomPolicy,
				mention_mode: mentionMode,
				auto_join: autoJoin,
				otp_self_approval: otpSelfApproval,
				otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldown),
				user_allowlist: splitLines(userAllowlist),
				room_allowlist: splitLines(roomAllowlist),
			};
			if (deviceDisplayName.trim()) oidcConfig.device_display_name = deviceDisplayName.trim();
			Object.assign(oidcConfig, advancedPatch.value);
			sendRpc("channels.oauth_start", {
				account_id: accountId,
				homeserver: homeserver.trim(),
				redirect_uri: redirectUri,
				config: oidcConfig,
			}).then((res) => {
				if (res?.ok && res.payload?.auth_url) {
					setOidcWaiting(true);
					setSaving(false);
					window.open(res.payload.auth_url, "_blank", "noopener");
					var pollCount = 0;
					oidcPollRef.current = setInterval(() => {
						pollCount++;
						if (pollCount > 120) {
							clearInterval(oidcPollRef.current);
							oidcPollRef.current = null;
							setOidcWaiting(false);
							setError("OIDC authentication timed out. Please try again.");
							return;
						}
						fetchChannelStatus().then((statusRes) => {
							if (!statusRes?.ok) return;
							var channels = statusRes.payload?.channels || [];
							if (channels.some((ch) => ch.account_id === accountId && ch.status === "connected")) {
								clearInterval(oidcPollRef.current);
								oidcPollRef.current = null;
								setOidcWaiting(false);
								onConnected(accountId.trim(), "matrix");
							}
						});
					}, 1000);
				} else {
					setSaving(false);
					setError((res?.error && (res.error.message || res.error.detail)) || "Failed to start OIDC login.");
				}
			});
			return;
		}

		var config = {
			homeserver: homeserver.trim(),
			ownership_mode:
				normalizeMatrixAuthMode(authMode) === "password" || normalizeMatrixAuthMode(authMode) === "oidc" ? normalizeMatrixOwnershipMode(ownershipMode) : "user_managed",
			dm_policy: dmPolicy,
			room_policy: roomPolicy,
			mention_mode: mentionMode,
			auto_join: autoJoin,
			otp_self_approval: otpSelfApproval,
			otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldown),
			user_allowlist: splitLines(userAllowlist),
			room_allowlist: splitLines(roomAllowlist),
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
			if (res?.ok) {
				onConnected(accountId.trim(), "matrix");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect Matrix.");
			}
		});
	}

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">Connect a Matrix bot user</span>
			<span>1. Leave the homeserver as <span class="font-mono">${MATRIX_DEFAULT_HOMESERVER}</span> for matrix.org accounts</span>
			<span>2. OIDC is the default for modern homeservers. Use Password for encrypted chats on older servers, or Access Token for plain traffic</span>
			<span>3. Moltis generates the local account ID automatically from the Matrix user or homeserver</span>
		</div>
		<div class="rounded-md border border-emerald-600/30 bg-emerald-50 p-3 text-xs text-emerald-900 flex flex-col gap-1">
			<span class="font-medium text-emerald-800">Encrypted chats require OIDC or Password auth</span>
			<span>${MATRIX_ENCRYPTION_GUIDANCE}</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Homeserver URL</label>
			<input type="text" class="provider-key-input w-full"
				value=${homeserver} onInput=${(e) => setHomeserver(e.target.value)}
				placeholder=${MATRIX_DEFAULT_HOMESERVER}
				autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="matrix_homeserver" autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Authentication</label>
			<select class="provider-key-input w-full cursor-pointer" value=${authMode} onChange=${(e) => setAuthMode(normalizeMatrixAuthMode(e.target.value))}>
				<option value="oidc">OIDC (recommended)</option>
				<option value="password">Password</option>
				<option value="access_token">Access token</option>
			</select>
			<div class="text-xs text-[var(--muted)] mt-1">${matrixAuthModeGuidance(authMode)}</div>
		</div>
		${oidcWaiting && html`
			<div class="rounded-md border border-blue-600/30 bg-blue-50 px-3 py-3 text-xs text-blue-900 flex items-center gap-2">
				<span class="animate-spin inline-block w-4 h-4 border-2 border-blue-300 border-t-transparent rounded-full"></span>
				<span>Waiting for OIDC authentication... Complete the login in the browser window that opened.</span>
			</div>
		`}
		${
			authMode === "password" || authMode === "oidc"
				? html`<label class="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3">
					<input
						type="checkbox"
						aria-label="Let Moltis own this Matrix account"
						checked=${normalizeMatrixOwnershipMode(ownershipMode) === "moltis_owned"}
						onChange=${(e) => setOwnershipMode(e.target.checked ? "moltis_owned" : "user_managed")} />
					<span class="flex flex-col gap-1">
						<span class="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
						<span class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(authMode, ownershipMode)}</span>
					</span>
				</label>`
				: html`<div class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(authMode, "user_managed")}</div>`
		}
		${authMode !== "oidc" && html`
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Matrix User ID${authMode === "password" ? " (required)" : " (optional)"}</label>
			<input type="text" class="provider-key-input w-full"
				value=${userId} onInput=${(e) => setUserId(e.target.value)}
				placeholder="@bot:example.com"
				autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="matrix_user_id" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">${matrixCredentialLabel(authMode)}</label>
			<input type="password" class="provider-key-input w-full"
				value=${credential} onInput=${(e) => setCredential(e.target.value)}
				placeholder=${matrixCredentialPlaceholder(authMode)}
				autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="matrix_credential" />
			<div class="text-xs text-[var(--muted)] mt-1">
				${
					authMode === "password"
						? html`Use the password for the dedicated Matrix bot account. This is the required mode for encrypted Matrix chats because Moltis needs to create and persist its own Matrix device keys.`
						: html`Get the access token in Element: <span class="font-mono">Settings -> Help & About -> Advanced -> Access Token</span>. Access token mode does <span class="font-medium">not</span> support encrypted Matrix chats because Moltis cannot import that existing device's private encryption keys.`
				}
				${" "}
				<a href=${MATRIX_DOCS_URL} target="_blank" rel="noreferrer" class="text-[var(--accent)] underline">Matrix setup docs</a>
			</div>
		</div>
		`}
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Device Display Name (optional)</label>
			<input type="text" class="provider-key-input w-full"
				value=${deviceDisplayName} onInput=${(e) => setDeviceDisplayName(e.target.value)}
				placeholder="Moltis Matrix Bot"
				autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="matrix_device_display_name" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="allowlist">Allowlist only (recommended)</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Room Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${roomPolicy} onChange=${(e) => setRoomPolicy(e.target.value)}>
				<option value="allowlist">Room allowlist only (recommended)</option>
				<option value="open">Open (any joined room)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Room Mention Mode</label>
			<select class="provider-key-input w-full cursor-pointer" value=${mentionMode} onChange=${(e) => setMentionMode(e.target.value)}>
				<option value="mention">Must mention bot</option>
				<option value="always">Always respond</option>
				<option value="none">Never respond in rooms</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Invite Auto-Join</label>
			<select class="provider-key-input w-full cursor-pointer" value=${autoJoin} onChange=${(e) => setAutoJoin(e.target.value)}>
				<option value="always">Always join invites</option>
				<option value="allowlist">Only when inviter or room is allowlisted</option>
				<option value="off">Do not auto-join</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Unknown DM Approval</label>
			<select class="provider-key-input w-full cursor-pointer" value=${otpSelfApproval ? "on" : "off"} onChange=${(e) => setOtpSelfApproval(e.target.value !== "off")}>
				<option value="on">PIN challenge enabled (recommended)</option>
				<option value="off">Reject unknown DMs without a PIN</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">PIN Cooldown Seconds</label>
			<input type="number" min="1" step="1" class="provider-key-input w-full"
				value=${otpCooldown} onInput=${(e) => setOtpCooldown(e.target.value)}
				name="matrix_otp_cooldown_secs" />
			<div class="text-xs text-[var(--muted)] mt-1">With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.</div>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Allowlist (Matrix user IDs)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${userAllowlist} onInput=${(e) => setUserAllowlist(e.target.value)}
				placeholder="@alice:example.com" style="resize:vertical;font-family:var(--font-body);" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Room Allowlist (room IDs or aliases)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${roomAllowlist} onInput=${(e) => setRoomAllowlist(e.target.value)}
				placeholder="!room:example.com" style="resize:vertical;font-family:var(--font-body);" />
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`matrix-${saving}-${oidcWaiting}`} type="submit" class="provider-btn" disabled=${saving || oidcWaiting}>${matrixSubmitLabel(saving, oidcWaiting, authMode)}</button>
	</form>`;
}

function WhatsAppForm({ onConnected, error, setError }) {
	var [accountId, setAccountId] = useState("");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);
	var [pairingStarted, setPairingStarted] = useState(false);
	var [qrData, setQrData] = useState(null);
	var [qrSvg, setQrSvg] = useState(null);
	var [qrSvgUrl, setQrSvgUrl] = useState(null);
	var [pairingError, setPairingError] = useState(null);
	var unsubRef = useRef(null);
	var hadQrRef = useRef(false);

	// Clean up event subscription on unmount.
	useEffect(() => {
		return () => {
			if (unsubRef.current) unsubRef.current();
		};
	}, []);

	// Poll channels.status as a fallback for both QR code display and
	// connection detection (WebSocket events may be missed).
	useEffect(() => {
		if (!pairingStarted) return undefined;
		var id = accountId.trim() || "main";
		var timer = setInterval(async () => {
			try {
				var res = await sendRpc("channels.status");
				if (!res?.ok) return;
				var ch = (res.payload?.channels || []).find((c) => c.type === "whatsapp" && c.account_id === id);
				if (!ch) return;
				if (ch.status === "connected") {
					onConnected(id, "whatsapp");
					return;
				}
				// QR cleared + not connected = pairing succeeded, connecting.
				if (hadQrRef.current && !ch.extra?.qr_data) {
					onConnected(id, "whatsapp");
					return;
				}
				if (ch.extra?.qr_data) {
					hadQrRef.current = true;
					setQrData(ch.extra.qr_data);
					if (ch.extra.qr_svg) setQrSvg(ch.extra.qr_svg);
				}
			} catch (_e) {
				/* ignore */
			}
		}, 2000);
		return () => clearInterval(timer);
	}, [pairingStarted]);

	useEffect(() => {
		if (!qrSvg) {
			setQrSvgUrl(null);
			return undefined;
		}
		var nextUrl = null;
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
		var id = accountId.trim() || "main";
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		setQrData(null);
		setQrSvg(null);
		setPairingError(null);

		// Subscribe to channel events BEFORE the API call so we don't
		// miss the QR code event that fires while the request is in flight.
		if (unsubRef.current) unsubRef.current();
		unsubRef.current = onEvent("channel", (p) => {
			if (p.account_id !== id) return;
			if (p.kind === "pairing_qr_code") {
				setQrData(p.qr_data);
				setQrSvg(p.qr_svg || null);
			}
			if (p.kind === "pairing_complete") onConnected(id, "whatsapp");
			if (p.kind === "pairing_failed") setPairingError(p.reason || "Pairing failed");
		});

		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		var config = {
			dm_policy: dmPolicy,
			allowlist: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		addChannel("whatsapp", id, config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setPairingStarted(true);
			} else {
				if (unsubRef.current) {
					unsubRef.current();
					unsubRef.current = null;
				}
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to start pairing.");
			}
		});
	}

	if (pairingStarted) {
		return html`<div class="flex flex-col gap-4 items-center">
			${
				pairingError
					? html`<${ErrorPanel} message=${pairingError} />`
					: qrData
						? html`<div class="rounded-lg bg-white p-3" style="width:200px;height:200px;display:flex;align-items:center;justify-content:center;">
							${
								qrSvgUrl
									? html`<img src=${qrSvgUrl} alt="WhatsApp pairing QR code" style="width:100%;height:100%;display:block;" />`
									: html`<div class="text-center text-xs text-gray-600">
								<div style="font-family:monospace;font-size:9px;word-break:break-all;max-height:180px;overflow:hidden;">${qrData.substring(0, 200)}</div>
							</div>`
							}
						</div>`
						: html`<div class="text-sm text-[var(--muted)]">Waiting for QR code...</div>`
			}
			<div class="text-xs text-[var(--muted)] text-center">
				Scan the QR code from your terminal, or open WhatsApp > Settings > Linked Devices > Link a Device.
			</div>
			<div class="text-xs text-[var(--muted)] text-center italic">
				Only new messages will be processed. Past conversations are not synced.
			</div>
		</div>`;
	}

	return html`<form onSubmit=${onStartPairing} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">Link your WhatsApp</span>
			<span>1. Click "Start Pairing" to generate a QR code</span>
			<span>2. Open WhatsApp > Settings > Linked Devices > Link a Device</span>
			<span>3. Scan the QR code to connect</span>
			<span class="mt-1 italic">Only new messages will be processed — past conversations are not synced.</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Account ID (optional)</label>
			<input type="text" class="provider-key-input w-full"
				value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
				placeholder="main"
				autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
				name="whatsapp_account_id" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="open">Open (anyone)</option>
				<option value="allowlist">Allowlist only</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Allowlist (optional)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${allowlist} onInput=${(e) => setAllowlist(e.target.value)}
				placeholder="phone number or identifier" style="resize:vertical;font-family:var(--font-body);" />
			<div class="text-xs text-[var(--muted)] mt-1">One per line. Only needed if DM policy is "Allowlist only".</div>
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`wa-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Starting\u2026" : "Start Pairing"}</button>
	</form>`;
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

function ChannelSuccess({ channelName, channelType: type, onAnother }) {
	var label = channelDisplayLabel(type);
	return html`<div class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--ok)] bg-[var(--surface)] p-4 flex gap-3 items-center">
			<span class="icon icon-lg icon-check-circle shrink-0" style="color:var(--ok)"></span>
			<div>
				<div class="text-sm font-medium text-[var(--text-strong)]">Channel connected</div>
				<div class="text-xs text-[var(--muted)] mt-0.5">${channelName} (${label}) is now linked to your agent.</div>
			</div>
		</div>
		${
			type === "discord" &&
			html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1.5">
			<span class="font-medium text-[var(--text-strong)]">Next steps</span>
			<span>\u2022 <strong>Invite to a server:</strong> the invite link was shown on the previous screen. You can also generate one in the <a href="https://discord.com/developers/applications" target="_blank" class="text-[var(--accent)] underline">Developer Portal</a> \u2192 OAuth2 \u2192 URL Generator (scope: bot, permissions: Send Messages, Attach Files, Read Message History).</span>
			<span>\u2022 <strong>DM the bot:</strong> search for the bot\u2019s username in Discord and click Message. Make sure your username is in the DM allowlist.</span>
			<span>\u2022 <strong>In a server:</strong> @mention the bot to get a response.</span>
		</div>`
		}
		<button type="button" class="text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none underline self-start" onClick=${onAnother}>Connect another channel</button>
	</div>`;
}

function NostrForm({ onConnected, error, setError }) {
	var [accountId, setAccountId] = useState("");
	var [secretKey, setSecretKey] = useState("");
	var [relays, setRelays] = useState("wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

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
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		var relayList = relays
			.split(",")
			.map((r) => r.trim())
			.filter(Boolean);
		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		var config = {
			secret_key: secretKey.trim(),
			relays: relayList,
			dm_policy: dmPolicy,
			allowed_pubkeys: allowlistEntries,
		};
		Object.assign(config, advancedPatch.value);
		addChannel("nostr", accountId.trim(), config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "nostr");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.");
			}
		});
	}

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">How to set up Nostr DMs</span>
			<span>1. Generate or use an existing Nostr secret key (nsec1... or hex)</span>
			<span>2. Configure relay URLs (defaults are provided)</span>
			<span>3. Add allowed public keys (npub1... or hex) to the allowlist</span>
			<span>4. Send a DM to the bot's public key from any Nostr client</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Account ID</label>
			<input type="text" class="provider-key-input w-full"
				value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
				placeholder="e.g. my-nostr-bot"
				autocomplete="off"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="nostr_account_id"
				autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Secret Key</label>
			<input type="password" class="provider-key-input w-full"
				value=${secretKey} onInput=${(e) => setSecretKey(e.target.value)}
				placeholder="nsec1... or 64-char hex"
				autocomplete="new-password"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="nostr_secret_key" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Relays (comma-separated)</label>
			<input type="text" class="provider-key-input w-full"
				value=${relays} onInput=${(e) => setRelays(e.target.value)}
				placeholder="wss://relay.damus.io, wss://nos.lol"
				name="nostr_relays" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="channel-select w-full" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="allowlist">Allowlist only</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Allowed Public Keys (one per line, npub1 or hex)</label>
			<textarea class="provider-key-input w-full" rows="3"
				value=${allowlist} onInput=${(e) => setAllowlist(e.target.value)}
				placeholder="npub1abc123...\nnpub1def456..."
				name="nostr_allowed_pubkeys" />
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig}
			onInput=${(value) => setAdvancedConfig(value)} />
		${error && html`<div class="text-xs text-[var(--error)]">${error}</div>`}
		<button key=${`nostr-${saving}`} type="submit" class="provider-btn self-start" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Nostr"}</button>
	</form>`;
}

function SlackForm({ onConnected, error, setError }) {
	var [accountId, setAccountId] = useState("");
	var [botToken, setBotToken] = useState("");
	var [connectionMode, setConnectionMode] = useState("socket_mode");
	var [appToken, setAppToken] = useState("");
	var [signingSecret, setSigningSecret] = useState("");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [advancedConfig, setAdvancedConfig] = useState("");
	var [saving, setSaving] = useState(false);

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
		var advancedPatch = parseChannelConfigPatch(advancedConfig);
		if (!advancedPatch.ok) {
			setError(advancedPatch.error);
			return;
		}
		setError(null);
		setSaving(true);
		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		var config = {
			bot_token: botToken.trim(),
			connection_mode: connectionMode,
			dm_policy: dmPolicy,
			mention_mode: "mention",
			allowlist: allowlistEntries,
		};
		if (connectionMode === "socket_mode") {
			config.app_token = appToken.trim();
		}
		if (connectionMode === "events_api") {
			config.signing_secret = signingSecret.trim();
		}
		Object.assign(config, advancedPatch.value);
		addChannel("slack", accountId.trim(), config).then((res) => {
			setSaving(false);
			if (res?.ok) {
				onConnected(accountId.trim(), "slack");
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect Slack.");
			}
		});
	}

	return html`<form onSubmit=${onSubmit} class="flex flex-col gap-3">
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">How to set up a Slack bot</span>
			<span>1. Go to <a href="https://api.slack.com/apps" target="_blank" class="text-[var(--accent)] underline">api.slack.com/apps</a> and create a new app</span>
			<span>2. Under OAuth & Permissions, add bot scopes: <code class="text-[var(--accent)]">chat:write</code>, <code class="text-[var(--accent)]">channels:history</code>, <code class="text-[var(--accent)]">im:history</code>, <code class="text-[var(--accent)]">app_mentions:read</code></span>
			<span>3. Install the app to your workspace and copy the Bot User OAuth Token</span>
			<span>4. For Socket Mode: enable it and generate an App-Level Token with <code class="text-[var(--accent)]">connections:write</code> scope</span>
			<span>5. For Events API: set the Request URL to your server\u2019s webhook endpoint</span>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Account ID</label>
			<input type="text" class="provider-key-input w-full"
				value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
				placeholder="e.g. my-slack-bot"
				autocomplete="off"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="slack_account_id"
				autofocus />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Bot Token (xoxb-...)</label>
			<input type="password" class="provider-key-input w-full"
				value=${botToken} onInput=${(e) => setBotToken(e.target.value)}
				placeholder="xoxb-..."
				autocomplete="new-password"
				autocapitalize="none"
				autocorrect="off"
				spellcheck="false"
				name="slack_bot_token" />
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Connection Mode</label>
			<select class="provider-key-input w-full cursor-pointer" value=${connectionMode} onChange=${(e) => setConnectionMode(e.target.value)}>
				<option value="socket_mode">Socket Mode (recommended)</option>
				<option value="events_api">Events API (HTTP webhook)</option>
			</select>
		</div>
		${
			connectionMode === "socket_mode" &&
			html`<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">App Token (xapp-...)</label>
				<input type="password" class="provider-key-input w-full"
					value=${appToken} onInput=${(e) => setAppToken(e.target.value)}
					placeholder="xapp-..."
					autocomplete="new-password"
					autocapitalize="none"
					autocorrect="off"
					spellcheck="false"
					name="slack_app_token" />
			</div>`
		}
		${
			connectionMode === "events_api" &&
			html`<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Signing Secret</label>
				<input type="password" class="provider-key-input w-full"
					value=${signingSecret} onInput=${(e) => setSigningSecret(e.target.value)}
					placeholder="Signing secret from Basic Information"
					autocomplete="new-password"
					autocapitalize="none"
					autocorrect="off"
					spellcheck="false"
					name="slack_signing_secret" />
			</div>`
		}
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">DM Policy</label>
			<select class="provider-key-input w-full cursor-pointer" value=${dmPolicy} onChange=${(e) => setDmPolicy(e.target.value)}>
				<option value="allowlist">Allowlist only (recommended)</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
		</div>
		<div>
			<label class="text-xs text-[var(--muted)] mb-1 block">Allowed Slack user(s)</label>
			<textarea class="provider-key-input w-full" rows="2"
				value=${allowlist} onInput=${(e) => setAllowlist(e.target.value)}
				placeholder="slack_username" style="resize:vertical;font-family:var(--font-body);" />
			<div class="text-xs text-[var(--muted)] mt-1">One per line. These users can DM your bot.</div>
		</div>
		<${AdvancedConfigPatchField} value=${advancedConfig} onInput=${setAdvancedConfig} />
		${error && html`<${ErrorPanel} message=${error} />`}
		<button key=${`slack-${saving}`} type="submit" class="provider-btn" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Slack"}</button>
	</form>`;
}

function ChannelStep({ onNext, onBack }) {
	var offeredList = getGon("channels_offered") || ["telegram", "whatsapp", "discord", "slack", "matrix"];
	var offered = new Set(offeredList);
	var singleType = offeredList.length === 1 ? offeredList[0] : null;

	var [phase, setPhase] = useState(singleType ? "form" : "select");
	var [selectedType, setSelectedType] = useState(singleType);
	var [connectedName, setConnectedName] = useState("");
	var [connectedType, setConnectedType] = useState(null);
	var [error, setError] = useState(null);

	function onSelectType(type) {
		setSelectedType(type);
		setPhase("form");
		setError(null);
	}

	function onConnected(name, type) {
		setConnectedName(name);
		setConnectedType(type);
		setPhase("success");
		setError(null);
	}

	function onAnother() {
		if (singleType) {
			setPhase("form");
			setError(null);
		} else {
			setPhase("select");
			setSelectedType(null);
			setError(null);
		}
	}

	var showBackSelector = phase === "form" && !singleType;

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Connect a Channel</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Connect a messaging channel so you can chat from your phone or team workspace. You can set this up later in Channels.</p>
		<${ChannelStorageNotice} />
		<div key=${`${phase}-${selectedType || "none"}`}>
		${phase === "select" && html`<${ChannelTypeSelector} onSelect=${onSelectType} offered=${offered} />`}
		${phase === "form" && selectedType === "telegram" && html`<${TelegramForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "whatsapp" && html`<${WhatsAppForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "msteams" && html`<${TeamsForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "discord" && html`<${DiscordForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "slack" && html`<${SlackForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "matrix" && html`<${MatrixForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "form" && selectedType === "nostr" && html`<${NostrForm} onConnected=${onConnected} error=${error} setError=${setError} />`}
		${phase === "success" && html`<${ChannelSuccess} channelName=${connectedName} channelType=${connectedType} onAnother=${onAnother} />`}
		</div>
		<div class="flex flex-wrap items-center gap-3 mt-1">
			<button type="button" class="provider-btn provider-btn-secondary" onClick=${showBackSelector ? () => setPhase("select") : onBack}>${t("common:actions.back")}</button>
			${phase === "success" && html`<button type="button" class="provider-btn" onClick=${onNext}>${t("common:actions.continue")}</button>`}
			<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>${t("common:actions.skip")}</button>
		</div>
	</div>`;
}

// ── Summary step helpers ─────────────────────────────────────

var LOW_MEMORY_THRESHOLD = 2 * 1024 * 1024 * 1024;

function formatMemBytes(bytes) {
	if (bytes == null) return "?";
	var gb = bytes / (1024 * 1024 * 1024);
	return `${gb.toFixed(1)} GB`;
}

function CheckIcon() {
	return html`<span class="icon icon-check-circle shrink-0" style="color:var(--ok)"></span>`;
}

function WarnIcon() {
	return html`<span class="icon icon-warn-triangle shrink-0" style="color:var(--warn)"></span>`;
}

function ErrorIcon() {
	return html`<span class="icon icon-x-circle shrink-0" style="color:var(--error)"></span>`;
}

function InfoIcon() {
	return html`<span class="icon icon-info-circle shrink-0" style="color:var(--muted)"></span>`;
}

function SummaryRow({ icon, label, children }) {
	return html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3 flex gap-3 items-start">
		<div class="mt-0.5">${icon}</div>
		<div class="flex-1 min-w-0">
			<div class="text-sm font-medium text-[var(--text-strong)]">${label}</div>
			<div class="text-xs text-[var(--muted)] mt-1">${children}</div>
		</div>
	</div>`;
}

// ── OpenClaw import step (conditional) ───────────────────────

function OpenClawImportStep({ onNext, onBack }) {
	var [loading, setLoading] = useState(true);
	var [scan, setScan] = useState(null);
	var [importing, setImporting] = useState(false);
	var [done, setDone] = useState(false);
	var [result, setResult] = useState(null);
	var [error, setError] = useState(null);
	var [selection, setSelection] = useState({
		identity: true,
		providers: true,
		skills: true,
		memory: true,
		channels: true,
		sessions: true,
		workspace_files: true,
	});

	useEffect(() => {
		var cancelled = false;
		var attempts = 0;
		var retryTimer = null;

		function loadScan() {
			if (cancelled) return;
			sendRpc("openclaw.scan", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setScan(res.payload);
					setLoading(false);
					return;
				}

				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					ensureWsConnected();
					retryTimer = window.setTimeout(loadScan, WS_RETRY_DELAY_MS);
					return;
				}

				setError(res?.error?.message || "Failed to scan OpenClaw installation");
				setLoading(false);
			});
		}

		ensureWsConnected();
		loadScan();
		return () => {
			cancelled = true;
			if (retryTimer) {
				window.clearTimeout(retryTimer);
				retryTimer = null;
			}
		};
	}, []);

	function toggleCategory(key) {
		setSelection((prev) => {
			var next = Object.assign({}, prev);
			next[key] = !prev[key];
			return next;
		});
	}

	async function doImport() {
		setImporting(true);
		setError(null);
		var res = await sendRpc("openclaw.import", selection);
		setImporting(false);
		if (res?.ok) {
			setResult(res.payload);
			await refreshGon();
			setDone(true);
		} else {
			setError(res?.error?.message || "Import failed");
		}
	}

	if (loading) {
		return html`<div class="flex flex-col items-center justify-center gap-3 min-h-[200px]">
			<div class="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin"></div>
			<div class="text-sm text-[var(--muted)]">Scanning OpenClaw installation\u2026</div>
		</div>`;
	}

	if (done && result) {
		var total = (result.categories || []).reduce((sum, cat) => sum + (Number(cat.items_imported) || 0), 0);
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Import Complete</h2>
			<p class="text-xs text-[var(--muted)] leading-relaxed">${total} item(s) imported from OpenClaw.</p>
			${
				result.categories
					? html`<div class="flex flex-col gap-1">
						${result.categories.map(
							(cat) => html`<div key=${cat.category} class="text-xs text-[var(--text)]">
								<span class="font-mono">[${cat.status === "success" ? "\u2713" : cat.status === "partial" ? "~" : cat.status === "skipped" ? "-" : "!"}]</span>
								${cat.category}: ${cat.items_imported} imported, ${cat.items_skipped} skipped
								${(cat.warnings || []).map((w) => html`<div class="text-[var(--warn)] ml-6">${w}</div>`)}
							</div>`,
						)}
					</div>`
					: null
			}
			${
				result.todos?.length > 0
					? html`<div class="text-xs text-[var(--muted)]">
						<div class="font-medium">Not yet supported in Moltis:</div>
						${result.todos.map((t) => html`<div key=${t.feature}>\u2022 ${t.feature}: ${t.description}</div>`)}
					</div>`
					: null
			}
			<div class="flex flex-wrap items-center gap-3 mt-1">
				<button class="provider-btn" onClick=${onNext}>Continue</button>
			</div>
		</div>`;
	}

	if (!scan?.detected) {
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Import from OpenClaw</h2>
			<p class="text-xs text-[var(--muted)]">Could not scan OpenClaw installation.</p>
			<div class="flex flex-wrap items-center gap-3 mt-1">
				${onBack ? html`<button class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>` : null}
				<button class="provider-btn" onClick=${onNext}>Skip</button>
			</div>
		</div>`;
	}

	var telegramAccounts = Number(scan.telegram_accounts) || 0;
	var discordAccounts = Number(scan.discord_accounts) || 0;
	var channelParts = [];
	if (telegramAccounts > 0) channelParts.push(`${telegramAccounts} Telegram account(s)`);
	if (discordAccounts > 0) channelParts.push(`${discordAccounts} Discord account(s)`);
	var channelDetail = channelParts.length > 0 ? channelParts.join(", ") : null;
	var unsupportedChannels = (scan.unsupported_channels || []).filter(
		(channel) => String(channel).toLowerCase() !== "discord",
	);

	var categories = [
		{
			key: "identity",
			label: "Identity",
			available: scan.identity_available,
			detail: [scan.identity_agent_name, scan.identity_theme].filter(Boolean).join(", ") || null,
		},
		{ key: "providers", label: "Providers", available: scan.providers_available },
		{ key: "skills", label: "Skills", available: scan.skills_count > 0, detail: `${scan.skills_count} skill(s)` },
		{
			key: "memory",
			label: "Memory",
			available: scan.memory_available,
			detail: `${scan.memory_files_count} memory file(s)`,
		},
		{
			key: "channels",
			label: "Channels",
			available: scan.channels_available,
			detail: channelDetail,
		},
		{
			key: "sessions",
			label: "Sessions",
			available: scan.sessions_count > 0,
			detail: `${scan.sessions_count} session(s)`,
		},
		{
			key: "workspace_files",
			label: "Workspace Files",
			available: scan.workspace_files_available,
			detail: scan.workspace_files_found?.length > 0 ? scan.workspace_files_found.join(", ") : null,
		},
	];
	var anySelected = categories.some((c) => c.available && selection[c.key]);

	var workspaceMissing = !scan.memory_available && scan.skills_count === 0 && !scan.identity_theme;

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Import from OpenClaw</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			We detected an OpenClaw installation at <code class="text-[var(--text)]">${scan.home_dir}</code>.
			Select the data you'd like to bring into Moltis.
		</p>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			This is a read-only copy \u2014 your OpenClaw installation will not be modified or removed.
			You can keep using OpenClaw alongside Moltis, and re-import at any time from Settings.
		</p>
		${
			workspaceMissing
				? html`<p class="text-xs text-[var(--muted)] leading-relaxed">
			If OpenClaw ran on another machine, copy its workspace directory
			(e.g. <code>clawd/</code>) into <code>${scan.home_dir}/</code> or <code>~/</code>
			for a full import including identity, memory, and skills.
		</p>`
				: null
		}
		${error ? html`<${ErrorPanel} message=${error} />` : null}
		<div class="flex flex-col gap-2" style="max-width:400px;">
			${categories.map(
				(cat) => html`<label
					key=${cat.key}
					class="flex items-center gap-2 text-sm cursor-pointer ${cat.available ? "text-[var(--text)]" : "text-[var(--muted)] opacity-60"}">
					<input
						type="checkbox"
						checked=${selection[cat.key] && cat.available}
						disabled=${!cat.available || importing}
						onChange=${() => toggleCategory(cat.key)}
					/>
					<span>${cat.label}</span>
					${cat.detail && cat.available ? html`<span class="text-xs text-[var(--muted)]">(${cat.detail})</span>` : null}
					${cat.available ? null : html`<span class="text-xs text-[var(--muted)]">(not found)</span>`}
				</label>`,
			)}
		</div>
		${
			scan.agents?.length > 1
				? html`<div class="text-xs text-[var(--muted)] leading-relaxed border border-[var(--border)] rounded p-2" style="max-width:400px;">
					<span class="font-medium text-[var(--text)]">${scan.agents.length} agents detected</span>
					<span class="ml-1">\u2014 non-default agents will be created as separate personas:</span>
					<ul class="mt-1 ml-4 list-disc">
						${scan.agents.map(
							(a) =>
								html`<li key=${a.openclaw_id}>
									<span class="text-[var(--text)]">${a.name || a.openclaw_id}</span>${a.is_default ? html`<span class="ml-1 text-[var(--muted)]">(default)</span>` : null}${a.theme ? html`<span class="ml-1 text-[var(--muted)]">\u2014 ${a.theme}</span>` : null}
								</li>`,
						)}
					</ul>
				</div>`
				: null
		}
		${
			unsupportedChannels.length > 0
				? html`<p class="text-xs text-[var(--muted)]">
					Unsupported channels (coming soon): ${unsupportedChannels.join(", ")}
				</p>`
				: null
		}
		<div class="flex flex-wrap items-center gap-3 mt-1">
			${onBack ? html`<button class="provider-btn provider-btn-secondary" onClick=${onBack} disabled=${importing}>Back</button>` : null}
			<button key=${`import-${importing}`} class="provider-btn" onClick=${doImport} disabled=${!anySelected || importing}>${importing ? "Importing\u2026" : "Import Selected"}
			</button>
			<button
				class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
				onClick=${onNext}
				disabled=${importing}
			>Skip for now</button>
		</div>
	</div>`;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: summary step fetches multiple data sources and renders conditional sections
function SummaryStep({ onBack, onFinish }) {
	var [loading, setLoading] = useState(true);
	var [data, setData] = useState(null);

	useEffect(() => {
		var cancelled = false;

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: parallel data fetches and conditional gon reads
		async function load() {
			await refreshGon();

			var identity = getGon("identity");
			var mem = getGon("mem");
			var update = getGon("update");
			var voiceEnabled = getGon("voice_enabled");

			var [providersRes, channelsRes, tailscaleRes, voiceRes, bootstrapRes] = await Promise.all([
				sendRpc("providers.available", {}).catch(() => null),
				fetchChannelStatus().catch(() => null),
				fetch("/api/tailscale/status")
					.then((r) => (r.ok ? r.json() : null))
					.catch(() => null),
				voiceEnabled ? fetchVoiceProviders().catch(() => null) : Promise.resolve(null),
				fetch(
					"/api/bootstrap?include_channels=false&include_sessions=false&include_models=false&include_projects=false&include_counts=false&include_identity=false",
				)
					.then((r) => (r.ok ? r.json() : null))
					.catch(() => null),
			]);

			if (cancelled) return;

			setData({
				identity,
				mem,
				update,
				voiceEnabled,
				providers: providersRes?.ok ? providersRes.payload || [] : [],
				channels: channelsRes?.ok ? channelsRes.payload?.channels || [] : [],
				tailscale: tailscaleRes,
				voice: voiceRes?.ok ? voiceRes.payload || { tts: [], stt: [] } : null,
				sandbox: bootstrapRes?.sandbox || null,
			});
			setLoading(false);
		}

		load();
		return () => {
			cancelled = true;
		};
	}, []);

	if (loading || !data) {
		return html`<div class="flex flex-col items-center justify-center gap-3 min-h-[200px]">
			<div class="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin"></div>
			<div class="text-sm text-[var(--muted)]">${t("onboarding:summary.loadingSummary")}</div>
		</div>`;
	}

	var activeModel = localStorage.getItem("moltis-model");
	var configuredProviders = data.providers.filter((p) => p.configured);

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("onboarding:summary.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Overview of your configuration. You can change any of these later in Settings.</p>

		<div class="flex flex-col gap-2">
			<!-- Identity -->
			<${SummaryRow}
				icon=${data.identity?.user_name && data.identity?.name ? html`<${CheckIcon} />` : html`<${WarnIcon} />`}
				label="Identity">
					${
						data.identity?.user_name && data.identity?.name
							? html`You: <span class="font-medium text-[var(--text)]">${data.identity.user_name}</span> Agent:
							<span class="font-medium text-[var(--text)]">${data.identity.emoji || ""} ${data.identity.name}</span>`
							: html`<span class="text-[var(--warn)]">Identity not fully configured</span>`
					}
				<//>

			<!-- LLMs -->
			<${SummaryRow}
				icon=${configuredProviders.length > 0 ? html`<${CheckIcon} />` : html`<${ErrorIcon} />`}
				label="LLMs">
				${
					configuredProviders.length > 0
						? html`<div class="flex flex-col gap-1">
						<div class="flex flex-wrap gap-1">
							${configuredProviders.map((p) => html`<span key=${p.name} class="provider-item-badge configured">${p.displayName}</span>`)}
						</div>
						${activeModel ? html`<div>Active model: <span class="font-mono font-medium text-[var(--text)]">${activeModel}</span></div>` : null}
					</div>`
						: html`<span class="text-[var(--error)]">No LLM providers configured</span>`
				}
			<//>

			<!-- Channels -->
			<${SummaryRow}
				icon=${
					data.channels.length > 0
						? data.channels.some((c) => c.status === "error")
							? html`<${ErrorIcon} />`
							: data.channels.some((c) => c.status === "disconnected")
								? html`<${WarnIcon} />`
								: html`<${CheckIcon} />`
						: html`<${InfoIcon} />`
				}
				label="Channels">
				${
					data.channels.length > 0
						? html`<div class="flex flex-col gap-1">
						${data.channels.map((ch) => {
							var statusColor =
								ch.status === "connected" ? "var(--ok)" : ch.status === "error" ? "var(--error)" : "var(--warn)";
							return html`<div key=${ch.account_id} class="flex items-center gap-1">
								<span style="color:${statusColor}">\u25CF</span>
								<span class="font-medium text-[var(--text)]">${ch.type}</span>: ${ch.name || ch.account_id}
								<span>(${ch.status})</span>
							</div>`;
						})}
					</div>`
						: html`No channels configured`
				}
			<//>

			<!-- System Memory -->
			<${SummaryRow}
				icon=${data.mem?.total && data.mem.total < LOW_MEMORY_THRESHOLD ? html`<${WarnIcon} />` : html`<${CheckIcon} />`}
				label="System Memory">
				${
					data.mem
						? html`Total: <span class="font-medium text-[var(--text)]">${formatMemBytes(data.mem.total)}</span>
						Available: <span class="font-medium text-[var(--text)]">${formatMemBytes(data.mem.available)}</span>
						${data.mem.total && data.mem.total < LOW_MEMORY_THRESHOLD ? html`<div class="text-[var(--warn)] mt-1">Low memory detected. Consider upgrading to an instance with more RAM.</div>` : null}`
						: html`Memory info unavailable`
				}
			<//>

			<!-- Sandbox -->
			<${SummaryRow}
				icon=${data.sandbox?.backend && data.sandbox.backend !== "none" ? html`<${CheckIcon} />` : html`<${InfoIcon} />`}
				label="Sandbox">
				${
					data.sandbox?.backend && data.sandbox.backend !== "none"
						? html`Backend: <span class="font-medium text-[var(--text)]">${data.sandbox.backend}</span>`
						: html`No container runtime detected`
				}
			<//>

			<!-- Version -->
			<${SummaryRow}
				icon=${data.update?.available ? html`<${WarnIcon} />` : html`<${CheckIcon} />`}
				label="Version">
				${
					data.update?.available
						? html`Update available: <a href=${data.update.release_url || "#"} target="_blank" class="text-[var(--accent)] underline font-medium">${data.update.latest_version}</a>`
						: html`You are running the latest version.`
				}
			<//>

			<!-- Tailscale (hidden if feature not compiled) -->
			${
				data.tailscale !== null
					? html`<${SummaryRow}
					icon=${data.tailscale?.tailscale_up ? html`<${CheckIcon} />` : data.tailscale?.installed ? html`<${WarnIcon} />` : html`<${InfoIcon} />`}
					label="Tailscale">
					${
						data.tailscale?.tailscale_up
							? html`Connected`
							: data.tailscale?.installed
								? html`Installed but not connected — <a href="/settings/remote-access" class="text-[var(--accent)] underline">Configure in Settings</a>`
								: html`Not installed. Install Tailscale for secure remote access.`
					}
				<//>`
					: null
			}

			<!-- Voice (hidden if not enabled) -->
			${
				data.voiceEnabled
					? html`<${SummaryRow}
					icon=${data.voice && ([...data.voice.tts, ...data.voice.stt].some((p) => p.enabled)) ? html`<${CheckIcon} />` : html`<${InfoIcon} />`}
					label="Voice">
					${(() => {
						if (!data.voice) return html`Voice providers unavailable`;
						var enabledStt = data.voice.stt.filter((p) => p.enabled).map((p) => p.name);
						var enabledTts = data.voice.tts.filter((p) => p.enabled).map((p) => p.name);
						if (enabledStt.length === 0 && enabledTts.length === 0) return html`No voice providers enabled`;
						return html`<div class="flex flex-col gap-0.5">
							${enabledStt.length > 0 ? html`<div>STT: <span class="font-medium text-[var(--text)]">${enabledStt.join(", ")}</span></div>` : null}
							${enabledTts.length > 0 ? html`<div>TTS: <span class="font-medium text-[var(--text)]">${enabledTts.join(", ")}</span></div>` : null}
						</div>`;
					})()}
				<//>`
					: null
			}
		</div>

		<div class="flex flex-wrap items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>${t("common:actions.back")}</button>
			<div class="flex-1" />
			<button class="provider-btn" onClick=${onFinish}>${data.identity?.emoji || ""} ${data.identity?.name || "Your agent"}, reporting for duty</button>
		</div>
	</div>`;
}

// ── Main page component ─────────────────────────────────────

function OnboardingPage() {
	var [step, setStep] = useState(-1); // -1 = checking
	var [authNeeded, setAuthNeeded] = useState(false);
	var [authSkippable, setAuthSkippable] = useState(false);
	var [voiceAvailable] = useState(() => getGon("voice_enabled") === true);
	var headerRef = useRef(null);
	var navRef = useRef(null);
	var sessionsPanelRef = useRef(null);

	// Hide nav, header, and banners for standalone experience
	useEffect(() => {
		var header = document.querySelector("header");
		var nav = document.getElementById("navPanel");
		var sessions = document.getElementById("sessionsPanel");
		var burger = document.getElementById("burgerBtn");
		var toggle = document.getElementById("sessionsToggle");
		var authBanner = document.getElementById("authDisabledBanner");
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
			// Don't restore authBanner — app.js will re-show it if needed
		};
	}, []);

	// Check auth status to decide whether to show step 0
	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((auth) => {
				if (auth?.setup_required || (auth?.auth_disabled && !auth?.localhost_only)) {
					setAuthNeeded(true);
					setAuthSkippable(!auth.setup_required);
					setStep(0);
				} else {
					setAuthNeeded(false);
					ensureWsConnected();
					setStep(1);
				}
			})
			.catch(() => {
				setAuthNeeded(false);
				ensureWsConnected();
				setStep(1);
			});
	}, []);

	if (step === -1) {
		return html`<div class="onboarding-card">
			<div class="text-sm text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	// Build step list dynamically based on auth + voice + openclaw availability
	var openclawDetected = getGon("openclaw_detected") === true;
	var allLabels = [t("onboarding:steps.security")];
	if (openclawDetected) allLabels.push(t("onboarding:steps.import"));
	allLabels.push(t("onboarding:steps.llm"));
	if (voiceAvailable) allLabels.push(t("onboarding:steps.voice"));
	allLabels.push(
		t("onboarding:steps.remoteAccess"),
		t("onboarding:steps.channel"),
		t("onboarding:steps.identity"),
		t("onboarding:steps.summary"),
	);
	var steps = authNeeded ? allLabels : allLabels.slice(1);
	var stepIndex = authNeeded ? step : step - 1;

	// Compute dynamic step indices: Auth(0) → Import? → LLM → Voice? →
	// Remote Access → Channel → Identity → Summary
	var nextIdx = 1;
	var importStep = openclawDetected ? nextIdx++ : -1;
	var llmStep = nextIdx++;
	var voiceStep = voiceAvailable ? nextIdx++ : -1;
	var remoteAccessStep = nextIdx++;
	var channelStep = nextIdx++;
	var identityStep = nextIdx++;
	var summaryStep = nextIdx;
	var lastStep = summaryStep;

	function goNext() {
		if (step === lastStep) {
			window.location.assign(preferredChatPath());
		} else {
			setStep(step + 1);
		}
	}

	function goFinish() {
		window.location.assign(preferredChatPath());
	}

	function goBack() {
		if (authNeeded) {
			setStep(Math.max(0, step - 1));
		} else {
			setStep(Math.max(1, step - 1));
		}
	}

	var startedAt = getGon("started_at");
	var version = String(getGon("version") || "").trim();

	return html`<div class="onboarding-card">
		<${StepIndicator} steps=${steps} current=${stepIndex} />
		<div class="mt-6">
			${step === 0 && html`<${AuthStep} onNext=${goNext} skippable=${authSkippable} />`}
			${step === importStep && html`<${OpenClawImportStep} onNext=${goNext} onBack=${authNeeded ? goBack : null} />`}
			${step === llmStep && html`<${ProviderStep} onNext=${goNext} onBack=${authNeeded || openclawDetected ? goBack : null} />`}
			${step === voiceStep && html`<${VoiceStep} onNext=${goNext} onBack=${goBack} />`}
			${step === remoteAccessStep && html`<${RemoteAccessStep} onNext=${goNext} onBack=${goBack} />`}
			${step === channelStep && html`<${ChannelStep} onNext=${goNext} onBack=${goBack} />`}
			${step === identityStep && html`<${IdentityStep} onNext=${goNext} onBack=${goBack} />`}
			${step === summaryStep && html`<${SummaryStep} onBack=${goBack} onFinish=${goFinish} />`}
		</div>
		${
			startedAt || version
				? html`<div class="text-xs text-[var(--muted)] text-center mt-4 pt-3 border-t border-[var(--border)]">
					${startedAt ? html`<span>Server started <time data-epoch-ms=${startedAt}></time></span>` : null}
					${startedAt && version ? html`<span> · </span>` : null}
					${version ? html`<span>${t("onboarding:summary.versionLabel")} v${version}</span>` : null}
				</div>`
				: null
		}
	</div>`;
}

// ── Page registration ───────────────────────────────────────

var containerRef = null;

export function mountOnboarding(container) {
	containerRef = container;
	container.style.cssText =
		"display:flex;align-items:flex-start;justify-content:center;min-height:100vh;padding:max(0.75rem, env(safe-area-inset-top)) max(0.75rem, env(safe-area-inset-right)) max(0.75rem, env(safe-area-inset-bottom)) max(0.75rem, env(safe-area-inset-left));box-sizing:border-box;width:100%;max-width:100vw;overflow-x:hidden;overflow-y:auto;";
	render(html`<${OnboardingPage} />`, container);
}

export function unmountOnboarding() {
	if (containerRef) render(null, containerRef);
	containerRef = null;
}
