// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Identity → Provider → Voice (conditional) → Channel
// No new Rust code — all existing RPC methods and REST endpoints.

import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { get as getGon, refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { startProviderOAuth } from "./provider-oauth.js";
import { validateProviderConnection } from "./provider-validation.js";
import { fetchPhrase } from "./tts-phrases.js";

// ── Step indicator ──────────────────────────────────────────

var BASE_STEP_LABELS = ["Security", "Identity", "Provider", "Channel"];
var VOICE_STEP_LABELS = ["Security", "Identity", "Provider", "Voice", "Channel"];

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
			${state === "completed" ? html`<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><path d="M5 13l4 4L19 7" /></svg>` : index + 1}
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
				setLoading(false);
			})
			.catch(() => setLoading(false));
	}, []);

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
					name: passkeyName.trim() || "Passkey",
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
					onNext();
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
	var [userName, setUserName] = useState("");
	var [name, setName] = useState("Moltis");
	var [emoji, setEmoji] = useState("\u{1f916}");
	var [creature, setCreature] = useState("");
	var [vibe, setVibe] = useState("");
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
var BYOM_PROVIDERS = ["ollama", "openrouter", "venice"];

// ── Provider row for multi-provider onboarding ──────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config forms for api-key, oauth, and local flows
function OnboardingProviderRow({
	provider,
	configuring,
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
	onCancelOAuth,
	onConfigureLocalModel,
	onCancelLocal,
}) {
	var isApiKeyForm = configuring === provider.name;
	var isOAuth = oauthProvider === provider.name;
	var isLocal = localProvider === provider.name;
	var isExpanded = isApiKeyForm || isOAuth || isLocal;
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

	return html`<div ref=${rowRef} class="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3">
		<div class="flex items-center gap-3">
			<div class="flex-1 min-w-0 flex flex-col gap-0.5">
				<div class="flex items-center gap-2 flex-wrap">
					<span class="text-sm font-medium text-[var(--text-strong)]">${provider.displayName}</span>
					${provider.configured ? html`<span class="provider-item-badge configured">configured</span>` : null}
					${
						validationResult?.ok === true
							? html`<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--ok)" stroke-width="2.5" class="inline-block">
								<path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75 11.25 15 15 9.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
							</svg>`
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
							onClick=${() => onStartConfigure(provider.name)}>${provider.configured ? "Reconfigure" : "Configure"}</button>`
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
						placeholder=${provider.name === "ollama" ? "(optional for Ollama)" : "sk-..."} />
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
							placeholder=${provider.name === "ollama" ? "llama3" : "model-id"} />
					</div>`
						: null
				}
				${error ? html`<${ErrorPanel} message=${error} />` : null}
				<div class="flex items-center gap-2 mt-1">
					<button type="submit" class="provider-btn provider-btn-sm" disabled=${saving}>${saving ? "Saving\u2026" : "Save"}</button>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onCancelConfigure}>Cancel</button>
				</div>
			</form>`
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
		var aIsLocal = a.authType === "local" || a.name === "ollama";
		var bIsLocal = b.authType === "local" || b.name === "ollama";
		if (aIsLocal && !bIsLocal) return -1;
		if (!aIsLocal && bIsLocal) return 1;
		return a.displayName.localeCompare(b.displayName);
	});
	return list;
}

function ProviderStep({ onNext, onBack }) {
	var [providers, setProviders] = useState([]);
	var [loading, setLoading] = useState(true);
	var [error, setError] = useState(null);

	// Which provider has an open inline form (by name), or null
	var [configuring, setConfiguring] = useState(null);
	var [oauthProvider, setOauthProvider] = useState(null);
	var [localProvider, setLocalProvider] = useState(null);

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

	function onStartConfigure(name) {
		closeAll();
		var p = providers.find((pr) => pr.name === name);
		if (!p) return;
		if (p.authType === "api-key") {
			setEndpoint(p.baseUrl || "");
			setModel(p.model || "");
			setConfiguring(name);
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
		if (!apiKey.trim() && p.name !== "ollama") {
			setError("API key is required.");
			return;
		}
		if (BYOM_PROVIDERS.includes(p.name) && !model.trim()) {
			setError("Model ID is required.");
			return;
		}
		setError(null);
		setSaving(true);
		var payload = { provider: p.name, apiKey: apiKey.trim() || "ollama" };
		if (endpoint.trim()) payload.baseUrl = endpoint.trim();
		if (model.trim()) payload.model = model.trim();
		sendRpc("providers.save_key", payload)
			.then(async (res) => {
				if (res?.ok) {
					var validation = await validateProviderConnection(p.name);
					setSaving(false);
					setValidationResults((prev) => ({ ...prev, [p.name]: validation }));
					if (!validation.ok) {
						setError(validation.message || "Credentials were saved, but provider validation failed.");
					}
					setConfiguring(null);
					setApiKey("");
					setEndpoint("");
					setModel("");
					setError(null);
					refreshProviders();
				} else {
					setSaving(false);
					setError(res?.error?.message || "Failed to save");
				}
			})
			.catch((err) => {
				setSaving(false);
				setError(err?.message || "Failed to save");
			});
	}

	// ── OAuth flow ───────────────────────────────────────────

	function startOAuth(p) {
		setOauthProvider(p.name);
		setOauthInfo({ status: "starting" });
		startProviderOAuth(p.name).then((result) => {
			if (result.status === "already") {
				sendRpc("models.detect_supported", {
					background: true,
					reason: "provider_connected",
					provider: p.name,
				});
				setOauthProvider(null);
				setOauthInfo(null);
				setValidationResults((prev) => ({ ...prev, [p.name]: { ok: true, message: null } }));
				refreshProviders();
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
					sendRpc("models.detect_supported", {
						background: true,
						reason: "provider_connected",
						provider: p.name,
					});
					setOauthProvider(null);
					setOauthInfo(null);
					setValidationResults((prev) => ({ ...prev, [p.name]: { ok: true, message: null } }));
					refreshProviders();
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
		return html`<div class="text-sm text-[var(--muted)]">Loading providers\u2026</div>`;
	}

	var configuredProviders = providers.filter((p) => p.configured);

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Add providers</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Configure one or more LLM providers to power your agent. You can add more later in Settings.</p>
		${
			configuredProviders.length > 0
				? html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2">
				<div class="text-xs text-[var(--muted)]">Detected providers</div>
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

function encodeBase64Safe(bytes) {
	var chunk = 0x8000;
	var str = "";
	for (var i = 0; i < bytes.length; i += chunk) {
		str += String.fromCharCode(...bytes.subarray(i, i + chunk));
	}
	return btoa(str);
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
				<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="14" height="14">
					<path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75 11.25 15 15 9.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
				</svg>
				<span>Audio played successfully</span>
			</div>`
				: null
		}
		${voiceTestResult?.error ? html`<span class="text-xs text-[var(--error)] mt-1 block">${voiceTestResult.error}</span>` : null}
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
					var blob = new Blob([bytes], { type: res.payload.content_type || "audio/mpeg" });
					var url = URL.createObjectURL(blob);
					var audio = new Audio(url);
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch(() => undefined);
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
					var buffer = await audioBlob.arrayBuffer();
					var base64 = encodeBase64Safe(new Uint8Array(buffer));

					var sttRes = await sendRpc("stt.transcribe", {
						audio: base64,
						format: "webm",
						provider: providerId,
					});

					if (sttRes?.ok && sttRes.payload?.text) {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: { text: sttRes.payload.text, error: null },
						}));
					} else {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: { text: null, error: sttRes?.error?.message || "STT test failed" },
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
				onNext();
			} else {
				setError((res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.");
			}
		});
	}

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Connect Telegram</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Connect a Telegram bot so you can chat from your phone. You can set this up later in Channels.</p>
		<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
			<span class="font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
			<span>1. Open <a href="https://t.me/BotFather" target="_blank" class="text-[var(--accent)] underline">@BotFather</a> in Telegram</span>
			<span>2. Send /newbot and follow the prompts</span>
			<span>3. Copy the bot token and paste it below</span>
		</div>
		<form onSubmit=${onSubmit} class="flex flex-col gap-3">
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Bot username</label>
				<input type="text" class="provider-key-input w-full"
					value=${accountId} onInput=${(e) => setAccountId(e.target.value)}
					placeholder="e.g. my_assistant_bot" autofocus />
			</div>
			<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Bot token (from @BotFather)</label>
				<input type="password" class="provider-key-input w-full"
					value=${token} onInput=${(e) => setToken(e.target.value)}
					placeholder="123456:ABC-DEF..." />
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
			<div class="flex items-center gap-3 mt-1">
				<button type="button" class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
				<button type="submit" class="provider-btn" disabled=${saving}>${saving ? "Connecting\u2026" : "Connect Bot"}</button>
				<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>
			</div>
		</form>
	</div>`;
}

// ── Main page component ─────────────────────────────────────

function OnboardingPage() {
	var [step, setStep] = useState(-1); // -1 = checking
	var [authNeeded, setAuthNeeded] = useState(false);
	var [authSkippable, setAuthSkippable] = useState(false);
	var [voiceAvailable, setVoiceAvailable] = useState(false);
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
					setStep(1);
				}
			})
			.catch(() => {
				setAuthNeeded(false);
				setStep(1);
			});
	}, []);

	// Probe voice feature availability
	useEffect(() => {
		var cancelled = false;
		var attempts = 0;

		function probe() {
			if (cancelled) return;
			sendRpc("voice.providers.all", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setVoiceAvailable(true);
					return;
				}
				if (res?.error?.message === "WebSocket not connected" && attempts < 30) {
					attempts += 1;
					window.setTimeout(probe, 200);
					return;
				}
				// Voice not compiled or other error — leave false
			});
		}

		probe();
		return () => {
			cancelled = true;
		};
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
	var lastStep = voiceAvailable ? 4 : 3;

	function goNext() {
		if (step === lastStep) {
			window.location.assign(preferredChatPath());
		} else {
			setStep(step + 1);
		}
	}

	function goBack() {
		if (authNeeded) {
			setStep(Math.max(0, step - 1));
		} else {
			setStep(Math.max(1, step - 1));
		}
	}

	// Determine which component to show for steps 3 and 4
	var channelStep = voiceAvailable ? 4 : 3;
	var voiceStep = voiceAvailable ? 3 : -1;

	return html`<div class="onboarding-card">
		<${StepIndicator} steps=${steps} current=${stepIndex} />
		<div class="mt-6">
			${step === 0 && html`<${AuthStep} onNext=${goNext} skippable=${authSkippable} />`}
			${step === 1 && html`<${IdentityStep} onNext=${goNext} onBack=${authNeeded ? goBack : null} />`}
			${step === 2 && html`<${ProviderStep} onNext=${goNext} onBack=${goBack} />`}
			${step === voiceStep && html`<${VoiceStep} onNext=${goNext} onBack=${goBack} />`}
			${step === channelStep && html`<${ChannelStep} onNext=${goNext} onBack=${goBack} />`}
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
