// ── Settings page (Preact + HTM + Signals) ───────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { onEvent } from "./events.js";
import * as gon from "./gon.js";
import { refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { setLocale, t } from "./i18n.js";
import { updateIdentity, validateIdentityFields } from "./identity-utils.js";
// Moved page init/teardown imports
import { initChannels, teardownChannels } from "./page-channels.js";
import { initCrons, teardownCrons } from "./page-crons.js";
import { initHooks, teardownHooks } from "./page-hooks.js";
import { initImages, teardownImages } from "./page-images.js";
import { initLogs, teardownLogs } from "./page-logs.js";
import { initMcp, teardownMcp } from "./page-mcp.js";
import { initMonitoring, teardownMonitoring } from "./page-metrics.js";
import { initProviders, teardownProviders } from "./page-providers.js";
import { initSkills, teardownSkills } from "./page-skills.js";
import { detectPasskeyName } from "./passkey-detect.js";
import * as push from "./push.js";
import { isStandalone } from "./pwa.js";
import { navigate, registerPrefix } from "./router.js";
import { routes, settingsPath } from "./routes.js";
import { connected } from "./signals.js";
import * as S from "./state.js";
import { fetchPhrase } from "./tts-phrases.js";
import { Modal } from "./ui.js";
import {
	decodeBase64Safe,
	fetchVoiceProviders,
	saveVoiceKey,
	testTts,
	toggleVoiceProvider,
	transcribeAudio,
} from "./voice-utils.js";

var identity = signal(null);
var loading = signal(true);
var activeSection = signal("identity");
var mounted = false;
var containerRef = null;

function rerender() {
	if (containerRef) render(html`<${SettingsPage} />`, containerRef);
}

function isSafariBrowser() {
	if (typeof navigator === "undefined") return false;
	var ua = navigator.userAgent || "";
	var vendor = navigator.vendor || "";
	if (!ua.includes("Safari/")) return false;
	if (/(Chrome|CriOS|Chromium|Edg|OPR|FxiOS|Firefox|SamsungBrowser)/.test(ua)) return false;
	return /Apple/i.test(vendor) || ua.includes("Safari/");
}

function fetchIdentity() {
	if (!mounted) return;
	sendRpc("agent.identity.get", {}).then((res) => {
		if (res?.ok) {
			identity.value = res.payload;
			loading.value = false;
			rerender();
		} else if (mounted && !S.connected) {
			setTimeout(fetchIdentity, 500);
		} else {
			loading.value = false;
			rerender();
		}
	});
}

// ── Sidebar navigation items ─────────────────────────────────

var sections = [
	{ group: () => t("settings:nav.general") },
	{
		id: "identity",
		label: () => t("settings:nav.identity"),
		icon: html`<span class="icon icon-person"></span>`,
	},
	{
		id: "environment",
		label: () => t("settings:nav.environment"),
		icon: html`<span class="icon icon-terminal"></span>`,
	},
	{
		id: "memory",
		label: () => t("settings:nav.memory"),
		icon: html`<span class="icon icon-database"></span>`,
	},
	{
		id: "notifications",
		label: () => t("settings:nav.notifications"),
		icon: html`<span class="icon icon-bell"></span>`,
	},
	{
		id: "crons",
		label: () => t("settings:nav.crons"),
		icon: html`<span class="icon icon-cron"></span>`,
		page: true,
	},
	{ group: () => t("settings:nav.security") },
	{
		id: "security",
		label: () => t("settings:nav.securityItem"),
		icon: html`<span class="icon icon-lock"></span>`,
	},
	{
		id: "tailscale",
		label: () => t("settings:nav.tailscale"),
		icon: html`<span class="icon icon-globe"></span>`,
	},
	{ group: () => t("settings:nav.integrations") },
	{
		id: "channels",
		label: () => t("settings:nav.channels"),
		icon: html`<span class="icon icon-channels"></span>`,
		page: true,
	},
	{
		id: "hooks",
		label: () => t("settings:nav.hooks"),
		icon: html`<span class="icon icon-wrench"></span>`,
		page: true,
	},
	{
		id: "providers",
		label: () => t("settings:nav.llms"),
		icon: html`<span class="icon icon-layers"></span>`,
		page: true,
	},
	{
		id: "mcp",
		label: () => t("settings:nav.mcp"),
		icon: html`<span class="icon icon-link"></span>`,
		page: true,
	},
	{
		id: "skills",
		label: () => t("settings:nav.skills"),
		icon: html`<span class="icon icon-sparkles"></span>`,
		page: true,
	},
	{
		id: "voice",
		label: () => t("settings:nav.voice"),
		icon: html`<span class="icon icon-microphone"></span>`,
	},
	{ group: () => t("settings:nav.systems") },
	{
		id: "sandboxes",
		label: () => t("settings:nav.sandboxes"),
		icon: html`<span class="icon icon-cube"></span>`,
		page: true,
	},
	{
		id: "monitoring",
		label: () => t("settings:nav.monitoring"),
		icon: html`<span class="icon icon-chart-bar"></span>`,
		page: true,
	},
	{
		id: "logs",
		label: () => t("settings:nav.logs"),
		icon: html`<span class="icon icon-document"></span>`,
		page: true,
	},
	{
		id: "config",
		label: () => t("settings:nav.configuration"),
		icon: html`<span class="icon icon-code"></span>`,
	},
];

function getVisibleSections() {
	var voiceEnabled = gon.get("voice_enabled");
	return sections.filter((s) => (typeof s.group === "function" ? s.group : s.id) && (s.id !== "voice" || voiceEnabled));
}

/** Return only items with an id (no group headings). */
function getSectionItems() {
	return sections.filter((s) => s.id);
}

function SettingsSidebar() {
	return html`<div class="settings-sidebar">
			<div class="settings-sidebar-header">
				<button
					class="settings-back-slot"
					onClick=${() => {
						navigate(routes.chats);
					}}
					title=${t("settings:nav.backToChats")}
			>
				<span class="icon icon-chat"></span>
				${t("settings:nav.backToChats")}
			</button>
		</div>
		<div class="settings-sidebar-nav">
			${getVisibleSections().map((s) =>
				s.group
					? html`<div key=${s.group()} class="settings-group-label">
							${s.group()}
						</div>`
					: html`<button
							key=${s.id}
							class="settings-nav-item ${activeSection.value === s.id ? "active" : ""}"
							onClick=${() => {
								navigate(settingsPath(s.id));
							}}
						>
							${s.icon}
							${s.label()}
						</button>`,
			)}
		</div>
	</div>`;
}

// EmojiPicker imported from emoji-picker.js

// ── Soul defaults ────────────────────────────────────────────

var DEFAULT_SOUL =
	"Be genuinely helpful, not performatively helpful. Skip the filler words \u2014 just help.\n" +
	"Have opinions. You're allowed to disagree, prefer things, find stuff amusing or boring.\n" +
	"Be resourceful before asking. Try to figure it out first \u2014 read the context, search for it \u2014 then ask if you're stuck.\n" +
	"Earn trust through competence. Be careful with external actions. Be bold with internal ones.\n" +
	"Remember you're a guest. You have access to someone's life. Treat it with respect.\n" +
	"Private things stay private. When in doubt, ask before acting externally.\n" +
	"Be concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just good.";

// ── Identity section (editable form) ─────────────────────────

function IdentitySection() {
	var id = identity.value;
	var isNew = !(id && (id.name || id.user_name));
	var storedLocale = localStorage.getItem("moltis-locale");

	var [name, setName] = useState(id?.name || "");
	var [emoji, setEmoji] = useState(id?.emoji || "");
	var [creature, setCreature] = useState(id?.creature || "");
	var [vibe, setVibe] = useState(id?.vibe || "");
	var [userName, setUserName] = useState(id?.user_name || "");
	var [soul, setSoul] = useState(id?.soul || "");
	var [uiLanguage, setUiLanguage] = useState(storedLocale || "auto");
	var [saving, setSaving] = useState(false);
	var [emojiSaving, setEmojiSaving] = useState(false);
	var [nameSaving, setNameSaving] = useState(false);
	var [userNameSaving, setUserNameSaving] = useState(false);
	var [languageSaving, setLanguageSaving] = useState(false);
	var [saved, setSaved] = useState(false);
	var [languageSaved, setLanguageSaved] = useState(false);
	var [showFaviconReloadHint, setShowFaviconReloadHint] = useState(false);
	var [error, setError] = useState(null);
	var [languageError, setLanguageError] = useState(null);

	// Sync state when identity loads asynchronously
	useEffect(() => {
		if (!id) return;
		setName(id.name || "");
		setEmoji(id.emoji || "");
		setCreature(id.creature || "");
		setVibe(id.vibe || "");
		setUserName(id.user_name || "");
		setSoul(id.soul || "");
	}, [id]);

	function flashSaved() {
		setSaved(true);
		setTimeout(() => {
			setSaved(false);
			rerender();
		}, 2000);
	}

	if (loading.value) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	function onSave(e) {
		e.preventDefault();
		var v = validateIdentityFields(name, userName);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		setError(null);
		setSaving(true);
		setSaved(false);

		updateIdentity({
			name: name.trim(),
			emoji: emoji.trim() || "",
			creature: creature.trim() || "",
			vibe: vibe.trim() || "",
			soul: soul.trim() || null,
			user_name: userName.trim(),
		}).then((res) => {
			setSaving(false);
			if (res?.ok) {
				identity.value = res.payload;
				gon.set("identity", res.payload);
				refreshGon();
				var emojiChanged = (emoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || t("settings:identity.failedToSave"));
			}
			rerender();
		});
	}

	function onEmojiSelect(nextEmoji) {
		setEmoji(nextEmoji);
		setError(null);
		setSaved(false);
		setEmojiSaving(true);
		updateIdentity({ emoji: nextEmoji.trim() || "" }).then((res) => {
			setEmojiSaving(false);
			if (res?.ok) {
				identity.value = res.payload;
				setEmoji(res.payload?.emoji || "");
				gon.set("identity", res.payload);
				refreshGon();
				var emojiChanged = (nextEmoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || t("settings:identity.failedToSaveEmoji"));
			}
			rerender();
		});
	}

	function autoSaveNameField(field, value) {
		if (saving || emojiSaving || nameSaving || userNameSaving) return;
		var trimmed = value.trim();
		var currentValue = (identity.value?.[field] || "").trim();
		if (trimmed === currentValue) return;

		if (!trimmed) {
			setError(field === "name" ? t("settings:identity.agentNameRequired") : t("settings:identity.yourNameRequired"));
			return;
		}

		setError(null);
		setSaved(false);
		if (field === "name") {
			setNameSaving(true);
		} else {
			setUserNameSaving(true);
		}

		var payload = {};
		payload[field] = trimmed;
		updateIdentity(payload).then((res) => {
			if (field === "name") {
				setNameSaving(false);
			} else {
				setUserNameSaving(false);
			}

			if (res?.ok) {
				identity.value = res.payload;
				gon.set("identity", res.payload);
				refreshGon();
				setName(res.payload?.name || "");
				setUserName(res.payload?.user_name || "");
				flashSaved();
			} else {
				setError(res?.error?.message || t("settings:identity.failedToSave"));
			}
			rerender();
		});
	}

	function onNameBlur() {
		autoSaveNameField("name", name);
	}

	function onUserNameBlur() {
		autoSaveNameField("user_name", userName);
	}

	function onResetSoul() {
		setSoul("");
		rerender();
	}

	function onReloadForFavicon() {
		window.location.reload();
	}

	function onApplyLanguage() {
		setLanguageSaving(true);
		setLanguageSaved(false);
		setLanguageError(null);

		var nextLanguage = uiLanguage === "auto" ? navigator.language || "en" : uiLanguage;
		setLocale(nextLanguage)
			.then(() => {
				if (uiLanguage === "auto") {
					localStorage.removeItem("moltis-locale");
				}
				setLanguageSaving(false);
				setLanguageSaved(true);
				setTimeout(() => {
					setLanguageSaved(false);
					rerender();
				}, 2000);
				rerender();
			})
			.catch((err) => {
				setLanguageSaving(false);
				setLanguageError(err?.message || t("settings:identity.failedToUpdateLanguage"));
				rerender();
			});
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:identity.title")}</h2>
		${
			isNew
				? html`<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
				${t("settings:identity.welcome")}
			</p>`
				: null
		}
		<form onSubmit=${onSave} style="max-width:600px;display:flex;flex-direction:column;gap:16px;">
			<!-- Agent section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:identity.agent")}</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">${t("settings:identity.agentSavedTo")}</p>
				<div style="display:grid;grid-template-columns:1fr 1fr;gap:8px 16px;">
						<div>
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:identity.nameLabel")}</div>
							<input type="text" class="provider-key-input" style="width:100%;"
								value=${name} onInput=${(e) => setName(e.target.value)} onBlur=${onNameBlur}
								placeholder=${t("settings:identity.namePlaceholder")} />
						</div>
						<div>
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:identity.emojiLabel")}</div>
							<${EmojiPicker} value=${emoji} onChange=${setEmoji} onSelect=${onEmojiSelect} />
						</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:identity.creatureLabel")}</div>
						<input type="text" class="provider-key-input" style="width:100%;"
							value=${creature} onInput=${(e) => setCreature(e.target.value)}
							placeholder=${t("settings:identity.creaturePlaceholder")} />
					</div>
						<div>
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:identity.vibeLabel")}</div>
							<input type="text" class="provider-key-input" style="width:100%;"
								value=${vibe} onInput=${(e) => setVibe(e.target.value)}
								placeholder=${t("settings:identity.vibePlaceholder")} />
						</div>
					</div>
					${
						showFaviconReloadHint
							? html`<div class="mt-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-xs text-[var(--muted)]">
								${t("settings:identity.faviconHint")} <button type="button" class="cursor-pointer bg-transparent p-0 text-xs text-[var(--text)] underline" onClick=${onReloadForFavicon}>${t("settings:identity.requiresReload")}</button>.
							</div>`
							: null
					}
				</div>

			<!-- User section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:identity.user")}</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">${t("settings:identity.userSavedTo")}</p>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:identity.yourNameLabel")}</div>
						<input type="text" class="provider-key-input" style="width:100%;max-width:280px;"
							value=${userName} onInput=${(e) => setUserName(e.target.value)} onBlur=${onUserNameBlur}
							placeholder=${t("settings:identity.yourNamePlaceholder")} />
					</div>
				</div>

			<!-- Language section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:identity.languageSection")}</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">${t("settings:identity.languageDescription")}</p>
				<div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;">
					<label for="identityLanguageSelect" class="text-xs text-[var(--muted)]">${t("settings:identity.languageLabel")}</label>
					<select
						id="identityLanguageSelect"
						class="provider-key-input"
						style="max-width:220px;"
						value=${uiLanguage}
						onChange=${(e) => {
							setUiLanguage(e.target.value);
							setLanguageSaved(false);
							setLanguageError(null);
							rerender();
						}}
					>
						<option value="auto">${t("settings:identity.languageAuto")}</option>
						<option value="en">${t("settings:identity.languageEnglish")}</option>
						<option value="fr">${t("settings:identity.languageFrench")}</option>
					</select>
					<button
						type="button"
						id="identityLanguageApplyBtn"
						class="provider-btn provider-btn-secondary"
						disabled=${languageSaving}
						onClick=${onApplyLanguage}
					>
						${languageSaving ? t("common:actions.saving") : t("settings:identity.applyLanguage")}
					</button>
					${languageSaved ? html`<span class="text-xs" style="color:var(--accent);">${t("settings:identity.languageUpdated")}</span>` : null}
					${languageError ? html`<span class="text-xs" style="color:var(--error);">${languageError}</span>` : null}
				</div>
			</div>

			<!-- Soul section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:4px;">${t("settings:identity.soul")}</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">${t("settings:identity.soulDescription")}</p>
				<textarea
					class="provider-key-input"
					rows="8"
					style="width:100%;min-height:8rem;resize:vertical;font-size:.8rem;line-height:1.5;"
					placeholder=${DEFAULT_SOUL}
					value=${soul}
					onInput=${(e) => setSoul(e.target.value)}
				/>
				${
					soul
						? html`<button type="button" class="provider-btn" style="margin-top:6px;font-size:.75rem;"
							onClick=${onResetSoul}>${t("common:actions.resetToDefault")}</button>`
						: null
				}
			</div>

					<div style="display:flex;align-items:center;gap:8px;">
						<button type="submit" class="provider-btn" disabled=${saving || emojiSaving || nameSaving || userNameSaving}>
							${saving || emojiSaving || nameSaving || userNameSaving ? t("common:status.saving") : t("common:actions.save")}
						</button>
				${saved ? html`<span class="text-xs" style="color:var(--accent);">${t("common:actions.saved")}</span>` : null}
				${error ? html`<span class="text-xs" style="color:var(--error);">${error}</span>` : null}
			</div>
		</form>
	</div>`;
}

// ── Environment section ──────────────────────────────────────

function EnvironmentSection() {
	var [envVars, setEnvVars] = useState([]);
	var [envLoading, setEnvLoading] = useState(true);
	var [newKey, setNewKey] = useState("");
	var [newValue, setNewValue] = useState("");
	var [envMsg, setEnvMsg] = useState(null);
	var [envErr, setEnvErr] = useState(null);
	var [saving, setSaving] = useState(false);
	var [updateId, setUpdateId] = useState(null);
	var [updateValue, setUpdateValue] = useState("");

	function envApiErrorMessage(payload) {
		if (!payload) return t("settings:environment.requestFailed");
		switch (payload.code) {
			case "ENV_KEY_REQUIRED":
				return t("settings:environment.keyRequired");
			case "ENV_KEY_INVALID":
				return t("settings:environment.keyInvalid");
			case "ENV_CREDENTIAL_STORE_UNAVAILABLE":
				return t("settings:environment.serviceUnavailable");
			case "ENV_LIST_FAILED":
			case "ENV_SET_FAILED":
			case "ENV_DELETE_FAILED":
				return t("settings:environment.requestFailed");
			default:
				return payload.error || t("settings:environment.requestFailed");
		}
	}

	function fetchEnvVars() {
		fetch("/api/env")
			.then((r) => {
				if (r.ok) return r.json();
				return r
					.json()
					.then((payload) => {
						setEnvErr(envApiErrorMessage(payload));
						return { env_vars: [] };
					})
					.catch(() => ({ env_vars: [] }));
			})
			.then((d) => {
				setEnvVars(d.env_vars || []);
				setEnvLoading(false);
				rerender();
			})
			.catch((err) => {
				setEnvErr(err?.message || t("settings:environment.requestFailed"));
				setEnvLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		fetchEnvVars();
	}, []);

	function onAdd(e) {
		e.preventDefault();
		setEnvErr(null);
		setEnvMsg(null);
		var key = newKey.trim();
		if (!key) {
			setEnvErr(t("settings:environment.keyRequired"));
			rerender();
			return;
		}
		if (!/^[A-Za-z0-9_]+$/.test(key)) {
			setEnvErr(t("settings:environment.keyInvalid"));
			rerender();
			return;
		}
		setSaving(true);
		rerender();
		fetch("/api/env", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ key, value: newValue }),
		})
			.then((r) => {
				if (r.ok) {
					setNewKey("");
					setNewValue("");
					setEnvMsg(t("settings:environment.variableSaved"));
					setTimeout(() => {
						setEnvMsg(null);
						rerender();
					}, 2000);
					fetchEnvVars();
				} else {
					return r
						.json()
						.then((payload) => setEnvErr(envApiErrorMessage(payload)))
						.catch(() => setEnvErr(t("settings:environment.requestFailed")));
				}
				setSaving(false);
				rerender();
			})
			.catch((err) => {
				setEnvErr(err?.message || t("settings:environment.requestFailed"));
				setSaving(false);
				rerender();
			});
	}

	function onDelete(id) {
		setEnvErr(null);
		fetch(`/api/env/${id}`, { method: "DELETE" })
			.then((r) => {
				if (r.ok) {
					fetchEnvVars();
					return;
				}
				return r
					.json()
					.then((payload) => {
						setEnvErr(envApiErrorMessage(payload));
						rerender();
					})
					.catch(() => {
						setEnvErr(t("settings:environment.requestFailed"));
						rerender();
					});
			})
			.catch((err) => {
				setEnvErr(err?.message || t("settings:environment.requestFailed"));
				rerender();
			});
	}

	function onStartUpdate(id) {
		setUpdateId(id);
		setUpdateValue("");
		rerender();
	}

	function onCancelUpdate() {
		setUpdateId(null);
		setUpdateValue("");
		rerender();
	}

	function onConfirmUpdate(key) {
		setEnvErr(null);
		fetch("/api/env", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ key, value: updateValue }),
		})
			.then((r) => {
				if (r.ok) {
					setUpdateId(null);
					setUpdateValue("");
					fetchEnvVars();
					return;
				}
				return r
					.json()
					.then((payload) => {
						setEnvErr(envApiErrorMessage(payload));
						rerender();
					})
					.catch(() => {
						setEnvErr(t("settings:environment.requestFailed"));
						rerender();
					});
			})
			.catch((err) => {
				setEnvErr(err?.message || t("settings:environment.requestFailed"));
				rerender();
			});
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:environment.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			${t("settings:environment.description")}
		</p>

		${
			envLoading
				? html`<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>`
				: html`
			<!-- Existing variables -->
			<div style="max-width:600px;">
				${
					envVars.length > 0
						? html`<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
					${envVars.map(
						(v) => html`<div class="provider-item" style="margin-bottom:0;" key=${v.id}>
						${
							updateId === v.id
								? html`<form style="display:flex;align-items:center;gap:6px;flex:1" onSubmit=${(e) => {
										e.preventDefault();
										onConfirmUpdate(v.key);
									}}>
									<code style="font-size:0.8rem;font-family:var(--font-mono);">${v.key}</code>
									<input type="password" class="provider-key-input"
										name="env_update_value"
										autocomplete="new-password"
										autocorrect="off"
										autocapitalize="off"
										spellcheck="false"
										value=${updateValue}
										onInput=${(e) => setUpdateValue(e.target.value)}
										placeholder=${t("common:actions.newValue")} style="flex:1" autofocus />
									<button type="submit" class="provider-btn">${t("common:actions.save")}</button>
									<button type="button" class="provider-btn" onClick=${onCancelUpdate}>${t("common:actions.cancel")}</button>
								</form>`
								: html`<div style="flex:1;min-width:0;">
									<div class="provider-item-name" style="font-family:var(--font-mono);font-size:.8rem;">${v.key}</div>
									<div style="font-size:.7rem;color:var(--muted);margin-top:2px;display:flex;gap:12px;">
										<span>\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022</span>
										<time datetime=${v.updated_at}>${v.updated_at}</time>
									</div>
								</div>
									<div style="display:flex;gap:4px;">
										<button class="provider-btn provider-btn-sm" onClick=${() => onStartUpdate(v.id)}>${t("common:actions.update")}</button>
										<button class="provider-btn provider-btn-sm provider-btn-danger"
											onClick=${() => onDelete(v.id)}>${t("common:actions.delete")}</button>
									</div>`
						}
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">${t("settings:environment.noVariables")}</div>`
				}
			</div>

			<!-- Add variable -->
			<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:environment.addVariable")}</h3>
				<form onSubmit=${onAdd}>
					<div style="display:flex;gap:8px;flex-wrap:wrap;">
						<input type="text" class="provider-key-input"
							name="env_key"
							autocomplete="off"
							autocorrect="off"
							autocapitalize="off"
							spellcheck="false"
							value=${newKey}
							onInput=${(e) => setNewKey(e.target.value)}
							placeholder=${t("settings:environment.keyPlaceholder")} style="flex:1;min-width:120px;font-family:var(--font-mono);font-size:.8rem;" />
						<input type="password" class="provider-key-input"
							name="env_value"
							autocomplete="new-password"
							autocorrect="off"
							autocapitalize="off"
							spellcheck="false"
							value=${newValue}
							onInput=${(e) => setNewValue(e.target.value)}
							placeholder=${t("settings:environment.valuePlaceholder")} style="flex:2;min-width:200px;" />
						<button type="submit" class="provider-btn" disabled=${saving || !newKey.trim()}>
							${saving ? t("common:status.saving") : t("common:actions.add")}
						</button>
					</div>
					${envMsg ? html`<div class="text-xs" style="margin-top:6px;color:var(--accent);">${envMsg}</div>` : null}
					${envErr ? html`<div class="text-xs" style="margin-top:6px;color:var(--error);">${envErr}</div>` : null}
				</form>
			</div>
		`
		}
	</div>`;
}

// ── Security section ─────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing auth, passwords, passkeys, and API keys
function SecuritySection() {
	var [authDisabled, setAuthDisabled] = useState(false);
	var [localhostOnly, setLocalhostOnly] = useState(false);
	var [hasPassword, setHasPassword] = useState(true);
	var [hasPasskeys, setHasPasskeys] = useState(false);
	var [setupComplete, setSetupComplete] = useState(false);
	var [authLoading, setAuthLoading] = useState(true);

	var [curPw, setCurPw] = useState("");
	var [newPw, setNewPw] = useState("");
	var [confirmPw, setConfirmPw] = useState("");
	var [pwMsg, setPwMsg] = useState(null);
	var [pwErr, setPwErr] = useState(null);
	var [pwSaving, setPwSaving] = useState(false);

	var [passkeys, setPasskeys] = useState([]);
	var [pkName, setPkName] = useState("");
	var [pkMsg, setPkMsg] = useState(null);
	var [pkLoading, setPkLoading] = useState(true);
	var [editingPk, setEditingPk] = useState(null);
	var [editingPkName, setEditingPkName] = useState("");
	var [passkeyOrigins, setPasskeyOrigins] = useState([]);

	var [apiKeys, setApiKeys] = useState([]);
	var [akLabel, setAkLabel] = useState("");
	var [akNew, setAkNew] = useState(null);
	var [akLoading, setAkLoading] = useState(true);
	var [akFullAccess, setAkFullAccess] = useState(true);
	var [akScopes, setAkScopes] = useState({
		"operator.read": false,
		"operator.write": false,
		"operator.approvals": false,
		"operator.pairing": false,
	});

	function notifyAuthStatusChanged() {
		window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
	}

	// A credential added while localhost-bypass is active can immediately make the
	// current session unauthenticated (no session cookie). Reload so middleware
	// can route to /login in that transition.
	function reloadIfAuthNowRequiresLogin() {
		return fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d) => {
				var mustLogin = !!(d && d.auth_disabled === false && d.setup_required === false && d.authenticated === false);
				if (mustLogin) {
					window.location.reload();
					return true;
				}
				return false;
			})
			.catch(() => false);
	}

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d) => {
				if (d?.auth_disabled) setAuthDisabled(true);
				if (d?.localhost_only) setLocalhostOnly(true);
				if (d?.has_password === false) setHasPassword(false);
				if (d?.has_passkeys === true) setHasPasskeys(true);
				if (d?.setup_complete) setSetupComplete(true);
				if (d?.passkey_origins) setPasskeyOrigins(d.passkey_origins);
				setAuthLoading(false);
				rerender();
			})
			.catch(() => {
				setAuthLoading(false);
				rerender();
			});
		fetch("/api/auth/passkeys")
			.then((r) => (r.ok ? r.json() : { passkeys: [] }))
			.then((d) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				setPkLoading(false);
				rerender();
			})
			.catch(() => setPkLoading(false));
		fetch("/api/auth/api-keys")
			.then((r) => (r.ok ? r.json() : { api_keys: [] }))
			.then((d) => {
				setApiKeys(d.api_keys || []);
				setAkLoading(false);
				rerender();
			})
			.catch(() => setAkLoading(false));
	}, []);

	function onChangePw(e) {
		e.preventDefault();
		setPwErr(null);
		setPwMsg(null);
		if (newPw.length < 8) {
			setPwErr(t("settings:security.passwordMinLength"));
			return;
		}
		if (newPw !== confirmPw) {
			setPwErr(t("settings:security.passwordsDoNotMatch"));
			return;
		}
		setPwSaving(true);
		var payload = { new_password: newPw };
		if (hasPassword) payload.current_password = curPw;
		fetch("/api/auth/password/change", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(payload),
		})
			.then((r) => {
				if (!r.ok) {
					return r.text().then((msg) => {
						setPwErr(msg);
						setPwSaving(false);
						rerender();
					});
				}

				setPwMsg(hasPassword ? t("settings:security.passwordChanged") : t("settings:security.passwordSet"));
				setCurPw("");
				setNewPw("");
				setConfirmPw("");
				setHasPassword(true);
				setSetupComplete(true);
				setAuthDisabled(false);
				return reloadIfAuthNowRequiresLogin().then((reloaded) => {
					if (!reloaded) notifyAuthStatusChanged();
					setPwSaving(false);
					rerender();
				});
			})
			.catch((err) => {
				setPwErr(err.message);
				setPwSaving(false);
				rerender();
			});
	}

	function onAddPasskey() {
		setPkMsg(null);
		if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
			setPkMsg(t("settings:security.passkeyRequiresDomain", { hostname: location.hostname }));
			rerender();
			return;
		}
		fetch("/api/auth/passkey/register/begin", { method: "POST" })
			.then((r) => r.json())
			.then((data) => {
				var opts = data.options;
				opts.publicKey.challenge = b64ToBuf(opts.publicKey.challenge);
				opts.publicKey.user.id = b64ToBuf(opts.publicKey.user.id);
				if (opts.publicKey.excludeCredentials) {
					for (var c of opts.publicKey.excludeCredentials) c.id = b64ToBuf(c.id);
				}
				return navigator.credentials
					.create({ publicKey: opts.publicKey })
					.then((cred) => ({ cred, challengeId: data.challenge_id }));
			})
			.then(({ cred, challengeId }) => {
				var body = {
					challenge_id: challengeId,
					name: pkName.trim() || detectPasskeyName(cred),
					credential: {
						id: cred.id,
						rawId: bufToB64(cred.rawId),
						type: cred.type,
						response: {
							attestationObject: bufToB64(cred.response.attestationObject),
							clientDataJSON: bufToB64(cred.response.clientDataJSON),
						},
					},
				};
				return fetch("/api/auth/passkey/register/finish", {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify(body),
				});
			})
			.then((r) => {
				if (r.ok) {
					setPkName("");
					return reloadIfAuthNowRequiresLogin().then((reloaded) => {
						if (reloaded) return;
						return fetch("/api/auth/passkeys")
							.then((r2) => r2.json())
							.then((d) => {
								setPasskeys(d.passkeys || []);
								setHasPasskeys((d.passkeys || []).length > 0);
								setSetupComplete(true);
								setAuthDisabled(false);
								setPkMsg(t("settings:security.passkeyAdded"));
								notifyAuthStatusChanged();
								rerender();
							});
					});
				} else
					return r.text().then((msg) => {
						setPkMsg(msg);
						rerender();
					});
			})
			.catch((err) => {
				setPkMsg(err.message || t("settings:security.failedToAddPasskey"));
				rerender();
			});
	}

	function onStartRename(id, currentName) {
		setEditingPk(id);
		setEditingPkName(currentName);
		rerender();
	}

	function onCancelRename() {
		setEditingPk(null);
		setEditingPkName("");
		rerender();
	}

	function onConfirmRename(id) {
		var name = editingPkName.trim();
		if (!name) return;
		fetch(`/api/auth/passkeys/${id}`, {
			method: "PATCH",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ name }),
		})
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d) => {
				setPasskeys(d.passkeys || []);
				setEditingPk(null);
				setEditingPkName("");
				rerender();
			});
	}

	function onRemovePasskey(id) {
		fetch(`/api/auth/passkeys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				notifyAuthStatusChanged();
				rerender();
			});
	}

	function onCreateApiKey() {
		if (!akLabel.trim()) return;
		setAkNew(null);
		// Build scopes array if not full access
		var scopes = null;
		if (!akFullAccess) {
			scopes = Object.entries(akScopes)
				.filter(([, v]) => v)
				.map(([k]) => k);
			if (scopes.length === 0) {
				// Require at least one scope if not full access
				return;
			}
		}
		fetch("/api/auth/api-keys", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ label: akLabel.trim(), scopes }),
		})
			.then((r) => r.json())
			.then((d) => {
				setAkNew(d.key);
				setAkLabel("");
				setAkFullAccess(true);
				setAkScopes({
					"operator.read": false,
					"operator.write": false,
					"operator.approvals": false,
					"operator.pairing": false,
				});
				return fetch("/api/auth/api-keys").then((r2) => r2.json());
			})
			.then((d) => {
				setApiKeys(d.api_keys || []);
				rerender();
			})
			.catch(() => rerender());
	}

	function toggleScope(scope) {
		setAkScopes((prev) => ({ ...prev, [scope]: !prev[scope] }));
		rerender();
	}

	function onRevokeApiKey(id) {
		fetch(`/api/auth/api-keys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/api-keys").then((r) => r.json()))
			.then((d) => {
				setApiKeys(d.api_keys || []);
				rerender();
			});
	}

	var [resetConfirm, setResetConfirm] = useState(false);
	var [resetBusy, setResetBusy] = useState(false);

	function onResetAuth() {
		if (!resetConfirm) {
			setResetConfirm(true);
			rerender();
			return;
		}
		setResetBusy(true);
		rerender();
		fetch("/api/auth/reset", { method: "POST" })
			.then((r) => {
				if (r.ok) {
					window.location.reload();
				} else {
					return r.text().then((msg) => {
						setPwErr(msg);
						setResetConfirm(false);
						setResetBusy(false);
						rerender();
					});
				}
			})
			.catch((err) => {
				setPwErr(err.message);
				setResetConfirm(false);
				setResetBusy(false);
				rerender();
			});
	}

	if (authLoading) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:security.title")}</h2>
			<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	if (authDisabled && !localhostOnly) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:security.title")}</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--error);background:color-mix(in srgb, var(--error) 5%, transparent);">
				<strong style="color:var(--error);">${t("settings:security.authDisabled")}</strong>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${t("settings:security.authDisabledWarning")}
				</p>
				<button type="button" class="provider-btn" style="margin-top:10px;"
					onClick=${() => {
						window.location.assign("/onboarding");
					}}>${t("settings:security.setupAuth")}</button>
			</div>
		</div>`;
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:security.title")}</h2>

		${
			authDisabled && localhostOnly
				? html`<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--error);background:color-mix(in srgb, var(--error) 5%, transparent);">
					<strong style="color:var(--error);">${t("settings:security.authDisabled")}</strong>
					<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
						${t("settings:security.localhostBypassWarning")}
					</p>
				</div>`
				: null
		}

		${
			localhostOnly && !hasPassword && !hasPasskeys && !authDisabled
				? html`<div class="alert-info-text max-w-form">
					<span class="alert-label-info">${t("settings:security.note")}</span>
					${t("settings:security.localhostBypassNote")}
				</div>`
				: null
		}

		<!-- Password -->
		<div style="max-width:600px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${hasPassword ? t("settings:security.changePassword") : t("settings:security.setPasswordTitle")}</h3>
			<form onSubmit=${onChangePw}>
				<div style="display:flex;flex-direction:column;gap:8px;margin-bottom:10px;">
					${
						hasPassword
							? html`<div>
								<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:security.currentPassword")}</div>
								<input type="password" class="provider-key-input" style="width:100%;" value=${curPw}
									onInput=${(e) => setCurPw(e.target.value)} />
							</div>`
							: null
					}
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${hasPassword ? t("settings:security.newPassword") : t("settings:security.passwordLabel")}</div>
						<input type="password" class="provider-key-input" style="width:100%;" value=${newPw}
							onInput=${(e) => setNewPw(e.target.value)} placeholder=${t("settings:security.passwordPlaceholder")} />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${hasPassword ? t("settings:security.confirmNewPassword") : t("settings:security.confirmPassword")}</div>
						<input type="password" class="provider-key-input" style="width:100%;" value=${confirmPw}
							onInput=${(e) => setConfirmPw(e.target.value)} />
					</div>
				</div>
				<div style="display:flex;align-items:center;gap:8px;">
					<button type="submit" class="provider-btn" disabled=${pwSaving}>
						${pwSaving ? (hasPassword ? t("settings:security.changing") : t("settings:security.settingPassword")) : hasPassword ? t("settings:security.changePasswordBtn") : t("settings:security.setPasswordBtn")}
					</button>
					${pwMsg ? html`<span class="text-xs" style="color:var(--accent);">${pwMsg}</span>` : null}
					${pwErr ? html`<span class="text-xs" style="color:var(--error);">${pwErr}</span>` : null}
				</div>
			</form>
		</div>

		<!-- Passkeys -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:security.passkeys")}</h3>
			${passkeyOrigins.length > 1 && html`<div class="text-xs text-[var(--muted)]" style="margin-bottom:8px;">${t("settings:security.passkeyOrigins", { origins: passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ") })}</div>`}
			${
				pkLoading
					? html`<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>`
					: html`
				${
					passkeys.length > 0
						? html`<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
					${passkeys.map(
						(pk) => html`<div class="provider-item" style="margin-bottom:0;" key=${pk.id}>
						${
							editingPk === pk.id
								? html`<form style="display:flex;align-items:center;gap:6px;flex:1" onSubmit=${(e) => {
										e.preventDefault();
										onConfirmRename(pk.id);
									}}>
									<input type="text" class="provider-key-input" value=${editingPkName}
										onInput=${(e) => setEditingPkName(e.target.value)}
										style="flex:1" autofocus />
									<button type="submit" class="provider-btn provider-btn-sm">${t("common:actions.save")}</button>
									<button type="button" class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${onCancelRename}>${t("common:actions.cancel")}</button>
								</form>`
								: html`<div style="flex:1;min-width:0;">
									<div class="provider-item-name" style="font-size:.85rem;">${pk.name}</div>
									<div style="font-size:.7rem;color:var(--muted);margin-top:2px;"><time datetime=${pk.created_at}>${pk.created_at}</time></div>
								</div>
								<div style="display:flex;gap:4px;">
									<button class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${() => onStartRename(pk.id, pk.name)}>${t("common:actions.rename")}</button>
									<button class="provider-btn provider-btn-sm provider-btn-danger"
										onClick=${() => onRemovePasskey(pk.id)}>${t("common:actions.remove")}</button>
								</div>`
						}
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0 12px;">${t("settings:security.noPasskeys")}</div>`
				}
				<div style="display:flex;gap:8px;align-items:center;">
					<input type="text" class="provider-key-input" value=${pkName}
						onInput=${(e) => setPkName(e.target.value)}
						placeholder=${t("settings:security.passkeyNamePlaceholder")} style="flex:1" />
					<button type="button" class="provider-btn" onClick=${onAddPasskey}>${t("settings:security.addPasskey")}</button>
				</div>
				${pkMsg ? html`<div class="text-xs text-[var(--muted)]" style="margin-top:6px;">${pkMsg}</div>` : null}
			`
			}
		</div>

		<!-- API Keys -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:4px;">${t("settings:security.apiKeys")}</h3>
			<p class="text-xs text-[var(--muted)] leading-relaxed" style="margin:0 0 12px;">
				${t("settings:security.apiKeysDescription")}
			</p>
			${
				akLoading
					? html`<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>`
					: html`
				${
					akNew
						? html`<div style="margin-bottom:12px;padding:10px 12px;background:var(--bg);border:1px solid var(--border);border-radius:6px;">
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${t("settings:security.apiKeyCopyWarning")}</div>
							<code style="font-family:var(--font-mono);font-size:.78rem;word-break:break-all;color:var(--text-strong);">${akNew}</code>
						</div>`
						: null
				}
				${
					apiKeys.length > 0
						? html`<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
					${apiKeys.map(
						(ak) => html`<div class="provider-item" style="margin-bottom:0;" key=${ak.id}>
						<div style="flex:1;min-width:0;">
							<div class="provider-item-name" style="font-size:.85rem;">${ak.label}</div>
							<div style="font-size:.7rem;color:var(--muted);margin-top:2px;display:flex;gap:12px;flex-wrap:wrap;">
								<span style="font-family:var(--font-mono);">${ak.key_prefix}...</span>
								<span><time datetime=${ak.created_at}>${ak.created_at}</time></span>
								${ak.scopes ? html`<span style="color:var(--accent);">${ak.scopes.join(", ")}</span>` : html`<span style="color:var(--accent);">${t("settings:security.fullAccess")}</span>`}
							</div>
						</div>
						<button class="provider-btn provider-btn-danger"
							onClick=${() => onRevokeApiKey(ak.id)}>${t("settings:security.revoke")}</button>
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0 12px;">${t("settings:security.noApiKeys")}</div>`
				}
				<div style="display:flex;flex-direction:column;gap:10px;">
					<div style="display:flex;gap:8px;align-items:center;">
						<input type="text" class="provider-key-input" value=${akLabel}
							onInput=${(e) => setAkLabel(e.target.value)}
							placeholder=${t("settings:security.apiKeyLabelPlaceholder")} style="flex:1" />
					</div>
					<div>
						<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
							<input type="checkbox" checked=${akFullAccess}
								onChange=${() => {
									setAkFullAccess(!akFullAccess);
									rerender();
								}} />
							<span class="text-xs text-[var(--text)]">${t("settings:security.fullAccessAll")}</span>
						</label>
					</div>
					${
						akFullAccess
							? null
							: html`<div style="padding-left:20px;display:flex;flex-direction:column;gap:6px;">
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:2px;">${t("settings:security.selectPermissions")}</div>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.read"]}
									onChange=${() => toggleScope("operator.read")} />
								<span class="text-xs text-[var(--text)]">${t("settings:security.scopeOperatorRead")}</span>
								<span class="text-xs text-[var(--muted)]">${t("settings:security.scopeOperatorReadDesc")}</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.write"]}
									onChange=${() => toggleScope("operator.write")} />
								<span class="text-xs text-[var(--text)]">${t("settings:security.scopeOperatorWrite")}</span>
								<span class="text-xs text-[var(--muted)]">${t("settings:security.scopeOperatorWriteDesc")}</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.approvals"]}
									onChange=${() => toggleScope("operator.approvals")} />
								<span class="text-xs text-[var(--text)]">${t("settings:security.scopeOperatorApprovals")}</span>
								<span class="text-xs text-[var(--muted)]">${t("settings:security.scopeOperatorApprovalsDesc")}</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.pairing"]}
									onChange=${() => toggleScope("operator.pairing")} />
								<span class="text-xs text-[var(--text)]">${t("settings:security.scopeOperatorPairing")}</span>
								<span class="text-xs text-[var(--muted)]">${t("settings:security.scopeOperatorPairingDesc")}</span>
							</label>
						</div>`
					}
					<div>
						<button type="button" class="provider-btn" onClick=${onCreateApiKey}
							disabled=${!(akLabel.trim() && (akFullAccess || Object.values(akScopes).some((v) => v)))}>
							${t("settings:security.generateKey")}
						</button>
					</div>
				</div>
			`
			}
		</div>

		<!-- Danger zone (only when auth has been set up) -->
		${
			setupComplete
				? html`<div style="max-width:600px;margin-top:8px;border-top:1px solid var(--error);padding-top:16px;">
			<h3 class="text-sm font-medium" style="color:var(--error);margin-bottom:8px;">${t("settings:security.dangerZone")}</h3>
			<div style="padding:12px 16px;border:1px solid var(--error);border-radius:6px;background:color-mix(in srgb, var(--error) 5%, transparent);">
				<strong class="text-sm" style="color:var(--text-strong);">${t("settings:security.removeAllAuth")}</strong>
				<p class="text-xs text-[var(--muted)]" style="margin:6px 0 0;">
					${t("settings:security.removeAllAuthWarning")}
				</p>
				${
					resetConfirm
						? html`<div style="display:flex;align-items:center;gap:8px;margin-top:10px;">
						<span class="text-xs" style="color:var(--error);">${t("settings:security.removeAllAuthConfirm")}</span>
						<button type="button" class="provider-btn provider-btn-danger" disabled=${resetBusy}
							onClick=${onResetAuth}>${resetBusy ? t("settings:security.removing") : t("settings:security.yesRemoveAllAuth")}</button>
						<button type="button" class="provider-btn" onClick=${() => {
							setResetConfirm(false);
							rerender();
						}}>${t("common:actions.cancel")}</button>
					</div>`
						: html`<button type="button" class="provider-btn provider-btn-danger" style="margin-top:10px;"
						onClick=${onResetAuth}>${t("settings:security.removeAllAuth")}</button>`
				}
			</div>
		</div>`
				: ""
		}
	</div>`;
}

function b64ToBuf(b64) {
	var str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	var bin = atob(str);
	var buf = new Uint8Array(bin.length);
	for (var i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer;
}

function bufToB64(buf) {
	var bytes = new Uint8Array(buf);
	var str = "";
	for (var b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ── Configuration section ─────────────────────────────────────

function ConfigSection() {
	var [toml, setToml] = useState("");
	var [configPath, setConfigPath] = useState("");
	var [configLoading, setConfigLoading] = useState(true);
	var [saving, setSaving] = useState(false);
	var [testing, setTesting] = useState(false);
	var [resettingTemplate, setResettingTemplate] = useState(false);
	var [restarting, setRestarting] = useState(false);
	var [msg, setMsg] = useState(null);
	var [err, setErr] = useState(null);
	var [warnings, setWarnings] = useState([]);

	function fetchConfig() {
		setConfigLoading(true);
		rerender();
		fetch("/api/config")
			.then((r) => {
				if (!r.ok) {
					return r.text().then((text) => {
						// Try to parse as JSON for structured error
						try {
							var json = JSON.parse(text);
							return { error: json.error || `HTTP ${r.status}: ${r.statusText}` };
						} catch (_e) {
							return { error: `HTTP ${r.status}: ${r.statusText}` };
						}
					});
				}
				return r.json().catch(() => ({ error: "Invalid JSON response from server" }));
			})
			.then((d) => {
				if (d.error) {
					setErr(d.error);
				} else {
					setToml(d.toml || "");
					setConfigPath(d.path || "");
					setErr(null);
				}
				setConfigLoading(false);
				rerender();
			})
			.catch((fetchErr) => {
				// Network error or other fetch failure
				var errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = t("settings:config.failedToConnect");
				}
				setErr(errMsg);
				setConfigLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		fetchConfig();
	}, []);

	function onTest(e) {
		e.preventDefault();
		setTesting(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config/validate", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ toml }),
		})
			.then((r) => r.json().catch(() => ({ error: t("settings:config.invalidJsonResponse") })))
			.then((d) => {
				setTesting(false);
				if (d.valid) {
					setMsg(t("settings:config.configValid"));
					setWarnings(d.warnings || []);
				} else {
					setErr(d.error || t("settings:config.configInvalid"));
				}
				rerender();
			})
			.catch((fetchErr) => {
				setTesting(false);
				var errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onSave(e) {
		e.preventDefault();
		setSaving(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ toml }),
		})
			.then((r) => r.json().catch(() => ({ error: t("settings:config.invalidJsonResponse") })))
			.then((d) => {
				setSaving(false);
				if (d.ok) {
					setMsg(t("settings:config.configSaved"));
				} else {
					setErr(d.error || t("settings:identity.failedToSave"));
				}
				rerender();
			})
			.catch((fetchErr) => {
				setSaving(false);
				var errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onRestart() {
		setRestarting(true);
		setMsg(t("settings:config.restartingMoltis"));
		setErr(null);
		rerender();

		fetch("/api/restart", { method: "POST" })
			.then((r) =>
				r
					.json()
					.catch(() => ({}))
					.then((d) => ({ status: r.status, data: d })),
			)
			.then(({ status, data }) => {
				if (status >= 400 && data.error) {
					// Server refused the restart (e.g. invalid config)
					setRestarting(false);
					setErr(data.error);
					setMsg(null);
					rerender();
				} else {
					// Server will restart, wait a bit then start polling for reconnection
					setTimeout(waitForRestart, 1000);
				}
			})
			.catch(() => {
				// Expected - server restarted before response
				setTimeout(waitForRestart, 1000);
			});
	}

	function waitForRestart() {
		var attempts = 0;
		var maxAttempts = 30;

		function check() {
			attempts++;
			fetch("/api/gon", { method: "GET" })
				.then((r) => {
					if (r.ok) {
						// Server is back up
						window.location.reload();
					} else if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr(t("settings:config.serverNotBack"));
						rerender();
					}
				})
				.catch(() => {
					if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr(t("settings:config.serverNotBack"));
						rerender();
					}
				});
		}

		check();
	}

	function onReset() {
		fetchConfig();
		setMsg(null);
		setErr(null);
		setWarnings([]);
	}

	function onResetToTemplate() {
		if (!confirm(t("settings:config.resetConfirm"))) {
			return;
		}
		setResettingTemplate(true);
		setMsg(null);
		setErr(null);
		setWarnings([]);
		rerender();

		fetch("/api/config/template")
			.then((r) => {
				if (!r.ok) {
					return { error: `HTTP ${r.status}: ${t("settings:config.failedToLoadTemplate")}` };
				}
				return r.json().catch(() => ({ error: t("settings:config.invalidJsonResponse") }));
			})
			.then((d) => {
				setResettingTemplate(false);
				if (d.error) {
					setErr(d.error);
				} else {
					setToml(d.toml || "");
					setMsg(t("settings:config.templateLoaded"));
				}
				rerender();
			})
			.catch((fetchErr) => {
				setResettingTemplate(false);
				var errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	if (configLoading) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:config.title")}</h2>
			<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:config.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:700px;margin:0;">
			${t("settings:config.description")}${" "}
			<a href="https://docs.moltis.org/configuration.html" target="_blank" rel="noopener"
				style="color:var(--accent);text-decoration:underline;">${t("settings:config.viewDocs")}</a>
		</p>
		${
			configPath
				? html`<div class="text-xs text-[var(--muted)]" style="font-family:var(--font-mono);">
			<span style="opacity:0.7;">${t("settings:config.fileLabel")}</span> ${configPath}
		</div>`
				: null
		}

		<form onSubmit=${onSave} style="max-width:800px;">
			<div style="margin-bottom:12px;">
				<textarea
					class="provider-key-input"
					rows="20"
					style="width:100%;min-height:320px;resize:vertical;font-family:var(--font-mono);font-size:.78rem;line-height:1.5;white-space:pre;overflow-wrap:normal;overflow-x:auto;"
					value=${toml}
					onInput=${(e) => {
						setToml(e.target.value);
						setMsg(null);
						setErr(null);
						setWarnings([]);
					}}
					spellcheck="false"
				/>
			</div>

			${
				warnings.length > 0
					? html`<div style="margin-bottom:12px;padding:10px 12px;background:color-mix(in srgb, orange 10%, transparent);border:1px solid orange;border-radius:6px;">
					<div class="text-xs font-medium" style="color:orange;margin-bottom:6px;">${t("settings:config.warnings")}</div>
					<ul style="margin:0;padding-left:16px;">
						${warnings.map((w) => html`<li class="text-xs text-[var(--muted)]" style="margin:4px 0;">${w}</li>`)}
					</ul>
				</div>`
					: null
			}

			<div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;">
				<button type="button" class="provider-btn provider-btn-secondary" onClick=${onTest} disabled=${testing || saving || resettingTemplate || restarting}>
					${testing ? t("settings:config.testingBtn") : t("settings:config.testBtn")}
				</button>
				<button type="button" class="provider-btn provider-btn-secondary" onClick=${onReset} disabled=${saving || testing || resettingTemplate || restarting}>
					${t("settings:config.reloadBtn")}
				</button>
				<button type="button" class="provider-btn provider-btn-secondary" onClick=${onResetToTemplate} disabled=${saving || testing || resettingTemplate || restarting}>
					${resettingTemplate ? t("settings:config.resetting") : t("settings:config.resetToDefaults")}
				</button>
				<button type="button" class="provider-btn provider-btn-danger" onClick=${onRestart} disabled=${saving || testing || resettingTemplate || restarting}>
					${restarting ? t("settings:config.restarting") : t("settings:config.restartBtn")}
				</button>
				<div style="flex:1;"></div>
				<button type="submit" class="provider-btn" disabled=${saving || testing || resettingTemplate || restarting}>
					${saving ? t("settings:config.savingBtn") : t("settings:config.saveBtn")}
				</button>
			</div>

			${msg ? html`<div class="text-xs" style="margin-top:8px;color:var(--accent);">${msg}</div>` : null}
			${err ? html`<div class="text-xs" style="margin-top:8px;color:var(--error);white-space:pre-wrap;font-family:var(--font-mono);">${err}</div>` : null}
			${
				restarting
					? html`<div class="text-xs text-[var(--muted)]" style="margin-top:8px;">
						${t("settings:config.autoReloadHint")}
					</div>`
					: null
			}
		</form>

		<div style="max-width:800px;margin-top:8px;padding-top:16px;border-top:1px solid var(--border);">
			<p class="text-xs text-[var(--muted)] leading-relaxed">
				<strong>${t("settings:config.tipLabel")}</strong> ${t("settings:config.tipText")}
			</p>
		</div>
	</div>`;
}

// ── Tailscale section ─────────────────────────────────────────

/** Populate a text node with plain text + clickable URLs. */
function setLinkedText(el, text) {
	el.textContent = "";
	var parts = String(text).split(/(https?:\/\/[^\s]+)/g);
	for (var p of parts) {
		if (/^https?:\/\//.test(p)) {
			var a = document.createElement("a");
			a.href = p;
			a.target = "_blank";
			a.rel = "noopener";
			a.style.cssText = "color:inherit;text-decoration:underline;word-break:break-all;";
			a.textContent = p;
			el.appendChild(a);
		} else {
			el.appendChild(document.createTextNode(p));
		}
	}
}

/** Clone a hidden element from index.html by ID. */
function cloneHidden(id) {
	var el = document.getElementById(id);
	if (!el) return null;
	var clone = el.cloneNode(true);
	clone.removeAttribute("id");
	clone.style.display = "";
	return clone;
}

function TailscaleSection() {
	var ref = useRef(null);
	var [tsStatus, setTsStatus] = useState(null);
	var [tsError, setTsError] = useState(null);
	var [tsLoading, setTsLoading] = useState(true);
	var [configuring, setConfiguring] = useState(false);
	var [configuringMode, setConfiguringMode] = useState(null);
	var [authReady, setAuthReady] = useState(false);

	function fetchTsStatus() {
		setTsLoading(true);
		rerender();
		fetch("/api/tailscale/status")
			.then((r) => {
				var ct = r.headers.get("content-type") || "";
				if (r.status === 404 || !ct.includes("application/json")) {
					setTsError(t("settings:tailscale.featureNotEnabled"));
					setTsLoading(false);
					rerender();
					return null;
				}
				return r.json();
			})
			.then((data) => {
				if (!data) return;
				if (data.error) {
					setTsError(data.error);
				} else {
					setTsStatus(data);
					setTsError(null);
				}
				setTsLoading(false);
				rerender();
			})
			.catch((e) => {
				setTsError(e.message);
				setTsLoading(false);
				rerender();
			});
	}

	function setMode(mode) {
		setConfiguring(true);
		setTsError(null);
		setConfiguringMode(mode);
		rerender();
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode }),
		})
			.then((r) => r.json())
			.then((data) => {
				if (data.error) {
					setTsError(data.error);
				} else {
					fetchTsStatus();
				}
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			})
			.catch((e) => {
				setTsError(e.message);
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			});
	}

	useEffect(() => {
		fetchTsStatus();
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d) => {
				if (!d) return;
				var ready = d.auth_disabled ? false : d.has_password === true;
				setAuthReady(ready);
				rerender();
			})
			.catch(() => {
				/* ignore auth status fetch errors */
			});
	}, []);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: DOM manipulation with multiple conditionals
	function renderInstalledBar(container, status) {
		var bar = cloneHidden("ts-installed-bar");
		if (!bar) return;
		var verEl = bar.querySelector("[data-ts-version]");
		if (verEl) verEl.textContent = status.version ? `v${status.version.split("-")[0]}` : "";
		var tailnetWrap = bar.querySelector("[data-ts-tailnet-wrap]");
		if (tailnetWrap && status.tailnet) {
			tailnetWrap.style.display = "";
			tailnetWrap.querySelector("[data-ts-tailnet]").textContent = status.tailnet;
		}
		var accountWrap = bar.querySelector("[data-ts-account-wrap]");
		if (accountWrap && status.login_name) {
			accountWrap.style.display = "";
			accountWrap.querySelector("[data-ts-account]").textContent = status.login_name;
		}
		var ipWrap = bar.querySelector("[data-ts-ip-wrap]");
		if (ipWrap && status.tailscale_ip) {
			ipWrap.style.display = "";
			ipWrap.querySelector("[data-ts-ip]").textContent = status.tailscale_ip;
		}
		container.appendChild(bar);
	}

	function createModeBtn(m, currentMode) {
		var btn = document.createElement("button");
		btn.textContent = m;
		btn.style.fontWeight = "500";
		var active = currentMode === m && !configuring;
		var base = "text-xs border px-3 py-1.5 rounded-md cursor-pointer transition-colors";
		var state = active
			? "ts-mode-active"
			: "text-[var(--muted)] border-[var(--border)] bg-transparent hover:text-[var(--text)] hover:border-[var(--border-strong)]";
		btn.className = `${base} ${state}${configuringMode === m ? " ts-mode-configuring" : ""}`;
		var funnelBlocked = m === "funnel" && !authReady;
		btn.disabled = configuring || funnelBlocked;
		if (funnelBlocked) {
			btn.style.opacity = "0.4";
			btn.style.cursor = "default";
			btn.style.pointerEvents = "none";
		} else {
			btn.addEventListener("click", setMode.bind(null, m));
		}
		if (configuringMode === m) {
			var spinner = document.createElement("span");
			spinner.className = "ts-spinner";
			btn.prepend(spinner);
		}
		return btn;
	}

	function renderModeButtons(container, status) {
		var modes = ["off", "serve", "funnel"];
		var currentMode = status?.mode || "off";
		var section = cloneHidden("ts-mode-section");
		if (!section) return currentMode;
		var btnContainer = section.querySelector("[data-ts-mode-btns]");
		for (var m of modes) btnContainer.appendChild(createModeBtn(m, currentMode));
		var cfgMsg = section.querySelector("[data-ts-configuring]");
		if (configuring && cfgMsg) {
			cfgMsg.style.display = "";
			cfgMsg.textContent = t("settings:tailscale.configuringMode", { mode: configuringMode });
		}
		container.appendChild(section);
		var warn = cloneHidden("ts-funnel-security-warning");
		if (warn) container.appendChild(warn);
		if (!authReady) {
			var authBtn = cloneHidden("ts-funnel-auth-btn");
			if (authBtn) container.appendChild(authBtn);
		}
		return currentMode;
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: DOM manipulation with multiple conditionals
	function renderHostnameAndUrl(container, currentMode) {
		if (tsStatus?.hostname) {
			var hn = cloneHidden("ts-hostname");
			if (hn) {
				hn.querySelector("[data-ts-hostname-value]").textContent = tsStatus.hostname;
				var hnLink = hn.querySelector("[data-ts-hostname-link]");
				if (hnLink && tsStatus.url && currentMode !== "off") {
					hnLink.href = tsStatus.url;
					hnLink.classList.remove("pointer-events-none", "text-[var(--text)]");
					hnLink.classList.add("text-[var(--accent)]");
				}
				container.appendChild(hn);
			}
		}
		if (tsStatus?.url && currentMode !== "off") {
			var urlEl = cloneHidden("ts-url");
			if (urlEl) {
				var link = urlEl.querySelector("[data-ts-url-link]");
				link.href = tsStatus.url;
				link.textContent = tsStatus.url;
				container.appendChild(urlEl);
			}
		}
	}

	function renderInstalledState(container) {
		if (tsStatus?.tailscale_up === false) {
			var warn = cloneHidden("ts-not-running");
			if (warn) container.appendChild(warn);
		}
		var currentMode = renderModeButtons(container, tsStatus);
		renderHostnameAndUrl(container, currentMode);
		if (currentMode === "funnel") {
			var fw = cloneHidden("ts-funnel-warning");
			if (fw) container.appendChild(fw);
		}
	}

	function renderTsError(container) {
		var errEl = cloneHidden("ts-error");
		if (errEl) {
			setLinkedText(errEl.querySelector("[data-ts-error-text]"), tsError);
			container.appendChild(errEl);
		}
	}

	function renderNotInstalled(container) {
		var notInst = cloneHidden("ts-not-installed");
		if (notInst) {
			notInst.querySelector("[data-ts-recheck]").addEventListener("click", fetchTsStatus);
			container.appendChild(notInst);
		}
	}

	// Build DOM from hidden elements after each render.
	useEffect(() => {
		var container = ref.current;
		if (!container) return;
		while (container.children.length > 2) container.removeChild(container.lastChild);

		if (tsLoading) {
			var loadEl = document.createElement("div");
			loadEl.className = "text-xs text-[var(--muted)]";
			loadEl.textContent = t("settings:tailscale.loadingSlowHint");
			container.appendChild(loadEl);
			return;
		}
		if (tsStatus?.installed) renderInstalledBar(container, tsStatus);
		if (tsError) renderTsError(container);
		if (tsStatus?.installed === false) {
			if (!tsError) renderNotInstalled(container);
			return;
		}
		renderInstalledState(container);
	});

	return html`<div ref=${ref} class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:tailscale.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
			${t("settings:tailscale.description")}
		</p>
	</div>`;
}

// ── Voice section ────────────────────────────────────────────

// Voice section signals
var voiceShowAddModal = signal(false);
var voiceSelectedProvider = signal(null);
var voiceSelectedProviderData = signal(null);

function VoiceSection() {
	var [allProviders, setAllProviders] = useState({ tts: [], stt: [] });
	var [voiceLoading, setVoiceLoading] = useState(true);
	var [voxtralReqs, setVoxtralReqs] = useState(null);
	var [savingProvider, setSavingProvider] = useState(null);
	var [voiceMsg, setVoiceMsg] = useState(null);
	var [voiceErr, setVoiceErr] = useState(null);
	var [voiceTesting, setVoiceTesting] = useState(null); // { id, type, phase } of provider being tested
	var [activeRecorder, setActiveRecorder] = useState(null); // MediaRecorder for STT stop functionality
	var [voiceTestResults, setVoiceTestResults] = useState({}); // { providerId: { text, error } }

	function fetchVoiceStatus(options) {
		if (!options?.silent) {
			setVoiceLoading(true);
			rerender();
		}
		Promise.all([fetchVoiceProviders(), sendRpc("voice.config.voxtral_requirements", {})])
			.then(([providers, voxtral]) => {
				if (providers?.ok) setAllProviders(providers.payload || { tts: [], stt: [] });
				if (voxtral?.ok) setVoxtralReqs(voxtral.payload);
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			})
			.catch(() => {
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		if (connected.value) fetchVoiceStatus();
	}, [connected.value]);

	function onToggleProvider(provider, enabled, providerType) {
		setVoiceErr(null);
		setVoiceMsg(null);
		setSavingProvider(provider.id);
		rerender();

		toggleVoiceProvider(provider.id, enabled, providerType)
			.then((res) => {
				setSavingProvider(null);
				if (res?.ok) {
					setVoiceMsg(`${provider.name} ${enabled ? "enabled" : "disabled"}.`);
					setTimeout(() => {
						setVoiceMsg(null);
						rerender();
					}, 2000);
					fetchVoiceStatus({ silent: true });
				} else {
					setVoiceErr(res?.error?.message || "Failed to toggle provider");
				}
				rerender();
			})
			.catch((err) => {
				setSavingProvider(null);
				setVoiceErr(err.message);
				rerender();
			});
	}

	function onConfigureProvider(providerId, providerData) {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = providerData || null;
		voiceShowAddModal.value = true;
	}

	function getUnconfiguredProviders() {
		return [...allProviders.stt, ...allProviders.tts].filter((p) => !p.available);
	}

	// Stop active STT recording
	function stopSttRecording() {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	// Test a voice provider (TTS or STT)
	async function testVoiceProvider(providerId, type) {
		// If already recording for this provider, stop it
		if (voiceTesting?.id === providerId && voiceTesting?.type === "stt" && voiceTesting?.phase === "recording") {
			stopSttRecording();
			return;
		}

		setVoiceErr(null);
		setVoiceMsg(null);
		setVoiceTesting({ id: providerId, type, phase: "testing" });
		rerender();

		if (type === "tts") {
			// Test TTS by converting sample text to audio and playing it
			try {
				var id = gon.get("identity");
				var user = id?.user_name || "friend";
				var bot = id?.name || "Moltis";
				var ttsText = await fetchPhrase("settings", user, bot);
				var res = await testTts(ttsText, providerId);
				if (res?.ok && res.payload?.audio) {
					// Decode base64 audio and play it
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
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: true, error: null },
					}));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: false, error: res?.error?.message || t("settings:voice.ttsTestFailed") },
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: err.message || t("settings:voice.ttsTestFailed") },
				}));
			}
			setVoiceTesting(null);
		} else {
			// Test STT by recording audio and transcribing
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
				rerender();

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (var track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });
					rerender();

					var audioBlob = new Blob(audioChunks, { type: "audio/webm" });

					try {
						var resp = await transcribeAudio(S.activeSessionKey, providerId, audioBlob);
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
										error: sttRes.transcriptionError || sttRes.error || t("settings:voice.sttTestFailed"),
									},
								}));
							}
						} else {
							var errBody = await resp.text();
							console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
							var errMsg = t("settings:voice.sttTestFailed");
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
							[providerId]: { text: null, error: fetchErr.message || t("settings:voice.sttTestFailed") },
						}));
					}
					setVoiceTesting(null);
					rerender();
				};
			} catch (err) {
				if (err.name === "NotAllowedError") {
					setVoiceErr(t("settings:voice.micDenied"));
				} else if (err.name === "NotFoundError") {
					setVoiceErr(t("settings:voice.noMicFound"));
				} else {
					setVoiceErr(err.message || t("settings:voice.sttTestFailed"));
				}
				setVoiceTesting(null);
			}
		}
		rerender();
	}

	if (voiceLoading || !connected.value) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:voice.title")}</h2>
			<div class="text-xs text-[var(--muted)]">${connected.value ? t("common:status.loading") : t("common:status.connecting")}</div>
		</div>`;
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:voice.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			${t("settings:voice.description")}
		</p>

		${voiceMsg ? html`<div class="text-xs text-[var(--accent)]">${voiceMsg}</div>` : null}
		${voiceErr ? html`<div class="text-xs text-[var(--error)]">${voiceErr}</div>` : null}

		<div style="max-width:700px;display:flex;flex-direction:column;gap:24px;">
			<!-- STT Providers -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">${t("settings:voice.sttHeading")}</h3>
				<div class="flex flex-col gap-2">
					${allProviders.stt.map((prov) => {
						var meta = prov;
						var testState = voiceTesting?.id === prov.id && voiceTesting?.type === "stt" ? voiceTesting : null;
						var testResult = voiceTestResults[prov.id] || null;
						return html`<${VoiceProviderRow}
							provider=${prov}
							meta=${meta}
							type="stt"
							saving=${savingProvider === prov.id}
							testState=${testState}
							testResult=${testResult}
							onToggle=${(enabled) => onToggleProvider(prov, enabled, "stt")}
							onConfigure=${() => onConfigureProvider(prov.id, prov)}
							onTest=${() => testVoiceProvider(prov.id, "stt")}
						/>`;
					})}
				</div>
			</div>

			<!-- TTS Providers -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">${t("settings:voice.ttsHeading")}</h3>
				<div class="flex flex-col gap-2">
					${allProviders.tts.map((prov) => {
						var meta = prov;
						var testState = voiceTesting?.id === prov.id && voiceTesting?.type === "tts" ? voiceTesting : null;
						var testResult = voiceTestResults[prov.id] || null;
						return html`<${VoiceProviderRow}
							provider=${prov}
							meta=${meta}
							type="tts"
							saving=${savingProvider === prov.id}
							testState=${testState}
							testResult=${testResult}
							onToggle=${(enabled) => onToggleProvider(prov, enabled, "tts")}
							onConfigure=${() => onConfigureProvider(prov.id, prov)}
							onTest=${() => testVoiceProvider(prov.id, "tts")}
						/>`;
					})}
				</div>
			</div>
		</div>

		<${AddVoiceProviderModal}
			unconfiguredProviders=${getUnconfiguredProviders()}
			voxtralReqs=${voxtralReqs}
			onSaved=${() => {
				fetchVoiceStatus();
				voiceShowAddModal.value = false;
				voiceSelectedProvider.value = null;
				voiceSelectedProviderData.value = null;
			}}
		/>
	</div>`;
}

// Individual provider row with enable toggle
function VoiceProviderRow({ provider, meta, type, saving, testState, testResult, onToggle, onConfigure, onTest }) {
	var canEnable = provider.available;
	var keySourceLabel =
		provider.keySource === "env"
			? t("settings:voice.fromEnv")
			: provider.keySource === "llm_provider"
				? t("settings:voice.fromLlmProvider")
				: "";
	var showTestBtn = canEnable && provider.enabled;

	// Determine button text based on test state
	var buttonText = t("common:actions.test");
	var buttonDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			buttonText = t("common:actions.stop");
		} else if (testState.phase === "transcribing") {
			buttonText = t("common:status.testing");
			buttonDisabled = true;
		} else {
			buttonText = t("common:status.testing");
			buttonDisabled = true;
		}
	}

	return html`<div class="provider-card" style="padding:10px 14px;border-radius:8px;display:flex;align-items:center;gap:12px;">
		<div style="flex:1;display:flex;flex-direction:column;gap:2px;">
			<div style="display:flex;align-items:center;gap:8px;">
				<span class="text-sm text-[var(--text-strong)]">${meta.name}</span>
				${provider.category === "local" ? html`<span class="provider-item-badge">${t("settings:voice.localBadge")}</span>` : null}
				${keySourceLabel ? html`<span class="text-xs text-[var(--muted)]">${keySourceLabel}</span>` : null}
			</div>
			<span class="text-xs text-[var(--muted)]">${meta.description}</span>
			${provider.settingsSummary ? html`<span class="text-xs text-[var(--muted)]">${t("settings:voice.voiceSummary", { summary: provider.settingsSummary })}</span>` : null}
			${provider.binaryPath ? html`<span class="text-xs text-[var(--muted)]">${t("settings:voice.foundAt", { path: provider.binaryPath })}</span>` : null}
			${!canEnable && provider.statusMessage ? html`<span class="text-xs text-[var(--muted)]">${provider.statusMessage}</span>` : null}
			${
				testState?.phase === "recording"
					? html`<div class="voice-recording-hint">
				<span class="voice-recording-dot"></span>
				<span>${t("settings:voice.speakNow")}</span>
			</div>`
					: null
			}
			${testState?.phase === "transcribing" ? html`<span class="text-xs text-[var(--muted)]">${t("settings:voice.transcribing")}</span>` : null}
			${testState?.phase === "testing" && type === "tts" ? html`<span class="text-xs text-[var(--muted)]">${t("settings:voice.playingAudio")}</span>` : null}
			${
				testResult?.text
					? html`<div class="voice-transcription-result">
				<span class="voice-transcription-label">${t("settings:voice.transcribed")}</span>
				<span class="voice-transcription-text">"${testResult.text}"</span>
			</div>`
					: null
			}
			${
				testResult?.success === true
					? html`<div class="voice-success-result">
				<span class="icon icon-md icon-check-circle"></span>
				<span>${t("settings:voice.audioSuccess")}</span>
			</div>`
					: null
			}
			${
				testResult?.error
					? html`<div class="voice-error-result">
				<span class="icon icon-md icon-x-circle"></span>
				<span>${testResult.error}</span>
			</div>`
					: null
			}
		</div>
		<div style="display:flex;align-items:center;gap:8px;">
			<button class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onConfigure}>
				${t("common:actions.configure")}
			</button>
			${
				showTestBtn
					? html`<button
						class="provider-btn provider-btn-secondary provider-btn-sm"
						onClick=${onTest}
						disabled=${buttonDisabled}
						title=${type === "tts" ? t("settings:voice.testVoiceOutput") : t("settings:voice.testVoiceInput")}>
						${buttonText}
					</button>`
					: null
			}
			${
				canEnable
					? html`<label class="toggle-switch">
						<input type="checkbox"
							checked=${provider.enabled}
							disabled=${saving}
							onChange=${(e) => onToggle(e.target.checked)} />
						<span class="toggle-slider"></span>
					</label>`
					: provider.category === "local"
						? html`<span class="text-xs text-[var(--muted)]">${t("settings:voice.installRequired")}</span>`
						: null
			}
		</div>
	</div>`;
}

// Local provider instructions component (uses hidden HTML elements)
function LocalProviderInstructions({ providerId, voxtralReqs }) {
	var ref = useRef(null);

	useEffect(() => {
		var container = ref.current;
		if (!container) return;
		while (container.firstChild) container.removeChild(container.firstChild);

		var templateId = {
			"whisper-cli": "voice-whisper-cli-instructions",
			"sherpa-onnx": "voice-sherpa-onnx-instructions",
			piper: "voice-piper-instructions",
			coqui: "voice-coqui-instructions",
			"voxtral-local": "voice-voxtral-instructions",
		}[providerId];

		if (!templateId) return;

		var el = cloneHidden(templateId);
		if (!el) return;

		// For voxtral-local, populate the requirements section
		if (providerId === "voxtral-local" && el.querySelector("[data-voxtral-requirements]")) {
			var reqsContainer = el.querySelector("[data-voxtral-requirements]");
			if (voxtralReqs) {
				var detected = `${voxtralReqs.os}/${voxtralReqs.arch}`;
				if (voxtralReqs.python?.available) detected += `, Python ${voxtralReqs.python.version}`;
				else detected += ", no Python";
				if (voxtralReqs.cuda?.available) {
					detected += `, ${voxtralReqs.cuda.gpu_name || "NVIDIA GPU"} (${Math.round((voxtralReqs.cuda.memory_mb || 0) / 1024)}GB)`;
				} else detected += ", no CUDA GPU";

				var reqEl = cloneHidden(
					voxtralReqs.compatible ? "voice-voxtral-requirements-ok" : "voice-voxtral-requirements-fail",
				);
				if (reqEl) {
					reqEl.querySelector("[data-voxtral-detected]").textContent = detected;
					if (!voxtralReqs.compatible && voxtralReqs.reasons?.length > 0) {
						var ul = reqEl.querySelector("[data-voxtral-reasons]");
						for (var r of voxtralReqs.reasons) {
							var li = document.createElement("li");
							li.style.margin = "2px 0";
							li.textContent = r;
							ul.appendChild(li);
						}
					}
					reqsContainer.appendChild(reqEl);
				}
			} else {
				var loadingEl = document.createElement("div");
				loadingEl.className = "text-xs text-[var(--muted)] mb-3";
				loadingEl.textContent = t("settings:voice.checkingRequirements");
				reqsContainer.appendChild(loadingEl);
			}
		}

		container.appendChild(el);
	}, [providerId, voxtralReqs]);

	return html`<div ref=${ref}></div>`;
}

// Add Voice Provider Modal
function AddVoiceProviderModal({ unconfiguredProviders, voxtralReqs, onSaved }) {
	var [apiKey, setApiKey] = useState("");
	var [voiceValue, setVoiceValue] = useState("");
	var [modelValue, setModelValue] = useState("");
	var [languageCodeValue, setLanguageCodeValue] = useState("");
	var [elevenlabsCatalog, setElevenlabsCatalog] = useState({ voices: [], models: [], warning: null });
	var [elevenlabsCatalogLoading, setElevenlabsCatalogLoading] = useState(false);
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState("");

	var selectedProvider = voiceSelectedProvider.value;
	var providerMeta = selectedProvider
		? unconfiguredProviders.find((p) => p.id === selectedProvider) || voiceSelectedProviderData.value
		: null;
	var isElevenLabsProvider = selectedProvider === "elevenlabs" || selectedProvider === "elevenlabs-stt";
	var supportsTtsVoiceSettings = providerMeta?.type === "tts";

	function onClose() {
		voiceShowAddModal.value = false;
		voiceSelectedProvider.value = null;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	function onSaveKey() {
		var hasApiKey = apiKey.trim().length > 0;
		var hasSettings = supportsTtsVoiceSettings && (voiceValue.trim() || modelValue.trim() || languageCodeValue.trim());
		if (!(hasApiKey || hasSettings)) {
			setError(t("settings:voice.provideKeyOrSetting"));
			return;
		}
		setError("");
		setSaving(true);

		var voiceOpts = supportsTtsVoiceSettings
			? {
					voice: voiceValue.trim() || undefined,
					model: modelValue.trim() || undefined,
					languageCode: languageCodeValue.trim() || undefined,
				}
			: undefined;
		var req = hasApiKey
			? saveVoiceKey(selectedProvider, apiKey.trim(), voiceOpts)
			: sendRpc("voice.config.save_settings", {
					provider: selectedProvider,
					voice: voiceOpts?.voice,
					voiceId: voiceOpts?.voice,
					model: voiceOpts?.model,
					languageCode: voiceOpts?.languageCode,
				});
		req
			.then((res) => {
				setSaving(false);
				if (res?.ok) {
					setApiKey("");
					onSaved();
				} else {
					setError(res?.error?.message || t("settings:voice.failedToSaveKey"));
				}
			})
			.catch((err) => {
				setSaving(false);
				setError(err.message);
			});
	}

	function onSelectProvider(providerId) {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	useEffect(() => {
		var settings = voiceSelectedProviderData.value?.settings;
		if (!settings) return;
		setVoiceValue(settings.voiceId || settings.voice || "");
		setModelValue(settings.model || "");
		setLanguageCodeValue(settings.languageCode || "");
	}, [selectedProvider, voiceSelectedProviderData.value]);

	useEffect(() => {
		if (!isElevenLabsProvider) {
			setElevenlabsCatalog({ voices: [], models: [], warning: null });
			return;
		}
		setElevenlabsCatalogLoading(true);
		sendRpc("voice.elevenlabs.catalog", {})
			.then((res) => {
				if (res?.ok) {
					setElevenlabsCatalog({
						voices: res.payload?.voices || [],
						models: res.payload?.models || [],
						warning: res.payload?.warning || null,
					});
				}
			})
			.catch(() => {
				setElevenlabsCatalog({ voices: [], models: [], warning: t("settings:voice.failedToFetchVoices") });
			})
			.finally(() => {
				setElevenlabsCatalogLoading(false);
				rerender();
			});
	}, [selectedProvider, isElevenLabsProvider]);

	// Group providers by type and category
	var sttCloud = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "cloud");
	var sttLocal = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "local");
	var ttsProviders = unconfiguredProviders.filter((p) => p.type === "tts");

	// If a provider is selected, show its configuration form
	if (selectedProvider && providerMeta) {
		// Cloud provider - show API key form
		if (providerMeta.category === "cloud") {
			return html`<${Modal} show=${voiceShowAddModal.value} onClose=${onClose} title=${t("settings:voice.addProvider", { name: providerMeta.name })}>
				<div class="channel-form">
					<div class="text-sm text-[var(--text-strong)]">${providerMeta.name}</div>
					<div class="text-xs text-[var(--muted)]" style="margin-bottom:12px;">${providerMeta.description}</div>

					<label class="text-xs text-[var(--muted)]">${t("settings:voice.apiKeyLabel")}</label>
					<input type="password" class="provider-key-input" style="width:100%;"
						value=${apiKey} onInput=${(e) => setApiKey(e.target.value)}
						placeholder=${providerMeta.keyPlaceholder || t("settings:voice.keepExistingPlaceholder")} />
					<div class="text-xs text-[var(--muted)]">
						${t("settings:voice.getApiKeyAt", { url: providerMeta.keyUrlLabel })} <a href=${providerMeta.keyUrl} target="_blank" rel="noopener" class="hover:underline text-[var(--accent)]">${providerMeta.keyUrlLabel}</a>
					</div>

					${
						supportsTtsVoiceSettings
							? html`<div class="flex flex-col gap-2">
					<label class="text-xs text-[var(--muted)]">${t("settings:voice.voiceFieldLabel")}</label>
					${isElevenLabsProvider && elevenlabsCatalogLoading ? html`<div class="text-xs text-[var(--muted)]">${t("settings:voice.loadingVoices")}</div>` : null}
					${isElevenLabsProvider && elevenlabsCatalog.warning ? html`<div class="text-xs text-[var(--muted)]">${elevenlabsCatalog.warning}</div>` : null}
					${
						isElevenLabsProvider && elevenlabsCatalog.voices.length > 0
							? html`<select class="provider-key-input" style="width:100%;" onChange=${(e) => setVoiceValue(e.target.value)}>
						<option value="">${t("settings:voice.pickVoice")}</option>
						${elevenlabsCatalog.voices.map((v) => html`<option value=${v.id}>${v.name} (${v.id})</option>`)}
					</select>`
							: null
					}
					<input type="text" class="provider-key-input" style="width:100%;"
						value=${voiceValue} onInput=${(e) => setVoiceValue(e.target.value)}
						list=${isElevenLabsProvider ? "elevenlabs-voice-options" : undefined}
						placeholder=${t("settings:voice.voiceIdPlaceholder")} />
					${
						isElevenLabsProvider
							? html`<datalist id="elevenlabs-voice-options">
						${elevenlabsCatalog.voices.map((v) => html`<option value=${v.id}>${v.name}</option>`)}
					</datalist>`
							: null
					}

					<label class="text-xs text-[var(--muted)]">${t("settings:voice.modelFieldLabel")}</label>
					${
						isElevenLabsProvider && elevenlabsCatalog.models.length > 0
							? html`<select class="provider-key-input" style="width:100%;" onChange=${(e) => setModelValue(e.target.value)}>
						<option value="">${t("settings:voice.pickModel")}</option>
						${elevenlabsCatalog.models.map((m) => html`<option value=${m.id}>${m.name} (${m.id})</option>`)}
					</select>`
							: null
					}
					<input type="text" class="provider-key-input" style="width:100%;"
						value=${modelValue} onInput=${(e) => setModelValue(e.target.value)}
						list=${isElevenLabsProvider ? "elevenlabs-model-options" : undefined}
						placeholder=${t("settings:voice.modelPlaceholder")} />
					${
						isElevenLabsProvider
							? html`<datalist id="elevenlabs-model-options">
						${elevenlabsCatalog.models.map((m) => html`<option value=${m.id}>${m.name}</option>`)}
					</datalist>`
							: null
					}

					${
						selectedProvider === "google" || selectedProvider === "google-tts"
							? html`<div class="flex flex-col gap-2">
							<label class="text-xs text-[var(--muted)]">${t("settings:voice.languageCode")}</label>
							<input type="text" class="provider-key-input" style="width:100%;"
								value=${languageCodeValue} onInput=${(e) => setLanguageCodeValue(e.target.value)}
								placeholder=${t("settings:voice.languageCodePlaceholder")} />
						</div>`
							: null
					}
					</div>`
							: null
					}

					${providerMeta.hint && html`<div class="text-xs text-[var(--muted)]" style="margin-top:8px;padding:8px;background:var(--surface-alt);border-radius:4px;font-style:italic;">${providerMeta.hint}</div>`}

					${error && html`<div class="text-xs" style="color:var(--error);">${error}</div>`}

					<div style="display:flex;gap:8px;margin-top:8px;">
						<button class="provider-btn provider-btn-secondary" onClick=${() => {
							voiceSelectedProvider.value = null;
							setApiKey("");
							setError("");
						}}>${t("common:actions.back")}</button>
						<button class="provider-btn" disabled=${saving} onClick=${onSaveKey}>
							${saving ? t("common:actions.saving") : t("common:actions.save")}
						</button>
					</div>
				</div>
			</${Modal}>`;
		}

		// Local provider - show setup instructions
		if (providerMeta.category === "local") {
			return html`<${Modal} show=${voiceShowAddModal.value} onClose=${onClose} title=${t("settings:voice.addProvider", { name: providerMeta.name })}>
				<div class="channel-form">
					<div class="text-sm text-[var(--text-strong)]">${providerMeta.name}</div>
					<div class="text-xs text-[var(--muted)]" style="margin-bottom:12px;">${providerMeta.description}</div>
					<${LocalProviderInstructions} providerId=${selectedProvider} voxtralReqs=${voxtralReqs} />
					<div style="display:flex;gap:8px;margin-top:12px;">
						<button class="provider-btn provider-btn-secondary" onClick=${() => {
							voiceSelectedProvider.value = null;
						}}>${t("common:actions.back")}</button>
					</div>
				</div>
			</${Modal}>`;
		}
	}

	// Show provider selection list
	return html`<${Modal} show=${voiceShowAddModal.value} onClose=${onClose} title=${t("settings:voice.addVoiceProvider")}>
		<div class="channel-form" style="gap:16px;">
			${
				sttCloud.length > 0
					? html`
				<div>
					<h4 class="text-xs font-medium text-[var(--muted)]" style="margin:0 0 8px;text-transform:uppercase;letter-spacing:0.5px;">${t("settings:voice.sttCloud")}</h4>
					<div style="display:flex;flex-direction:column;gap:6px;">
						${sttCloud.map(
							(p) => html`
							<button class="provider-card" style="padding:10px 12px;border-radius:6px;cursor:pointer;text-align:left;border:1px solid var(--border);background:var(--surface);"
								onClick=${() => onSelectProvider(p.id)}>
								<div style="display:flex;align-items:center;gap:8px;">
									<div style="flex:1;">
										<div class="text-sm text-[var(--text-strong)]">${p.name}</div>
										<div class="text-xs text-[var(--muted)]">${p.description}</div>
									</div>
									<span class="icon icon-chevron-right" style="color:var(--muted);"></span>
								</div>
							</button>
						`,
						)}
					</div>
				</div>
			`
					: null
			}

			${
				sttLocal.length > 0
					? html`
				<div>
					<h4 class="text-xs font-medium text-[var(--muted)]" style="margin:0 0 8px;text-transform:uppercase;letter-spacing:0.5px;">${t("settings:voice.sttLocal")}</h4>
					<div style="display:flex;flex-direction:column;gap:6px;">
						${sttLocal.map(
							(p) => html`
							<button class="provider-card" style="padding:10px 12px;border-radius:6px;cursor:pointer;text-align:left;border:1px solid var(--border);background:var(--surface);"
								onClick=${() => onSelectProvider(p.id)}>
								<div style="display:flex;align-items:center;gap:8px;">
									<div style="flex:1;">
										<div class="text-sm text-[var(--text-strong)]">${p.name}</div>
										<div class="text-xs text-[var(--muted)]">${p.description}</div>
									</div>
									<span class="icon icon-chevron-right" style="color:var(--muted);"></span>
								</div>
							</button>
						`,
						)}
					</div>
				</div>
			`
					: null
			}

			${
				ttsProviders.length > 0
					? html`
				<div>
					<h4 class="text-xs font-medium text-[var(--muted)]" style="margin:0 0 8px;text-transform:uppercase;letter-spacing:0.5px;">${t("settings:voice.ttsCategory")}</h4>
					<div style="display:flex;flex-direction:column;gap:6px;">
						${ttsProviders.map(
							(p) => html`
							<button class="provider-card" style="padding:10px 12px;border-radius:6px;cursor:pointer;text-align:left;border:1px solid var(--border);background:var(--surface);"
								onClick=${() => onSelectProvider(p.id)}>
								<div style="display:flex;align-items:center;gap:8px;">
									<div style="flex:1;">
										<div class="text-sm text-[var(--text-strong)]">${p.name}</div>
										<div class="text-xs text-[var(--muted)]">${p.description}</div>
									</div>
									<span class="icon icon-chevron-right" style="color:var(--muted);"></span>
								</div>
							</button>
						`,
						)}
					</div>
				</div>
			`
					: null
			}

			${
				unconfiguredProviders.length === 0
					? html`
				<div class="text-sm text-[var(--muted)]" style="text-align:center;padding:20px 0;">
					${t("settings:voice.allProvidersConfigured")}
				</div>
			`
					: null
			}
		</div>
	</${Modal}>`;
}

// ── Memory section ────────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing memory settings with QMD integration
function MemorySection() {
	var [memStatus, setMemStatus] = useState(null);
	var [memConfig, setMemConfig] = useState(null);
	var [qmdStatus, setQmdStatus] = useState(null);
	var [memLoading, setMemLoading] = useState(true);
	var [saving, setSaving] = useState(false);
	var [saved, setSaved] = useState(false);
	var [error, setError] = useState(null);

	// Form state
	var [backend, setBackend] = useState("builtin");
	var [citations, setCitations] = useState("auto");
	var [llmReranking, setLlmReranking] = useState(false);
	var [sessionExport, setSessionExport] = useState(false);

	useEffect(() => {
		// Fetch memory status, config, and QMD status
		Promise.all([sendRpc("memory.status", {}), sendRpc("memory.config.get", {}), sendRpc("memory.qmd.status", {})])
			.then(([statusRes, configRes, qmdRes]) => {
				if (statusRes?.ok) {
					setMemStatus(statusRes.payload);
				}
				if (configRes?.ok) {
					var cfg = configRes.payload;
					setMemConfig(cfg);
					setBackend(cfg.backend || "builtin");
					setCitations(cfg.citations || "auto");
					setLlmReranking(cfg.llm_reranking ?? false);
					setSessionExport(cfg.session_export ?? false);
				}
				if (qmdRes?.ok) {
					setQmdStatus(qmdRes.payload);
				}
				setMemLoading(false);
				rerender();
			})
			.catch(() => {
				setMemLoading(false);
				rerender();
			});
	}, []);

	function onSave(e) {
		e.preventDefault();
		setError(null);
		setSaving(true);
		setSaved(false);

		sendRpc("memory.config.update", {
			backend,
			citations,
			llm_reranking: llmReranking,
			session_export: sessionExport,
		}).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setMemConfig(res.payload);
				setSaved(true);
				setTimeout(() => {
					setSaved(false);
					rerender();
				}, 2000);
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
	}

	if (memLoading) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:memory.title")}</h2>
			<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	var qmdFeatureEnabled = memConfig?.qmd_feature_enabled !== false;
	var qmdAvailable = qmdStatus?.available === true;

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:memory.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
			${t("settings:memory.description")}
		</p>

		<!-- Status -->
		${
			memStatus
				? html`
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--bg);">
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:memory.statusHeading")}</h3>
				<div style="display:grid;grid-template-columns:repeat(2,1fr);gap:8px 16px;font-size:.8rem;">
					<div>
						<span class="text-[var(--muted)]">${t("settings:memory.files")}</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.total_files || 0}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">${t("settings:memory.chunks")}</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.total_chunks || 0}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">${t("settings:memory.model")}</span>
						<span class="text-[var(--text)]" style="margin-left:6px;font-family:var(--font-mono);font-size:.75rem;">${memStatus.embedding_model || t("settings:memory.modelNone")}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">${t("settings:memory.dbSize")}</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.db_size_display || t("settings:memory.dbSizeEmpty")}</span>
					</div>
				</div>
			</div>
		`
				: null
		}

		<!-- Configuration -->
		<form onSubmit=${onSave} style="max-width:600px;display:flex;flex-direction:column;gap:16px;">
			<!-- Backend selection -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:memory.backendHeading")}</h3>

				<!-- Comparison table -->
				<div style="margin-bottom:12px;padding:12px;border-radius:6px;border:1px solid var(--border);background:var(--bg);font-size:.75rem;">
					<table style="width:100%;border-collapse:collapse;">
						<thead>
							<tr style="border-bottom:1px solid var(--border);">
								<th style="text-align:left;padding:4px 8px 8px 0;color:var(--muted);font-weight:500;">${t("settings:memory.feature")}</th>
								<th style="text-align:center;padding:4px 8px 8px;color:var(--muted);font-weight:500;">${t("settings:memory.builtIn")}</th>
								<th style="text-align:center;padding:4px 8px 8px;color:var(--muted);font-weight:500;">${t("settings:memory.qmd")}</th>
							</tr>
						</thead>
						<tbody>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.searchType")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.builtInSearchType")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.qmdSearchType")}</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.externalDependency")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">${t("settings:memory.noDependency")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.nodejsBun")}</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.embeddingCache")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">\u2713</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.openAiBatch")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">${t("settings:memory.openAiBatchDiscount")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.providerFallback")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">\u2713</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.llmReranking")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.optional")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">${t("settings:memory.builtInLabel")}</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">${t("settings:memory.bestFor")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.mostUsers")}</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">${t("settings:memory.powerUsers")}</td>
							</tr>
						</tbody>
					</table>
				</div>

				<div style="display:flex;gap:8px;">
					<button type="button"
						class="provider-btn ${backend === "builtin" ? "" : "provider-btn-secondary"}"
						onClick=${() => {
							setBackend("builtin");
							rerender();
						}}>
						${t("settings:memory.builtInRecommended")}
					</button>
					<button type="button"
						class="provider-btn ${backend === "qmd" ? "" : "provider-btn-secondary"}"
						disabled=${!qmdFeatureEnabled}
						onClick=${() => {
							setBackend("qmd");
							rerender();
						}}>
						${t("settings:memory.qmd")}
					</button>
				</div>

				${
					qmdFeatureEnabled
						? null
						: html`
					<div class="text-xs text-[var(--error)]" style="margin-top:8px;">
						${t("settings:memory.qmdNotEnabled")}
					</div>
				`
				}

				${
					backend === "qmd"
						? html`
					<div style="margin-top:12px;padding:12px;border-radius:6px;border:1px solid var(--border);background:var(--bg);">
						<h4 class="text-xs font-medium text-[var(--text-strong)]" style="margin:0 0 8px;">${t("settings:memory.qmdStatus")}</h4>
						${
							qmdAvailable
								? html`
							<div class="text-xs" style="color:var(--accent);display:flex;align-items:center;gap:6px;">
								<span>\u2713</span> ${t("settings:memory.qmdInstalled")} ${qmdStatus?.version ? html`<span class="text-[var(--muted)]">(${qmdStatus.version})</span>` : null}
							</div>
						`
								: html`
							<div class="text-xs" style="color:var(--error);margin-bottom:8px;">
								\u2717 ${t("settings:memory.qmdNotInstalled")}
							</div>
							<div class="text-xs text-[var(--muted)]" style="line-height:1.6;">
								<strong style="color:var(--text);">${t("settings:memory.installation")}</strong><br/>
								<code style="font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">npm install -g @anthropic/qmd</code>
								<span style="margin:0 4px;">or</span>
								<code style="font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">bun install -g @anthropic/qmd</code>
								<br/><br/>
								${t("settings:memory.thenStartDaemon")}
								<code style="display:block;margin-top:4px;font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">qmd daemon</code>
								<br/>
								<a href="https://github.com/anthropics/qmd" target="_blank" rel="noopener"
									style="color:var(--accent);">${t("settings:memory.viewDocumentation")}</a>
							</div>
						`
						}
					</div>
				`
						: null
				}
			</div>

			<!-- Citations -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${t("settings:memory.citations")}</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">
					${t("settings:memory.citationsDescription")}
				</p>
				<select class="provider-key-input" style="width:auto;min-width:150px;"
					value=${citations} onChange=${(e) => {
						setCitations(e.target.value);
						rerender();
					}}>
					<option value="auto">${t("settings:memory.citationsAuto")}</option>
					<option value="on">${t("settings:memory.citationsAlways")}</option>
					<option value="off">${t("settings:memory.citationsNever")}</option>
				</select>
			</div>

			<!-- LLM Reranking -->
			<div>
				<label style="display:flex;align-items:center;gap:8px;cursor:pointer;">
					<input type="checkbox" checked=${llmReranking}
						onChange=${(e) => {
							setLlmReranking(e.target.checked);
							rerender();
						}} />
					<div>
						<span class="text-sm font-medium text-[var(--text-strong)]">${t("settings:memory.llmRerankingLabel")}</span>
						<p class="text-xs text-[var(--muted)]" style="margin:2px 0 0;">
							${t("settings:memory.llmRerankingDescription")}
						</p>
					</div>
				</label>
			</div>

			<!-- Session Export -->
			<div>
				<label style="display:flex;align-items:center;gap:8px;cursor:pointer;">
					<input type="checkbox" checked=${sessionExport}
						onChange=${(e) => {
							setSessionExport(e.target.checked);
							rerender();
						}} />
					<div>
						<span class="text-sm font-medium text-[var(--text-strong)]">${t("settings:memory.sessionExport")}</span>
						<p class="text-xs text-[var(--muted)]" style="margin:2px 0 0;">
							${t("settings:memory.sessionExportDescription")}
						</p>
					</div>
				</label>
			</div>

			<div style="display:flex;align-items:center;gap:8px;padding-top:8px;border-top:1px solid var(--border);">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? t("common:actions.saving") : t("common:actions.save")}
				</button>
				${saved ? html`<span class="text-xs" style="color:var(--accent);">${t("common:actions.saved")}</span>` : null}
				${error ? html`<span class="text-xs" style="color:var(--error);">${error}</span>` : null}
			</div>
		</form>
	</div>`;
}

// ── Notifications section ─────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Notifications section handles multiple states and conditions
function NotificationsSection() {
	var [supported, setSupported] = useState(false);
	var [permission, setPermission] = useState("default");
	var [subscribed, setSubscribed] = useState(false);
	var [isLoading, setIsLoading] = useState(true);
	var [toggling, setToggling] = useState(false);
	var [error, setError] = useState(null);
	var [serverStatus, setServerStatus] = useState(null);

	async function checkStatus() {
		setIsLoading(true);
		rerender();

		var pushSupported = push.isPushSupported();
		setSupported(pushSupported);

		if (pushSupported) {
			setPermission(push.getPermissionState());
			await push.initPushState();
			setSubscribed(push.isSubscribed());

			// Check server status
			var status = await push.getPushStatus();
			setServerStatus(status);
		}

		setIsLoading(false);
		rerender();
	}

	async function refreshStatus() {
		var status = await push.getPushStatus();
		setServerStatus(status);
		rerender();
	}

	async function onRemoveSubscription(endpoint) {
		var result = await push.removeSubscription(endpoint);
		if (!result.success) {
			setError(result.error || "Failed to remove subscription");
			rerender();
		}
		// The WebSocket event will trigger refreshStatus automatically
	}

	useEffect(() => {
		checkStatus();
		// Listen for subscription changes via WebSocket
		var off = onEvent("push.subscriptions", () => {
			refreshStatus();
		});
		return off;
	}, []);

	async function onToggle() {
		setError(null);
		setToggling(true);
		rerender();

		var result = subscribed ? await push.unsubscribeFromPush() : await push.subscribeToPush();

		if (result.success) {
			setSubscribed(!subscribed);
			if (!subscribed) setPermission("granted");
		} else {
			setError(result.error || (subscribed ? "Failed to unsubscribe" : "Failed to subscribe"));
		}

		setToggling(false);
		rerender();
	}

	if (isLoading) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:notifications.title")}</h2>
			<div class="text-xs text-[var(--muted)]">${t("common:status.loading")}</div>
		</div>`;
	}

	if (!supported) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:notifications.title")}</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--surface);">
				<p class="text-sm text-[var(--text)]" style="margin:0;">
					${t("settings:notifications.notSupported")}
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${t("settings:notifications.trySupportedBrowser")}
				</p>
			</div>
		</div>`;
	}

	if (serverStatus === null) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:notifications.title")}</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--surface);">
				<p class="text-sm text-[var(--text)]" style="margin:0;">
					${t("settings:notifications.notConfigured")}
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${t("settings:notifications.featureNotBuilt")}
				</p>
			</div>
		</div>`;
	}

	// Check if running as installed PWA - push notifications require installation on Safari
	var standalone = isStandalone();
	var needsInstall = !standalone && /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent);

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">${t("settings:notifications.title")}</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			${t("settings:notifications.description")}
		</p>

		<!-- Push notifications toggle -->
		<div style="max-width:600px;">
			<div class="provider-item" style="margin-bottom:0;">
				<div style="flex:1;min-width:0;">
					<div class="provider-item-name" style="font-size:.9rem;">${t("settings:notifications.pushNotifications")}</div>
					<div style="font-size:.75rem;color:var(--muted);margin-top:2px;">
						${
							needsInstall
								? t("settings:notifications.addToDock")
								: subscribed
									? t("settings:notifications.willReceive")
									: permission === "denied"
										? t("settings:notifications.blocked")
										: t("settings:notifications.enableHint")
						}
					</div>
				</div>
				<button
					class="provider-btn ${subscribed ? "provider-btn-danger" : ""}"
					onClick=${onToggle}
					disabled=${toggling || permission === "denied" || needsInstall}
				>
					${toggling ? t("settings:notifications.toggling") : subscribed ? t("common:actions.disable") : t("common:actions.enable")}
				</button>
			</div>
			${error ? html`<div class="text-xs" style="margin-top:8px;color:var(--error);">${error}</div>` : null}
		</div>

		<!-- Install required notice -->
		${
			needsInstall
				? html`
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--surface);">
				<p class="text-sm text-[var(--text)]" style="margin:0;font-weight:500;">
					${t("settings:notifications.installRequired")}
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${t("settings:notifications.installRequiredSafari")}
				</p>
			</div>
		`
				: null
		}

		<!-- Permission status -->
		${
			permission === "denied" && !needsInstall
				? html`
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--error);background:color-mix(in srgb, var(--error) 5%, transparent);">
				<p class="text-sm" style="color:var(--error);margin:0;font-weight:500;">
					${t("settings:notifications.notificationsBlocked")}
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${t("settings:notifications.blockedExplanation")}
				</p>
			</div>
		`
				: null
		}

		<!-- Subscribed devices -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;margin-top:8px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">
				${t("settings:notifications.subscribedDevices", { count: serverStatus?.subscription_count || 0 })}
			</h3>
			${
				serverStatus?.subscriptions?.length > 0
					? html`<div style="display:flex;flex-direction:column;gap:6px;">
					${serverStatus.subscriptions.map(
						(sub) => html`<div class="provider-item" style="margin-bottom:0;" key=${sub.endpoint}>
						<div style="flex:1;min-width:0;">
							<div class="provider-item-name" style="font-size:.85rem;">${sub.device}</div>
							<div style="font-size:.7rem;color:var(--muted);margin-top:2px;display:flex;gap:12px;flex-wrap:wrap;">
								${sub.ip ? html`<span style="font-family:var(--font-mono);">${sub.ip}</span>` : null}
								<time datetime=${sub.created_at}>${new Date(sub.created_at).toLocaleDateString()}</time>
							</div>
						</div>
						<button
							class="provider-btn provider-btn-danger"
							onClick=${() => onRemoveSubscription(sub.endpoint)}
						>
							${t("common:actions.remove")}
						</button>
					</div>`,
					)}
				</div>`
					: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0;">${t("settings:notifications.noDevicesYet")}</div>`
			}
		</div>
	</div>`;
}

// ── Page-section init/teardown map ──────────────────────────

var pageSectionHandlers = {
	crons: {
		init: (container) => initCrons(container, null, { syncRoute: false }),
		teardown: teardownCrons,
	},
	providers: { init: initProviders, teardown: teardownProviders },
	channels: { init: initChannels, teardown: teardownChannels },
	mcp: { init: initMcp, teardown: teardownMcp },
	hooks: { init: initHooks, teardown: teardownHooks },
	skills: { init: initSkills, teardown: teardownSkills },
	sandboxes: { init: initImages, teardown: teardownImages },
	monitoring: {
		init: (container) => initMonitoring(container, null, { syncPath: false }),
		teardown: teardownMonitoring,
	},
	logs: { init: initLogs, teardown: teardownLogs },
};

/** Wrapper that mounts a page init/teardown pair into a ref div. */
function PageSection({ initFn, teardownFn }) {
	var ref = useRef(null);
	useEffect(() => {
		if (ref.current) initFn(ref.current);
		return () => {
			if (teardownFn) teardownFn();
		};
	}, []);
	return html`<div
		ref=${ref}
		class="flex-1 flex flex-col min-w-0 overflow-hidden"
	/>`;
}

// ── Main layout ──────────────────────────────────────────────

function SettingsPage() {
	useEffect(() => {
		fetchIdentity();
	}, []);

	var section = activeSection.value;
	var ps = pageSectionHandlers[section];

	return html`<div class="settings-layout">
		<${SettingsSidebar} />
		${ps ? html`<${PageSection} key=${section} initFn=${ps.init} teardownFn=${ps.teardown} />` : null}
		${section === "identity" ? html`<${IdentitySection} />` : null}
		${section === "memory" ? html`<${MemorySection} />` : null}
		${section === "environment" ? html`<${EnvironmentSection} />` : null}
		${section === "security" ? html`<${SecuritySection} />` : null}
		${section === "tailscale" ? html`<${TailscaleSection} />` : null}
		${section === "voice" ? html`<${VoiceSection} />` : null}
		${section === "notifications" ? html`<${NotificationsSection} />` : null}
		${section === "config" ? html`<${ConfigSection} />` : null}
	</div>`;
}

var DEFAULT_SECTION = "identity";

registerPrefix(
	routes.settings,
	(container, param) => {
		mounted = true;
		containerRef = container;
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		var isValidSection = param && getSectionItems().some((s) => s.id === param);
		var section = isValidSection ? param : DEFAULT_SECTION;
		activeSection.value = section;
		if (!isValidSection) {
			history.replaceState(null, "", settingsPath(section));
		}
		render(html`<${SettingsPage} />`, container);
		fetchIdentity();
	},
	() => {
		mounted = false;
		if (containerRef) render(null, containerRef);
		containerRef = null;
		identity.value = null;
		loading.value = true;
		activeSection.value = DEFAULT_SECTION;
	},
);
