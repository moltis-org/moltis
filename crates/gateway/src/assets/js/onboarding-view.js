// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Identity → Provider → Voice (conditional) → Channel → Summary
// No new Rust code — all existing RPC methods and REST endpoints.

import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { get as getGon, refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { detectPasskeyName } from "./passkey-detect.js";
import { providerApiKeyHelp } from "./provider-key-help.js";
import { startProviderOAuth } from "./provider-oauth.js";
import { isModelServiceNotConfigured, testModel, validateProviderKey } from "./provider-validation.js";
import * as S from "./state.js";
import { fetchPhrase } from "./tts-phrases.js";
import { connectWs } from "./ws-connect.js";

var wsStarted = false;
function ensureWsConnected() {
	if (wsStarted) return;
	wsStarted = true;
	connectWs({ backoff: { factor: 2, max: 10000 } });
}

// ── Step indicator ──────────────────────────────────────────

var BASE_STEP_LABELS = ["Security", "Identity", "LLM", "Channel", "Summary"];
var VOICE_STEP_LABELS = ["Security", "Identity", "LLM", "Voice", "Channel", "Summary"];

function preferredChatPath() {
	var key = localStorage.getItem("moltis-session") || "main";
	return `/chats/${key.replace(/:/g, "/")}`;
}

function ErrorPanel({ message }) {
	return html`<div role="alert" class="alert-error-text whitespace-pre-line">
		<span class="text-[var(--error)] font-medium">Error:</span> ${message}
	</div>`;
}

function StepIndicator({ steps, current }) {
	return html`<div class="onboarding-steps">
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

	var isIpAddress = /^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[");
	var browserSupportsWebauthn = !!window.PublicKeyCredential;
	var passkeyEnabled = webauthnAvailable && browserSupportsWebauthn && !isIpAddress;

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => r.json())
			.then((data) => {
				if (data.setup_code_required) setCodeRequired(true);
				if (data.localhost_only) setLocalhostOnly(true);
				if (data.webauthn_available) setWebauthnAvailable(true);
				if (data.passkey_origins) setPasskeyOrigins(data.passkey_origins);
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
			if (password.length < 8) {
				setError("Password must be at least 8 characters.");
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
					onNext();
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
					setError(err.message || "Passkey registration failed");
				}
				setSaving(false);
			});
	}

	function onOptionalPassword(e) {
		e.preventDefault();
		setError(null);
		if (optPw.length < 8) {
			setError("Password must be at least 8 characters.");
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
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Secure your instance</h2>

			<div class="flex items-center gap-2 text-sm text-[var(--accent)]">
				<span class="icon icon-checkmark"></span>
				Passkey registered successfully!
			</div>

			<p class="text-xs text-[var(--muted)] leading-relaxed">
				Optionally set a password as a fallback for when passkeys aren't available.
			</p>

			<form onSubmit=${onOptionalPassword} class="flex flex-col gap-3">
				<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">Password</label>
					<input type="password" class="provider-key-input w-full"
						value=${optPw} onInput=${(e) => setOptPw(e.target.value)}
						placeholder="At least 8 characters" autofocus />
				</div>
				<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">Confirm password</label>
					<input type="password" class="provider-key-input w-full"
						value=${optPwConfirm} onInput=${(e) => setOptPwConfirm(e.target.value)}
						placeholder="Repeat password" />
				</div>
				${error && html`<${ErrorPanel} message=${error} />`}
				<div class="flex items-center gap-3 mt-1">
					<button type="submit" class="provider-btn" disabled=${optPwSaving}>
						${optPwSaving ? "Setting\u2026" : "Set password & continue"}
					</button>
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
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Secure your instance</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			${localhostOnly ? "Choose how to secure your instance, or skip for now." : "Choose how to secure your instance."}
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
				<div class="flex items-center justify-between">
					<span class="text-sm font-medium text-[var(--text)]">Passkey</span>
					<div class="flex gap-2">
						${passkeyEnabled ? html`<span class="recommended-badge">Recommended</span>` : null}
						${passkeyDisabledReason ? html`<span class="tier-badge">${passkeyDisabledReason}</span>` : null}
					</div>
				</div>
				<div class="text-xs text-[var(--muted)] mt-1">Use Touch ID, Face ID, or a security key</div>
			</div>
			<div class=${`backend-card ${method === "password" ? "selected" : ""}`}
				onClick=${() => setMethod("password")}>
				<div class="flex items-center justify-between">
					<span class="text-sm font-medium text-[var(--text)]">Password</span>
				</div>
				<div class="text-xs text-[var(--muted)] mt-1">Set a traditional password</div>
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
			<div class="flex items-center gap-3 mt-1">
				<button type="button" class="provider-btn" disabled=${saving} onClick=${onPasskeyRegister}>
					${saving ? "Registering\u2026" : "Register passkey"}
				</button>
				${skippable && html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>`}
			</div>
		</div>`
		}

		${
			method === "password" &&
			html`<form onSubmit=${onPasswordSubmit} class="flex flex-col gap-3">
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Password${localhostOnly ? "" : " *"}</label>
				<input type="password" class="provider-key-input w-full"
					value=${password} onInput=${(e) => setPassword(e.target.value)}
					placeholder=${localhostOnly ? "Optional on localhost" : "At least 8 characters"} autofocus />
			</div>
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Confirm password</label>
				<input type="password" class="provider-key-input w-full"
					value=${confirm} onInput=${(e) => setConfirm(e.target.value)}
					placeholder="Repeat password" />
			</div>
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex items-center gap-3 mt-1">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Setting up\u2026" : localhostOnly && !password ? "Skip" : "Set password"}
				</button>
				${skippable && html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>`}
			</div>
		</form>`
		}

		${
			method === null &&
			html`<div class="flex items-center gap-3 mt-1">
			${skippable && html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>`}
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
	var [creature, setCreature] = useState(identity.creature || "");
	var [vibe, setVibe] = useState(identity.vibe || "");
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);

	function onSubmit(e) {
		e.preventDefault();
		if (!userName.trim()) {
			setError("Your name is required.");
			return;
		}
		if (!name.trim()) {
			setError("Agent name is required.");
			return;
		}
		setError(null);
		setSaving(true);
		sendRpc("agent.identity.update", {
			name: name.trim(),
			emoji: emoji.trim() || "",
			creature: creature.trim() || "",
			vibe: vibe.trim() || "",
			user_name: userName.trim(),
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
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Set up your identity</h2>
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
			<div class="grid grid-cols-2 gap-x-4 gap-y-3">
				<div>
					<div class="text-xs text-[var(--muted)] mb-1">Agent name *</div>
					<input type="text" class="provider-key-input w-full"
						value=${name} onInput=${(e) => setName(e.target.value)}
						placeholder="e.g. Rex" />
				</div>
				<div>
					<div class="text-xs text-[var(--muted)] mb-1">Emoji</div>
					<${EmojiPicker} value=${emoji} onChange=${setEmoji} />
				</div>
				<div>
					<div class="text-xs text-[var(--muted)] mb-1">Creature</div>
					<input type="text" class="provider-key-input w-full"
						value=${creature} onInput=${(e) => setCreature(e.target.value)}
						placeholder="e.g. dog" />
				</div>
				<div>
					<div class="text-xs text-[var(--muted)] mb-1">Vibe</div>
					<input type="text" class="provider-key-input w-full"
						value=${vibe} onInput=${(e) => setVibe(e.target.value)}
						placeholder="e.g. chill" />
				</div>
			</div>
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex items-center gap-3 mt-1">
				${onBack && html`<button type="button" class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>`}
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Saving\u2026" : "Continue"}
				</button>
			</div>
		</form>
	</div>`;
}

// ── Provider step ───────────────────────────────────────────

var OPENAI_COMPATIBLE = ["openai", "mistral", "openrouter", "cerebras", "minimax", "moonshot", "venice", "ollama"];
var BYOM_PROVIDERS = ["openrouter", "venice"];

// ── Provider row for multi-provider onboarding ──────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config forms for api-key, oauth, and local flows
function OnboardingProviderRow({
	provider,
	configuring,
	phase,
	providerModels,
	selectedModel,
	modelTestError,
	modelSearch,
	setModelSearch,
	oauthProvider,
	oauthInfo,
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
	error,
	validationResult,
	onStartConfigure,
	onCancelConfigure,
	onSaveKey,
	onSelectModel,
	onCancelOAuth,
	onConfigureLocalModel,
	onCancelLocal,
}) {
	var isApiKeyForm = configuring === provider.name && (phase === "form" || phase === "validating");
	var isModelSelect = configuring === provider.name && (phase === "selectModel" || phase === "testingModel");
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

	// Filter models for the model selector.
	var filteredModels = (providerModels || []).filter(
		(m) =>
			!modelSearch ||
			m.displayName.toLowerCase().includes(modelSearch.toLowerCase()) ||
			m.id.toLowerCase().includes(modelSearch.toLowerCase()),
	);

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
					<button type="submit" class="provider-btn provider-btn-sm" disabled=${phase === "validating"}>${phase === "validating" ? "Validating\u2026" : "Save & Validate"}</button>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onCancelConfigure} disabled=${phase === "validating"}>Cancel</button>
				</div>
			</form>`
				: null
		}
		${
			isModelSelect
				? html`<div class="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
				<div class="text-xs font-medium text-[var(--text-strong)]">Select a model</div>
				${
					(providerModels || []).length > 5
						? html`<input type="text" class="provider-key-input w-full text-xs"
							placeholder="Search models\u2026"
							value=${modelSearch}
							onInput=${(e) => setModelSearch(e.target.value)} />`
						: null
				}
				<div class="flex flex-col gap-2 max-h-56 overflow-y-auto">
					${
						filteredModels.length === 0
							? html`<div class="text-xs text-[var(--muted)] py-4 text-center">No models match your search.</div>`
							: filteredModels.map(
									(m) => html`<div key=${m.id}
									class="model-card ${selectedModel === m.id ? "selected" : ""}"
									onClick=${() => {
										if (phase !== "testingModel") onSelectModel(m.id);
									}}>
									<div class="flex items-center justify-between">
										<span class="text-sm font-medium text-[var(--text)]">${m.displayName}</span>
										<div class="flex gap-2">
											${m.supportsTools ? html`<span class="recommended-badge">Tools</span>` : null}
											${phase === "testingModel" && selectedModel === m.id ? html`<span class="tier-badge">Testing\u2026</span>` : null}
										</div>
									</div>
									<div class="text-xs text-[var(--muted)] mt-1 font-mono">${m.id}</div>
								</div>`,
								)
					}
				</div>
				${modelTestError ? html`<${ErrorPanel} message=${modelTestError} />` : null}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm self-start" onClick=${onCancelConfigure}>Cancel</button>
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
										<div class="flex items-center justify-between">
											<span class="text-sm font-medium text-[var(--text)]">${b.name}</span>
											<div class="flex gap-2">
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
						<div class="flex flex-col gap-2 max-h-48 overflow-y-auto">
							${
								localModels.filter((m) => m.backend === selectedBackend).length === 0
									? html`<div class="text-xs text-[var(--muted)] py-4 text-center">No models available for ${selectedBackend}</div>`
									: localModels
											.filter((m) => m.backend === selectedBackend)
											.map(
												(mdl) => html`<div key=${mdl.id} class="model-card" onClick=${() => onConfigureLocalModel(mdl)}>
											<div class="flex items-center justify-between">
												<span class="text-sm font-medium text-[var(--text)]">${mdl.displayName}</span>
												<div class="flex gap-2">
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
	};
}

function ProviderStep({ onNext, onBack }) {
	var [providers, setProviders] = useState([]);
	var [loading, setLoading] = useState(true);
	var [error, setError] = useState(null);

	// Which provider has an open inline form (by name), or null
	var [configuring, setConfiguring] = useState(null);
	var [oauthProvider, setOauthProvider] = useState(null);
	var [localProvider, setLocalProvider] = useState(null);

	// Phase: "form" | "validating" | "selectModel" | "testingModel"
	var [phase, setPhase] = useState("form");
	var [providerModels, setProviderModels] = useState([]);
	var [selectedModel, setSelectedModel] = useState(null);
	var [modelTestError, setModelTestError] = useState(null);
	var [modelSearch, setModelSearch] = useState("");

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

				if (res?.error?.message === "WebSocket not connected" && attempts < 30) {
					attempts += 1;
					window.setTimeout(loadProviders, 200);
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
		setSelectedModel(null);
		setModelTestError(null);
		setModelSearch("");
		setApiKey("");
		setEndpoint("");
		setModel("");
		setError(null);
		setOauthInfo(null);
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
		setModelSelectProvider(provider.name);
		setConfiguring(provider.name);
		setProviderModels(existingModels);
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
			.then((result) => {
				if (!result.valid) {
					// Validation failed — stay on the form.
					setPhase("form");
					setError(result.error || "Validation failed. Please check your credentials.");
					return;
				}

				// BYOM providers: we already tested the specific model during validation,
				// so save immediately without showing the model selector.
				if (BYOM_PROVIDERS.includes(p.name)) {
					return saveAndFinish(p.name, keyVal, endpointVal, modelVal);
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

	function onSelectModel(modelId) {
		setSelectedModel(modelId);
		setModelTestError(null);
		setPhase("testingModel");

		if (modelSelectProvider) {
			// Existing credentials flow: only test + save preferred model.
			testModel(modelId).then((testResult) => {
				if (!testResult.ok) {
					setPhase("selectModel");
					setModelTestError(testResult.error || "Model test failed. Try another model.");
					return;
				}
				sendRpc("providers.save_model", { provider: modelSelectProvider, model: modelId }).then(() => {
					localStorage.setItem("moltis-model", modelId);
					setValidationResults((prev) => ({ ...prev, [modelSelectProvider]: { ok: true } }));
					setModelSelectProvider(null);
					setConfiguring(null);
					setPhase("form");
					setProviderModels([]);
					setSelectedModel(null);
					setModelTestError(null);
					setModelSearch("");
					setError(null);
					refreshProviders();
				});
			});
			return;
		}

		var p = providers.find((pr) => pr.name === configuring);
		if (!p) return;

		// Save credentials first so the model is available in the live registry.
		var keyVal = apiKey.trim() || p.name;
		var endpointVal = endpoint.trim() || null;
		var modelVal = model.trim() || null;

		saveAndFinish(p.name, keyVal, endpointVal, modelVal, modelId);
	}

	function saveAndFinish(providerName, keyVal, endpointVal, modelVal, selectedModelId) {
		var p = providers.find((pr) => pr.name === providerName);
		var effectiveModelVal = p?.keyOptional && selectedModelId ? selectedModelId : modelVal;
		var payload = { provider: providerName, apiKey: keyVal };
		if (endpointVal) payload.baseUrl = endpointVal;
		if (effectiveModelVal) payload.model = effectiveModelVal;

		sendRpc("providers.save_key", payload)
			.then(async (res) => {
				if (!res?.ok) {
					setPhase("form");
					setError(res?.error?.message || "Failed to save credentials.");
					return;
				}

				// If a specific model was selected, test it from the live registry.
				if (selectedModelId) {
					var testResult = await testModel(selectedModelId);
					var modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
					if (!(testResult.ok || modelServiceUnavailable)) {
						// Model test failed — let user pick another.
						setPhase("selectModel");
						setModelTestError(testResult.error || "Model test failed. Try another model.");
						return;
					}
					await sendRpc("providers.save_model", { provider: providerName, model: selectedModelId });
					if (modelServiceUnavailable) {
						console.warn("models.test unavailable during onboarding, saved selected model without probe");
					}
					// Store chosen model in localStorage for the UI.
					localStorage.setItem("moltis-model", selectedModelId);
				}

				// Success — close the form and update state.
				setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
				setConfiguring(null);
				setPhase("form");
				setProviderModels([]);
				setSelectedModel(null);
				setModelTestError(null);
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
			}
		});
	}

	async function onOAuthAuthenticated(providerName) {
		var provModels = await loadModelsForProvider(providerName);

		setOauthProvider(null);
		setOauthInfo(null);

		if (provModels.length > 0) {
			setModelSelectProvider(providerName);
			setConfiguring(providerName);
			setProviderModels(provModels);
			setPhase("selectModel");
		} else {
			sendRpc("models.detect_supported", {
				background: true,
				reason: "provider_connected",
				provider: providerName,
			});
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
		setError(null);
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
		return html`<div class="text-sm text-[var(--muted)]">Loading LLMs\u2026</div>`;
	}

	var configuredProviders = providers.filter((p) => p.configured);

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Add LLMs</h2>
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
		<div class="flex flex-col gap-2 max-h-80 overflow-y-auto">
			${providers.map(
				(p) => html`<${OnboardingProviderRow}
				key=${p.name}
				provider=${p}
				configuring=${configuring}
				phase=${configuring === p.name ? phase : "form"}
				providerModels=${configuring === p.name ? providerModels : []}
				selectedModel=${configuring === p.name ? selectedModel : null}
				modelTestError=${configuring === p.name ? modelTestError : null}
				modelSearch=${configuring === p.name ? modelSearch : ""}
				setModelSearch=${setModelSearch}
				oauthProvider=${oauthProvider}
				oauthInfo=${oauthInfo}
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
				error=${configuring === p.name || oauthProvider === p.name || localProvider === p.name ? error : null}
				validationResult=${validationResults[p.name] || null}
				onStartConfigure=${onStartConfigure}
				onCancelConfigure=${closeAll}
				onSaveKey=${onSaveKey}
				onSelectModel=${onSelectModel}
				onCancelOAuth=${cancelOAuth}
				onConfigureLocalModel=${configureLocalModel}
				onCancelLocal=${cancelLocal}
			/>`,
			)}
		</div>
		${error && !configuring && !oauthProvider && !localProvider ? html`<${ErrorPanel} message=${error} />` : null}
		<div class="flex items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
			<button class="provider-btn" onClick=${onNext}>Continue</button>
			<button class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>
		</div>
	</div>`;
}

// ── Voice helpers ────────────────────────────────────────────

function decodeBase64Safe(input) {
	if (!input) return new Uint8Array();
	var normalized = String(input).replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/");
	while (normalized.length % 4) normalized += "=";
	var binary = "";
	try {
		binary = atob(normalized);
	} catch (_err) {
		throw new Error("Invalid base64 audio payload");
	}
	var bytes = new Uint8Array(binary.length);
	for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

// ── Voice provider row for onboarding ────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config form and test state
function OnboardingVoiceRow({
	provider,
	type,
	configuring,
	apiKey,
	setApiKey,
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
					${provider.available ? html`<span class="provider-item-badge configured">configured</span>` : html`<span class="provider-item-badge">needs key</span>`}
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
					provider.keyUrl
						? html`<div class="text-xs text-[var(--muted)]">
					Get your key at <a href=${provider.keyUrl} target="_blank" class="text-[var(--accent)] underline">${provider.keyUrlLabel || provider.keyUrl}</a>
				</div>`
						: null
				}
				${provider.hint ? html`<div class="text-xs text-[var(--accent)]">${provider.hint}</div>` : null}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<div class="flex items-center gap-2 mt-1">
					<button type="submit" class="provider-btn provider-btn-sm" disabled=${saving}>${saving ? "Saving\u2026" : "Save"}</button>
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
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);
	var [voiceTesting, setVoiceTesting] = useState(null); // { id, type, phase }
	var [voiceTestResults, setVoiceTestResults] = useState({});
	var [activeRecorder, setActiveRecorder] = useState(null);
	var [enableSaving, setEnableSaving] = useState(false);

	function fetchProviders() {
		return sendRpc("voice.providers.all", {}).then((res) => {
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
			sendRpc("voice.providers.all", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setAllProviders(res.payload || { tts: [], stt: [] });
					setLoading(false);
					return;
				}
				if (res?.error?.message === "WebSocket not connected" && attempts < 30) {
					attempts += 1;
					window.setTimeout(load, 200);
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
		if (firstStt) toggles.push(sendRpc("voice.provider.toggle", { provider: firstStt.id, enabled: true, type: "stt" }));
		if (firstTts) toggles.push(sendRpc("voice.provider.toggle", { provider: firstTts.id, enabled: true, type: "tts" }));
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
		setConfiguring(providerId);
		setApiKey("");
		setError(null);
	}

	function onCancelConfigure() {
		setConfiguring(null);
		setApiKey("");
		setError(null);
	}

	function onSaveKey(e) {
		e.preventDefault();
		if (!apiKey.trim()) {
			setError("API key is required.");
			return;
		}
		setError(null);
		setSaving(true);
		var providerId = configuring;
		sendRpc("voice.config.save_key", { provider: providerId, api_key: apiKey.trim() }).then(async (res) => {
			if (res?.ok) {
				// Auto-enable in onboarding: toggle on for each type this provider appears in.
				// IDs differ between TTS and STT (e.g. "elevenlabs" vs "elevenlabs-stt"),
				// so also check the counterpart ID.
				var COUNTERPART_IDS = {
					elevenlabs: "elevenlabs-stt",
					"elevenlabs-stt": "elevenlabs",
					"google-tts": "google",
					google: "google-tts",
				};
				var counterId = COUNTERPART_IDS[providerId];
				var toggles = [];
				var sttMatch =
					allProviders.stt.find((p) => p.id === providerId) ||
					(counterId && allProviders.stt.find((p) => p.id === counterId));
				var ttsMatch =
					allProviders.tts.find((p) => p.id === providerId) ||
					(counterId && allProviders.tts.find((p) => p.id === counterId));
				if (sttMatch) {
					toggles.push(sendRpc("voice.provider.toggle", { provider: sttMatch.id, enabled: true, type: "stt" }));
				}
				if (ttsMatch) {
					toggles.push(sendRpc("voice.provider.toggle", { provider: ttsMatch.id, enabled: true, type: "tts" }));
				}
				await Promise.all(toggles);
				setSaving(false);
				setConfiguring(null);
				setApiKey("");
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
			var toggleRes = await sendRpc("voice.provider.toggle", { provider: providerId, enabled: true, type });
			if (!toggleRes?.ok) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: toggleRes?.error?.message || "Failed to enable provider" },
				}));
				setVoiceTesting(null);
				return;
			}
			// ElevenLabs/Google share API keys — enable the counterpart too.
			// IDs differ between TTS and STT (e.g. "elevenlabs" vs "elevenlabs-stt"),
			// so map to the counterpart ID before looking it up.
			var counterType = type === "stt" ? "tts" : "stt";
			var counterList = type === "stt" ? allProviders.tts : allProviders.stt;
			var COUNTERPART_IDS = {
				elevenlabs: "elevenlabs-stt",
				"elevenlabs-stt": "elevenlabs",
				"google-tts": "google",
				google: "google-tts",
			};
			var counterId = COUNTERPART_IDS[providerId] || providerId;
			var counterProv = counterList.find((p) => p.id === counterId);
			if (counterProv?.available && !counterProv?.enabled) {
				await sendRpc("voice.provider.toggle", { provider: counterId, enabled: true, type: counterType });
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
				var res = await sendRpc("tts.convert", {
					text: ttsText,
					provider: providerId,
				});
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

					var audioBlob = new Blob(audioChunks, { type: "audio/webm" });

					try {
						var resp = await fetch(
							`/api/sessions/${encodeURIComponent(S.activeSessionKey)}/upload?transcribe=true&provider=${encodeURIComponent(providerId)}`,
							{
								method: "POST",
								headers: { "Content-Type": audioBlob.type || "audio/webm" },
								body: audioBlob,
							},
						);
						console.log("[STT] upload response: status=%d ok=%s", resp.status, resp.ok);
						if (resp.ok) {
							var sttRes = await resp.json();

							if (sttRes.ok && sttRes.transcription?.text) {
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: { text: sttRes.transcription.text, error: null },
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
		<div class="flex items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
			<button class="provider-btn" onClick=${onNext}>Continue</button>
			<button class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>
		</div>
	</div>`;
}

// ── Channel step ────────────────────────────────────────────

function ChannelStep({ onNext, onBack }) {
	var [accountId, setAccountId] = useState("");
	var [token, setToken] = useState("");
	var [dmPolicy, setDmPolicy] = useState("allowlist");
	var [allowlist, setAllowlist] = useState("");
	var [saving, setSaving] = useState(false);
	var [connected, setConnected] = useState(false);
	var [connectedName, setConnectedName] = useState("");
	var [error, setError] = useState(null);

	function onSubmit(e) {
		e.preventDefault();
		if (!accountId.trim()) {
			setError("Bot username is required.");
			return;
		}
		if (!token.trim()) {
			setError("Bot token is required.");
			return;
		}
		setError(null);
		setSaving(true);
		var allowlistEntries = allowlist
			.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		sendRpc("channels.add", {
			type: "telegram",
			account_id: accountId.trim(),
			config: {
				token: token.trim(),
				dm_policy: dmPolicy,
				mention_mode: "mention",
				allowlist: allowlistEntries,
			},
		}).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setConnected(true);
				setConnectedName(accountId.trim());
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Connect Telegram</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Connect a Telegram bot so you can chat from your phone. You can set this up later in Channels.</p>
		${
			connected
				? html`<div class="rounded-md border border-[var(--ok)] bg-[var(--surface)] p-4 flex gap-3 items-center">
				<span class="icon icon-lg icon-check-circle shrink-0" style="color:var(--ok)"></span>
				<div>
					<div class="text-sm font-medium text-[var(--text-strong)]">Bot connected</div>
					<div class="text-xs text-[var(--muted)] mt-0.5">@${connectedName} is now linked to your agent.</div>
				</div>
			</div>`
				: html`<form onSubmit=${onSubmit} class="flex flex-col gap-3 max-h-80 overflow-y-auto -mr-4 pr-4">
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
				${error && html`<${ErrorPanel} message=${error} />`}
			</form>`
		}
		<div class="flex items-center gap-3 mt-1">
			<button type="button" class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
			${
				connected
					? html`<button type="button" class="provider-btn" onClick=${onNext}>Continue</button>`
					: html`<button type="button" class="provider-btn" disabled=${saving} onClick=${onSubmit}>${saving ? "Connecting\u2026" : "Connect Bot"}</button>`
			}
			<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>
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
				sendRpc("channels.status", {}).catch(() => null),
				fetch("/api/tailscale/status")
					.then((r) => (r.ok ? r.json() : null))
					.catch(() => null),
				voiceEnabled ? sendRpc("voice.providers.all", {}).catch(() => null) : Promise.resolve(null),
				fetch("/api/bootstrap")
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
		return html`<div class="text-sm text-[var(--muted)]">Loading summary\u2026</div>`;
	}

	var activeModel = localStorage.getItem("moltis-model");
	var configuredProviders = data.providers.filter((p) => p.configured);

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Setup Summary</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Overview of your configuration. You can change any of these later in Settings.</p>

		<div class="flex flex-col gap-2 max-h-80 overflow-y-auto -mr-4 pr-4">
			<!-- Identity -->
			<${SummaryRow}
				icon=${data.identity?.user_name && data.identity?.name ? html`<${CheckIcon} />` : html`<${WarnIcon} />`}
				label="Identity">
				${
					data.identity?.user_name && data.identity?.name
						? html`You: <span class="font-medium text-[var(--text)]">${data.identity.user_name}</span>
						Agent: <span class="font-medium text-[var(--text)]">${data.identity.emoji || ""} ${data.identity.name}</span>`
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
					icon=${data.tailscale?.connected ? html`<${CheckIcon} />` : data.tailscale?.installed ? html`<${WarnIcon} />` : html`<${InfoIcon} />`}
					label="Tailscale">
					${
						data.tailscale?.connected
							? html`Connected`
							: data.tailscale?.installed
								? html`Installed but not connected`
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

		<div class="flex items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
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
			<div class="text-sm text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	// Build step list dynamically based on auth + voice availability
	var allLabels = voiceAvailable ? VOICE_STEP_LABELS : BASE_STEP_LABELS;
	var steps = authNeeded ? allLabels : allLabels.slice(1);
	var stepIndex = authNeeded ? step : step - 1;
	var lastStep = voiceAvailable ? 5 : 4;

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

	// Determine which component to show for each step
	var channelStep = voiceAvailable ? 4 : 3;
	var voiceStep = voiceAvailable ? 3 : -1;
	var summaryStep = voiceAvailable ? 5 : 4;

	return html`<div class="onboarding-card">
		<${StepIndicator} steps=${steps} current=${stepIndex} />
		<div class="mt-6">
			${step === 0 && html`<${AuthStep} onNext=${goNext} skippable=${authSkippable} />`}
			${step === 1 && html`<${IdentityStep} onNext=${goNext} onBack=${authNeeded ? goBack : null} />`}
			${step === 2 && html`<${ProviderStep} onNext=${goNext} onBack=${goBack} />`}
			${step === voiceStep && html`<${VoiceStep} onNext=${goNext} onBack=${goBack} />`}
			${step === channelStep && html`<${ChannelStep} onNext=${goNext} onBack=${goBack} />`}
			${step === summaryStep && html`<${SummaryStep} onBack=${goBack} onFinish=${goFinish} />`}
		</div>
	</div>`;
}

// ── Page registration ───────────────────────────────────────

var containerRef = null;

export function mountOnboarding(container) {
	containerRef = container;
	container.style.cssText = "display:flex;align-items:center;justify-content:center;min-height:100vh;padding:1rem;";
	render(html`<${OnboardingPage} />`, container);
}

export function unmountOnboarding() {
	if (containerRef) render(null, containerRef);
	containerRef = null;
}
