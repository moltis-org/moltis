// ── Settings > Agents page (Preact + JSX) ───────────────────
//
// CRUD UI for agent personas. "main" agent is editable inline
// and cannot be deleted.

import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Loading, TabBar } from "../components/forms";
import { EmojiPicker } from "../emoji-picker";
import { refresh as refreshGon } from "../gon";
import { parseAgentsListPayload, sendRpc } from "../helpers";
import { fetchSessions } from "../sessions";
import { targetValue } from "../typed-events";
import type { RpcResponse } from "../types";
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
	system_prompt_suffix?: string;
	toml?: string;
	provenance?: "built_in" | "user_override" | "custom";
}

interface ModePreset {
	id: string;
	name: string;
	description: string;
	prompt: string;
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
	creating: boolean;
	onCreate: (preset: ConfigPreset) => void;
	onRevert?: (id: string) => void;
}

interface ModeCardProps {
	mode: ModePreset;
}

interface UnknownRecord {
	[key: string]: unknown;
}

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

let containerRef: HTMLElement | null = null;

function isRecord(value: unknown): value is UnknownRecord {
	return typeof value === "object" && value !== null;
}

function parseModePayload(value: unknown): ModePreset | null {
	if (!isRecord(value)) return null;
	const id = typeof value.id === "string" ? value.id : "";
	if (!id) return null;
	return {
		id,
		name: typeof value.name === "string" && value.name.trim() ? value.name : id,
		description: typeof value.description === "string" ? value.description : "",
		prompt: typeof value.prompt === "string" ? value.prompt : "",
	};
}

function parseModesPayload(value: unknown): ModePreset[] {
	if (!(isRecord(value) && Array.isArray(value.modes))) return [];
	return value.modes.map(parseModePayload).filter((mode): mode is ModePreset => mode !== null);
}

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
timeout_secs = 30

[tools]
allow = ["read_file", "grep", "glob"]
deny = ["exec"]

# MCP server access: allow_servers OR deny_servers (not both)
# [mcp]
# allow_servers = ["github", "memory"]
# deny_servers = ["home-assistant"]

# Sandbox mode override
# [sandbox]
# mode = "all"        # "off" | "all" | "non-main"

# Skill access control
# [skills]
# deny = ["gaming", "social-media"]`;

// ── Capability controls types ────────────────────────────────

interface McpServer {
	name: string;
	enabled?: boolean;
	display_name?: string;
}

interface PresetFields {
	model?: string | null;
	mcp?: { mode: string; servers?: string[] };
	sandbox?: { mode?: string | null };
	skills?: { allow?: string[] | null; deny?: string[] | null };
}

/** Parse a comma-separated string into a trimmed, non-empty array. */
function parseCsvList(value: string): string[] {
	return value
		.split(",")
		.map((s) => s.trim())
		.filter(Boolean);
}

/**
 * Remove named TOML sections (e.g. [mcp], [sandbox], [skills]) and their
 * key-value lines from a TOML string.  A section runs from its `[name]`
 * header to the next `[…]` header or end-of-string.
 */
function stripTomlSections(toml: string, sectionNames: string[]): string {
	const lines = toml.split("\n");
	const result: string[] = [];
	let skipping = false;
	const headers = new Set(sectionNames.map((n) => `[${n}]`));
	for (const line of lines) {
		const trimmed = line.trim();
		if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
			skipping = headers.has(trimmed);
		}
		if (!skipping) {
			result.push(line);
		}
	}
	return result.join("\n");
}

/** Escape a string for TOML double-quoted values. */
function tomlEscape(s: string): string {
	return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

/** Build a TOML array literal from strings. */
function tomlArray(values: string[]): string {
	return `[${values.map((v) => `"${tomlEscape(v)}"`).join(", ")}]`;
}

/** Build TOML from structured capability fields. */
function buildCapabilitiesToml(fields: PresetFields): string {
	const lines: string[] = [];
	if (fields.model) lines.push(`model = "${tomlEscape(fields.model)}"`);
	// MCP — always emit allow_servers when mode is "allow" (even if empty,
	// since allow_servers = [] means "deny all MCP tools").
	if (fields.mcp && fields.mcp.mode !== "all") {
		lines.push("");
		lines.push("[mcp]");
		const key = fields.mcp.mode === "allow" ? "allow_servers" : "deny_servers";
		lines.push(`${key} = ${tomlArray(fields.mcp.servers || [])}`);
	}
	// Sandbox — only mode is enforced at runtime via SandboxRouter override.
	if (fields.sandbox?.mode) {
		lines.push("");
		lines.push("[sandbox]");
		lines.push(`mode = "${tomlEscape(fields.sandbox.mode)}"`);
	}
	// Skills — emit allow/deny when present (including empty allow = [] which
	// means "deny all skills", matching the MCP allow_servers = [] semantics).
	const sk = fields.skills;
	if (sk && (sk.allow != null || (sk.deny && sk.deny.length > 0))) {
		lines.push("");
		lines.push("[skills]");
		if (sk.allow != null) lines.push(`allow = ${tomlArray(sk.allow)}`);
		if (sk.deny && sk.deny.length > 0) lines.push(`deny = ${tomlArray(sk.deny)}`);
	}
	return lines.join("\n");
}

function AgentForm({ agent, onSave, onCancel }: AgentFormProps): VNode {
	const isEdit = !!agent;
	const [id, setId] = useState(agent?.id || "");
	const [name, setName] = useState(agent?.name || "");
	const [emoji, setEmoji] = useState(agent?.emoji || "");
	const [theme, setTheme] = useState(agent?.theme || "");
	const [soul, setSoul] = useState("");
	const [presetToml, setPresetToml] = useState("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	// Structured capability fields
	const [mcpMode, setMcpMode] = useState<"all" | "allow" | "deny">("all");
	const [mcpServers, setMcpServers] = useState<string[]>([]);
	const [availableMcpServers, setAvailableMcpServers] = useState<McpServer[]>([]);
	const [sandboxMode, setSandboxMode] = useState("");
	const [skillsAllow, setSkillsAllow] = useState("");
	const [skillsAllowSet, setSkillsAllowSet] = useState(false);
	const [skillsDeny, setSkillsDeny] = useState("");
	const [capabilitiesOpen, setCapabilitiesOpen] = useState(false);
	const [advancedTomlOpen, setAdvancedTomlOpen] = useState(false);

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

	// Fetch available MCP servers
	useEffect(() => {
		sendRpc("mcp.list", {}).then((res) => {
			if (res?.ok && Array.isArray(res.payload)) {
				setAvailableMcpServers(
					(res.payload as McpServer[]).map((s) => ({
						name: typeof s.name === "string" ? s.name : "",
						enabled: s.enabled !== false,
						display_name: typeof s.display_name === "string" ? s.display_name : undefined,
					})),
				);
			}
		});
	}, []);

	// Load preset: structured fields + TOML for edits
	useEffect(() => {
		if (!isEdit) return;
		sendRpc("agents.preset.get", { id: agent?.id }).then((res) => {
			if (!res?.ok) return;
			const payload = res.payload as { toml?: string; fields?: PresetFields };
			if (payload?.toml?.trim()) {
				setPresetToml(payload.toml);
			}
			const f = payload?.fields;
			if (f) {
				if (f.mcp) {
					setMcpMode(f.mcp.mode as "all" | "allow" | "deny");
					setMcpServers(f.mcp.servers || []);
					if (f.mcp.mode !== "all") setCapabilitiesOpen(true);
				}
				if (f.sandbox) {
					setSandboxMode(f.sandbox.mode || "");
					if (f.sandbox.mode) setCapabilitiesOpen(true);
				}
				if (f.skills) {
					if (Array.isArray(f.skills.allow)) {
						setSkillsAllow(f.skills.allow.join(", "));
						setSkillsAllowSet(true);
					}
					setSkillsDeny((f.skills.deny || []).join(", "));
					if (Array.isArray(f.skills.allow) || (f.skills.deny && f.skills.deny.length > 0)) {
						setCapabilitiesOpen(true);
					}
				}
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
		// Build TOML: merge structured capability fields with any raw TOML.
		// The raw TOML textarea always preserves user content (tools, model,
		// timeouts, etc.). Structured fields generate [mcp], [sandbox], [skills]
		// sections that are prepended. Apply structured fields whenever they
		// contain non-default values, even if the user collapsed the panel before
		// saving.
		const capabilitiesConfigured =
			mcpMode !== "all" || mcpServers.length > 0 || sandboxMode !== "" || skillsAllowSet || skillsDeny.trim() !== "";
		let tomlToSave = presetToml.trim();
		if (capabilitiesOpen || capabilitiesConfigured) {
			const generated = buildCapabilitiesToml({
				mcp: { mode: mcpMode, servers: mcpServers },
				sandbox: { mode: sandboxMode || null },
				skills: {
					allow: skillsAllowSet ? parseCsvList(skillsAllow) : null,
					deny: parseCsvList(skillsDeny),
				},
			});
			// Merge: strip [mcp], [sandbox], [skills] sections from the raw TOML
			// to avoid duplicates. Put raw top-level keys FIRST so they stay at
			// the TOML document root, then APPEND generated sections — this
			// prevents model/timeout_secs/etc. from being misassigned to the
			// last generated section header.
			const rawWithoutStructured = stripTomlSections(tomlToSave, ["mcp", "sandbox", "skills"]).trim();
			tomlToSave = rawWithoutStructured ? `${rawWithoutStructured}\n\n${generated}` : generated;
		}
		// Always save when capabilities are active — an empty TOML string
		// clears the preset, which is correct when the user has removed all
		// restrictions. Without this, old restrictions survive silently.
		const savingToml = capabilitiesOpen || capabilitiesConfigured || !!tomlToSave;
		if (savingToml) {
			pending.push(sendRpc("agents.preset.save", { id: agentId, toml: tomlToSave }));
		}
		if (pending.length > 0) {
			Promise.all(pending).then((results) => {
				const tomlResult = savingToml
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
					onClick={() => setCapabilitiesOpen(!capabilitiesOpen)}
				>
					<span style={{ fontSize: "0.6rem" }}>{capabilitiesOpen ? "\u25BC" : "\u25B6"}</span>
					Capabilities
				</button>
				{capabilitiesOpen && (
					<div className="flex flex-col gap-3 mt-1">
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Control what this agent can access. Assign agents to channels via the Agent field in channel settings.
						</p>

						{/* MCP Server Access */}
						<fieldset className="flex flex-col gap-1 border border-[var(--border)] rounded p-2">
							<legend className="text-xs font-medium text-[var(--text-strong)] px-1">MCP Servers</legend>
							<div className="flex gap-3 text-xs">
								<label className="flex items-center gap-1">
									<input type="radio" name="mcp-mode" checked={mcpMode === "all"} onChange={() => setMcpMode("all")} />
									All
								</label>
								<label className="flex items-center gap-1">
									<input
										type="radio"
										name="mcp-mode"
										checked={mcpMode === "allow"}
										onChange={() => setMcpMode("allow")}
									/>
									Only selected
								</label>
								<label className="flex items-center gap-1">
									<input
										type="radio"
										name="mcp-mode"
										checked={mcpMode === "deny"}
										onChange={() => setMcpMode("deny")}
									/>
									All except
								</label>
							</div>
							{mcpMode !== "all" && availableMcpServers.length > 0 && (
								<div className="flex flex-col gap-1 mt-1">
									{availableMcpServers.map((s) => (
										<label key={s.name} className="flex items-center gap-1 text-xs">
											<input
												type="checkbox"
												checked={mcpServers.includes(s.name)}
												onChange={(e) => {
													const checked = (e.target as HTMLInputElement).checked;
													setMcpServers(checked ? [...mcpServers, s.name] : mcpServers.filter((n) => n !== s.name));
												}}
											/>
											{s.display_name || s.name}
											{!s.enabled && <span className="text-[var(--muted)]">(disabled)</span>}
										</label>
									))}
								</div>
							)}
							{mcpMode !== "all" && availableMcpServers.length === 0 && (
								<span className="text-xs text-[var(--muted)]">No MCP servers configured</span>
							)}
						</fieldset>

						{/* Sandbox Mode */}
						<fieldset className="flex flex-col gap-2 border border-[var(--border)] rounded p-2">
							<legend className="text-xs font-medium text-[var(--text-strong)] px-1">Sandbox</legend>
							<label className="flex flex-col gap-1 text-xs">
								<span className="text-[var(--muted)]">Mode</span>
								<select
									className="provider-key-input"
									value={sandboxMode}
									onChange={(e) => setSandboxMode(targetValue(e))}
									style={{ fontSize: "0.75rem", padding: "3px 6px" }}
								>
									<option value="">Inherit global</option>
									<option value="all">Always sandbox</option>
									<option value="off">No sandbox</option>
									<option value="non-main">Non-main only</option>
								</select>
							</label>
						</fieldset>

						{/* Skills */}
						<fieldset className="flex flex-col gap-2 border border-[var(--border)] rounded p-2">
							<legend className="text-xs font-medium text-[var(--text-strong)] px-1">Skills</legend>
							<label className="flex flex-col gap-1">
								<span className="text-xs text-[var(--muted)]">Allowed (comma-separated, empty = all)</span>
								<input
									type="text"
									className="provider-key-input"
									value={skillsAllow}
									onInput={(e) => {
										const val = targetValue(e);
										setSkillsAllow(val);
										setSkillsAllowSet(val.trim().length > 0);
									}}
									placeholder="web_search, research"
									style={{ fontSize: "0.75rem" }}
								/>
							</label>
							<label className="flex flex-col gap-1">
								<span className="text-xs text-[var(--muted)]">Denied (comma-separated)</span>
								<input
									type="text"
									className="provider-key-input"
									value={skillsDeny}
									onInput={(e) => setSkillsDeny(targetValue(e))}
									placeholder="gaming, social-media"
									style={{ fontSize: "0.75rem" }}
								/>
							</label>
						</fieldset>

						{/* Advanced TOML fallback */}
						<button
							type="button"
							className="text-xs text-[var(--muted)] text-left flex items-center gap-1"
							onClick={() => setAdvancedTomlOpen(!advancedTomlOpen)}
						>
							<span style={{ fontSize: "0.6rem" }}>{advancedTomlOpen ? "\u25BC" : "\u25B6"}</span>
							Advanced TOML
						</button>
						{advancedTomlOpen && (
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
						)}
					</div>
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
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						style={{ fontSize: "0.7rem", padding: "3px 8px" }}
						onClick={() => onEdit(agent)}
					>
						Edit
					</button>
					{!isMain && (
						<button
							type="button"
							className="provider-btn provider-btn-danger"
							style={{ fontSize: "0.7rem", padding: "3px 8px" }}
							onClick={() => onDelete(agent)}
						>
							Delete
						</button>
					)}
					{!isDefault && (
						<button
							type="button"
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

// ── Config-only preset card ─────────────────────────────────

function provenanceBadge(provenance?: string): VNode | null {
	if (provenance === "built_in") return <span className="recommended-badge">Built-in</span>;
	if (provenance === "user_override") return <span className="tier-badge">Overridden</span>;
	if (provenance === "custom") return <span className="tier-badge">Custom</span>;
	return null;
}

function PresetCard({ preset, creating, onCreate, onRevert }: PresetCardProps): VNode {
	const [expanded, setExpanded] = useState(false);
	const isOverridden = preset.provenance === "user_override";
	return (
		<div className="backend-card" style={{ opacity: preset.provenance === "built_in" ? 0.7 : 1 }}>
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					{preset.emoji && <span className="text-lg">{preset.emoji}</span>}
					<span className="text-sm font-medium text-[var(--text-strong)]">{preset.name}</span>
					{provenanceBadge(preset.provenance)}
					{preset.model && <span className="text-xs text-[var(--muted)]">{preset.model}</span>}
				</div>
				<div className="flex gap-2">
					<button
						type="button"
						className="provider-btn"
						style={{ fontSize: "0.7rem", padding: "3px 8px" }}
						disabled={creating}
						onClick={() => onCreate(preset)}
					>
						{creating ? "Adding..." : "Add to Chat"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						style={{ fontSize: "0.7rem", padding: "3px 8px" }}
						onClick={() => setExpanded(!expanded)}
					>
						{expanded ? "Hide" : "View"}
					</button>
					{isOverridden && onRevert && (
						<button
							type="button"
							className="provider-btn provider-btn-secondary"
							style={{ fontSize: "0.7rem", padding: "3px 8px" }}
							onClick={() => onRevert(preset.id)}
						>
							Revert to built-in
						</button>
					)}
				</div>
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

// ── Mode card ───────────────────────────────────────────────

function ModeCard({ mode }: ModeCardProps): VNode {
	const [expanded, setExpanded] = useState(false);
	const title = mode.name || mode.id;
	return (
		<div className="backend-card">
			<div className="flex items-center justify-between gap-3">
				<div className="flex min-w-0 flex-col gap-1">
					<div className="flex items-center gap-2">
						<span className="text-sm font-medium text-[var(--text-strong)]">{title}</span>
						<span className="tier-badge">{mode.id}</span>
					</div>
					{mode.description && <div className="text-xs text-[var(--muted)]">{mode.description}</div>}
				</div>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					style={{ fontSize: "0.7rem", padding: "3px 8px" }}
					onClick={() => setExpanded(!expanded)}
				>
					{expanded ? "Hide" : "View"}
				</button>
			</div>
			{expanded && (
				<pre className="text-xs mt-2 p-2 rounded bg-[var(--bg-offset)] font-mono whitespace-pre-wrap overflow-x-auto max-h-[200px] overflow-y-auto">
					{mode.prompt}
				</pre>
			)}
		</div>
	);
}

// ── Main page ───────────────────────────────────────────────

function AgentsPageComponent({ subPath }: { subPath?: string }): VNode {
	const [agents, setAgents] = useState<AgentPersona[]>([]);
	const [configPresets, setConfigPresets] = useState<ConfigPreset[]>([]);
	const [modes, setModes] = useState<ModePreset[]>([]);
	const [defaultId, setDefaultId] = useState("main");
	const [isLoading, setIsLoading] = useState(true);
	const [editing, setEditing] = useState<null | "new" | AgentPersona>(null);
	const [creatingPresetId, setCreatingPresetId] = useState<string | null>(null);
	const [activeTab, setActiveTab] = useState("chat");
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
		let attempts = 0;
		function load(): void {
			sendRpc("agents.presets_list", {}).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				if (res?.ok && (res.payload as { presets?: ConfigPreset[] })?.presets) {
					setConfigPresets((res.payload as { presets: ConfigPreset[] }).presets);
				}
			});
		}
		load();
	}

	function fetchModes(): void {
		let attempts = 0;
		function load(): void {
			sendRpc("modes.list", {}).then((res) => {
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				if (res?.ok) {
					setModes(parseModesPayload(res.payload));
				}
			});
		}
		load();
	}

	useEffect(() => {
		fetchAgents();
		fetchConfigPresets();
		fetchModes();
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

	function onRevertPreset(id: string): void {
		confirmDialog(`Revert preset "${id}" to the built-in default? Your local override will be removed.`).then((yes) => {
			if (!yes) return;
			// Remove the user override by saving an empty TOML (removes from moltis.toml)
			sendRpc("agents.preset.save", { id, toml: "" }).then((res) => {
				if (res?.ok) {
					fetchConfigPresets();
				} else {
					setError(res?.error?.message || "Failed to revert");
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

	function onCreateFromPreset(preset: ConfigPreset): void {
		setError(null);
		setCreatingPresetId(preset.id);
		sendRpc("agents.create", {
			id: preset.id,
			name: preset.name || preset.id,
			emoji: preset.emoji || null,
			theme: preset.theme || null,
		}).then((createRes) => {
			if (!createRes?.ok) {
				setCreatingPresetId(null);
				setError(createRes?.error?.message || "Failed to create agent from preset");
				return;
			}
			const promptSuffix = preset.system_prompt_suffix?.trim();
			const afterSoul: Promise<RpcResponse> = promptSuffix
				? sendRpc("agents.identity.update_soul", { agent_id: preset.id, soul: promptSuffix })
				: Promise.resolve({ ok: true, payload: undefined, error: undefined });
			afterSoul.then((soulRes) => {
				setCreatingPresetId(null);
				if (!soulRes?.ok) {
					setError(soulRes?.error?.message || "Created agent, but failed to copy preset prompt");
					return;
				}
				refreshGon();
				fetchSessions();
				fetchAgents();
				fetchConfigPresets();
			});
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
			<div className="flex items-center gap-3 flex-wrap">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Agents</h2>
				{activeTab === "chat" && (
					<button
						type="button"
						className="provider-btn"
						style={{ fontSize: "0.75rem", padding: "4px 10px" }}
						onClick={() => setEditing("new")}
					>
						New Agent
					</button>
				)}
			</div>
			<TabBar
				tabs={[
					{ id: "chat", label: "Chat Agents", badge: agents.length || undefined },
					{ id: "subagents", label: "Sub-Agents", badge: configPresets.length || undefined },
					{ id: "modes", label: "Modes", badge: modes.length || undefined },
				]}
				active={activeTab}
				onChange={setActiveTab}
			/>

			{error && (
				<span className="text-xs" style={{ color: "var(--error)" }}>
					{error}
				</span>
			)}

			{activeTab === "chat" && (
				<section className="flex flex-col gap-3 max-w-[600px]" aria-label="Chat Agents panel">
					<div className="flex flex-col gap-1">
						<h3 className="text-xs font-medium text-[var(--muted)]">Chat Agents</h3>
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Persistent identities with their own memory, system prompt, sessions, and capability boundaries (model,
							MCP servers, sandbox policy, skills). Assign agents to channels for different users or contexts.
						</p>
					</div>
					<div className="flex flex-col gap-2">
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
				</section>
			)}

			{activeTab === "subagents" && (
				<section className="flex flex-col gap-2 max-w-[600px]" aria-label="Sub-Agents panel">
					<div className="flex flex-col gap-1">
						<h3 className="text-xs font-medium text-[var(--muted)]">Sub-Agent Presets</h3>
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Defined in <code>[agents.presets]</code> in <code>moltis.toml</code>. These roles are usable by
							spawn_agent for delegated work. Add one to chat to make it a persistent agent with memory and sessions.
						</p>
					</div>
					{configPresets.length > 0 ? (
						configPresets.map((preset) => (
							<PresetCard
								key={preset.id}
								preset={preset}
								creating={creatingPresetId === preset.id}
								onCreate={onCreateFromPreset}
								onRevert={onRevertPreset}
							/>
						))
					) : (
						<div className="backend-card text-xs text-[var(--muted)]">
							All configured sub-agent presets are already available as chat agents.
						</div>
					)}
				</section>
			)}

			{activeTab === "modes" && (
				<section className="flex flex-col gap-2 max-w-[600px]" aria-label="Modes panel">
					<div className="flex flex-col gap-1">
						<h3 className="text-xs font-medium text-[var(--muted)]">Modes</h3>
						<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
							Defined in <code>[modes]</code> in <code>moltis.toml</code>. Temporary per-session prompt overlays. Use
							/mode in chat or any connected channel to switch how the current agent works without changing its
							identity, memory, or presets.
						</p>
					</div>
					{modes.length > 0 ? (
						modes.map((mode) => <ModeCard key={mode.id} mode={mode} />)
					) : (
						<div className="backend-card text-xs text-[var(--muted)]">No modes are configured.</div>
					)}
				</section>
			)}
		</div>
	);
}
