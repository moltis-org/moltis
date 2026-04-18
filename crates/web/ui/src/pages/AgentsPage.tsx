// ── Settings > Agents page (Preact + JSX) ───────────────────
//
// CRUD UI for agent personas. "main" agent links to the
// Identity settings section and cannot be deleted.

import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Loading } from "../components/forms";
import { EmojiPicker } from "../emoji-picker";
import { refresh as refreshGon } from "../gon";
import { parseAgentsListPayload, sendRpc } from "../helpers";
import { navigate } from "../router";
import { settingsPath } from "../routes";
import { fetchSessions } from "../sessions";
import { targetValue } from "../typed-events";
import { confirmDialog } from "../ui";

// ── Types ───────────────────────────────────────────────────

interface AgentPersona {
	id: string;
	name: string;
	emoji?: string;
	theme?: string;
	is_default?: boolean;
	workspace_prompt_files?: WorkspacePromptFile[];
}

interface WorkspacePromptFile {
	name?: string;
	source?: string;
	truncated?: boolean;
	original_chars?: number;
	limit_chars?: number;
	truncated_chars?: number;
}

interface ConfigPreset {
	id: string;
	name: string;
	emoji?: string;
	theme?: string;
	model?: string;
	toml?: string;
}

interface AgentFormProps {
	agent: AgentPersona | null;
	onSave: () => void;
	onCancel: () => void;
}

interface AgentCardProps {
	agent: AgentPersona;
	defaultId: string;
	onEdit: (agent: AgentPersona) => void;
	onDelete: (agent: AgentPersona) => void;
	onSetDefault: (agent: AgentPersona) => void;
}

interface PresetCardProps {
	preset: ConfigPreset;
}

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

let containerRef: HTMLElement | null = null;

export function initAgents(container: HTMLElement, subPath?: string | null): void {
	containerRef = container;
	render(<AgentsPageComponent subPath={subPath || undefined} />, container);
}

export function teardownAgents(): void {
	if (containerRef) render(null, containerRef);
	containerRef = null;
}

// ── Create / Edit form ──────────────────────────────────────

const PRESET_TOML_PLACEHOLDER = `model = "haiku"
delegate_only = false
timeout_secs = 30

[tools]
allow = ["read_file", "grep", "glob"]
deny = ["exec"]`;

function AgentForm({ agent, onSave, onCancel }: AgentFormProps): VNode {
	const isEdit = !!agent;
	const [id, setId] = useState(agent?.id || "");
	const [name, setName] = useState(agent?.name || "");
	const [emoji, setEmoji] = useState(agent?.emoji || "");
	const [theme, setTheme] = useState(agent?.theme || "");
	const [soul, setSoul] = useState("");
	const [presetToml, setPresetToml] = useState("");
	const [presetOpen, setPresetOpen] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	// Load soul: for edits fetch the agent's soul, for new agents fetch main's soul as default
	useEffect(() => {
		const agentId = isEdit ? agent?.id : "main";
		let attempts = 0;
		function load(): void {
			sendRpc("agents.identity.get", { agent_id: agentId }).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				if (res?.ok && (res.payload as { soul?: string })?.soul) {
					setSoul((res.payload as { soul: string }).soul);
				}
			});
		}
		load();
	}, [isEdit, agent?.id]);

	// Load preset TOML for edits
	useEffect(() => {
		if (!isEdit) return;
		sendRpc("agents.preset.get", { id: agent?.id }).then((res) => {
			if (res?.ok && (res.payload as { toml?: string })?.toml) {
				const toml = (res.payload as { toml: string }).toml;
				setPresetToml(toml);
				if (toml.trim()) setPresetOpen(true);
			}
		});
	}, [isEdit, agent?.id]);

	interface AgentParams {
		name: string;
		emoji: string | null;
		theme: string | null;
		id?: string;
	}

	function buildParams(): AgentParams {
		const base: AgentParams = {
			name: name.trim(),
			emoji: emoji.trim() || null,
			theme: theme.trim() || null,
		};
		base.id = isEdit ? agent?.id : id.trim();
		return base;
	}

	function finishSave(agentId: string): void {
		const trimmedSoul = soul.trim();
		const pending: Promise<unknown>[] = [];
		if (trimmedSoul) {
			pending.push(sendRpc("agents.identity.update_soul", { agent_id: agentId, soul: trimmedSoul }));
		}
		// Save preset TOML if the section was opened or has content
		if (presetToml.trim()) {
			pending.push(sendRpc("agents.preset.save", { id: agentId, toml: presetToml.trim() }));
		}
		if (pending.length > 0) {
			Promise.all(pending).then((results) => {
				const tomlResult = presetToml.trim()
					? (results[results.length - 1] as { ok?: boolean; error?: { message?: string } })
					: null;
				if (tomlResult && !tomlResult?.ok) {
					setSaving(false);
					setError(tomlResult?.error?.message || "Failed to save preset TOML");
					return;
				}
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

	function onSubmit(e: Event): void {
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

		const method = isEdit ? "agents.update" : "agents.create";
		sendRpc(method, buildParams()).then((res) => {
			if (!res?.ok) {
				setSaving(false);
				setError(res?.error?.message || "Failed to save");
				return;
			}
			finishSave(isEdit ? agent?.id : id.trim());
		});
	}

	return (
		<form onSubmit={onSubmit} className="flex flex-col gap-3" style={{ maxWidth: "500px" }}>
			<h3 className="text-sm font-medium text-[var(--text-strong)]">
				{isEdit ? `Edit ${agent?.name}` : "Create Agent"}
			</h3>

			{!isEdit && (
				<label className="flex flex-col gap-1">
					<span className="text-xs text-[var(--muted)]">ID (slug, cannot change later)</span>
					<input
						type="text"
						className="provider-key-input"
						value={id}
						onInput={(e) =>
							setId(
								targetValue(e)
									.toLowerCase()
									.replace(/[^a-z0-9-]/g, ""),
							)
						}
						placeholder="e.g. writer, coder, researcher"
						maxLength={50}
					/>
				</label>
			)}

			<label className="flex flex-col gap-1">
				<span className="text-xs text-[var(--muted)]">Name</span>
				<input
					type="text"
					className="provider-key-input"
					value={name}
					onInput={(e) => setName(targetValue(e))}
					placeholder="Creative Writer"
				/>
			</label>

			<div className="flex flex-col gap-1">
				<span className="text-xs text-[var(--muted)]">Emoji</span>
				<EmojiPicker value={emoji} onChange={setEmoji} />
			</div>

			<label className="flex flex-col gap-1">
				<span className="text-xs text-[var(--muted)]">Theme</span>
				<input
					type="text"
					className="provider-key-input"
					value={theme}
					onInput={(e) => setTheme(targetValue(e))}
					placeholder={"wise owl, chill fox, witty robot\u2026"}
				/>
			</label>

			<label className="flex flex-col gap-1">
				<span className="text-xs text-[var(--muted)]">Soul (system prompt personality)</span>
				<textarea
					className="provider-key-input"
					value={soul}
					onInput={(e) => setSoul(targetValue(e))}
					placeholder={"You are a creative writing assistant\u2026"}
					rows={4}
					style={{ resize: "vertical", fontFamily: "var(--font-mono)", fontSize: "0.75rem" }}
				/>
			</label>

			<div className="flex flex-col gap-1">
				<button
					type="button"
					className="text-xs text-[var(--muted)] text-left flex items-center gap-1"
					onClick={() => setPresetOpen(!presetOpen)}
				>
					<span style={{ fontSize: "0.6rem" }}>{presetOpen ? "\u25BC" : "\u25B6"}</span>
					Spawn Settings (TOML)
				</button>
				{presetOpen && (
					<>
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Configure how this agent behaves when spawned as a sub-agent via spawn_agent.
						</p>
						<textarea
							className="provider-key-input"
							value={presetToml}
							onInput={(e) => setPresetToml(targetValue(e))}
							placeholder={PRESET_TOML_PLACEHOLDER}
							rows={6}
							style={{
								resize: "vertical",
								fontFamily: "var(--font-mono)",
								fontSize: "0.7rem",
								whiteSpace: "pre",
								overflowX: "auto",
							}}
						/>
					</>
				)}
			</div>

			{error && (
				<span className="text-xs" style={{ color: "var(--error)" }}>
					{error}
				</span>
			)}

			<div className="flex gap-2">
				<button type="submit" className="provider-btn" disabled={saving}>
					{saving ? "Saving\u2026" : isEdit ? "Save" : "Create"}
				</button>
				<button type="button" className="provider-btn provider-btn-secondary" onClick={onCancel}>
					Cancel
				</button>
			</div>
		</form>
	);
}

// ── Agent card ──────────────────────────────────────────────

function AgentCard({ agent, defaultId, onEdit, onDelete, onSetDefault }: AgentCardProps): VNode {
	const isMain = agent.id === "main";
	const isDefault = !!agent.is_default || agent.id === defaultId;
	const workspacePromptFiles = Array.isArray(agent.workspace_prompt_files) ? agent.workspace_prompt_files : [];
	const truncatedWorkspacePromptFiles = workspacePromptFiles.filter((file) => file?.truncated);
	return (
		<div className="backend-card">
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					{agent.emoji && <span className="text-lg">{agent.emoji}</span>}
					<span className="text-sm font-medium text-[var(--text-strong)]">{agent.name}</span>
					{isDefault && <span className="recommended-badge">Default</span>}
				</div>
				<div className="flex gap-2">
					{isMain ? (
						<button
							className="provider-btn provider-btn-secondary"
							style={{ fontSize: "0.7rem", padding: "3px 8px" }}
							onClick={() => navigate(settingsPath("identity"))}
						>
							Identity Settings
						</button>
					) : (
						<>
							<button
								className="provider-btn provider-btn-secondary"
								style={{ fontSize: "0.7rem", padding: "3px 8px" }}
								onClick={() => onEdit(agent)}
							>
								Edit
							</button>
							<button
								className="provider-btn provider-btn-danger"
								style={{ fontSize: "0.7rem", padding: "3px 8px" }}
								onClick={() => onDelete(agent)}
							>
								Delete
							</button>
						</>
					)}
					{!isDefault && (
						<button
							className="provider-btn provider-btn-secondary"
							style={{ fontSize: "0.7rem", padding: "3px 8px" }}
							onClick={() => onSetDefault(agent)}
						>
							Set Default
						</button>
					)}
				</div>
			</div>
			{agent.theme && <div className="text-xs text-[var(--muted)] mt-1">{agent.theme}</div>}
			{truncatedWorkspacePromptFiles.length > 0 && (
				<div className="text-xs mt-2 rounded-md border border-[var(--border)] bg-[var(--surface)] p-2 text-[var(--text)]">
					{truncatedWorkspacePromptFiles.map((file, index) => {
						const name = typeof file.name === "string" ? file.name : "workspace file";
						const charCount = Number(file.original_chars || 0).toLocaleString();
						const limitChars = Number(file.limit_chars || 0).toLocaleString();
						const truncatedChars = Number(file.truncated_chars || 0).toLocaleString();
						const source = typeof file.source === "string" ? ` (${file.source})` : "";
						const line = `${name}${source}: ${charCount} chars, limit ${limitChars}, truncated by ${truncatedChars}`;
						return <div key={`${name}-${index}`}>{line}</div>;
					})}
				</div>
			)}
		</div>
	);
}

// ── Config-only preset card (read-only) ─────────────────────

function PresetCard({ preset }: PresetCardProps): VNode {
	const [expanded, setExpanded] = useState(false);
	return (
		<div className="backend-card" style={{ opacity: 0.7 }}>
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					{preset.emoji && <span className="text-lg">{preset.emoji}</span>}
					<span className="text-sm font-medium text-[var(--text-strong)]">{preset.name}</span>
					<span className="tier-badge">config</span>
					{preset.model && <span className="text-xs text-[var(--muted)]">{preset.model}</span>}
				</div>
				<button
					className="provider-btn provider-btn-secondary"
					style={{ fontSize: "0.7rem", padding: "3px 8px" }}
					onClick={() => setExpanded(!expanded)}
				>
					{expanded ? "Hide" : "View"}
				</button>
			</div>
			{preset.theme && <div className="text-xs text-[var(--muted)] mt-1">{preset.theme}</div>}
			{expanded && preset.toml && (
				<pre
					className="text-xs mt-2 p-2 rounded"
					style={{
						background: "var(--bg-offset)",
						fontFamily: "var(--font-mono)",
						whiteSpace: "pre-wrap",
						overflowX: "auto",
						maxHeight: "200px",
						overflowY: "auto",
					}}
				>
					{preset.toml}
				</pre>
			)}
		</div>
	);
}

// ── Main page ───────────────────────────────────────────────

function AgentsPageComponent({ subPath }: { subPath?: string }): VNode {
	const [agents, setAgents] = useState<AgentPersona[]>([]);
	const [configPresets, setConfigPresets] = useState<ConfigPreset[]>([]);
	const [defaultId, setDefaultId] = useState("main");
	const [isLoading, setIsLoading] = useState(true);
	const [editing, setEditing] = useState<null | "new" | AgentPersona>(null);
	const [error, setError] = useState<string | null>(null);

	function fetchAgents(): void {
		setIsLoading(true);
		let attempts = 0;
		function load(): void {
			sendRpc("agents.list", {}).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				setIsLoading(false);
				if (res?.ok) {
					const parsed = parseAgentsListPayload(res.payload as Parameters<typeof parseAgentsListPayload>[0]);
					setDefaultId(parsed.defaultId);
					setAgents(parsed.agents.map((a) => ({ ...a, id: a.id || "", name: a.name || a.id || "" }) as AgentPersona));
				} else {
					setError(res?.error?.message || "Failed to load agents");
				}
			});
		}
		load();
	}

	function fetchConfigPresets(): void {
		sendRpc("agents.presets_list", {}).then((res) => {
			if (res?.ok && (res.payload as { presets?: ConfigPreset[] })?.presets) {
				setConfigPresets((res.payload as { presets: ConfigPreset[] }).presets);
			}
		});
	}

	useEffect(() => {
		fetchAgents();
		fetchConfigPresets();
		// Auto-open create form when navigating to /settings/agents/new
		if (subPath === "new") {
			setEditing("new");
		}
	}, []);

	function onDelete(agent: AgentPersona): void {
		confirmDialog(
			`Delete agent "${agent.name}"? Sessions using this agent will be reassigned to the default agent.`,
		).then((yes) => {
			if (!yes) return;
			sendRpc("agents.delete", { id: agent.id }).then((res) => {
				if (res?.ok) {
					refreshGon();
					fetchSessions();
					fetchAgents();
					fetchConfigPresets();
				} else {
					setError(res?.error?.message || "Failed to delete");
				}
			});
		});
	}

	function onSetDefault(agent: AgentPersona): void {
		sendRpc("agents.set_default", { id: agent.id }).then((res) => {
			if (res?.ok) {
				refreshGon();
				fetchAgents();
			} else {
				setError(res?.error?.message || "Failed to set default");
			}
		});
	}

	if (isLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<Loading />
			</div>
		);
	}

	if (editing) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<AgentForm
					agent={editing === "new" ? null : editing}
					onSave={() => {
						setEditing(null);
						fetchAgents();
						fetchConfigPresets();
					}}
					onCancel={() => setEditing(null)}
				/>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Agents</h2>
				<button
					className="provider-btn"
					style={{ fontSize: "0.75rem", padding: "4px 10px" }}
					onClick={() => setEditing("new")}
				>
					New Agent
				</button>
			</div>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Create agent personas with different identities and personalities. Each agent has its own memory and system
				prompt.
			</p>

			{error && (
				<span className="text-xs" style={{ color: "var(--error)" }}>
					{error}
				</span>
			)}

			<div className="flex flex-col gap-2" style={{ maxWidth: "600px" }}>
				{agents.map((agent) => (
					<AgentCard
						key={agent.id}
						agent={agent}
						defaultId={defaultId}
						onEdit={(a) => setEditing(a)}
						onDelete={onDelete}
						onSetDefault={onSetDefault}
					/>
				))}
			</div>

			{configPresets.length > 0 && (
				<div className="flex flex-col gap-2 mt-2" style={{ maxWidth: "600px" }}>
					<h3 className="text-xs font-medium text-[var(--muted)]">Config-only Presets</h3>
					{configPresets.map((preset) => (
						<PresetCard key={preset.id} preset={preset} />
					))}
				</div>
			)}
		</div>
	);
}
