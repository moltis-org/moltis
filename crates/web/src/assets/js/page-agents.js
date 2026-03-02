// ── Settings > Agents page (Preact + HTM) ─────────────────
//
// CRUD UI for agent personas. "main" agent links to the
// Identity settings section and cannot be deleted.

import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { EmojiPicker } from "./emoji-picker.js";
import { refresh as refreshGon } from "./gon.js";
import { parseAgentsListPayload, sendRpc } from "./helpers.js";
import { navigate } from "./router.js";
import { settingsPath } from "./routes.js";
import { fetchSessions } from "./sessions.js";
import { confirmDialog } from "./ui.js";

var WS_RETRY_LIMIT = 75;
var WS_RETRY_DELAY_MS = 200;

var _mounted = false;
var containerRef = null;

export function initAgents(container, subPath) {
	_mounted = true;
	containerRef = container;
	render(html`<${AgentsPage} subPath=${subPath} />`, container);
}

export function teardownAgents() {
	_mounted = false;
	if (containerRef) render(null, containerRef);
	containerRef = null;
}

// ── Create / Edit form ──────────────────────────────────────

function AgentForm({ agent, onSave, onCancel }) {
	var isEdit = !!agent;
	var [id, setId] = useState(agent?.id || "");
	var [name, setName] = useState(agent?.name || "");
	var [emoji, setEmoji] = useState(agent?.emoji || "");
	var [theme, setTheme] = useState(agent?.theme || "");
	var [soul, setSoul] = useState("");
	var [saving, setSaving] = useState(false);
	var [error, setError] = useState(null);

	// Load soul: for edits fetch the agent's soul, for new agents fetch main's soul as default
	useEffect(() => {
		var agentId = isEdit ? agent.id : "main";
		var attempts = 0;
		function load() {
			sendRpc("agents.identity.get", { agent_id: agentId }).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				if (res?.ok && res.payload?.soul) {
					setSoul(res.payload.soul);
				}
			});
		}
		load();
	}, [isEdit, agent?.id]);

	function buildParams() {
		var base = {
			name: name.trim(),
			emoji: emoji.trim() || null,
			theme: theme.trim() || null,
		};
		base.id = isEdit ? agent.id : id.trim();
		return base;
	}

	function finishSave(agentId) {
		var trimmedSoul = soul.trim();
		if (trimmedSoul) {
			sendRpc("agents.identity.update_soul", { agent_id: agentId, soul: trimmedSoul }).then(() => {
				setSaving(false);
				refreshGon();
				onSave();
			});
		} else {
			setSaving(false);
			refreshGon();
			onSave();
		}
	}

	function onSubmit(e) {
		e.preventDefault();
		if (!name.trim()) {
			setError("Name is required.");
			return;
		}
		if (!(isEdit || id.trim())) {
			setError("ID is required.");
			return;
		}
		setError(null);
		setSaving(true);

		var method = isEdit ? "agents.update" : "agents.create";
		sendRpc(method, buildParams()).then((res) => {
			if (!res?.ok) {
				setSaving(false);
				setError(res?.error?.message || "Failed to save");
				return;
			}
			finishSave(isEdit ? agent.id : id.trim());
		});
	}

	return html`
		<form onSubmit=${onSubmit} class="flex flex-col gap-3" style="max-width:500px;">
			<h3 class="text-sm font-medium text-[var(--text-strong)]">
				${isEdit ? `Edit ${agent.name}` : "Create Agent"}
			</h3>

			${
				!isEdit &&
				html`
				<label class="flex flex-col gap-1">
					<span class="text-xs text-[var(--muted)]">ID (slug, cannot change later)</span>
					<input
						type="text"
						class="provider-key-input"
						value=${id}
						onInput=${(e) => setId(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ""))}
						placeholder="e.g. writer, coder, researcher"
						maxLength="50"
					/>
				</label>
			`
			}

			<label class="flex flex-col gap-1">
				<span class="text-xs text-[var(--muted)]">Name</span>
				<input
					type="text"
					class="provider-key-input"
					value=${name}
					onInput=${(e) => setName(e.target.value)}
					placeholder="Creative Writer"
				/>
			</label>

			<div class="flex flex-col gap-1">
				<span class="text-xs text-[var(--muted)]">Emoji</span>
				<${EmojiPicker} value=${emoji} onChange=${setEmoji} />
			</div>

			<label class="flex flex-col gap-1">
				<span class="text-xs text-[var(--muted)]">Theme</span>
				<input
					type="text"
					class="provider-key-input"
					value=${theme}
					onInput=${(e) => setTheme(e.target.value)}
					placeholder="wise owl, chill fox, witty robot\u2026"
				/>
			</label>

			<label class="flex flex-col gap-1">
				<span class="text-xs text-[var(--muted)]">Soul (system prompt personality)</span>
				<textarea
					class="provider-key-input"
					value=${soul}
					onInput=${(e) => setSoul(e.target.value)}
					placeholder="You are a creative writing assistant\u2026"
					rows="4"
					style="resize:vertical;font-family:var(--font-mono);font-size:0.75rem;"
				/>
			</label>

			${error && html`<span class="text-xs" style="color:var(--error);">${error}</span>`}

			<div class="flex gap-2">
				<button type="submit" class="provider-btn" disabled=${saving}>
					${saving ? "Saving\u2026" : isEdit ? "Save" : "Create"}
				</button>
				<button type="button" class="provider-btn provider-btn-secondary" onClick=${onCancel}>
					Cancel
				</button>
			</div>
		</form>
	`;
}

// ── Agent card ──────────────────────────────────────────────

function AgentCard({ agent, defaultId, onEdit, onDelete, onSetDefault }) {
	var isMain = agent.id === "main";
	var isDefault = !!agent.is_default || agent.id === defaultId;
	return html`
		<div class="backend-card">
			<div class="flex items-center justify-between">
				<div class="flex items-center gap-2">
					${agent.emoji && html`<span class="text-lg">${agent.emoji}</span>`}
					<span class="text-sm font-medium text-[var(--text-strong)]">${agent.name}</span>
					${isDefault && html`<span class="recommended-badge">Default</span>`}
				</div>
				<div class="flex gap-2">
					${
						isMain
							? html`<button
							class="provider-btn provider-btn-secondary"
							style="font-size:0.7rem;padding:3px 8px;"
							onClick=${() => navigate(settingsPath("identity"))}
						>Identity Settings</button>`
							: html`
							<button
								class="provider-btn provider-btn-secondary"
								style="font-size:0.7rem;padding:3px 8px;"
								onClick=${() => onEdit(agent)}
							>Edit</button>
							<button
								class="provider-btn provider-btn-danger"
								style="font-size:0.7rem;padding:3px 8px;"
								onClick=${() => onDelete(agent)}
							>Delete</button>
						`
					}
					${
						!isDefault &&
						html`
						<button
							class="provider-btn provider-btn-secondary"
							style="font-size:0.7rem;padding:3px 8px;"
							onClick=${() => onSetDefault(agent)}
						>Set Default</button>
					`
					}
				</div>
			</div>
			${
				agent.theme &&
				html`
				<div class="text-xs text-[var(--muted)] mt-1">
					${agent.theme}
				</div>
			`
			}
		</div>
	`;
}

// ── Main page ───────────────────────────────────────────────

function AgentsPage({ subPath }) {
	var [agents, setAgents] = useState([]);
	var [defaultId, setDefaultId] = useState("main");
	var [loading, setLoading] = useState(true);
	var [editing, setEditing] = useState(null); // null | "new" | AgentPersona
	var [error, setError] = useState(null);

	function fetchAgents() {
		setLoading(true);
		var attempts = 0;
		function load() {
			sendRpc("agents.list", {}).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				setLoading(false);
				if (res?.ok) {
					var parsed = parseAgentsListPayload(res.payload);
					setDefaultId(parsed.defaultId);
					setAgents(parsed.agents);
				} else {
					setError(res?.error?.message || "Failed to load agents");
				}
			});
		}
		load();
	}

	useEffect(() => {
		fetchAgents();
		// Auto-open create form when navigating to /settings/agents/new
		if (subPath === "new") {
			setEditing("new");
		}
	}, []);

	function onDelete(agent) {
		confirmDialog(
			`Delete agent "${agent.name}"? Sessions using this agent will be reassigned to the default agent.`,
		).then((yes) => {
			if (!yes) return;
			sendRpc("agents.delete", { id: agent.id }).then((res) => {
				if (res?.ok) {
					refreshGon();
					fetchSessions();
					fetchAgents();
				} else {
					setError(res?.error?.message || "Failed to delete");
				}
			});
		});
	}

	function onSetDefault(agent) {
		sendRpc("agents.set_default", { id: agent.id }).then((res) => {
			if (res?.ok) {
				refreshGon();
				fetchAgents();
			} else {
				setError(res?.error?.message || "Failed to set default");
			}
		});
	}

	if (loading) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div class="text-xs text-[var(--muted)]">Loading\u2026</div>
		</div>`;
	}

	if (editing) {
		return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<${AgentForm}
				agent=${editing === "new" ? null : editing}
				onSave=${() => {
					setEditing(null);
					fetchAgents();
				}}
				onCancel=${() => setEditing(null)}
			/>
		</div>`;
	}

	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<div class="flex items-center gap-3">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Agents</h2>
			<button class="provider-btn" style="font-size:0.75rem;padding:4px 10px;" onClick=${() => setEditing("new")}>
				New Agent
			</button>
		</div>
		<p class="text-xs text-[var(--muted)] leading-relaxed" style="max-width:600px;margin:0;">
			Create agent personas with different identities and personalities.
			Each agent has its own memory and system prompt.
		</p>

		${error && html`<span class="text-xs" style="color:var(--error);">${error}</span>`}

		<div class="flex flex-col gap-2" style="max-width:600px;">
				${agents.map(
					(agent) => html`
					<${AgentCard}
						key=${agent.id}
						agent=${agent}
						defaultId=${defaultId}
						onEdit=${(a) => setEditing(a)}
						onDelete=${onDelete}
						onSetDefault=${onSetDefault}
					/>
				`,
				)}
		</div>
	</div>`;
}
