// ── Settings page (Preact + HTM + Signals) ───────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { onEvent } from "./events.js";
import { refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import * as push from "./push.js";
import { isStandalone } from "./pwa.js";
import { navigate, registerPrefix } from "./router.js";
import * as S from "./state.js";

var identity = signal(null);
var loading = signal(true);
var activeSection = signal("identity");
var mounted = false;
var containerRef = null;

function rerender() {
	if (containerRef) render(html`<${SettingsPage} />`, containerRef);
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
	{
		id: "identity",
		label: "Identity",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z"/></svg>`,
	},
	{
		id: "memory",
		label: "Memory",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125"/></svg>`,
	},
	{
		id: "environment",
		label: "Environment",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="m6.75 7.5 3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0 0 21 18V6a2.25 2.25 0 0 0-2.25-2.25H5.25A2.25 2.25 0 0 0 3 6v12a2.25 2.25 0 0 0 2.25 2.25Z"/></svg>`,
	},
	{
		id: "security",
		label: "Security",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 1 0-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 0 0 2.25-2.25v-6.75a2.25 2.25 0 0 0-2.25-2.25H6.75a2.25 2.25 0 0 0-2.25 2.25v6.75a2.25 2.25 0 0 0 2.25 2.25Z"/></svg>`,
	},
	{
		id: "tailscale",
		label: "Tailscale",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5a17.92 17.92 0 0 1-8.716-2.247m0 0A8.966 8.966 0 0 1 3 12c0-1.264.26-2.466.73-3.558"/></svg>`,
	},
	{
		id: "notifications",
		label: "Notifications",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M14.857 17.082a23.848 23.848 0 0 0 5.454-1.31A8.967 8.967 0 0 1 18 9.75V9A6 6 0 0 0 6 9v.75a8.967 8.967 0 0 1-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 0 1-5.714 0m5.714 0a3 3 0 1 1-5.714 0"/></svg>`,
	},
];

function SettingsSidebar() {
	return html`<div class="settings-sidebar">
		<div class="settings-sidebar-nav">
			${sections.map(
				(s) => html`
				<button
					key=${s.id}
					class="settings-nav-item ${activeSection.value === s.id ? "active" : ""}"
					onClick=${() => {
						navigate(`/settings/${s.id}`);
					}}
				>
					${s.icon}
					${s.label}
				</button>
			`,
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

	var [name, setName] = useState(id?.name || "");
	var [emoji, setEmoji] = useState(id?.emoji || "");
	var [creature, setCreature] = useState(id?.creature || "");
	var [vibe, setVibe] = useState(id?.vibe || "");
	var [userName, setUserName] = useState(id?.user_name || "");
	var [soul, setSoul] = useState(id?.soul || "");
	var [saving, setSaving] = useState(false);
	var [saved, setSaved] = useState(false);
	var [error, setError] = useState(null);

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

	if (loading.value) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div class="text-xs text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	function onSave(e) {
		e.preventDefault();
		if (!(name.trim() || userName.trim())) {
			setError("Agent name and your name are required.");
			return;
		}
		if (!name.trim()) {
			setError("Agent name is required.");
			return;
		}
		if (!userName.trim()) {
			setError("Your name is required.");
			return;
		}
		setError(null);
		setSaving(true);
		setSaved(false);

		sendRpc("agent.identity.update", {
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
				refreshGon();
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

	function onResetSoul() {
		setSoul("");
		rerender();
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Identity</h2>
		${
			isNew
				? html`<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
				Welcome! Set up your agent's identity to get started.
			</p>`
				: null
		}
		<form onSubmit=${onSave} style="max-width:600px;display:flex;flex-direction:column;gap:16px;">
			<!-- Agent section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Agent</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">Saved to <code>IDENTITY.md</code> in your workspace root.</p>
				<div style="display:grid;grid-template-columns:1fr 1fr;gap:8px 16px;">
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Name *</div>
						<input type="text" class="provider-key-input" style="width:100%;"
							value=${name} onInput=${(e) => setName(e.target.value)}
							placeholder="e.g. Rex" />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Emoji</div>
						<${EmojiPicker} value=${emoji} onChange=${setEmoji} />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Creature</div>
						<input type="text" class="provider-key-input" style="width:100%;"
							value=${creature} onInput=${(e) => setCreature(e.target.value)}
							placeholder="e.g. dog" />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Vibe</div>
						<input type="text" class="provider-key-input" style="width:100%;"
							value=${vibe} onInput=${(e) => setVibe(e.target.value)}
							placeholder="e.g. chill" />
					</div>
				</div>
			</div>

			<!-- User section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">User</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">Saved to <code>USER.md</code> in your workspace root.</p>
				<div>
					<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Your name *</div>
					<input type="text" class="provider-key-input" style="width:100%;max-width:280px;"
						value=${userName} onInput=${(e) => setUserName(e.target.value)}
						placeholder="e.g. Alice" />
				</div>
			</div>

			<!-- Soul section -->
			<div>
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:4px;">Soul</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">Personality and tone injected into every conversation. Saved to <code>SOUL.md</code> in your workspace root. Leave empty for the default.</p>
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
							onClick=${onResetSoul}>Reset to default</button>`
						: null
				}
			</div>

			<div style="display:flex;align-items:center;gap:8px;">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Saving\u2026" : "Save"}
				</button>
				${saved ? html`<span class="text-xs" style="color:var(--accent);">Saved</span>` : null}
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

	function fetchEnvVars() {
		fetch("/api/env")
			.then((r) => (r.ok ? r.json() : { env_vars: [] }))
			.then((d) => {
				setEnvVars(d.env_vars || []);
				setEnvLoading(false);
				rerender();
			})
			.catch(() => {
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
			setEnvErr("Key is required.");
			rerender();
			return;
		}
		if (!/^[A-Za-z0-9_]+$/.test(key)) {
			setEnvErr("Key must contain only letters, digits, and underscores.");
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
					setEnvMsg("Variable saved.");
					setTimeout(() => {
						setEnvMsg(null);
						rerender();
					}, 2000);
					fetchEnvVars();
				} else {
					return r.json().then((d) => setEnvErr(d.error || "Failed to save"));
				}
				setSaving(false);
				rerender();
			})
			.catch((err) => {
				setEnvErr(err.message);
				setSaving(false);
				rerender();
			});
	}

	function onDelete(id) {
		fetch(`/api/env/${id}`, { method: "DELETE" }).then(() => fetchEnvVars());
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
		fetch("/api/env", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ key, value: updateValue }),
		}).then((r) => {
			if (r.ok) {
				setUpdateId(null);
				setUpdateValue("");
				fetchEnvVars();
			}
		});
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Environment Variables</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			Environment variables are injected into sandbox command execution. Values are write-only and never displayed.
		</p>

		${
			envLoading
				? html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`
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
									<input type="password" class="provider-key-input" value=${updateValue}
										onInput=${(e) => setUpdateValue(e.target.value)}
										placeholder="New value" style="flex:1" autofocus />
									<button type="submit" class="provider-btn">Save</button>
									<button type="button" class="provider-btn" onClick=${onCancelUpdate}>Cancel</button>
								</form>`
								: html`<div style="flex:1;min-width:0;">
									<div class="provider-item-name" style="font-family:var(--font-mono);font-size:.8rem;">${v.key}</div>
									<div style="font-size:.7rem;color:var(--muted);margin-top:2px;display:flex;gap:12px;">
										<span>\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022</span>
										<time datetime=${v.updated_at}>${v.updated_at}</time>
									</div>
								</div>
								<div style="display:flex;gap:4px;">
									<button class="provider-btn" onClick=${() => onStartUpdate(v.id)}>Update</button>
									<button class="provider-btn provider-btn-danger"
										onClick=${() => onDelete(v.id)}>Delete</button>
								</div>`
						}
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">No environment variables set.</div>`
				}
			</div>

			<!-- Add variable -->
			<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Add Variable</h3>
				<form onSubmit=${onAdd}>
					<div style="display:flex;gap:8px;flex-wrap:wrap;">
						<input type="text" class="provider-key-input" value=${newKey}
							onInput=${(e) => setNewKey(e.target.value)}
							placeholder="KEY_NAME" style="flex:1;min-width:120px;font-family:var(--font-mono);font-size:.8rem;" />
						<input type="password" class="provider-key-input" value=${newValue}
							onInput=${(e) => setNewValue(e.target.value)}
							placeholder="Value" style="flex:2;min-width:200px;" />
						<button type="submit" class="provider-btn" disabled=${saving || !newKey.trim()}>
							${saving ? "Saving\u2026" : "Add"}
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

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d) => {
				if (d?.auth_disabled) setAuthDisabled(true);
				if (d?.localhost_only) setLocalhostOnly(true);
				if (d?.has_password === false) setHasPassword(false);
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
			setPwErr("New password must be at least 8 characters.");
			return;
		}
		if (newPw !== confirmPw) {
			setPwErr("Passwords do not match.");
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
				if (r.ok) {
					setPwMsg(hasPassword ? "Password changed." : "Password set.");
					setCurPw("");
					setNewPw("");
					setConfirmPw("");
					setHasPassword(true);
				} else return r.text().then((t) => setPwErr(t));
				setPwSaving(false);
				rerender();
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
			setPkMsg(`Passkeys require a domain name. Use localhost instead of ${location.hostname}`);
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
					name: pkName.trim() || "Passkey",
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
					return fetch("/api/auth/passkeys")
						.then((r2) => r2.json())
						.then((d) => {
							setPasskeys(d.passkeys || []);
							setPkMsg("Passkey added.");
							rerender();
						});
				} else
					return r.text().then((t) => {
						setPkMsg(t);
						rerender();
					});
			})
			.catch((err) => {
				setPkMsg(err.message || "Failed to add passkey");
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
					return r.text().then((t) => {
						setPwErr(t);
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
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Security</h2>
			<div class="text-xs text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	if (authDisabled) {
		var isScary = !localhostOnly;
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Security</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--error);background:color-mix(in srgb, var(--error) 5%, transparent);">
				<strong style="color:var(--error);">Authentication is disabled</strong>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					${
						isScary
							? "Anyone with network access can control moltis and your computer. Set up a password to protect your instance."
							: "Authentication has been removed. While localhost-only access is safe, you should set up a password before exposing moltis to the network."
					}
				</p>
				<button type="button" class="provider-btn" style="margin-top:10px;"
					onClick=${() => {
						navigate("/onboarding");
					}}>Set up authentication</button>
			</div>
		</div>`;
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Security</h2>

		${
			localhostOnly && !hasPassword
				? html`<div class="alert-info-text max-w-form">
					<span class="alert-label-info">Note:</span>${" "}
					Moltis is running on localhost, so you have full access without a password.
					Set a password before exposing moltis to the network.
				</div>`
				: null
		}

		<!-- Password -->
		<div style="max-width:600px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">${hasPassword ? "Change Password" : "Set Password"}</h3>
			<form onSubmit=${onChangePw}>
				<div style="display:flex;flex-direction:column;gap:8px;margin-bottom:10px;">
					${
						hasPassword
							? html`<div>
								<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Current password</div>
								<input type="password" class="provider-key-input" style="width:100%;" value=${curPw}
									onInput=${(e) => setCurPw(e.target.value)} />
							</div>`
							: null
					}
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">${hasPassword ? "New password" : "Password"}</div>
						<input type="password" class="provider-key-input" style="width:100%;" value=${newPw}
							onInput=${(e) => setNewPw(e.target.value)} placeholder="At least 8 characters" />
					</div>
					<div>
						<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Confirm ${hasPassword ? "new " : ""}password</div>
						<input type="password" class="provider-key-input" style="width:100%;" value=${confirmPw}
							onInput=${(e) => setConfirmPw(e.target.value)} />
					</div>
				</div>
				<div style="display:flex;align-items:center;gap:8px;">
					<button type="submit" class="provider-btn" disabled=${pwSaving}>
						${pwSaving ? (hasPassword ? "Changing\u2026" : "Setting\u2026") : hasPassword ? "Change password" : "Set password"}
					</button>
					${pwMsg ? html`<span class="text-xs" style="color:var(--accent);">${pwMsg}</span>` : null}
					${pwErr ? html`<span class="text-xs" style="color:var(--error);">${pwErr}</span>` : null}
				</div>
			</form>
		</div>

		<!-- Passkeys -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Passkeys</h3>
			${
				pkLoading
					? html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`
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
									<button type="submit" class="provider-btn">Save</button>
									<button type="button" class="provider-btn" onClick=${onCancelRename}>Cancel</button>
								</form>`
								: html`<div style="flex:1;min-width:0;">
									<div class="provider-item-name" style="font-size:.85rem;">${pk.name}</div>
									<div style="font-size:.7rem;color:var(--muted);margin-top:2px;">${pk.created_at}</div>
								</div>
								<div style="display:flex;gap:4px;">
									<button class="provider-btn" onClick=${() => onStartRename(pk.id, pk.name)}>Rename</button>
									<button class="provider-btn provider-btn-danger"
										onClick=${() => onRemovePasskey(pk.id)}>Remove</button>
								</div>`
						}
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0 12px;">No passkeys registered.</div>`
				}
				<div style="display:flex;gap:8px;align-items:center;">
					<input type="text" class="provider-key-input" value=${pkName}
						onInput=${(e) => setPkName(e.target.value)}
						placeholder="Passkey name (e.g. MacBook Touch ID)" style="flex:1" />
					<button type="button" class="provider-btn" onClick=${onAddPasskey}>Add passkey</button>
				</div>
				${pkMsg ? html`<div class="text-xs text-[var(--muted)]" style="margin-top:6px;">${pkMsg}</div>` : null}
			`
			}
		</div>

		<!-- API Keys -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:4px;">API Keys</h3>
			<p class="text-xs text-[var(--muted)] leading-relaxed" style="margin:0 0 12px;">
				API keys authenticate external tools and scripts connecting to moltis over the WebSocket protocol. Pass the key as the <code style="font-family:var(--font-mono);font-size:.75rem;">api_key</code> field in the <code style="font-family:var(--font-mono);font-size:.75rem;">auth</code> object of the <code style="font-family:var(--font-mono);font-size:.75rem;">connect</code> handshake.
			</p>
			${
				akLoading
					? html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`
					: html`
				${
					akNew
						? html`<div style="margin-bottom:12px;padding:10px 12px;background:var(--bg);border:1px solid var(--border);border-radius:6px;">
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:4px;">Copy this key now. It won't be shown again.</div>
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
								<span>${ak.created_at}</span>
								${ak.scopes ? html`<span style="color:var(--accent);">${ak.scopes.join(", ")}</span>` : html`<span style="color:var(--accent);">Full access</span>`}
							</div>
						</div>
						<button class="provider-btn provider-btn-danger"
							onClick=${() => onRevokeApiKey(ak.id)}>Revoke</button>
					</div>`,
					)}
				</div>`
						: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0 12px;">No API keys.</div>`
				}
				<div style="display:flex;flex-direction:column;gap:10px;">
					<div style="display:flex;gap:8px;align-items:center;">
						<input type="text" class="provider-key-input" value=${akLabel}
							onInput=${(e) => setAkLabel(e.target.value)}
							placeholder="Key label (e.g. CLI tool)" style="flex:1" />
					</div>
					<div>
						<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
							<input type="checkbox" checked=${akFullAccess}
								onChange=${() => {
									setAkFullAccess(!akFullAccess);
									rerender();
								}} />
							<span class="text-xs text-[var(--text)]">Full access (all permissions)</span>
						</label>
					</div>
					${
						akFullAccess
							? null
							: html`<div style="padding-left:20px;display:flex;flex-direction:column;gap:6px;">
							<div class="text-xs text-[var(--muted)]" style="margin-bottom:2px;">Select permissions:</div>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.read"]}
									onChange=${() => toggleScope("operator.read")} />
								<span class="text-xs text-[var(--text)]">operator.read</span>
								<span class="text-xs text-[var(--muted)]">\u2014 View data and status</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.write"]}
									onChange=${() => toggleScope("operator.write")} />
								<span class="text-xs text-[var(--text)]">operator.write</span>
								<span class="text-xs text-[var(--muted)]">\u2014 Create, update, delete</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.approvals"]}
									onChange=${() => toggleScope("operator.approvals")} />
								<span class="text-xs text-[var(--text)]">operator.approvals</span>
								<span class="text-xs text-[var(--muted)]">\u2014 Handle exec approvals</span>
							</label>
							<label style="display:flex;align-items:center;gap:6px;cursor:pointer;">
								<input type="checkbox" checked=${akScopes["operator.pairing"]}
									onChange=${() => toggleScope("operator.pairing")} />
								<span class="text-xs text-[var(--text)]">operator.pairing</span>
								<span class="text-xs text-[var(--muted)]">\u2014 Device/node pairing</span>
							</label>
						</div>`
					}
					<div>
						<button type="button" class="provider-btn" onClick=${onCreateApiKey}
							disabled=${!(akLabel.trim() && (akFullAccess || Object.values(akScopes).some((v) => v)))}>
							Generate key
						</button>
					</div>
				</div>
			`
			}
		</div>

		<!-- Danger zone -->
		<div style="max-width:600px;margin-top:8px;border-top:1px solid var(--error);padding-top:16px;">
			<h3 class="text-sm font-medium" style="color:var(--error);margin-bottom:8px;">Danger Zone</h3>
			<div style="padding:12px 16px;border:1px solid var(--error);border-radius:6px;background:color-mix(in srgb, var(--error) 5%, transparent);">
				<strong class="text-sm" style="color:var(--text-strong);">Remove all authentication</strong>
				<p class="text-xs text-[var(--muted)]" style="margin:6px 0 0;">
					If you know what you're doing, you can fully disable authentication.
					Anyone with network access will be able to access moltis and your computer.
					This removes your password, all passkeys, all API keys, and all sessions.
				</p>
				${
					resetConfirm
						? html`<div style="display:flex;align-items:center;gap:8px;margin-top:10px;">
						<span class="text-xs" style="color:var(--error);">Are you sure? This cannot be undone.</span>
						<button type="button" class="provider-btn provider-btn-danger" disabled=${resetBusy}
							onClick=${onResetAuth}>${resetBusy ? "Removing\u2026" : "Yes, remove all auth"}</button>
						<button type="button" class="provider-btn" onClick=${() => {
							setResetConfirm(false);
							rerender();
						}}>Cancel</button>
					</div>`
						: html`<button type="button" class="provider-btn provider-btn-danger" style="margin-top:10px;"
						onClick=${onResetAuth}>Remove all authentication</button>`
				}
			</div>
		</div>
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
					setTsError("Tailscale feature is not enabled. Rebuild with --features tailscale.");
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
			cfgMsg.textContent = `Configuring tailscale ${configuringMode}\u2026 This can take up to 10 seconds.`;
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
			loadEl.textContent = "Loading\u2026 this can take a few seconds.";
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
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Tailscale</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
			Expose the gateway via Tailscale Serve (tailnet-only HTTPS) or Funnel
			(public HTTPS). The gateway stays bound to localhost; Tailscale proxies
			traffic to it.
		</p>
	</div>`;
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
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Memory</h2>
			<div class="text-xs text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	var qmdFeatureEnabled = memConfig?.qmd_feature_enabled !== false;
	var qmdAvailable = qmdStatus?.available === true;

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Memory</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
			Configure how the agent stores and retrieves long-term memory. Memory enables the agent
			to recall past conversations, notes, and context across sessions.
		</p>

		<!-- Status -->
		${
			memStatus
				? html`
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--bg);">
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Status</h3>
				<div style="display:grid;grid-template-columns:repeat(2,1fr);gap:8px 16px;font-size:.8rem;">
					<div>
						<span class="text-[var(--muted)]">Files:</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.total_files || 0}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">Chunks:</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.total_chunks || 0}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">Model:</span>
						<span class="text-[var(--text)]" style="margin-left:6px;font-family:var(--font-mono);font-size:.75rem;">${memStatus.embedding_model || "none"}</span>
					</div>
					<div>
						<span class="text-[var(--muted)]">DB Size:</span>
						<span class="text-[var(--text)]" style="margin-left:6px;">${memStatus.db_size_display || "0 B"}</span>
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
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Backend</h3>

				<!-- Comparison table -->
				<div style="margin-bottom:12px;padding:12px;border-radius:6px;border:1px solid var(--border);background:var(--bg);font-size:.75rem;">
					<table style="width:100%;border-collapse:collapse;">
						<thead>
							<tr style="border-bottom:1px solid var(--border);">
								<th style="text-align:left;padding:4px 8px 8px 0;color:var(--muted);font-weight:500;">Feature</th>
								<th style="text-align:center;padding:4px 8px 8px;color:var(--muted);font-weight:500;">Built-in</th>
								<th style="text-align:center;padding:4px 8px 8px;color:var(--muted);font-weight:500;">QMD</th>
							</tr>
						</thead>
						<tbody>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">Search type</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">FTS5 + vector</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">BM25 + vector + LLM</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">External dependency</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">None</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">Node.js/Bun</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">Embedding cache</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">\u2713</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">OpenAI batch API</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">\u2713 (50% cheaper)</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">Provider fallback</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">\u2713</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">\u2717</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">LLM reranking</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">Optional</td>
								<td style="padding:6px 8px;text-align:center;color:var(--accent);">Built-in</td>
							</tr>
							<tr>
								<td style="padding:6px 8px 6px 0;color:var(--text);">Best for</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">Most users</td>
								<td style="padding:6px 8px;text-align:center;color:var(--muted);">Power users</td>
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
						Built-in (Recommended)
					</button>
					<button type="button"
						class="provider-btn ${backend === "qmd" ? "" : "provider-btn-secondary"}"
						disabled=${!qmdFeatureEnabled}
						onClick=${() => {
							setBackend("qmd");
							rerender();
						}}>
						QMD
					</button>
				</div>

				${
					qmdFeatureEnabled
						? null
						: html`
					<div class="text-xs text-[var(--error)]" style="margin-top:8px;">
						QMD feature is not enabled. Rebuild moltis with <code style="font-family:var(--font-mono);font-size:.7rem;">--features qmd</code>
					</div>
				`
				}

				${
					backend === "qmd"
						? html`
					<div style="margin-top:12px;padding:12px;border-radius:6px;border:1px solid var(--border);background:var(--bg);">
						<h4 class="text-xs font-medium text-[var(--text-strong)]" style="margin:0 0 8px;">QMD Status</h4>
						${
							qmdAvailable
								? html`
							<div class="text-xs" style="color:var(--accent);display:flex;align-items:center;gap:6px;">
								<span>\u2713</span> QMD is installed ${qmdStatus?.version ? html`<span class="text-[var(--muted)]">(${qmdStatus.version})</span>` : null}
							</div>
						`
								: html`
							<div class="text-xs" style="color:var(--error);margin-bottom:8px;">
								\u2717 QMD is not installed or not found in PATH
							</div>
							<div class="text-xs text-[var(--muted)]" style="line-height:1.6;">
								<strong style="color:var(--text);">Installation:</strong><br/>
								<code style="font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">npm install -g @anthropic/qmd</code>
								<span style="margin:0 4px;">or</span>
								<code style="font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">bun install -g @anthropic/qmd</code>
								<br/><br/>
								Then start the QMD daemon:
								<code style="display:block;margin-top:4px;font-family:var(--font-mono);font-size:.7rem;background:var(--surface);padding:2px 4px;border-radius:3px;">qmd daemon</code>
								<br/>
								<a href="https://github.com/anthropics/qmd" target="_blank" rel="noopener"
									style="color:var(--accent);">View documentation \u2192</a>
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
				<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">Citations</h3>
				<p class="text-xs text-[var(--muted)]" style="margin:0 0 8px;">
					Include source file and line number with search results to help track where information comes from.
				</p>
				<select class="provider-key-input" style="width:auto;min-width:150px;"
					value=${citations} onChange=${(e) => {
						setCitations(e.target.value);
						rerender();
					}}>
					<option value="auto">Auto (multi-file only)</option>
					<option value="on">Always</option>
					<option value="off">Never</option>
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
						<span class="text-sm font-medium text-[var(--text-strong)]">LLM Reranking</span>
						<p class="text-xs text-[var(--muted)]" style="margin:2px 0 0;">
							Use the LLM to rerank search results for better relevance (slower but more accurate).
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
						<span class="text-sm font-medium text-[var(--text-strong)]">Session Export</span>
						<p class="text-xs text-[var(--muted)]" style="margin:2px 0 0;">
							Export session transcripts to memory for cross-run recall of past conversations.
						</p>
					</div>
				</label>
			</div>

			<div style="display:flex;align-items:center;gap:8px;padding-top:8px;border-top:1px solid var(--border);">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Saving\u2026" : "Save"}
				</button>
				${saved ? html`<span class="text-xs" style="color:var(--accent);">Saved</span>` : null}
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
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
			<div class="text-xs text-[var(--muted)]">Loading…</div>
		</div>`;
	}

	if (!supported) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--surface);">
				<p class="text-sm text-[var(--text)]" style="margin:0;">
					Push notifications are not supported in this browser.
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					Try using Safari, Chrome, or Firefox on a device that supports web push.
				</p>
			</div>
		</div>`;
	}

	if (serverStatus === null) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
			<div style="max-width:600px;padding:12px 16px;border-radius:6px;border:1px solid var(--border);background:var(--surface);">
				<p class="text-sm text-[var(--text)]" style="margin:0;">
					Push notifications are not configured on the server.
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					The server was built without the <code style="font-family:var(--font-mono);font-size:.75rem;">push-notifications</code> feature.
				</p>
			</div>
		</div>`;
	}

	// Check if running as installed PWA - push notifications require installation on Safari
	var standalone = isStandalone();
	var needsInstall = !standalone && /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent);

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			Receive push notifications when the agent completes a task or needs your attention.
		</p>

		<!-- Push notifications toggle -->
		<div style="max-width:600px;">
			<div class="provider-item" style="margin-bottom:0;">
				<div style="flex:1;min-width:0;">
					<div class="provider-item-name" style="font-size:.9rem;">Push Notifications</div>
					<div style="font-size:.75rem;color:var(--muted);margin-top:2px;">
						${
							needsInstall
								? "Add this app to your Dock to enable notifications."
								: subscribed
									? "You will receive notifications on this device."
									: permission === "denied"
										? "Notifications are blocked. Enable them in browser settings."
										: "Enable to receive notifications on this device."
						}
					</div>
				</div>
				<button
					class="provider-btn ${subscribed ? "provider-btn-danger" : ""}"
					onClick=${onToggle}
					disabled=${toggling || permission === "denied" || needsInstall}
				>
					${toggling ? "…" : subscribed ? "Disable" : "Enable"}
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
					Installation required
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					On Safari, push notifications are only available for installed apps. Add moltis to your Dock using <strong>File → Add to Dock</strong> (or Share → Add to Dock on iOS), then open it from there.
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
					Notifications are blocked
				</p>
				<p class="text-xs text-[var(--muted)]" style="margin:8px 0 0;">
					You previously blocked notifications for this site. To enable them, you'll need to update your browser's site settings and allow notifications for this origin.
				</p>
			</div>
		`
				: null
		}

		<!-- Subscribed devices -->
		<div style="max-width:600px;border-top:1px solid var(--border);padding-top:16px;margin-top:8px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]" style="margin-bottom:8px;">
				Subscribed Devices (${serverStatus?.subscription_count || 0})
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
							Remove
						</button>
					</div>`,
					)}
				</div>`
					: html`<div class="text-xs text-[var(--muted)]" style="padding:4px 0;">No devices subscribed yet.</div>`
			}
		</div>
	</div>`;
}

// ── Main layout ──────────────────────────────────────────────

function SettingsPage() {
	useEffect(() => {
		fetchIdentity();
	}, []);

	var section = activeSection.value;

	return html`<div class="settings-layout">
		<${SettingsSidebar} />
		${section === "identity" ? html`<${IdentitySection} />` : null}
		${section === "memory" ? html`<${MemorySection} />` : null}
		${section === "environment" ? html`<${EnvironmentSection} />` : null}
		${section === "security" ? html`<${SecuritySection} />` : null}
		${section === "tailscale" ? html`<${TailscaleSection} />` : null}
		${section === "notifications" ? html`<${NotificationsSection} />` : null}
	</div>`;
}

registerPrefix(
	"/settings",
	(container, param) => {
		mounted = true;
		containerRef = container;
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		var isValidSection = param && sections.some((s) => s.id === param);
		var section = isValidSection ? param : "identity";
		activeSection.value = section;
		if (!isValidSection) {
			history.replaceState(null, "", `/settings/${section}`);
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
		activeSection.value = "identity";
	},
);
