// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Identity → Provider → Channel
// No new Rust code — all existing RPC methods and REST endpoints.

import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { startProviderOAuth } from "./provider-oauth.js";
import { validateProviderConnection } from "./provider-validation.js";

// ── Step indicator ──────────────────────────────────────────

var STEP_LABELS = ["Security", "Identity", "Provider", "Channel"];

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

// ── Auth step ───────────────────────────────────────────────

function AuthStep({ onNext, skippable }) {
	var [password, setPassword] = useState("");
	var [confirm, setConfirm] = useState("");
	var [setupCode, setSetupCode] = useState("");
	var [codeRequired, setCodeRequired] = useState(false);
	var [localhostOnly, setLocalhostOnly] = useState(false);
	var [error, setError] = useState(null);
	var [saving, setSaving] = useState(false);
	var [loading, setLoading] = useState(true);

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => r.json())
			.then((data) => {
				if (data.setup_code_required) setCodeRequired(true);
				if (data.localhost_only) setLocalhostOnly(true);
				setLoading(false);
			})
			.catch(() => setLoading(false));
	}, []);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: auth form handles password+code validation
	function onSubmit(e) {
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

	if (loading) {
		return html`<div class="text-sm text-[var(--muted)]">Checking authentication\u2026</div>`;
	}

	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Secure your instance</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">
			${localhostOnly ? "Set a password to secure your instance, or skip for now." : "Set a password to secure your instance."}
		</p>
		<form onSubmit=${onSubmit} class="flex flex-col gap-3">
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
			${
				codeRequired &&
				html`<div>
				<label class="text-xs text-[var(--muted)] mb-1 block">Setup code</label>
				<input type="text" class="provider-key-input w-full" inputmode="numeric" pattern="[0-9]*"
					value=${setupCode} onInput=${(e) => setSetupCode(e.target.value)}
					placeholder="6-digit code from terminal" />
				<div class="text-xs text-[var(--muted)] mt-1">Hint: find this code in the moltis process log (stdout).</div>
			</div>`
			}
			${error && html`<${ErrorPanel} message=${error} />`}
			<div class="flex items-center gap-3 mt-1">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Setting up\u2026" : localhostOnly && !password ? "Skip" : "Set password"}
				</button>
				${skippable && html`<button type="button" class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline" onClick=${onNext}>Skip for now</button>`}
			</div>
		</form>
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

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider step manages multiple auth flows inline
function ProviderStep({ onNext, onBack }) {
	var [providers, setProviders] = useState([]);
	var [loading, setLoading] = useState(true);
	var [selected, setSelected] = useState(null);
	var [phase, setPhase] = useState("list"); // list | form | oauth | local | success

	// API key form state
	var [apiKey, setApiKey] = useState("");
	var [endpoint, setEndpoint] = useState("");
	var [model, setModel] = useState("");
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);

	// OAuth state
	var [oauthInfo, setOauthInfo] = useState(null);

	// Local state
	var [sysInfo, setSysInfo] = useState(null);
	var [localModels, setLocalModels] = useState([]);
	var [selectedBackend, setSelectedBackend] = useState(null);

	useEffect(() => {
		var cancelled = false;
		var attempts = 0;

		function loadProviders() {
			if (cancelled) return;
			sendRpc("providers.available", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					var list = res.payload || [];
					// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider ordering keeps local options first.
					list.sort((a, b) => {
						var aIsLocal = a.authType === "local" || a.name === "ollama";
						var bIsLocal = b.authType === "local" || b.name === "ollama";
						if (aIsLocal && !bIsLocal) return -1;
						if (!aIsLocal && bIsLocal) return 1;
						return a.displayName.localeCompare(b.displayName);
					});
					setProviders(list);
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

	function selectProvider(p) {
		setSelected(p);
		setError(null);
		setApiKey("");
		setEndpoint("");
		setModel("");
		if (p.authType === "api-key") setPhase("form");
		else if (p.authType === "oauth") startOAuth(p);
		else if (p.authType === "local") startLocal(p);
	}

	function backToList() {
		setPhase("list");
		setSelected(null);
		setError(null);
	}

	// ── API key form ─────────────────────────────────────────

	function onSaveKey(e) {
		e.preventDefault();
		if (!apiKey.trim() && selected.name !== "ollama") {
			setError("API key is required.");
			return;
		}
		if (BYOM_PROVIDERS.includes(selected.name) && !model.trim()) {
			setError("Model ID is required.");
			return;
		}
		setError(null);
		setSaving(true);
		var payload = { provider: selected.name, apiKey: apiKey.trim() || "ollama" };
		if (endpoint.trim()) payload.baseUrl = endpoint.trim();
		if (model.trim()) payload.model = model.trim();
		sendRpc("providers.save_key", payload)
			.then(async (res) => {
				if (res?.ok) {
					var validation = await validateProviderConnection(selected.name);
					setSaving(false);
					if (!validation.ok) {
						setError(validation.message || "Credentials were saved, but provider validation failed.");
						return;
					}
					setPhase("success");
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
		setPhase("oauth");
		setOauthInfo({ status: "starting" });
		startProviderOAuth(p.name).then((result) => {
			if (result.status === "already") {
				sendRpc("models.detect_supported", {
					background: true,
					reason: "provider_connected",
					provider: p.name,
				});
				setPhase("success");
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
				setOauthInfo(null);
				setPhase("list");
			}
		});
	}

	function pollOAuth(p) {
		var attempts = 0;
		var timer = setInterval(() => {
			attempts++;
			if (attempts > 60) {
				clearInterval(timer);
				setError("OAuth timed out. Please try again.");
				setPhase("list");
				return;
			}
			sendRpc("providers.oauth.status", { provider: p.name }).then((res) => {
				if (res?.ok && res.payload?.authenticated) {
					clearInterval(timer);
					sendRpc("models.detect_supported", {
						background: true,
						reason: "provider_connected",
						provider: p.name,
					});
					setPhase("success");
				}
			});
		}, 2000);
	}

	// ── Local model flow ─────────────────────────────────────

	function startLocal(_p) {
		setPhase("local");
		sendRpc("providers.local.system_info", {}).then((sysRes) => {
			if (!sysRes?.ok) {
				setError(sysRes?.error?.message || "Failed to get system info");
				setPhase("list");
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
		setSaving(true);
		setError(null);
		sendRpc("providers.local.configure", { modelId: mdl.id, backend: selectedBackend }).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setPhase("success");
			} else {
				setError(res?.error?.message || "Failed to configure model");
			}
		});
	}

	// ── Render ────────────────────────────────────────────────

	if (loading) {
		return html`<div class="text-sm text-[var(--muted)]">Loading providers\u2026</div>`;
	}

	// Success screen
	if (phase === "success") {
		return html`<div class="flex flex-col gap-4 items-center text-center py-4">
			<div class="text-2xl">\u2705</div>
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${selected?.displayName || "Provider"} configured!</h2>
			<button class="provider-btn" onClick=${onNext}>Continue</button>
		</div>`;
	}

	// API key form
	if (phase === "form" && selected) {
		var supportsEndpoint = OPENAI_COMPATIBLE.includes(selected.name);
		var needsModel = BYOM_PROVIDERS.includes(selected.name);
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${selected.displayName}</h2>
			<form onSubmit=${onSaveKey} class="flex flex-col gap-3">
				<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">API Key</label>
					<input type="password" class="provider-key-input w-full"
						value=${apiKey} onInput=${(e) => setApiKey(e.target.value)}
						placeholder=${selected.name === "ollama" ? "(optional for Ollama)" : "sk-..."} autofocus />
				</div>
				${
					supportsEndpoint &&
					html`<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">Endpoint (optional)</label>
					<input type="text" class="provider-key-input w-full"
						value=${endpoint} onInput=${(e) => setEndpoint(e.target.value)}
						placeholder=${selected.defaultBaseUrl || "https://api.example.com/v1"} />
					<div class="text-xs text-[var(--muted)] mt-1">Leave empty to use the default endpoint.</div>
				</div>`
				}
				${
					needsModel &&
					html`<div>
					<label class="text-xs text-[var(--muted)] mb-1 block">Model ID</label>
					<input type="text" class="provider-key-input w-full"
						value=${model} onInput=${(e) => setModel(e.target.value)}
						placeholder=${selected.name === "ollama" ? "llama3" : "model-id"} />
				</div>`
				}
				${error && html`<${ErrorPanel} message=${error} />`}
				<div class="flex items-center gap-3 mt-1">
					<button type="button" class="provider-btn provider-btn-secondary" onClick=${backToList}>Back</button>
					<button type="submit" class="provider-btn" disabled=${saving}>${saving ? "Saving\u2026" : "Save"}</button>
				</div>
			</form>
		</div>`;
	}

	// OAuth waiting
	if (phase === "oauth") {
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${selected?.displayName}</h2>
			${
				oauthInfo?.status === "device"
					? html`<div class="text-sm text-[var(--text)]">
					Open <a href=${oauthInfo.uri} target="_blank" class="text-[var(--accent)] underline">${oauthInfo.uri}</a> and enter code:<strong class="font-mono ml-1">${oauthInfo.code}</strong>
				</div>`
					: html`<div class="text-sm text-[var(--muted)]">Waiting for authentication\u2026</div>`
			}
			${error && html`<${ErrorPanel} message=${error} />`}
			<button class="provider-btn provider-btn-secondary" onClick=${backToList}>Cancel</button>
		</div>`;
	}

	// Local model selection
	if (phase === "local" && sysInfo) {
		var backends = sysInfo.availableBackends || [];
		var filteredModels = localModels.filter((m) => m.backend === selectedBackend);
		return html`<div class="flex flex-col gap-4">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Local Models</h2>
			<div class="flex gap-3 text-xs text-[var(--muted)]">
				<span>RAM: ${sysInfo.totalRamGb}GB</span>
				<span>Tier: ${sysInfo.memoryTier}</span>
				${sysInfo.hasGpu && html`<span class="text-[var(--ok)]">GPU available</span>`}
			</div>
			${
				sysInfo.isAppleSilicon &&
				backends.length > 0 &&
				html`<div class="flex flex-col gap-2">
				<div class="text-xs font-medium text-[var(--text-strong)]">Backend</div>
				<div class="flex flex-col gap-2">
					${backends.map(
						(b) => html`<div key=${b.id}
						class="backend-card ${b.id === selectedBackend ? "selected" : ""} ${b.available ? "" : "disabled"}"
						onClick=${() => {
							if (b.available) setSelectedBackend(b.id);
						}}>
						<div class="flex items-center justify-between">
							<span class="text-sm font-medium text-[var(--text)]">${b.name}</span>
							<div class="flex gap-2">
								${b.id === sysInfo.recommendedBackend && b.available && html`<span class="recommended-badge">Recommended</span>`}
								${!b.available && html`<span class="tier-badge">Not installed</span>`}
							</div>
						</div>
						<div class="text-xs text-[var(--muted)] mt-1">${b.description}</div>
					</div>`,
					)}
				</div>
			</div>`
			}
			<div class="text-xs font-medium text-[var(--text-strong)]">Select a model</div>
			<div class="flex flex-col gap-2 max-h-48 overflow-y-auto">
				${
					filteredModels.length === 0
						? html`<div class="text-xs text-[var(--muted)] py-4 text-center">No models available for ${selectedBackend}</div>`
						: filteredModels.map(
								(mdl) => html`<div key=${mdl.id} class="model-card" onClick=${() => configureLocalModel(mdl)}>
						<div class="flex items-center justify-between">
							<span class="text-sm font-medium text-[var(--text)]">${mdl.displayName}</span>
							<div class="flex gap-2">
								<span class="tier-badge">${mdl.minRamGb}GB</span>
								${mdl.suggested && html`<span class="recommended-badge">Recommended</span>`}
							</div>
						</div>
						<div class="text-xs text-[var(--muted)] mt-1">Context: ${(mdl.contextWindow / 1000).toFixed(0)}k tokens</div>
					</div>`,
							)
				}
			</div>
			${error && html`<${ErrorPanel} message=${error} />`}
			${saving && html`<div class="text-xs text-[var(--muted)]">Configuring\u2026</div>`}
			<div class="flex items-center gap-3 mt-1">
				<button class="provider-btn provider-btn-secondary" onClick=${backToList}>Back</button>
			</div>
		</div>`;
	}

	// Provider list
	var configuredProviders = providers.filter((p) => p.configured);
	return html`<div class="flex flex-col gap-4">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Add a provider</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed">Pick an LLM provider to power your agent. You can add more later in Settings.</p>
		${
			configuredProviders.length > 0 &&
			html`<div class="rounded-md border border-[var(--ok)]/30 bg-[var(--ok)]/8 p-3 flex flex-col gap-2">
			<div class="text-xs text-[var(--muted)]">Detected providers</div>
			<div class="flex flex-wrap gap-2">
				${configuredProviders.map((p) => html`<span key=${p.name} class="provider-item-badge configured">${p.displayName}</span>`)}
			</div>
			<button class="provider-btn self-start" onClick=${onNext}>Continue with detected providers</button>
		</div>`
		}
		<div class="text-xs text-[var(--muted)]">Choose a provider to connect</div>
		<div class="flex flex-col gap-2 max-h-64 overflow-y-auto">
			${providers.map(
				(p) => html`<div key=${p.name} class="provider-item" onClick=${() => selectProvider(p)}>
				<span class="provider-item-name">${p.displayName}</span>
				<div class="flex gap-2">
					${p.configured && html`<span class="provider-item-badge configured">configured</span>`}
					<span class="provider-item-badge ${p.authType}">
						${p.authType === "oauth" ? "OAuth" : p.authType === "local" ? "Local" : "API Key"}
					</span>
				</div>
			</div>`,
			)}
		</div>
		${error && html`<${ErrorPanel} message=${error} />`}
		<div class="flex items-center gap-3 mt-1">
			<button class="provider-btn provider-btn-secondary" onClick=${onBack}>Back</button>
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

// ── Finish screen ───────────────────────────────────────────

function FinishStep() {
	return html`<div class="flex flex-col gap-4 items-center text-center py-6">
		<div class="text-4xl">\u{1f389}</div>
		<h2 class="text-xl font-medium text-[var(--text-strong)]">You\u2019re all set!</h2>
		<p class="text-sm text-[var(--muted)]">Your agent is ready to go. Start chatting or customise further in Settings.</p>
		<button class="provider-btn" onClick=${() => window.location.assign(preferredChatPath())}>Get Started</button>
	</div>`;
}

// ── Main page component ─────────────────────────────────────

function OnboardingPage() {
	var [step, setStep] = useState(-1); // -1 = checking
	var [authNeeded, setAuthNeeded] = useState(false);
	var [authSkippable, setAuthSkippable] = useState(false);
	var [finished, setFinished] = useState(false);
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
					// Show auth step: either first-run setup or remote with auth disabled
					setAuthNeeded(true);
					setAuthSkippable(!auth.setup_required);
					setStep(0);
				} else {
					setAuthNeeded(false);
					setStep(1); // skip auth, go straight to identity
				}
			})
			.catch(() => {
				setAuthNeeded(false);
				setStep(1);
			});
	}, []);

	if (step === -1) {
		return html`<div class="onboarding-card">
			<div class="text-sm text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	if (finished) {
		return html`<div class="onboarding-card"><${FinishStep} /></div>`;
	}

	// Build step list (auth may be removed)
	var steps = authNeeded ? STEP_LABELS : STEP_LABELS.slice(1);
	var stepIndex = authNeeded ? step : step - 1;

	function goNext() {
		if (step === 3) {
			setFinished(true);
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

	return html`<div class="onboarding-card">
		<${StepIndicator} steps=${steps} current=${stepIndex} />
		<div class="mt-6">
			${step === 0 && html`<${AuthStep} onNext=${goNext} skippable=${authSkippable} />`}
			${step === 1 && html`<${IdentityStep} onNext=${goNext} onBack=${authNeeded ? goBack : null} />`}
			${step === 2 && html`<${ProviderStep} onNext=${goNext} onBack=${goBack} />`}
			${step === 3 && html`<${ChannelStep} onNext=${goNext} onBack=${goBack} />`}
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
