// ── Settings page (Preact + Signals) ───────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { EmojiPicker } from "../emoji-picker";
import { onEvent } from "../events";
import * as gon from "../gon";
import { refresh as refreshGon } from "../gon";
import { localizedApiErrorMessage, sendRpc } from "../helpers";
import { setLocale } from "../i18n";
import { updateIdentity, validateIdentityFields } from "../identity-utils";
import { detectPasskeyName } from "../passkey-detect";
import * as push from "../push";
import { isStandalone } from "../pwa";
import { navigate, registerPrefix } from "../router";
import { routes, settingsPath } from "../routes";
import { connected } from "../signals";
import * as S from "../state";
import { fetchPhrase } from "../tts-phrases";
import { Modal, showToast } from "../ui";
import {
	decodeBase64Safe,
	fetchVoiceProviders,
	saveVoiceKey,
	saveVoiceSettings,
	testTts,
	toggleVoiceProvider,
	transcribeAudio,
} from "../voice-utils";
import { initAgents, teardownAgents } from "./AgentsPage";
import { initChannels, teardownChannels } from "./ChannelsPage";
import { initCrons, teardownCrons } from "./CronsPage";
import { initHooks, teardownHooks } from "./HooksPage";
import { initImages, teardownImages } from "./ImagesPage";
import { initLogs, teardownLogs } from "./LogsPage";
import { initMcp, teardownMcp } from "./McpPage";
import { initMonitoring, teardownMonitoring } from "./MetricsPage";
import { initNetworkAudit, teardownNetworkAudit } from "./NetworkAuditPage";
import { initNodes, teardownNodes } from "./NodesPage";
import { initProjects, teardownProjects } from "./ProjectsPage";
import { initProviders, teardownProviders } from "./ProvidersPage";
import { initSkills, teardownSkills } from "./SkillsPage";
import { initTerminal, teardownTerminal } from "./TerminalPage";
import { initWebhooks, teardownWebhooks } from "./WebhooksPage";

// ── Types ───────────────────────────────────────────────────

interface IdentityData {
	name?: string;
	emoji?: string;
	theme?: string;
	user_name?: string;
	soul?: string;
	[key: string]: unknown;
}

interface RpcResponse {
	ok?: boolean;
	payload?: unknown;
	error?: { message?: string };
}

interface SectionItem {
	id?: string;
	label?: string;
	icon?: VNode;
	page?: boolean;
	group?: string;
}

interface EnvVar {
	id: string;
	key: string;
	encrypted?: boolean;
	updated_at?: string;
}

interface PasskeyEntry {
	id: string;
	name: string;
	created_at?: string;
}

interface ApiKeyEntry {
	id: string;
	label: string;
	key_prefix?: string;
	created_at?: string;
	scopes?: string[];
}

interface SshKeyEntry {
	id: number;
	name: string;
	fingerprint?: string;
	public_key?: string;
	encrypted?: boolean;
	target_count?: number;
}

interface SshTargetEntry {
	id: number;
	label: string;
	target: string;
	port?: number;
	auth_mode?: string;
	key_name?: string;
	known_host?: string;
	is_default?: boolean;
}

interface TestResult {
	reachable?: boolean;
	failure_hint?: string;
}

interface ToolEntry {
	name?: string;
	description?: string;
}

interface ToolGroup {
	label: string;
	tools: ToolEntry[];
}

interface SkillEntry {
	name?: string;
	description?: string;
	source?: string;
}

interface McpServerEntry {
	name?: string;
	state?: string;
	tool_count?: number;
}

interface ToolsContextData {
	session?: { model?: string; provider?: string; label?: string };
	execution?: { mode?: string; promptSymbol?: string };
	sandbox?: { enabled?: boolean; backend?: string };
	tools?: ToolEntry[];
	skills?: SkillEntry[];
	mcpServers?: McpServerEntry[];
	supportsTools?: boolean;
	mcpDisabled?: boolean;
}

interface NodeInventoryEntry {
	platform?: string;
	[key: string]: unknown;
}

interface RemoteExecSummary {
	pairedNodes: number;
	sshTargets: number;
}

interface AkScopes {
	"operator.read": boolean;
	"operator.write": boolean;
	"operator.approvals": boolean;
	"operator.pairing": boolean;
	[key: string]: boolean;
}

// ── Module-level state ───────────────────────────────────────

const identity = signal<IdentityData | null>(null);
const loading = signal(true);
const activeSection = signal("identity");
const activeSubPath = signal("");
const mobileSidebarVisible = signal(true);
let mounted = false;
let containerRef: HTMLElement | null = null;

function rerender(): void {
	if (containerRef) render(<SettingsPage />, containerRef);
}

function isMobileViewport(): boolean {
	return window.innerWidth < 768;
}

function isSafariBrowser(): boolean {
	if (typeof navigator === "undefined") return false;
	const ua = navigator.userAgent || "";
	const vendor = navigator.vendor || "";
	if (!ua.includes("Safari/")) return false;
	if (/(Chrome|CriOS|Chromium|Edg|OPR|FxiOS|Firefox|SamsungBrowser)/.test(ua)) return false;
	return /Apple/i.test(vendor) || ua.includes("Safari/");
}

function isMissingMethodError(res: RpcResponse | null): boolean {
	const message = res?.error?.message;
	if (typeof message !== "string") return false;
	const lower = message.toLowerCase();
	return lower.includes("method") && (lower.includes("not found") || lower.includes("unknown"));
}

function fetchMainIdentity(): Promise<RpcResponse> {
	return sendRpc("agents.identity.get", { agent_id: "main" }).then((res: RpcResponse) => {
		if (res?.ok || !isMissingMethodError(res)) return res;
		return sendRpc("agent.identity.get", {});
	});
}

function fetchIdentity(): void {
	if (!mounted) return;
	fetchMainIdentity().then((res: RpcResponse) => {
		if (res?.ok) {
			identity.value = res.payload as IdentityData;
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

const sections: SectionItem[] = [
	{ group: "General" },
	{
		id: "identity",
		label: "Identity",
		icon: <span className="icon icon-person" />,
	},
	{
		id: "agents",
		label: "Agents",
		icon: <span className="icon icon-users" />,
		page: true,
	},
	{
		id: "nodes",
		label: "Nodes",
		icon: <span className="icon icon-nodes" />,
		page: true,
	},
	{
		id: "projects",
		label: "Projects",
		icon: <span className="icon icon-folder" />,
		page: true,
	},
	{
		id: "environment",
		label: "Environment",
		icon: <span className="icon icon-terminal" />,
	},
	{
		id: "memory",
		label: "Memory",
		icon: <span className="icon icon-database" />,
	},
	{
		id: "notifications",
		label: "Notifications",
		icon: <span className="icon icon-bell" />,
	},
	{
		id: "crons",
		label: "Crons",
		icon: <span className="icon icon-cron" />,
		page: true,
	},
	{
		id: "webhooks",
		label: "Webhooks",
		icon: <span className="icon icon-webhooks" />,
		page: true,
	},
	{
		id: "heartbeat",
		label: "Heartbeat",
		icon: <span className="icon icon-heart" />,
		page: true,
	},
	{ group: "Security" },
	{
		id: "security",
		label: "Authentication",
		icon: <span className="icon icon-key" />,
	},
	{
		id: "vault",
		label: "Encryption",
		icon: <span className="icon icon-lock" />,
	},
	{
		id: "ssh",
		label: "SSH",
		icon: <span className="icon icon-ssh" />,
	},
	{
		id: "remote-access",
		label: "Remote Access",
		icon: <span className="icon icon-share" />,
	},
	{
		id: "network-audit",
		label: "Network Audit",
		icon: <span className="icon icon-shield-check" />,
		page: true,
	},
	{
		id: "sandboxes",
		label: "Sandboxes",
		icon: <span className="icon icon-cube" />,
		page: true,
	},
	{ group: "Integrations" },
	{
		id: "channels",
		label: "Channels",
		icon: <span className="icon icon-channels" />,
		page: true,
	},
	{
		id: "hooks",
		label: "Hooks",
		icon: <span className="icon icon-wrench" />,
		page: true,
	},
	{
		id: "providers",
		label: "LLMs",
		icon: <span className="icon icon-layers" />,
		page: true,
	},
	{
		id: "tools",
		label: "Tools",
		icon: <span className="icon icon-settings-gear" />,
	},
	{
		id: "mcp",
		label: "MCP",
		icon: <span className="icon icon-link" />,
		page: true,
	},
	{
		id: "skills",
		label: "Skills",
		icon: <span className="icon icon-sparkles" />,
		page: true,
	},
	{
		id: "import",
		label: "OpenClaw Import",
		icon: <span className="icon icon-openclaw" />,
	},
	{
		id: "voice",
		label: "Voice",
		icon: <span className="icon icon-microphone" />,
	},
	{ group: "Systems" },
	{ id: "terminal", label: "Terminal", page: true },
	{ id: "monitoring", label: "Monitoring", page: true },
	{ id: "logs", label: "Logs", page: true },
	{ id: "graphql", label: "GraphQL" },
	{ id: "config", label: "Configuration" },
];

function getVisibleSections(): SectionItem[] {
	const vs = gon.get("vault_status");
	return sections.filter((s) => {
		if (!s.id) return true;
		if (s.id === "graphql" && !gon.get("graphql_enabled")) return false;
		if (s.id === "import" && !gon.get("openclaw_detected")) return false;
		if (s.id === "vault" && (!vs || vs === "disabled")) return false;
		return true;
	});
}

/** Return only items with an id (no group headings). */
function getSectionItems(): SectionItem[] {
	return getVisibleSections().filter((s) => s.id);
}

function pluralizeToolsCount(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function toolsOverviewCategory(name: string | undefined): string {
	if (typeof name !== "string" || !name) return "Core";
	if (name.startsWith("mcp__")) return "MCP";
	if (name === "exec" || name.startsWith("node") || name.startsWith("sandbox") || name.includes("checkpoint")) {
		return "Execution";
	}
	if (name.startsWith("session") || name.startsWith("sessions_")) return "Sessions";
	if (name.startsWith("memory") || name.includes("memory")) return "Memory";
	if (name.startsWith("browser") || name.startsWith("web_") || name.includes("screenshot") || name.includes("fetch")) {
		return "Web & Browser";
	}
	if (name.startsWith("skill") || name.includes("skill")) return "Skills";
	return "Core";
}

function groupToolsForOverview(tools: ToolEntry[]): ToolGroup[] {
	const grouped = new Map<string, ToolEntry[]>();
	(tools || []).forEach((tool) => {
		const category = toolsOverviewCategory(tool?.name);
		if (!grouped.has(category)) grouped.set(category, []);
		grouped.get(category)!.push(tool);
	});
	const order = ["Execution", "Sessions", "Memory", "Web & Browser", "Skills", "MCP", "Core"];
	return order
		.filter((label) => grouped.has(label))
		.map((label) => ({
			label,
			tools: grouped
				.get(label)!
				.slice()
				.sort((left, right) => String(left?.name || "").localeCompare(String(right?.name || ""))),
		}));
}

function summarizeRemoteExecInventory(entries: NodeInventoryEntry[]): RemoteExecSummary {
	const summary: RemoteExecSummary = { pairedNodes: 0, sshTargets: 0 };
	(entries || []).forEach((entry) => {
		if (!entry || typeof entry !== "object") return;
		if (entry.platform === "ssh") {
			summary.sshTargets += 1;
			return;
		}
		summary.pairedNodes += 1;
	});
	return summary;
}

function SettingsSidebar(): VNode {
	return (
		<div className="settings-sidebar">
			<div className="settings-sidebar-header">
				<button
					className="settings-back-slot"
					onClick={() => {
						navigate(routes.chats as string);
					}}
					title="Back to chat sessions"
				>
					<span className="icon icon-chat" />
					Back to Chats
				</button>
			</div>
			<div className="settings-sidebar-nav">
				{getVisibleSections().map((s) =>
					s.group ? (
						<div key={s.group} className="settings-group-label">
							{s.group}
						</div>
					) : (
						<button
							key={s.id}
							className={`settings-nav-item ${activeSection.value === s.id ? "active" : ""}`}
							data-section={s.id}
							onClick={() => {
								if (isMobileViewport()) {
									mobileSidebarVisible.value = false;
									rerender();
								}
								navigate(settingsPath(s.id!));
							}}
						>
							{s.label}
						</button>
					),
				)}
			</div>
		</div>
	);
}

// EmojiPicker imported from emoji-picker.ts

// ── Soul defaults ────────────────────────────────────────────

const DEFAULT_SOUL =
	"Be genuinely helpful, not performatively helpful. Skip the filler words \u2014 just help.\n" +
	"Have opinions. You're allowed to disagree, prefer things, find stuff amusing or boring.\n" +
	"Be resourceful before asking. Try to figure it out first \u2014 read the context, search for it \u2014 then ask if you're stuck.\n" +
	"Earn trust through competence. Be careful with external actions. Be bold with internal ones.\n" +
	"Remember you're a guest. You have access to someone's life. Treat it with respect.\n" +
	"Private things stay private. When in doubt, ask before acting externally.\n" +
	"Be concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just good.";

// ── Identity section (editable form) ─────────────────────────

function IdentitySection(): VNode {
	const id = identity.value;
	const isNew = !(id && (id.name || id.user_name));
	const storedLocale = localStorage.getItem("moltis-locale");

	const [name, setName] = useState(id?.name || "");
	const [emoji, setEmoji] = useState(id?.emoji || "");
	const [theme, setTheme] = useState(id?.theme || "");
	const [userName, setUserName] = useState(id?.user_name || "");
	const [soul, setSoul] = useState(id?.soul || "");
	const [uiLanguage, setUiLanguage] = useState(storedLocale || "auto");
	const [saving, setSaving] = useState(false);
	const [emojiSaving, setEmojiSaving] = useState(false);
	const [nameSaving, setNameSaving] = useState(false);
	const [userNameSaving, setUserNameSaving] = useState(false);
	const [languageSaving, setLanguageSaving] = useState(false);
	const [saved, setSaved] = useState(false);
	const [languageSaved, setLanguageSaved] = useState(false);
	const [showFaviconReloadHint, setShowFaviconReloadHint] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [languageError, setLanguageError] = useState<string | null>(null);

	// Sync state when identity loads asynchronously
	useEffect(() => {
		if (!id) return;
		setName(id.name || "");
		setEmoji(id.emoji || "");
		setTheme(id.theme || "");
		setUserName(id.user_name || "");
		setSoul(id.soul || "");
	}, [id]);

	const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	function flashSaved(): void {
		if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
		setSaved(true);
		savedTimerRef.current = setTimeout(() => {
			savedTimerRef.current = null;
			setSaved(false);
			rerender();
		}, 2000);
	}

	if (loading.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	function onSave(e: Event): void {
		e.preventDefault();
		const v = validateIdentityFields(name, userName);
		if (!v.valid) {
			setError(v.error);
			return;
		}
		setError(null);
		setSaving(true);
		setSaved(false);

		updateIdentity(
			{
				name: name.trim(),
				emoji: emoji.trim() || "",
				theme: theme.trim() || "",
				soul: soul.trim() || undefined,
				user_name: userName.trim(),
			},
			{ agentId: "main" },
		).then((res: RpcResponse) => {
			setSaving(false);
			if (res?.ok) {
				const payload = res.payload as IdentityData;
				identity.value = payload;
				gon.set("identity", payload as import("../types/gon").ResolvedIdentity);
				refreshGon();
				const emojiChanged = (emoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
	}

	function onEmojiSelect(nextEmoji: string): void {
		setEmoji(nextEmoji);
		setError(null);
		setSaved(false);
		setEmojiSaving(true);
		updateIdentity({ emoji: nextEmoji.trim() || "" }, { agentId: "main" }).then((res: RpcResponse) => {
			setEmojiSaving(false);
			if (res?.ok) {
				const payload = res.payload as IdentityData;
				identity.value = payload;
				setEmoji(payload?.emoji || "");
				gon.set("identity", payload as import("../types/gon").ResolvedIdentity);
				refreshGon();
				const emojiChanged = (nextEmoji.trim() || "") !== (id?.emoji || "").trim();
				setShowFaviconReloadHint(emojiChanged && isSafariBrowser());
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save emoji");
			}
			rerender();
		});
	}

	function autoSaveNameField(field: string, value: string): void {
		if (saving || emojiSaving || nameSaving || userNameSaving) return;
		const trimmed = value.trim();
		const currentValue = ((identity.value?.[field] as string) || "").trim();
		if (trimmed === currentValue) return;

		if (!trimmed) {
			setError(field === "name" ? "Agent name is required." : "Your name is required.");
			return;
		}

		setError(null);
		setSaved(false);
		if (field === "name") {
			setNameSaving(true);
		} else {
			setUserNameSaving(true);
		}

		const payload: Record<string, string> = {};
		payload[field] = trimmed;
		updateIdentity(payload, { agentId: "main" }).then((res: RpcResponse) => {
			if (field === "name") {
				setNameSaving(false);
			} else {
				setUserNameSaving(false);
			}

			if (res?.ok) {
				const resPayload = res.payload as IdentityData;
				identity.value = resPayload;
				gon.set("identity", resPayload as import("../types/gon").ResolvedIdentity);
				refreshGon();
				setName(resPayload?.name || "");
				setUserName(resPayload?.user_name || "");
				flashSaved();
			} else {
				setError(res?.error?.message || "Failed to save");
			}
			rerender();
		});
	}

	function onNameBlur(e: Event): void {
		autoSaveNameField("name", (e.target as HTMLInputElement).value);
	}

	function onUserNameBlur(e: Event): void {
		autoSaveNameField("user_name", (e.target as HTMLInputElement).value);
	}

	function onResetSoul(): void {
		setSoul("");
		rerender();
	}

	function onReloadForFavicon(): void {
		window.location.reload();
	}

	function onApplyLanguage(): void {
		setLanguageSaving(true);
		setLanguageSaved(false);
		setLanguageError(null);

		const nextLanguage = uiLanguage === "auto" ? navigator.language || "en" : uiLanguage;
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
			.catch((err: Error) => {
				setLanguageSaving(false);
				setLanguageError(err?.message || "Failed to update language");
				rerender();
			});
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Identity</h2>
			{isNew ? (
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
					Welcome! Set up your agent's identity to get started.
				</p>
			) : null}
			<form onSubmit={onSave} style={{ maxWidth: "600px", display: "flex", flexDirection: "column", gap: "16px" }}>
				{/* Agent section */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Agent
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Saved to <code>IDENTITY.md</code> in your workspace root.
					</p>
					<div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "8px 16px" }}>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Name *
							</div>
							<input
								type="text"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={name}
								onInput={(e: Event) => setName((e.target as HTMLInputElement).value)}
								onBlur={onNameBlur}
								placeholder="e.g. Rex"
							/>
						</div>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Emoji
							</div>
							<EmojiPicker value={emoji} onChange={setEmoji} onSelect={onEmojiSelect} />
						</div>
						<div style={{ gridColumn: "1/-1" }}>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Theme
							</div>
							<input
								type="text"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={theme}
								onInput={(e: Event) => setTheme((e.target as HTMLInputElement).value)}
								placeholder="e.g. wise owl, chill fox"
							/>
						</div>
					</div>
					{showFaviconReloadHint ? (
						<div className="mt-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-xs text-[var(--muted)]">
							favicon updates requires reload and may be cached for minutes,{" "}
							<button
								type="button"
								className="cursor-pointer bg-transparent p-0 text-xs text-[var(--text)] underline"
								onClick={onReloadForFavicon}
							>
								requires reload
							</button>
							.
						</div>
					) : null}
				</div>

				{/* User section */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						User
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Saved to your user profile. Depending on memory settings, Moltis may also mirror it to <code>USER.md</code>.
					</p>
					<div>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
							Your name *
						</div>
						<input
							type="text"
							className="provider-key-input"
							style={{ width: "100%", maxWidth: "280px" }}
							value={userName}
							onInput={(e: Event) => setUserName((e.target as HTMLInputElement).value)}
							onBlur={onUserNameBlur}
							placeholder="e.g. Alice"
						/>
					</div>
				</div>

				{/* Language section */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Language
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose the UI language for this browser.
					</p>
					<div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
						<label htmlFor="identityLanguageSelect" className="text-xs text-[var(--muted)]">
							UI language
						</label>
						<select
							id="identityLanguageSelect"
							className="provider-key-input"
							style={{ maxWidth: "220px" }}
							value={uiLanguage}
							onChange={(e: Event) => {
								setUiLanguage((e.target as HTMLSelectElement).value);
								setLanguageSaved(false);
								setLanguageError(null);
								rerender();
							}}
						>
							<option value="auto">Browser default</option>
							<option value="en">English</option>
							<option value="fr">French</option>
							<option value="zh">{"\u7B80\u4F53\u4E2D\u6587"}</option>
						</select>
						<button
							type="button"
							id="identityLanguageApplyBtn"
							className="provider-btn provider-btn-secondary"
							disabled={languageSaving}
							onClick={onApplyLanguage}
						>
							{languageSaving ? "Applying..." : "Apply language"}
						</button>
						{languageSaved ? (
							<span className="text-xs" style={{ color: "var(--accent)" }}>
								Language updated
							</span>
						) : null}
						{languageError ? (
							<span className="text-xs" style={{ color: "var(--error)" }}>
								{languageError}
							</span>
						) : null}
					</div>
				</div>

				{/* Soul section */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "4px" }}>
						Soul
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Personality and tone injected into every conversation. Saved to <code>SOUL.md</code> in your workspace root.
						Leave empty for the default.
					</p>
					<textarea
						className="provider-key-input"
						rows={8}
						style={{ width: "100%", minHeight: "8rem", resize: "vertical", fontSize: ".8rem", lineHeight: 1.5 }}
						placeholder={DEFAULT_SOUL}
						value={soul}
						onInput={(e: Event) => setSoul((e.target as HTMLTextAreaElement).value)}
					/>
					{soul ? (
						<button
							type="button"
							className="provider-btn"
							style={{ marginTop: "6px", fontSize: ".75rem" }}
							onClick={onResetSoul}
						>
							Reset to default
						</button>
					) : null}
				</div>

				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<button
						type="submit"
						className="provider-btn"
						disabled={saving || emojiSaving || nameSaving || userNameSaving}
					>
						{saving || emojiSaving || nameSaving || userNameSaving ? "Saving\u2026" : "Save"}
					</button>
					{saved ? (
						<span className="text-xs" style={{ color: "var(--accent)" }}>
							Saved
						</span>
					) : null}
					{error ? (
						<span className="text-xs" style={{ color: "var(--error)" }}>
							{error}
						</span>
					) : null}
				</div>
			</form>
			{gon.get("version") ? (
				<p className="text-xs text-[var(--muted)]" style={{ marginTop: "auto", paddingTop: "16px" }}>
					v{gon.get("version")}
				</p>
			) : null}
		</div>
	);
}

// ── Environment section ──────────────────────────────────────

function EnvironmentSection(): VNode {
	const [envVars, setEnvVars] = useState<EnvVar[]>([]);
	const [envLoading, setEnvLoading] = useState(true);
	const [newKey, setNewKey] = useState("");
	const [newValue, setNewValue] = useState("");
	const [envMsg, setEnvMsg] = useState<string | null>(null);
	const [envErr, setEnvErr] = useState<string | null>(null);
	const [saving, setSaving] = useState(false);
	const [updateId, setUpdateId] = useState<string | null>(null);
	const [updateValue, setUpdateValue] = useState("");

	function fetchEnvVars(): void {
		fetch("/api/env")
			.then((r) => (r.ok ? r.json() : { env_vars: [] }))
			.then((d: { env_vars?: EnvVar[] }) => {
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

	function onAdd(e: Event): void {
		e.preventDefault();
		setEnvErr(null);
		setEnvMsg(null);
		const key = newKey.trim();
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
					return r
						.json()
						.then((d: unknown) =>
							setEnvErr(
								localizedApiErrorMessage(d as Parameters<typeof localizedApiErrorMessage>[0], "Failed to save"),
							),
						);
				}
				setSaving(false);
				rerender();
			})
			.catch((err: Error) => {
				setEnvErr(err.message);
				setSaving(false);
				rerender();
			});
	}

	function onDelete(id: string): void {
		fetch(`/api/env/${id}`, { method: "DELETE" }).then(() => fetchEnvVars());
	}

	function onStartUpdate(id: string): void {
		setUpdateId(id);
		setUpdateValue("");
		rerender();
	}

	function onCancelUpdate(): void {
		setUpdateId(null);
		setUpdateValue("");
		rerender();
	}

	function onConfirmUpdate(key: string): void {
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

	const envVaultStatus = gon.get("vault_status");

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Environment Variables</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Environment variables are injected into sandbox command execution. Values are write-only and never displayed.
			</p>
			{envVaultStatus && envVaultStatus !== "disabled" ? (
				<div
					className="text-xs"
					style={{
						maxWidth: "600px",
						padding: "8px 12px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--bg)",
					}}
				>
					{envVaultStatus === "unsealed" ? (
						<>
							<span style={{ color: "var(--accent)" }}>Vault unlocked.</span> Your keys are stored encrypted.
						</>
					) : envVaultStatus === "sealed" ? (
						<>
							<span style={{ color: "var(--warning,var(--error))" }}>Vault locked.</span> Encrypted keys can{"\u2019"}t
							be read {"\u2014"} sandbox commands won{"\u2019"}t work.{" "}
							<a href="/settings/vault" style={{ color: "inherit", textDecoration: "underline" }}>
								Unlock in Encryption settings.
							</a>
						</>
					) : (
						<>
							<span className="text-[var(--muted)]">Vault not set up.</span>{" "}
							<a href="/settings/security" style={{ color: "inherit", textDecoration: "underline" }}>
								Set a password
							</a>{" "}
							to encrypt your stored keys.
						</>
					)}
				</div>
			) : null}

			{envLoading ? (
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			) : (
				<>
					{/* Existing variables */}
					<div style={{ maxWidth: "600px" }}>
						{envVars.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{envVars.map((v) => (
									<div className="provider-item" style={{ marginBottom: 0 }} key={v.id}>
										{updateId === v.id ? (
											<form
												style={{ display: "flex", alignItems: "center", gap: "6px", flex: 1 }}
												onSubmit={(e: Event) => {
													e.preventDefault();
													onConfirmUpdate(v.key);
												}}
											>
												<code style={{ fontSize: "0.8rem", fontFamily: "var(--font-mono)" }}>{v.key}</code>
												{v.encrypted ? (
													<span className="provider-item-badge configured">Encrypted</span>
												) : (
													<span className="provider-item-badge muted">Plaintext</span>
												)}
												<input
													type="password"
													className="provider-key-input"
													name="env_update_value"
													autoComplete="new-password"
													autoCorrect="off"
													autoCapitalize="off"
													spellcheck={false}
													value={updateValue}
													onInput={(e: Event) => setUpdateValue((e.target as HTMLInputElement).value)}
													placeholder="New value"
													style={{ flex: 1 }}
													autoFocus
												/>
												<button type="submit" className="provider-btn">
													Save
												</button>
												<button type="button" className="provider-btn" onClick={onCancelUpdate}>
													Cancel
												</button>
											</form>
										) : (
											<>
												<div style={{ flex: 1, minWidth: 0 }}>
													<div
														className="provider-item-name"
														style={{ fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
													>
														{v.key}
														{v.encrypted ? (
															<span className="provider-item-badge configured" style={{ marginLeft: "6px" }}>
																Encrypted
															</span>
														) : (
															<span className="provider-item-badge muted" style={{ marginLeft: "6px" }}>
																Plaintext
															</span>
														)}
													</div>
													<div
														style={{
															fontSize: ".7rem",
															color: "var(--muted)",
															marginTop: "2px",
															display: "flex",
															gap: "12px",
														}}
													>
														<span>{"\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022"}</span>
														<time dateTime={v.updated_at}>{v.updated_at}</time>
													</div>
												</div>
												<div style={{ display: "flex", gap: "4px" }}>
													<button className="provider-btn provider-btn-sm" onClick={() => onStartUpdate(v.id)}>
														Update
													</button>
													<button
														className="provider-btn provider-btn-sm provider-btn-danger"
														onClick={() => onDelete(v.id)}
													>
														Delete
													</button>
												</div>
											</>
										)}
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)]" style={{ padding: "12px 0" }}>
								No environment variables set.
							</div>
						)}
					</div>

					{/* Add variable */}
					<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
						<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
							Add Variable
						</h3>
						<form onSubmit={onAdd}>
							<div style={{ display: "flex", gap: "8px", flexWrap: "wrap" }}>
								<input
									type="text"
									className="provider-key-input"
									name="env_key"
									autoComplete="off"
									autoCorrect="off"
									autoCapitalize="off"
									spellcheck={false}
									value={newKey}
									onInput={(e: Event) => setNewKey((e.target as HTMLInputElement).value)}
									placeholder="KEY_NAME"
									style={{ flex: 1, minWidth: "120px", fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
								/>
								<input
									type="password"
									className="provider-key-input"
									name="env_value"
									autoComplete="new-password"
									autoCorrect="off"
									autoCapitalize="off"
									spellcheck={false}
									value={newValue}
									onInput={(e: Event) => setNewValue((e.target as HTMLInputElement).value)}
									placeholder="Value"
									style={{ flex: 2, minWidth: "200px" }}
								/>
								<button type="submit" className="provider-btn" disabled={saving || !newKey.trim()}>
									{saving ? "Saving\u2026" : "Add"}
								</button>
							</div>
							{envMsg ? (
								<div className="text-xs" style={{ marginTop: "6px", color: "var(--accent)" }}>
									{envMsg}
								</div>
							) : null}
							{envErr ? (
								<div className="text-xs" style={{ marginTop: "6px", color: "var(--error)" }}>
									{envErr}
								</div>
							) : null}
						</form>
					</div>
				</>
			)}
		</div>
	);
}

// ── Security section ─────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing auth, passwords, passkeys, and API keys
function SecuritySection(): VNode {
	const [authDisabled, setAuthDisabled] = useState(false);
	const [localhostOnly, setLocalhostOnly] = useState(false);
	const [hasPassword, setHasPassword] = useState(true);
	const [hasPasskeys, setHasPasskeys] = useState(false);
	const [setupComplete, setSetupComplete] = useState(false);
	const [authLoading, setAuthLoading] = useState(true);

	const [curPw, setCurPw] = useState("");
	const [newPw, setNewPw] = useState("");
	const [confirmPw, setConfirmPw] = useState("");
	const [pwMsg, setPwMsg] = useState<string | null>(null);
	const [pwErr, setPwErr] = useState<string | null>(null);
	const [pwSaving, setPwSaving] = useState(false);
	const [pwAwaitingReauth, setPwAwaitingReauth] = useState(false);
	const [pwRecoveryKey, setPwRecoveryKey] = useState<string | null>(null);
	const [pwRecoveryCopied, setPwRecoveryCopied] = useState(false);

	const [passkeys, setPasskeys] = useState<PasskeyEntry[]>([]);
	const [pkName, setPkName] = useState("");
	const [pkMsg, setPkMsg] = useState<string | null>(null);
	const [pkLoading, setPkLoading] = useState(true);
	const [editingPk, setEditingPk] = useState<string | null>(null);
	const [editingPkName, setEditingPkName] = useState("");
	const [passkeyOrigins, setPasskeyOrigins] = useState<string[]>([]);
	const [passkeyHostUpdateHosts, setPasskeyHostUpdateHosts] = useState<string[]>([]);

	const [apiKeys, setApiKeys] = useState<ApiKeyEntry[]>([]);
	const [akLabel, setAkLabel] = useState("");
	const [akNew, setAkNew] = useState<string | null>(null);
	const [akLoading, setAkLoading] = useState(true);
	const [akFullAccess, setAkFullAccess] = useState(true);
	const [akScopes, setAkScopes] = useState<AkScopes>({
		"operator.read": false,
		"operator.write": false,
		"operator.approvals": false,
		"operator.pairing": false,
	});

	function notifyAuthStatusChanged(): void {
		window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
	}

	function deferNextPasswordChangedRedirect(): void {
		(window as unknown as Record<string, boolean>).__moltisSuppressNextPasswordChangedRedirect = true;
	}

	function clearPasswordChangedRedirectDeferral(): void {
		(window as unknown as Record<string, boolean>).__moltisSuppressNextPasswordChangedRedirect = false;
	}

	// A credential added while localhost-bypass is active can immediately make the
	// current session unauthenticated (no session cookie). Reload so middleware
	// can route to /login in that transition.
	function refreshPasskeyHostStatus(): Promise<void> {
		return fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((status: { passkey_host_update_hosts?: string[]; passkey_origins?: string[] } | null) => {
				if (Array.isArray(status?.passkey_host_update_hosts))
					setPasskeyHostUpdateHosts(status!.passkey_host_update_hosts);
				if (Array.isArray(status?.passkey_origins)) setPasskeyOrigins(status!.passkey_origins);
			});
	}

	function reloadIfAuthNowRequiresLogin({ reload = true } = {}): Promise<boolean> {
		return fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d: { auth_disabled?: boolean; setup_required?: boolean; authenticated?: boolean } | null) => {
				const mustLogin = !!(d && d.auth_disabled === false && d.setup_required === false && d.authenticated === false);
				if (mustLogin && reload) {
					window.location.reload();
					return true;
				}
				return mustLogin;
			})
			.catch(() => false);
	}

	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then(
				(
					d: {
						auth_disabled?: boolean;
						localhost_only?: boolean;
						has_password?: boolean;
						has_passkeys?: boolean;
						setup_complete?: boolean;
						passkey_origins?: string[];
						passkey_host_update_hosts?: string[];
					} | null,
				) => {
					if (typeof d?.auth_disabled === "boolean") setAuthDisabled(d.auth_disabled);
					if (typeof d?.localhost_only === "boolean") setLocalhostOnly(d.localhost_only);
					if (typeof d?.has_password === "boolean") setHasPassword(d.has_password);
					if (typeof d?.has_passkeys === "boolean") setHasPasskeys(d.has_passkeys);
					if (typeof d?.setup_complete === "boolean") setSetupComplete(d.setup_complete);
					if (Array.isArray(d?.passkey_origins)) setPasskeyOrigins(d!.passkey_origins!);
					if (Array.isArray(d?.passkey_host_update_hosts)) setPasskeyHostUpdateHosts(d!.passkey_host_update_hosts!);
					setAuthLoading(false);
					rerender();
				},
			)
			.catch(() => {
				setAuthLoading(false);
				rerender();
			});
		fetch("/api/auth/passkeys")
			.then((r) => (r.ok ? r.json() : { passkeys: [] }))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				setPkLoading(false);
				rerender();
			})
			.catch(() => setPkLoading(false));
		fetch("/api/auth/api-keys")
			.then((r) => (r.ok ? r.json() : { api_keys: [] }))
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				setAkLoading(false);
				rerender();
			})
			.catch(() => setAkLoading(false));
	}, []);

	function onChangePw(e: Event): void {
		e.preventDefault();
		setPwErr(null);
		setPwMsg(null);
		if (newPw.length < 12) {
			setPwErr("New password must be at least 12 characters.");
			return;
		}
		if (newPw !== confirmPw) {
			setPwErr("Passwords do not match.");
			return;
		}
		setPwSaving(true);
		setPwAwaitingReauth(false);
		const settingFirstPassword = !hasPassword;
		if (settingFirstPassword) deferNextPasswordChangedRedirect();
		const payload: { new_password: string; current_password?: string } = { new_password: newPw };
		if (hasPassword) payload.current_password = curPw;
		fetch("/api/auth/password/change", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(payload),
		})
			.then((r) => {
				if (!r.ok) {
					return r.text().then((t) => {
						clearPasswordChangedRedirectDeferral();
						setPwErr(t);
						setPwSaving(false);
						setPwAwaitingReauth(false);
						rerender();
					});
				}

				return r.json().then((data: { recovery_key?: string }) => {
					const hasRecoveryKey = !!data.recovery_key;
					setPwMsg(hasPassword ? "Password changed." : "Password set.");
					setCurPw("");
					setNewPw("");
					setConfirmPw("");
					setHasPassword(true);
					setSetupComplete(true);
					setAuthDisabled(false);
					if (hasRecoveryKey) {
						setPwRecoveryKey(data.recovery_key!);
						refreshGon();
					}
					return reloadIfAuthNowRequiresLogin({ reload: !hasRecoveryKey }).then((requiresLoginOrReloaded) => {
						if (hasRecoveryKey && requiresLoginOrReloaded) {
							setPwAwaitingReauth(true);
							setPwMsg("Password set. Save the recovery key, then continue to sign in.");
							setPwSaving(false);
							rerender();
							return;
						}
						clearPasswordChangedRedirectDeferral();
						setPwAwaitingReauth(false);
						if (!requiresLoginOrReloaded) notifyAuthStatusChanged();
						setPwSaving(false);
						rerender();
					});
				});
			})
			.catch((err: Error) => {
				clearPasswordChangedRedirectDeferral();
				setPwErr(err.message);
				setPwSaving(false);
				setPwAwaitingReauth(false);
				rerender();
			});
	}

	function onAddPasskey(): void {
		setPkMsg(null);
		if (/^\d+\.\d+\.\d+\.\d+$/.test(location.hostname) || location.hostname.startsWith("[")) {
			setPkMsg(`Passkeys require a domain name. Use localhost instead of ${location.hostname}`);
			rerender();
			return;
		}
		let requestedRpId: string | null = null;
		fetch("/api/auth/passkey/register/begin", { method: "POST" })
			.then((r) => r.json())
			.then(
				(data: {
					options: { publicKey: PublicKeyCredentialCreationOptions & { challenge: string; user: { id: string } } };
					challenge_id: string;
				}) => {
					const opts = data.options;
					requestedRpId = (opts.publicKey.rp as { id?: string })?.id || null;
					(opts.publicKey as unknown as Record<string, unknown>).challenge = b64ToBuf(
						opts.publicKey.challenge as unknown as string,
					);
					(opts.publicKey.user as unknown as Record<string, unknown>).id = b64ToBuf(
						opts.publicKey.user.id as unknown as string,
					);
					if (
						(opts.publicKey as unknown as { excludeCredentials?: Array<{ id: string | ArrayBuffer }> })
							.excludeCredentials
					) {
						for (const c of (opts.publicKey as unknown as { excludeCredentials: Array<{ id: string | ArrayBuffer }> })
							.excludeCredentials)
							(c as unknown as Record<string, unknown>).id = b64ToBuf(c.id as string);
					}
					return navigator.credentials
						.create({ publicKey: opts.publicKey as unknown as PublicKeyCredentialCreationOptions })
						.then((cred) => ({ cred: cred as PublicKeyCredential, challengeId: data.challenge_id }));
				},
			)
			.then(({ cred, challengeId }: { cred: PublicKeyCredential; challengeId: string }) => {
				const response = cred.response as AuthenticatorAttestationResponse;
				const body = {
					challenge_id: challengeId,
					name: pkName.trim() || detectPasskeyName(cred),
					credential: {
						id: cred.id,
						rawId: bufToB64(cred.rawId),
						type: cred.type,
						response: {
							attestationObject: bufToB64(response.attestationObject),
							clientDataJSON: bufToB64(response.clientDataJSON),
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
							.then((d: { passkeys?: PasskeyEntry[] }) => {
								setPasskeys(d.passkeys || []);
								setHasPasskeys((d.passkeys || []).length > 0);
								setSetupComplete(true);
								setAuthDisabled(false);
								return refreshPasskeyHostStatus().then(() => {
									setPkMsg("Passkey added.");
									notifyAuthStatusChanged();
									rerender();
								});
							});
					});
				}
				return r.text().then((t) => {
					setPkMsg(t);
					rerender();
				});
			})
			.catch((err: Error) => {
				let msg = err.message || "Failed to add passkey";
				if (requestedRpId) {
					msg += ` (RPID: "${requestedRpId}", current origin: "${location.origin}")`;
				}
				setPkMsg(msg);
				rerender();
			});
	}

	function onStartRename(id: string, currentName: string): void {
		setEditingPk(id);
		setEditingPkName(currentName);
		rerender();
	}

	function onCancelRename(): void {
		setEditingPk(null);
		setEditingPkName("");
		rerender();
	}

	function onConfirmRename(id: string): void {
		const name = editingPkName.trim();
		if (!name) return;
		fetch(`/api/auth/passkeys/${id}`, {
			method: "PATCH",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ name }),
		})
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setEditingPk(null);
				setEditingPkName("");
				rerender();
			});
	}

	function onRemovePasskey(id: string): void {
		fetch(`/api/auth/passkeys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/passkeys").then((r) => r.json()))
			.then((d: { passkeys?: PasskeyEntry[] }) => {
				setPasskeys(d.passkeys || []);
				setHasPasskeys((d.passkeys || []).length > 0);
				return refreshPasskeyHostStatus().then(() => {
					notifyAuthStatusChanged();
					rerender();
				});
			});
	}

	function onCreateApiKey(): void {
		if (!akLabel.trim()) return;
		setAkNew(null);
		// Build scopes array if not full access
		let scopes: string[] | null = null;
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
			.then((d: { key?: string }) => {
				setAkNew(d.key || null);
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
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				rerender();
			})
			.catch(() => rerender());
	}

	function toggleScope(scope: string): void {
		setAkScopes((prev) => ({ ...prev, [scope]: !prev[scope] }));
		rerender();
	}

	function onRevokeApiKey(id: string): void {
		fetch(`/api/auth/api-keys/${id}`, { method: "DELETE" })
			.then(() => fetch("/api/auth/api-keys").then((r) => r.json()))
			.then((d: { api_keys?: ApiKeyEntry[] }) => {
				setApiKeys(d.api_keys || []);
				rerender();
			});
	}

	const [resetConfirm, setResetConfirm] = useState(false);
	const [resetBusy, setResetBusy] = useState(false);

	function onResetAuth(): void {
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
			.catch((err: Error) => {
				setPwErr(err.message);
				setResetConfirm(false);
				setResetBusy(false);
				rerender();
			});
	}

	if (authLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	if (authDisabled && !localhostOnly) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<strong style={{ color: "var(--error)" }}>Authentication is disabled</strong>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Anyone with network access can control moltis and your computer. Set up a password to protect your instance.
					</p>
					<button
						type="button"
						className="provider-btn"
						style={{ marginTop: "10px" }}
						onClick={() => {
							window.location.assign("/onboarding");
						}}
					>
						Set up authentication
					</button>
				</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Authentication</h2>

			{authDisabled && localhostOnly ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<strong style={{ color: "var(--error)" }}>Authentication is disabled</strong>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Localhost-only access is safe, but localhost bypass is active. Until you add a password or passkey, this
						browser has full access and Sign out has no effect. Add credentials below to require login on localhost and
						before exposing Moltis to your network.
					</p>
				</div>
			) : null}

			{localhostOnly && !hasPassword && !hasPasskeys && !authDisabled ? (
				<div className="alert-info-text max-w-form">
					<span className="alert-label-info">Note: </span>
					Localhost bypass is active. Until you add a password or passkey, this browser has full access and Sign out has
					no effect. Add credentials to require login on localhost and before exposing Moltis to your network.
				</div>
			) : null}

			{/* Password */}
			<div style={{ maxWidth: "600px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					{hasPassword ? "Change Password" : "Set Password"}
				</h3>
				<form onSubmit={onChangePw}>
					<div style={{ display: "flex", flexDirection: "column", gap: "8px", marginBottom: "10px" }}>
						{hasPassword ? (
							<div>
								<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
									Current password
								</div>
								<input
									type="password"
									className="provider-key-input"
									style={{ width: "100%" }}
									value={curPw}
									onInput={(e: Event) => setCurPw((e.target as HTMLInputElement).value)}
								/>
							</div>
						) : null}
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								{hasPassword ? "New password" : "Password"}
							</div>
							<input
								type="password"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={newPw}
								onInput={(e: Event) => setNewPw((e.target as HTMLInputElement).value)}
								placeholder="At least 12 characters"
							/>
						</div>
						<div>
							<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
								Confirm {hasPassword ? "new " : ""}password
							</div>
							<input
								type="password"
								className="provider-key-input"
								style={{ width: "100%" }}
								value={confirmPw}
								onInput={(e: Event) => setConfirmPw((e.target as HTMLInputElement).value)}
							/>
						</div>
					</div>
					<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
						<button type="submit" className="provider-btn" disabled={pwSaving}>
							{pwSaving
								? hasPassword
									? "Changing\u2026"
									: "Setting\u2026"
								: hasPassword
									? "Change password"
									: "Set password"}
						</button>
						{pwMsg ? (
							<span className="text-xs" style={{ color: "var(--accent)" }}>
								{pwMsg}
							</span>
						) : null}
						{pwErr ? (
							<span className="text-xs" style={{ color: "var(--error)" }}>
								{pwErr}
							</span>
						) : null}
					</div>
				</form>
				{pwRecoveryKey ? (
					<div
						style={{
							marginTop: "12px",
							padding: "12px 16px",
							borderRadius: "6px",
							border: "1px solid var(--border)",
							background: "var(--bg)",
						}}
					>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
							Vault initialized {"\u2014"} save this recovery key
						</div>
						<code
							className="select-all break-all"
							style={{
								fontFamily: "var(--font-mono)",
								fontSize: ".8rem",
								color: "var(--text-strong)",
								display: "block",
								lineHeight: 1.5,
							}}
						>
							{pwRecoveryKey}
						</code>
						<div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "8px" }}>
							<button
								type="button"
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									navigator.clipboard.writeText(pwRecoveryKey).then(() => {
										setPwRecoveryCopied(true);
										setTimeout(() => {
											setPwRecoveryCopied(false);
											rerender();
										}, 2000);
										rerender();
									});
								}}
							>
								{pwRecoveryCopied ? "Copied!" : "Copy"}
							</button>
							{pwAwaitingReauth ? (
								<button
									type="button"
									className="provider-btn"
									onClick={() => {
										clearPasswordChangedRedirectDeferral();
										window.location.assign("/login");
									}}
								>
									Continue to sign in
								</button>
							) : null}
						</div>
						<div className="text-xs" style={{ color: "var(--error)", marginTop: "8px" }}>
							This key will not be shown again. You need it to unlock the vault if you forget your password.
						</div>
					</div>
				) : null}
			</div>

			{/* Passkeys */}
			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					Passkeys
				</h3>
				{passkeyOrigins.length > 1 && (
					<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "8px" }}>
						Passkeys will work when visiting: {passkeyOrigins.map((o) => o.replace(/^https?:\/\//, "")).join(", ")}
					</div>
				)}
				{hasPasskeys && passkeyHostUpdateHosts.length > 0 ? (
					<div className="alert-warning-text max-w-form" style={{ marginBottom: "8px" }}>
						<span className="alert-label-warning">Passkey update needed: </span>
						New host detected ({passkeyHostUpdateHosts.join(", ")}). Sign in with your password on that host, then
						register a new passkey there.
					</div>
				) : null}
				{pkLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
				) : (
					<>
						{passkeys.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{passkeys.map((pk) => (
									<div className="provider-item" style={{ marginBottom: 0 }} key={pk.id}>
										{editingPk === pk.id ? (
											<form
												style={{ display: "flex", alignItems: "center", gap: "6px", flex: 1 }}
												onSubmit={(e: Event) => {
													e.preventDefault();
													onConfirmRename(pk.id);
												}}
											>
												<input
													type="text"
													className="provider-key-input"
													value={editingPkName}
													onInput={(e: Event) => setEditingPkName((e.target as HTMLInputElement).value)}
													style={{ flex: 1 }}
													autoFocus
												/>
												<button type="submit" className="provider-btn provider-btn-sm">
													Save
												</button>
												<button
													type="button"
													className="provider-btn provider-btn-sm provider-btn-secondary"
													onClick={onCancelRename}
												>
													Cancel
												</button>
											</form>
										) : (
											<>
												<div style={{ flex: 1, minWidth: 0 }}>
													<div className="provider-item-name" style={{ fontSize: ".85rem" }}>
														{pk.name}
													</div>
													<div style={{ fontSize: ".7rem", color: "var(--muted)", marginTop: "2px" }}>
														<time dateTime={pk.created_at}>{pk.created_at}</time>
													</div>
												</div>
												<div style={{ display: "flex", gap: "4px" }}>
													<button
														className="provider-btn provider-btn-sm provider-btn-secondary"
														onClick={() => onStartRename(pk.id, pk.name)}
													>
														Rename
													</button>
													<button
														className="provider-btn provider-btn-sm provider-btn-danger"
														onClick={() => onRemovePasskey(pk.id)}
													>
														Remove
													</button>
												</div>
											</>
										)}
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)]" style={{ padding: "4px 0 12px" }}>
								No passkeys registered.
							</div>
						)}
						<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
							<input
								type="text"
								className="provider-key-input"
								value={pkName}
								onInput={(e: Event) => setPkName((e.target as HTMLInputElement).value)}
								placeholder="Passkey name (e.g. MacBook Touch ID)"
								style={{ flex: 1 }}
							/>
							<button type="button" className="provider-btn" onClick={onAddPasskey}>
								Add passkey
							</button>
						</div>
						{pkMsg ? (
							<div className="text-xs text-[var(--muted)]" style={{ marginTop: "6px" }}>
								{pkMsg}
							</div>
						) : null}
					</>
				)}
			</div>

			{/* API Keys */}
			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "4px" }}>
					API Keys
				</h3>
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: "0 0 12px" }}>
					API keys authenticate external tools and scripts connecting to moltis over the WebSocket protocol. Pass the
					key as the <code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>api_key</code> field in the{" "}
					<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>auth</code> object of the{" "}
					<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>connect</code> handshake.
				</p>
				{akLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
				) : (
					<>
						{akNew ? (
							<div
								style={{
									marginBottom: "12px",
									padding: "10px 12px",
									background: "var(--bg)",
									border: "1px solid var(--border)",
									borderRadius: "6px",
								}}
							>
								<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "4px" }}>
									Copy this key now. It won't be shown again.
								</div>
								<code
									style={{
										fontFamily: "var(--font-mono)",
										fontSize: ".78rem",
										wordBreak: "break-all",
										color: "var(--text-strong)",
									}}
								>
									{akNew}
								</code>
							</div>
						) : null}
						{apiKeys.length > 0 ? (
							<div style={{ display: "flex", flexDirection: "column", gap: "6px", marginBottom: "12px" }}>
								{apiKeys.map((ak) => (
									<div className="provider-item" style={{ marginBottom: 0 }} key={ak.id}>
										<div style={{ flex: 1, minWidth: 0 }}>
											<div className="provider-item-name" style={{ fontSize: ".85rem" }}>
												{ak.label}
											</div>
											<div
												style={{
													fontSize: ".7rem",
													color: "var(--muted)",
													marginTop: "2px",
													display: "flex",
													gap: "12px",
													flexWrap: "wrap",
												}}
											>
												<span style={{ fontFamily: "var(--font-mono)" }}>{ak.key_prefix}...</span>
												<span>
													<time dateTime={ak.created_at}>{ak.created_at}</time>
												</span>
												{ak.scopes ? (
													<span style={{ color: "var(--accent)" }}>{ak.scopes.join(", ")}</span>
												) : (
													<span style={{ color: "var(--accent)" }}>Full access</span>
												)}
											</div>
										</div>
										<button className="provider-btn provider-btn-danger" onClick={() => onRevokeApiKey(ak.id)}>
											Revoke
										</button>
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)]" style={{ padding: "4px 0 12px" }}>
								No API keys.
							</div>
						)}
						<div style={{ display: "flex", flexDirection: "column", gap: "10px" }}>
							<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
								<input
									type="text"
									className="provider-key-input"
									value={akLabel}
									onInput={(e: Event) => setAkLabel((e.target as HTMLInputElement).value)}
									placeholder="Key label (e.g. CLI tool)"
									style={{ flex: 1 }}
								/>
							</div>
							<div>
								<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
									<input
										type="checkbox"
										checked={akFullAccess}
										onChange={() => {
											setAkFullAccess(!akFullAccess);
											rerender();
										}}
									/>
									<span className="text-xs text-[var(--text)]">Full access (all permissions)</span>
								</label>
							</div>
							{akFullAccess ? null : (
								<div style={{ paddingLeft: "20px", display: "flex", flexDirection: "column", gap: "6px" }}>
									<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "2px" }}>
										Select permissions:
									</div>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.read"]}
											onChange={() => toggleScope("operator.read")}
										/>
										<span className="text-xs text-[var(--text)]">operator.read</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} View data and status</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.write"]}
											onChange={() => toggleScope("operator.write")}
										/>
										<span className="text-xs text-[var(--text)]">operator.write</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Create, update, delete</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.approvals"]}
											onChange={() => toggleScope("operator.approvals")}
										/>
										<span className="text-xs text-[var(--text)]">operator.approvals</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Handle exec approvals</span>
									</label>
									<label style={{ display: "flex", alignItems: "center", gap: "6px", cursor: "pointer" }}>
										<input
											type="checkbox"
											checked={akScopes["operator.pairing"]}
											onChange={() => toggleScope("operator.pairing")}
										/>
										<span className="text-xs text-[var(--text)]">operator.pairing</span>
										<span className="text-xs text-[var(--muted)]">{"\u2014"} Device/node pairing</span>
									</label>
								</div>
							)}
							<div>
								<button
									type="button"
									className="provider-btn"
									onClick={onCreateApiKey}
									disabled={!(akLabel.trim() && (akFullAccess || Object.values(akScopes).some((v) => v)))}
								>
									Generate key
								</button>
							</div>
						</div>
					</>
				)}
			</div>

			{/* Danger zone (only when auth has been set up) */}
			{setupComplete ? (
				<div style={{ maxWidth: "600px", marginTop: "8px", borderTop: "1px solid var(--error)", paddingTop: "16px" }}>
					<h3 className="text-sm font-medium" style={{ color: "var(--error)", marginBottom: "8px" }}>
						Danger Zone
					</h3>
					<div
						style={{
							padding: "12px 16px",
							border: "1px solid var(--error)",
							borderRadius: "6px",
							background: "color-mix(in srgb, var(--error) 5%, transparent)",
						}}
					>
						<strong className="text-sm" style={{ color: "var(--text-strong)" }}>
							Remove all authentication
						</strong>
						<p className="text-xs text-[var(--muted)]" style={{ margin: "6px 0 0" }}>
							If you know what you're doing, you can fully disable authentication. Anyone with network access will be
							able to access moltis and your computer. This removes your password, all passkeys, all API keys, and all
							sessions.
						</p>
						{resetConfirm ? (
							<div style={{ display: "flex", alignItems: "center", gap: "8px", marginTop: "10px" }}>
								<span className="text-xs" style={{ color: "var(--error)" }}>
									Are you sure? This cannot be undone.
								</span>
								<button
									type="button"
									className="provider-btn provider-btn-danger"
									disabled={resetBusy}
									onClick={onResetAuth}
								>
									{resetBusy ? "Removing\u2026" : "Yes, remove all auth"}
								</button>
								<button
									type="button"
									className="provider-btn"
									onClick={() => {
										setResetConfirm(false);
										rerender();
									}}
								>
									Cancel
								</button>
							</div>
						) : (
							<button
								type="button"
								className="provider-btn provider-btn-danger"
								style={{ marginTop: "10px" }}
								onClick={onResetAuth}
							>
								Remove all authentication
							</button>
						)}
					</div>
				</div>
			) : (
				""
			)}
		</div>
	);
}

// ── Vault (Encryption) section ──────────────────────────────

function VaultSection(): VNode {
	const [vaultStatus, setVaultStatus] = useState(gon.get("vault_status") || null);
	const [unlockPw, setUnlockPw] = useState("");
	const [recoveryKey, setRecoveryKey] = useState("");
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [unlockingPw, setUnlockingPw] = useState(false);
	const [unlockingRk, setUnlockingRk] = useState(false);

	useEffect(() => {
		return gon.onChange("vault_status", (val: unknown) => {
			setVaultStatus(val as string);
			rerender();
		});
	}, []);

	function onUnlockPw(e: Event): void {
		e.preventDefault();
		if (!unlockPw.trim()) return;
		setErr(null);
		setMsg(null);
		setUnlockingPw(true);
		rerender();
		doUnlock("/api/auth/vault/unlock", { password: unlockPw }, () => setUnlockingPw(false));
	}

	function onUnlockRecovery(e: Event): void {
		e.preventDefault();
		if (!recoveryKey.trim()) return;
		setErr(null);
		setMsg(null);
		setUnlockingRk(true);
		rerender();
		doUnlock("/api/auth/vault/recovery", { recovery_key: recoveryKey }, () => setUnlockingRk(false));
	}

	function doUnlock(url: string, body: Record<string, string>, done: () => void): void {
		fetch(url, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		})
			.then((r) => {
				if (r.ok) {
					setMsg("Vault unlocked.");
					setUnlockPw("");
					setRecoveryKey("");
					refreshGon();
				} else {
					return r.text().then((t) => setErr(t || "Unlock failed"));
				}
				done();
				rerender();
			})
			.catch((error: Error) => {
				setErr(error.message);
				done();
				rerender();
			});
	}

	if (!vaultStatus || vaultStatus === "disabled") {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Encryption</h2>
				<p className="text-xs text-[var(--muted)]">Encryption at rest is not available in this build.</p>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Encryption</h2>

			<div style={{ maxWidth: "600px" }}>
				<div className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3 mb-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
						Your API keys and secrets are encrypted at rest using{" "}
						<strong className="text-[var(--text)]">XChaCha20-Poly1305</strong> AEAD with keys derived from your password
						via <strong className="text-[var(--text)]">Argon2id</strong>.
					</p>
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0 mb-1.5">
						The vault uses a two-layer key hierarchy: your password derives a Key Encryption Key (KEK) which unwraps a
						random 256-bit Data Encryption Key (DEK). Changing your password only re-wraps the DEK {"\u2014"} all
						encrypted data stays intact. A recovery key (shown once at setup) provides emergency access if you forget
						your password.
					</p>
					<p className="text-xs text-[var(--muted)] leading-relaxed m-0">
						The vault locks automatically when the server restarts and unlocks when you log in.
					</p>
				</div>

				<div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "12px" }}>
					<span
						className={`provider-item-badge ${vaultStatus === "unsealed" ? "configured" : vaultStatus === "sealed" ? "warning" : "muted"}`}
					>
						{vaultStatus === "unsealed" ? "Unlocked" : vaultStatus === "sealed" ? "Locked" : "Off"}
					</span>
					<span className="text-xs text-[var(--muted)]">
						{vaultStatus === "unsealed"
							? "Your API keys and secrets are encrypted in the database. Everything is working."
							: vaultStatus === "sealed"
								? "Log in or unlock below to access your encrypted keys."
								: "Set a password in Authentication settings to start encrypting your stored keys."}
					</span>
				</div>

				{vaultStatus === "sealed" ? (
					<div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
						<form onSubmit={onUnlockPw} style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							<div className="text-xs text-[var(--muted)]">Unlock with password</div>
							<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
								<input
									type="password"
									className="provider-key-input"
									style={{ flex: 1 }}
									value={unlockPw}
									onInput={(e: Event) => setUnlockPw((e.target as HTMLInputElement).value)}
									placeholder="Your password"
								/>
								<button type="submit" className="provider-btn" disabled={unlockingPw || !unlockPw.trim()}>
									{unlockingPw ? "Unlocking\u2026" : "Unlock"}
								</button>
							</div>
						</form>
						<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
							<div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
							<span className="text-xs text-[var(--muted)]">or</span>
							<div style={{ flex: 1, borderTop: "1px solid var(--border)" }} />
						</div>
						<form onSubmit={onUnlockRecovery} style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							<div className="text-xs text-[var(--muted)]">Unlock with recovery key</div>
							<div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
								<input
									type="password"
									className="provider-key-input"
									style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: ".78rem" }}
									value={recoveryKey}
									onInput={(e: Event) => setRecoveryKey((e.target as HTMLInputElement).value)}
									placeholder="XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"
								/>
								<button type="submit" className="provider-btn" disabled={unlockingRk || !recoveryKey.trim()}>
									{unlockingRk ? "Unlocking\u2026" : "Unlock"}
								</button>
							</div>
						</form>
						{msg ? (
							<div className="text-xs" style={{ color: "var(--accent)" }}>
								{msg}
							</div>
						) : null}
						{err ? (
							<div className="text-xs" style={{ color: "var(--error)" }}>
								{err}
							</div>
						) : null}
					</div>
				) : null}

				{vaultStatus === "uninitialized" ? (
					<div style={{ marginTop: "4px" }}>
						<a
							href="/settings/security"
							className="provider-btn provider-btn-secondary"
							style={{ fontSize: ".75rem", textDecoration: "none", display: "inline-block" }}
						>
							Set a password
						</a>
					</div>
				) : null}
			</div>
		</div>
	);
}

function ToolsSection(): VNode {
	const [loadingTools, setLoadingTools] = useState(true);
	const [toolData, setToolData] = useState<ToolsContextData | null>(null);
	const [nodeInventory, setNodeInventory] = useState<NodeInventoryEntry[]>([]);
	const [toolsErr, setToolsErr] = useState<string | null>(null);

	function loadToolsOverview(): void {
		setLoadingTools(true);
		setToolsErr(null);
		Promise.allSettled([sendRpc("chat.context", {}), sendRpc("node.list", {})])
			.then((results) => {
				const contextResult = results[0];
				if (contextResult.status !== "fulfilled" || !(contextResult.value as RpcResponse)?.ok) {
					const errValue = contextResult.status === "fulfilled" ? (contextResult.value as RpcResponse) : null;
					throw new Error(errValue?.error?.message || "Failed to load tools overview.");
				}
				const nextToolData = ((contextResult.value as RpcResponse).payload || {}) as ToolsContextData;
				const nodesResult = results[1];
				const nextNodeInventory =
					nodesResult.status === "fulfilled" &&
					(nodesResult.value as RpcResponse)?.ok &&
					Array.isArray((nodesResult.value as RpcResponse).payload)
						? ((nodesResult.value as RpcResponse).payload as NodeInventoryEntry[])
						: [];
				setToolData(nextToolData);
				setNodeInventory(nextNodeInventory);
				setLoadingTools(false);
			})
			.catch((error: Error) => {
				setLoadingTools(false);
				setToolsErr(error.message);
			});
	}

	useEffect(() => {
		loadToolsOverview();
	}, []);

	const data = toolData || ({} as ToolsContextData);
	const session = data.session || {};
	const execution = data.execution || {};
	const sandbox = data.sandbox || {};
	const tools: ToolEntry[] = Array.isArray(data.tools) ? data.tools : [];
	const toolGroups = groupToolsForOverview(tools);
	const skills: SkillEntry[] = Array.isArray(data.skills) ? data.skills : [];
	const pluginCount = skills.filter((entry) => entry?.source === "plugin").length;
	const personalSkillCount = skills.length - pluginCount;
	const mcpServers: McpServerEntry[] = Array.isArray(data.mcpServers) ? data.mcpServers : [];
	const runningMcpServers = mcpServers.filter((entry) => entry?.state === "running");
	const runningMcpToolCount = runningMcpServers.reduce((sum, entry) => sum + (Number(entry?.tool_count) || 0), 0);
	const remoteExecInventory = summarizeRemoteExecInventory(nodeInventory);
	const routeDetails: string[] = [];
	routeDetails.push(execution.mode === "sandbox" ? "sandboxed commands" : "host commands");
	if (remoteExecInventory.pairedNodes > 0) {
		routeDetails.push(pluralizeToolsCount(remoteExecInventory.pairedNodes, "paired node"));
	}
	if (remoteExecInventory.sshTargets > 0) {
		routeDetails.push(pluralizeToolsCount(remoteExecInventory.sshTargets, "SSH target"));
	}
	if (remoteExecInventory.pairedNodes === 0 && remoteExecInventory.sshTargets === 0) {
		routeDetails.push("local only");
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-start justify-between gap-3 flex-wrap max-w-[1100px]">
				<div className="min-w-0">
					<h2 className="text-lg font-medium text-[var(--text-strong)]">Tools</h2>
					<p className="text-xs text-[var(--muted)] mt-1 max-w-[900px] leading-relaxed">
						This page shows the effective tool inventory for the active session and model. Change the current LLM,
						disable MCP for a session, or switch execution routes and the inventory here will change with it.
					</p>
				</div>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={loadToolsOverview}
					disabled={loadingTools}
				>
					{loadingTools ? "Refreshing\u2026" : "Refresh"}
				</button>
			</div>

			<div className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3 max-w-[1100px]">
				<div className="text-xs text-[var(--muted)] leading-relaxed">
					Use this as the operator view of what the model can currently reach. For setup changes, jump straight to the
					relevant control surface.
				</div>
				<div className="mt-3 flex gap-2 flex-wrap">
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => navigate(settingsPath("providers"))}
					>
						LLMs
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => navigate(settingsPath("mcp"))}
					>
						MCP
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => navigate(settingsPath("skills"))}
					>
						Skills
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => navigate(settingsPath("nodes"))}
					>
						Nodes
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => navigate(settingsPath("ssh"))}
					>
						SSH
					</button>
				</div>
			</div>

			{toolsErr ? <div className="text-xs text-[var(--error)] max-w-[1100px]">{toolsErr}</div> : null}

			<div className="grid gap-4 md:grid-cols-2 max-w-[1100px]">
				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Tool Calling</div>
					<div className="mt-2 flex items-center gap-2 flex-wrap">
						<span className={`provider-item-badge ${data.supportsTools === false ? "warning" : "configured"}`}>
							{data.supportsTools === false ? "Disabled" : "Enabled"}
						</span>
						<span className="text-sm font-medium text-[var(--text)]">
							{tools.length} registered tool{tools.length === 1 ? "" : "s"}
						</span>
					</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{data.supportsTools === false
							? "The current model is chat-only, so the agent cannot call tools in this session."
							: "Built-in, MCP, and runtime-routed tools available to the active model."}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Active Model</div>
					<div className="mt-2 text-sm font-medium text-[var(--text)] break-words">
						{session.model || "Default model selection"}
					</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{session.provider ? `Provider: ${session.provider}` : "Provider selected automatically."}
						{session.label ? ` Session: ${session.label}.` : ""}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">MCP</div>
					<div className="mt-2 flex items-center gap-2 flex-wrap">
						<span
							className={`provider-item-badge ${
								data.supportsTools === false || data.mcpDisabled
									? "warning"
									: runningMcpServers.length > 0
										? "configured"
										: "muted"
							}`}
						>
							{data.supportsTools === false
								? "Unavailable"
								: data.mcpDisabled
									? "Off for session"
									: runningMcpServers.length > 0
										? "Active"
										: "No running servers"}
						</span>
						<span className="text-sm font-medium text-[var(--text)]">
							{pluralizeToolsCount(runningMcpToolCount, "MCP tool")}
						</span>
					</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{pluralizeToolsCount(runningMcpServers.length, "running server")}
						{data.mcpDisabled ? ", disabled explicitly for this session." : "."}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Execution Routes</div>
					<div className="mt-2 text-sm font-medium text-[var(--text)]">{routeDetails.join(" \u00b7 ")}</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{sandbox.enabled ? `Sandbox backend: ${sandbox.backend || "configured"}. ` : ""}
						{execution.promptSymbol ? `Prompt symbol: ${execution.promptSymbol}. ` : ""}
						The <code className="text-[var(--text)]">exec</code> tool uses these routes rather than exposing SSH as a
						separate command runner.
					</div>
				</div>
			</div>

			{data.supportsTools === false ? (
				<div className="rounded border border-[var(--warn)] bg-[var(--surface2)] p-3 max-w-[1100px]">
					<div className="text-xs text-[var(--muted)] leading-relaxed">
						Tools are unavailable because the current model does not support tool calling. Switch to a tool-capable
						model in <strong className="text-[var(--text)]">Settings {"\u2192"} LLMs</strong> and refresh this page.
					</div>
				</div>
			) : null}

			<div className="grid gap-4 md:grid-cols-2 max-w-[1100px]">
				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="flex items-center justify-between gap-2 flex-wrap">
						<h3 className="text-sm font-medium text-[var(--text-strong)] m-0">Registered Tools</h3>
						<span className="provider-item-badge muted">{tools.length}</span>
					</div>
					{toolGroups.length > 0 ? (
						<div className="mt-3 flex flex-col gap-3">
							{toolGroups.map((group) => (
								<div key={group.label}>
									<div className="text-xs uppercase tracking-wide text-[var(--muted)] mb-2">
										{group.label} {"\u00b7"} {group.tools.length}
									</div>
									<div className="flex flex-col gap-2">
										{group.tools.map((tool) => (
											<div key={tool.name} className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3">
												<div className="flex items-center justify-between gap-2 flex-wrap">
													<div className="text-xs font-medium text-[var(--text)] break-words">{tool.name}</div>
													{tool.name?.startsWith("mcp__") ? (
														<span className="provider-item-badge configured">MCP</span>
													) : null}
												</div>
												<div className="text-xs text-[var(--muted)] mt-1 leading-relaxed">
													{tool.description || "No description provided."}
												</div>
											</div>
										))}
									</div>
								</div>
							))}
						</div>
					) : (
						<div className="text-xs text-[var(--muted)] mt-3">No tools are currently exposed to this session.</div>
					)}
				</div>

				<div className="flex flex-col gap-4">
					<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
						<div className="flex items-center justify-between gap-2 flex-wrap">
							<h3 className="text-sm font-medium text-[var(--text-strong)] m-0">Skills & Plugins</h3>
							<span className="provider-item-badge muted">{skills.length}</span>
						</div>
						<div className="text-xs text-[var(--muted)] mt-3 leading-relaxed">
							{pluralizeToolsCount(personalSkillCount, "skill")}, {pluralizeToolsCount(pluginCount, "plugin")}.
						</div>
						{skills.length > 0 ? (
							<div className="mt-3 flex flex-col gap-2">
								{skills.map((entry) => (
									<div key={entry.name} className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3">
										<div className="flex items-center justify-between gap-2 flex-wrap">
											<div className="text-xs font-medium text-[var(--text)]">{entry.name}</div>
											<span className={`provider-item-badge ${entry.source === "plugin" ? "configured" : "muted"}`}>
												{entry.source === "plugin" ? "Plugin" : "Skill"}
											</span>
										</div>
										<div className="text-xs text-[var(--muted)] mt-1 leading-relaxed">
											{entry.description || "No description provided."}
										</div>
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)] mt-3">No skills or plugins enabled.</div>
						)}
					</div>

					<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
						<div className="flex items-center justify-between gap-2 flex-wrap">
							<h3 className="text-sm font-medium text-[var(--text-strong)] m-0">MCP Servers</h3>
							<span className="provider-item-badge muted">{mcpServers.length}</span>
						</div>
						{mcpServers.length > 0 ? (
							<div className="mt-3 flex flex-col gap-2">
								{mcpServers.map((entry) => (
									<div key={entry.name} className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3">
										<div className="flex items-center justify-between gap-2 flex-wrap">
											<div className="text-xs font-medium text-[var(--text)]">{entry.name}</div>
											<span className={`provider-item-badge ${entry.state === "running" ? "configured" : "warning"}`}>
												{entry.state || "unknown"}
											</span>
										</div>
										<div className="text-xs text-[var(--muted)] mt-1 leading-relaxed">
											{pluralizeToolsCount(Number(entry.tool_count) || 0, "tool")}
										</div>
									</div>
								))}
							</div>
						) : (
							<div className="text-xs text-[var(--muted)] mt-3">No MCP servers configured.</div>
						)}
					</div>
				</div>
			</div>
		</div>
	);
}

function SshSection(): VNode {
	const [loadingSsh, setLoadingSsh] = useState(true);
	const [keys, setKeys] = useState<SshKeyEntry[]>([]);
	const [targets, setTargets] = useState<SshTargetEntry[]>([]);
	const [sshMsg, setSshMsg] = useState<string | null>(null);
	const [sshErr, setSshErr] = useState<string | null>(null);
	const [busyAction, setBusyAction] = useState("");
	const [generateName, setGenerateName] = useState("");
	const [importName, setImportName] = useState("");
	const [importPrivateKey, setImportPrivateKey] = useState("");
	const [importPassphrase, setImportPassphrase] = useState("");
	const [targetLabel, setTargetLabel] = useState("");
	const [targetHost, setTargetHost] = useState("");
	const [targetPort, setTargetPort] = useState("");
	const [targetKnownHost, setTargetKnownHost] = useState("");
	const [targetAuthMode, setTargetAuthMode] = useState("managed");
	const [targetKeyId, setTargetKeyId] = useState("");
	const [targetIsDefault, setTargetIsDefault] = useState(true);
	const [copiedKeyId, setCopiedKeyId] = useState<number | null>(null);
	const [testResults, setTestResults] = useState<Record<number, TestResult>>({});
	const vaultStatus = gon.get("vault_status");

	function setMessage(message: string): void {
		setSshMsg(message);
		setSshErr(null);
	}

	function setError(message: string): void {
		setSshErr(message);
		setSshMsg(null);
	}

	function clearFlash(): void {
		setSshMsg(null);
		setSshErr(null);
	}

	function fetchSshStatus(): Promise<void> {
		setLoadingSsh(true);
		rerender();
		return fetch("/api/ssh")
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to load SSH settings"));
				}
				return response.json();
			})
			.then((data: { keys?: SshKeyEntry[]; targets?: SshTargetEntry[] }) => {
				setKeys(data.keys || []);
				setTargets(data.targets || []);
				if (!targetKeyId && (data.keys || []).length > 0) {
					setTargetKeyId(String(data.keys![0].id));
				}
				setLoadingSsh(false);
				rerender();
			})
			.catch((error: Error) => {
				setLoadingSsh(false);
				setError(error.message);
				rerender();
			});
	}

	useEffect(() => {
		fetchSshStatus();
	}, []);

	function runSshAction(
		actionKey: string,
		url: string,
		payload: Record<string, unknown> | null,
		successMessage: string,
		afterSuccess?: (data: unknown) => void | Promise<void>,
	): Promise<void> {
		clearFlash();
		setBusyAction(actionKey);
		rerender();
		return fetch(url, {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: payload ? JSON.stringify(payload) : "{}",
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "SSH action failed"));
				}
				return response.json().catch(() => ({}));
			})
			.then(async (data: unknown) => {
				if (afterSuccess) await afterSuccess(data);
				setMessage(successMessage);
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onGenerateKey(e: Event): void {
		e.preventDefault();
		const name = generateName.trim();
		if (!name) {
			setError("Key name is required.");
			return;
		}
		runSshAction("generate-key", "/api/ssh/keys/generate", { name }, "Deploy key generated.", () => {
			setGenerateName("");
		});
	}

	function onImportKey(e: Event): void {
		e.preventDefault();
		const name = importName.trim();
		if (!name) {
			setError("Key name is required.");
			return;
		}
		if (!importPrivateKey.trim()) {
			setError("Private key is required.");
			return;
		}
		runSshAction(
			"import-key",
			"/api/ssh/keys/import",
			{
				name,
				private_key: importPrivateKey,
				passphrase: importPassphrase.trim() ? importPassphrase : null,
			},
			"Private key imported.",
			() => {
				setImportName("");
				setImportPrivateKey("");
				setImportPassphrase("");
			},
		);
	}

	function onDeleteKey(id: number): void {
		clearFlash();
		setBusyAction(`delete-key:${id}`);
		rerender();
		fetch(`/api/ssh/keys/${id}`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to delete key"));
				}
				setMessage("SSH key deleted.");
				await fetchSshStatus();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onCreateTarget(e: Event): void {
		e.preventDefault();
		const label = targetLabel.trim();
		const target = targetHost.trim();
		const port = targetPort.trim() ? Number.parseInt(targetPort.trim(), 10) : null;
		const keyId = targetAuthMode === "managed" && targetKeyId ? Number.parseInt(targetKeyId, 10) : null;
		if (!label) {
			setError("Target label is required.");
			return;
		}
		if (!target) {
			setError("SSH target is required.");
			return;
		}
		if (targetAuthMode === "managed" && !keyId) {
			setError("Choose a managed SSH key for this target.");
			return;
		}
		if (Number.isNaN(port)) {
			setError("Port must be a valid number.");
			return;
		}
		runSshAction(
			"create-target",
			"/api/ssh/targets",
			{
				label,
				target,
				port,
				auth_mode: targetAuthMode,
				key_id: keyId,
				known_host: targetKnownHost.trim() ? targetKnownHost : null,
				is_default: targetIsDefault,
			},
			"SSH target saved.",
			() => {
				setTargetLabel("");
				setTargetHost("");
				setTargetPort("");
				setTargetKnownHost("");
				setTargetIsDefault(targets.length === 0);
			},
		);
	}

	function onScanCreateTargetHost(): void {
		const target = targetHost.trim();
		const port = targetPort.trim() ? Number.parseInt(targetPort.trim(), 10) : null;
		if (!target) {
			setError("SSH target is required before scanning.");
			return;
		}
		if (Number.isNaN(port)) {
			setError("Port must be a valid number.");
			return;
		}
		clearFlash();
		setBusyAction("scan-create-target");
		rerender();
		fetch("/api/ssh/host-key/scan", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ target, port }),
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to scan host key"));
				}
				return response.json();
			})
			.then((data: { known_host?: string; host?: string; port?: number }) => {
				setTargetKnownHost(data.known_host || "");
				setMessage(`Scanned host key for ${data.host}${data.port ? `:${data.port}` : ""}.`);
				showToast("Host key scanned", "success");
				rerender();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onDeleteTarget(id: number): void {
		clearFlash();
		setBusyAction(`delete-target:${id}`);
		rerender();
		fetch(`/api/ssh/targets/${id}`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to delete target"));
				}
				setMessage("SSH target deleted.");
				await fetchSshStatus();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onSetDefaultTarget(id: number): void {
		runSshAction(`default-target:${id}`, `/api/ssh/targets/${id}/default`, null, "Default SSH target updated.");
	}

	function onTestTarget(id: number): void {
		clearFlash();
		setBusyAction(`test-target:${id}`);
		rerender();
		fetch(`/api/ssh/targets/${id}/test`, { method: "POST" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "SSH connectivity test failed"));
				}
				return response.json();
			})
			.then((data: TestResult) => {
				setTestResults({
					...testResults,
					[id]: data,
				});
				setMessage(
					data.reachable ? "SSH connectivity test passed." : data.failure_hint || "SSH connectivity test failed.",
				);
				rerender();
			})
			.catch((error: Error) => setError(error.message))
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onScanAndPinTarget(entry: SshTargetEntry): void {
		clearFlash();
		setBusyAction(`pin-target:${entry.id}`);
		rerender();
		fetch("/api/ssh/host-key/scan", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ target: entry.target, port: entry.port ?? null }),
		})
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to scan host key"));
				}
				return response.json();
			})
			.then(async (scanData: { known_host?: string; host?: string; port?: number }) => {
				const pinResponse = await fetch(`/api/ssh/targets/${entry.id}/pin`, {
					method: "POST",
					headers: { "Content-Type": "application/json" },
					body: JSON.stringify({ known_host: scanData.known_host }),
				});
				if (!pinResponse.ok) {
					throw new Error(localizedApiErrorMessage(await pinResponse.json(), "Failed to pin host key"));
				}
				setMessage(
					`${entry.known_host ? "Refreshed" : "Pinned"} host key for ${scanData.host}${scanData.port ? `:${scanData.port}` : ""}.`,
				);
				showToast(entry.known_host ? "Host pin refreshed" : "Host pinned", "success");
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onClearTargetPin(entry: SshTargetEntry): void {
		clearFlash();
		setBusyAction(`clear-pin:${entry.id}`);
		rerender();
		fetch(`/api/ssh/targets/${entry.id}/pin`, { method: "DELETE" })
			.then(async (response) => {
				if (!response.ok) {
					throw new Error(localizedApiErrorMessage(await response.json(), "Failed to clear host pin"));
				}
				setMessage(`Cleared host pin for ${entry.label}.`);
				showToast("Host pin cleared", "success");
				await fetchSshStatus();
			})
			.catch((error: Error) => {
				setError(error.message);
				showToast(error.message, "error");
			})
			.finally(() => {
				setBusyAction("");
				rerender();
			});
	}

	function onCopyPublicKey(entry: SshKeyEntry): void {
		navigator.clipboard
			.writeText(entry.public_key || "")
			.then(() => {
				setCopiedKeyId(entry.id);
				setTimeout(() => {
					setCopiedKeyId(null);
					rerender();
				}, 1500);
				rerender();
			})
			.catch((error: Error) => setError(error.message));
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">SSH</h2>
			<div className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3 max-w-[760px]">
				<p className="text-xs text-[var(--muted)] m-0 mb-1.5 leading-relaxed">
					Manage outbound SSH keys and named remote exec targets. Generated deploy keys use{" "}
					<strong className="text-[var(--text)]">Ed25519</strong>, the private half stays inside Moltis, and the public
					half is shown so you can install it in <code className="text-[var(--text)]">authorized_keys</code>.
				</p>
				<p className="text-xs text-[var(--muted)] m-0 leading-relaxed">
					Current auth path:
					<strong className="text-[var(--text)]">
						{vaultStatus === "unsealed"
							? " vault-backed managed keys are available"
							: vaultStatus === "sealed"
								? " vault is locked, managed keys cannot be used until unlocked"
								: " system OpenSSH remains available, managed keys stay plaintext until the vault is enabled"}
					</strong>
				</p>
			</div>

			{sshMsg ? <div className="text-xs text-[var(--accent)]">{sshMsg}</div> : null}
			{sshErr ? <div className="text-xs text-[var(--error)]">{sshErr}</div> : null}

			<div className="grid gap-4 lg:grid-cols-2 max-w-[1100px]">
				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<h3 className="text-sm font-medium text-[var(--text-strong)] m-0 mb-2">Deploy Keys</h3>
					<p className="text-xs text-[var(--muted)] m-0 mb-3">
						Generate a new keypair for a host, or import an existing private key. Passphrase-protected imports are
						decrypted once and then stored under Moltis control.
					</p>
					<div className="mb-3 rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-xs text-[var(--muted)] leading-relaxed">
						Recommended flow: generate one deploy key per remote host, copy the public key below, add it to that
						host&apos;s <code className="text-[var(--text)]">~/.ssh/authorized_keys</code>, then pin the host key with
						<code className="text-[var(--text)]">ssh-keyscan -H host</code> when creating the target.
					</div>
					<form onSubmit={onGenerateKey} className="flex flex-col gap-2 mb-4">
						<label className="text-xs text-[var(--muted)]">Generate deploy key</label>
						<div className="flex gap-2 flex-wrap">
							<input
								className="provider-key-input flex-1 min-w-[180px]"
								type="text"
								value={generateName}
								onInput={(e: Event) => setGenerateName((e.target as HTMLInputElement).value)}
								placeholder="production-box"
							/>
							<button type="submit" className="provider-btn" disabled={busyAction === "generate-key"}>
								{busyAction === "generate-key" ? "Generating\u2026" : "Generate"}
							</button>
						</div>
					</form>

					<form onSubmit={onImportKey} className="flex flex-col gap-2">
						<label className="text-xs text-[var(--muted)]">Import private key</label>
						<input
							className="provider-key-input"
							type="text"
							value={importName}
							onInput={(e: Event) => setImportName((e.target as HTMLInputElement).value)}
							placeholder="existing-deploy-key"
						/>
						<textarea
							className="provider-key-input min-h-[140px] font-mono text-xs"
							value={importPrivateKey}
							onInput={(e: Event) => setImportPrivateKey((e.target as HTMLTextAreaElement).value)}
							placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
						/>
						<input
							className="provider-key-input"
							type="password"
							value={importPassphrase}
							onInput={(e: Event) => setImportPassphrase((e.target as HTMLInputElement).value)}
							placeholder="Optional import passphrase"
						/>
						<button type="submit" className="provider-btn self-start" disabled={busyAction === "import-key"}>
							{busyAction === "import-key" ? "Importing\u2026" : "Import Key"}
						</button>
					</form>

					<div className="mt-4 flex flex-col gap-2">
						{loadingSsh ? (
							<div className="text-xs text-[var(--muted)]">Loading keys{"\u2026"}</div>
						) : keys.length === 0 ? (
							<div className="text-xs text-[var(--muted)]">No managed SSH keys yet.</div>
						) : (
							keys.map((entry) => (
								<div className="provider-item items-start gap-4" key={entry.id}>
									<div className="flex-1 min-w-0">
										<div className="provider-item-name">{entry.name}</div>
										<div className="text-xs text-[var(--muted)] break-all mt-1">
											<span className="text-[var(--text)]">Fingerprint (SHA256):</span> {entry.fingerprint}
										</div>
										<div className="text-xs text-[var(--muted)] mt-1">
											{entry.encrypted ? "Encrypted in vault" : "Stored plaintext until the vault is available"}
											{(entry.target_count ?? 0) > 0
												? `, used by ${entry.target_count} target${entry.target_count === 1 ? "" : "s"}`
												: ""}
										</div>
										<pre className="mt-3 whitespace-pre-wrap break-all rounded border border-[var(--border)] bg-[var(--surface2)] p-2 text-[11px] leading-relaxed text-[var(--muted)]">
											{entry.public_key}
										</pre>
									</div>
									<div className="flex flex-col gap-2 shrink-0 self-start">
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onCopyPublicKey(entry)}
										>
											{copiedKeyId === entry.id ? "Copied" : "Copy Public Key"}
										</button>
										<button
											type="button"
											className="provider-btn provider-btn-danger"
											onClick={() => onDeleteKey(entry.id)}
											disabled={busyAction === `delete-key:${entry.id}` || (entry.target_count ?? 0) > 0}
										>
											{busyAction === `delete-key:${entry.id}` ? "Deleting\u2026" : "Delete"}
										</button>
									</div>
								</div>
							))
						)}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<h3 className="text-sm font-medium text-[var(--text-strong)] m-0 mb-2">SSH Targets</h3>
					<p className="text-xs text-[var(--muted)] m-0 mb-3">
						Add named hosts for remote execution. Targets can use your system OpenSSH setup or one of the managed keys
						above.
					</p>
					<form onSubmit={onCreateTarget} className="flex flex-col gap-2 mb-4">
						<input
							className="provider-key-input"
							type="text"
							value={targetLabel}
							onInput={(e: Event) => setTargetLabel((e.target as HTMLInputElement).value)}
							placeholder="prod-box"
						/>
						<input
							className="provider-key-input"
							type="text"
							value={targetHost}
							onInput={(e: Event) => setTargetHost((e.target as HTMLInputElement).value)}
							placeholder="deploy@example.com"
						/>
						<div className="flex gap-2 flex-wrap">
							<input
								className="provider-key-input w-[120px]"
								type="number"
								min={1}
								max={65535}
								value={targetPort}
								onInput={(e: Event) => setTargetPort((e.target as HTMLInputElement).value)}
								placeholder="22"
							/>
							<select
								className="provider-key-input flex-1 min-w-[180px]"
								value={targetAuthMode}
								onInput={(e: Event) => setTargetAuthMode((e.target as HTMLSelectElement).value)}
							>
								<option value="managed">Managed key</option>
								<option value="system">System OpenSSH</option>
							</select>
						</div>
						<textarea
							className="provider-key-input min-h-[96px] font-mono text-xs"
							value={targetKnownHost}
							onInput={(e: Event) => setTargetKnownHost((e.target as HTMLTextAreaElement).value)}
							placeholder="Optional known_hosts line from ssh-keyscan -H host"
						/>
						<div className="text-xs text-[var(--muted)]">
							If you paste a <code className="text-[var(--text)]">known_hosts</code> line here, Moltis will use strict
							host-key checking for this target instead of trusting your global SSH config.
						</div>
						<button
							type="button"
							className="provider-btn provider-btn-secondary self-start"
							onClick={onScanCreateTargetHost}
							disabled={busyAction === "scan-create-target"}
						>
							{busyAction === "scan-create-target" ? "Scanning\u2026" : "Scan Host Key"}
						</button>
						{targetAuthMode === "managed" ? (
							<select
								className="provider-key-input"
								value={targetKeyId}
								onInput={(e: Event) => setTargetKeyId((e.target as HTMLSelectElement).value)}
							>
								<option value="">Choose a managed key</option>
								{keys.map((entry) => (
									<option key={entry.id} value={entry.id}>
										{entry.name}
									</option>
								))}
							</select>
						) : null}
						{targetAuthMode === "managed" && keys.length === 0 ? (
							<div className="text-xs text-[var(--muted)]">
								Generate or import a deploy key first. Moltis cannot connect with a managed target until a private key
								exists.
							</div>
						) : null}
						<label className="text-xs text-[var(--muted)] flex items-center gap-2">
							<input
								type="checkbox"
								checked={targetIsDefault}
								onInput={(e: Event) => setTargetIsDefault((e.target as HTMLInputElement).checked)}
							/>
							Set as default remote SSH target
						</label>
						<button
							type="submit"
							className="provider-btn self-start"
							disabled={busyAction === "create-target" || (targetAuthMode === "managed" && keys.length === 0)}
						>
							{busyAction === "create-target" ? "Saving\u2026" : "Add Target"}
						</button>
					</form>

					<div className="flex flex-col gap-2">
						{loadingSsh ? (
							<div className="text-xs text-[var(--muted)]">Loading targets{"\u2026"}</div>
						) : targets.length === 0 ? (
							<div className="text-xs text-[var(--muted)]">No SSH targets configured.</div>
						) : (
							targets.map((entry) => (
								<div className="provider-item" key={entry.id}>
									<div className="flex-1 min-w-0">
										<div className="provider-item-name flex items-center gap-2 flex-wrap">
											<span>{entry.label}</span>
											{entry.is_default ? <span className="provider-item-badge configured">Default</span> : null}
											<span className="provider-item-badge muted">
												{entry.auth_mode === "managed" ? "Managed key" : "System SSH"}
											</span>
											{entry.known_host ? (
												<span className="provider-item-badge configured">Host pinned</span>
											) : (
												<span className="provider-item-badge warning">Uses global known_hosts</span>
											)}
										</div>
										<div className="text-xs text-[var(--muted)] break-all">
											{entry.target}
											{entry.port ? `:${entry.port}` : ""}
										</div>
										<div className="text-xs text-[var(--muted)]">
											{entry.key_name ? `Key: ${entry.key_name}` : "Uses your local ssh config / agent"}
										</div>
										{testResults[entry.id] ? (
											<div className="mt-1">
												<div
													className={`text-xs ${testResults[entry.id].reachable ? "text-[var(--accent)]" : "text-[var(--error)]"}`}
												>
													{testResults[entry.id].reachable ? "Reachable" : "Unreachable"}
												</div>
												{testResults[entry.id].failure_hint ? (
													<div className="text-xs text-[var(--text-muted)] mt-1">
														Hint: {testResults[entry.id].failure_hint}
													</div>
												) : null}
											</div>
										) : null}
									</div>
									<div className="flex flex-col gap-2">
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onTestTarget(entry.id)}
											disabled={busyAction === `test-target:${entry.id}`}
										>
											{busyAction === `test-target:${entry.id}` ? "Testing\u2026" : "Test"}
										</button>
										<button
											type="button"
											className="provider-btn provider-btn-secondary"
											onClick={() => onScanAndPinTarget(entry)}
											disabled={busyAction === `pin-target:${entry.id}`}
										>
											{busyAction === `pin-target:${entry.id}`
												? "Scanning\u2026"
												: entry.known_host
													? "Refresh Pin"
													: "Scan & Pin"}
										</button>
										{entry.known_host ? (
											<button
												type="button"
												className="provider-btn provider-btn-secondary"
												onClick={() => onClearTargetPin(entry)}
												disabled={busyAction === `clear-pin:${entry.id}`}
											>
												{busyAction === `clear-pin:${entry.id}` ? "Clearing\u2026" : "Clear Pin"}
											</button>
										) : null}
										{entry.is_default ? null : (
											<button
												type="button"
												className="provider-btn provider-btn-secondary"
												onClick={() => onSetDefaultTarget(entry.id)}
												disabled={busyAction === `default-target:${entry.id}`}
											>
												Make Default
											</button>
										)}
										<button
											type="button"
											className="provider-btn provider-btn-danger"
											onClick={() => onDeleteTarget(entry.id)}
											disabled={busyAction === `delete-target:${entry.id}`}
										>
											{busyAction === `delete-target:${entry.id}` ? "Deleting\u2026" : "Delete"}
										</button>
									</div>
								</div>
							))
						)}
					</div>
				</div>
			</div>
		</div>
	);
}

// ── b64/buf helpers (used by passkey registration) ──────────

function b64ToBuf(b64: string): ArrayBuffer {
	let str = b64.replace(/-/g, "+").replace(/_/g, "/");
	while (str.length % 4) str += "=";
	const bin = atob(str);
	const buf = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) buf[i] = bin.charCodeAt(i);
	return buf.buffer;
}

function bufToB64(buf: ArrayBuffer): string {
	const bytes = new Uint8Array(buf);
	let str = "";
	for (const b of bytes) str += String.fromCharCode(b);
	return btoa(str).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// ── OpenClaw Import section ───────────────────────────────────

interface ScanResult {
	detected?: boolean;
	home_dir?: string;
	telegram_accounts?: number;
	discord_accounts?: number;
	unsupported_channels?: string[];
	identity_available?: boolean;
	providers_available?: boolean;
	skills_count?: number;
	memory_available?: boolean;
	memory_files_count?: number;
	channels_available?: boolean;
	sessions_count?: number;
}

interface ImportCategory {
	category: string;
	status: string;
	items_imported: number;
	items_skipped: number;
}

interface ImportResult {
	categories?: ImportCategory[];
}

interface ImportSelection {
	identity: boolean;
	providers: boolean;
	skills: boolean;
	memory: boolean;
	channels: boolean;
	sessions: boolean;
	[key: string]: boolean;
}

function OpenClawImportSection(): VNode {
	const [importLoading, setImportLoading] = useState(true);
	const [scan, setScan] = useState<ScanResult | null>(null);
	const [importing, setImporting] = useState(false);
	const [done, setDone] = useState(false);
	const [result, setResult] = useState<ImportResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [selection, setSelection] = useState<ImportSelection>({
		identity: true,
		providers: true,
		skills: true,
		memory: true,
		channels: true,
		sessions: true,
	});

	useEffect(() => {
		let cancelled = false;
		sendRpc("openclaw.scan", {}).then((res: RpcResponse) => {
			if (cancelled) return;
			if (res?.ok) setScan(res.payload as ScanResult);
			else setError("Failed to scan OpenClaw installation");
			setImportLoading(false);
			rerender();
		});
		return () => {
			cancelled = true;
		};
	}, []);

	function toggleCategory(key: string): void {
		setSelection((prev) => {
			const next = Object.assign({}, prev);
			next[key] = !prev[key];
			return next;
		});
	}

	function doImport(): void {
		setImporting(true);
		setError(null);
		sendRpc("openclaw.import", selection).then((res: RpcResponse) => {
			setImporting(false);
			if (res?.ok) {
				setResult(res.payload as ImportResult);
				setDone(true);
			} else {
				setError((res?.error as { message?: string })?.message || "Import failed");
			}
			rerender();
		});
	}

	if (importLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">OpenClaw Import</h2>
				<div className="text-xs text-[var(--muted)]">Scanning{"\u2026"}</div>
			</div>
		);
	}

	if (!scan?.detected) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">OpenClaw Import</h2>
				<div className="text-xs text-[var(--muted)]">No OpenClaw installation detected.</div>
			</div>
		);
	}

	const telegramAccounts = Number(scan.telegram_accounts) || 0;
	const discordAccounts = Number(scan.discord_accounts) || 0;
	const channelParts: string[] = [];
	if (telegramAccounts > 0) channelParts.push(`${telegramAccounts} Telegram account(s)`);
	if (discordAccounts > 0) channelParts.push(`${discordAccounts} Discord account(s)`);
	const channelDetail = channelParts.length > 0 ? channelParts.join(", ") : null;
	const unsupportedChannels = (scan.unsupported_channels || []).filter(
		(channel) => String(channel).toLowerCase() !== "discord",
	);

	const categories = [
		{ key: "identity", label: "Identity", available: scan.identity_available, detail: undefined as string | undefined },
		{
			key: "providers",
			label: "Providers",
			available: scan.providers_available,
			detail: undefined as string | undefined,
		},
		{
			key: "skills",
			label: "Skills",
			available: (scan.skills_count || 0) > 0,
			detail: `${scan.skills_count} skill(s)`,
		},
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
			detail: channelDetail || undefined,
		},
		{
			key: "sessions",
			label: "Sessions",
			available: (scan.sessions_count || 0) > 0,
			detail: `${scan.sessions_count} session(s)`,
		},
	];
	const anySelected = categories.some((c) => c.available && selection[c.key]);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">OpenClaw Import</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Import data from your OpenClaw installation at <code className="text-[var(--text)]">{scan.home_dir}</code>. This
				is a read-only copy {"\u2014"} your OpenClaw files will not be modified or removed. You can keep using both side
				by side and re-import whenever you like.
			</p>
			{error ? (
				<div role="alert" className="alert-error-text whitespace-pre-line" style={{ maxWidth: "600px" }}>
					<span className="text-[var(--error)] font-medium">Error:</span> {error}
				</div>
			) : null}
			{done && result ? (
				<div className="flex flex-col gap-2" style={{ maxWidth: "600px" }}>
					<div className="text-sm font-medium text-[var(--ok)]">
						Import complete:{" "}
						{(result.categories || []).reduce((sum, cat) => sum + (Number(cat.items_imported) || 0), 0)} item(s)
						imported.
					</div>
					{result.categories ? (
						<div className="flex flex-col gap-1">
							{result.categories.map((cat) => (
								<div key={cat.category} className="text-xs text-[var(--text)]">
									<span className="font-mono">
										[
										{cat.status === "success"
											? "\u2713"
											: cat.status === "partial"
												? "~"
												: cat.status === "skipped"
													? "-"
													: "!"}
										]
									</span>{" "}
									{cat.category}: {cat.items_imported} imported, {cat.items_skipped} skipped
								</div>
							))}
						</div>
					) : null}
					<button
						className="provider-btn provider-btn-secondary mt-2"
						style={{ width: "fit-content" }}
						onClick={() => {
							setDone(false);
							setResult(null);
							rerender();
						}}
					>
						Import Again
					</button>
				</div>
			) : (
				<div className="flex flex-col gap-2" style={{ maxWidth: "400px" }}>
					{categories.map((cat) => (
						<label
							key={cat.key}
							className={`flex items-center gap-2 text-sm cursor-pointer ${cat.available ? "text-[var(--text)]" : "text-[var(--muted)] opacity-60"}`}
						>
							<input
								type="checkbox"
								checked={selection[cat.key] && cat.available}
								disabled={!cat.available || importing}
								onChange={() => toggleCategory(cat.key)}
							/>
							<span>{cat.label}</span>
							{cat.detail && cat.available ? <span className="text-xs text-[var(--muted)]">({cat.detail})</span> : null}
							{cat.available ? null : <span className="text-xs text-[var(--muted)]">(not found)</span>}
						</label>
					))}
				</div>
			)}
			{!done && unsupportedChannels.length > 0 ? (
				<p className="text-xs text-[var(--muted)]" style={{ maxWidth: "600px" }}>
					Unsupported channels (coming soon): {unsupportedChannels.join(", ")}
				</p>
			) : null}
			{done ? null : (
				<button
					className="provider-btn mt-2"
					style={{ width: "fit-content" }}
					onClick={doImport}
					disabled={!anySelected || importing}
				>
					{importing ? "Importing\u2026" : "Import Selected"}
				</button>
			)}
		</div>
	);
}

// ── Configuration section ─────────────────────────────────────

function GraphqlSection(): VNode {
	const [loadingConfig, setLoadingConfig] = useState(true);
	const [enabled, setEnabled] = useState(false);
	const [saving, setSaving] = useState(false);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const origin = window.location.origin;
	const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
	const httpEndpoint = `${origin}/graphql`;
	const wsEndpoint = `${wsProtocol}//${window.location.host}/graphql`;

	function loadGraphqlConfig(): void {
		if (!connected.value) {
			setLoadingConfig(true);
			return;
		}
		setLoadingConfig(true);
		sendRpc("graphql.config.get", {})
			.then((res: RpcResponse) => {
				if (res?.ok) {
					setEnabled((res.payload as { enabled?: boolean })?.enabled !== false);
					setErr(null);
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to load GraphQL config");
				}
				setLoadingConfig(false);
				rerender();
			})
			.catch((error: Error) => {
				setErr(error?.message || "Failed to load GraphQL config");
				setLoadingConfig(false);
				rerender();
			});
	}

	useEffect(() => {
		if (connected.value) {
			loadGraphqlConfig();
		} else {
			setLoadingConfig(true);
			setSaving(false);
			setMsg(null);
		}
	}, [connected.value]);

	function onToggle(nextEnabled: boolean): void {
		if (!connected.value) {
			setErr("WebSocket not connected");
			rerender();
			return;
		}
		setSaving(true);
		setMsg(null);
		setErr(null);
		rerender();

		sendRpc("graphql.config.set", { enabled: nextEnabled })
			.then((res: RpcResponse) => {
				setSaving(false);
				if (res?.ok) {
					const payload = res.payload as { enabled?: boolean; persisted?: boolean };
					setEnabled(payload?.enabled !== false);
					if (payload?.persisted === false) {
						setMsg("GraphQL updated for this runtime, but failed to persist to config. It may revert on restart.");
					}
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to update GraphQL setting");
				}
				rerender();
			})
			.catch((error: Error) => {
				setSaving(false);
				setErr(error?.message || "Failed to update GraphQL setting");
				rerender();
			});
	}

	if (!connected.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div className="text-xs text-[var(--muted)]">Connecting{"\u2026"}</div>
			</div>
		);
	}

	if (loadingConfig) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div className="text-xs text-[var(--muted)]">Loading...</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div
				style={{
					maxWidth: "900px",
					padding: "12px 14px",
					borderRadius: "8px",
					border: "1px solid var(--border)",
					background: "var(--surface)",
				}}
			>
				<div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "12px" }}>
					<div>
						<div className="text-sm font-medium text-[var(--text-strong)]">GraphQL server</div>
						{enabled ? (
							<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
								<div>
									HTTP endpoint: <code>{httpEndpoint}</code>
								</div>
								<div style={{ marginTop: "2px" }}>
									WebSocket endpoint: <code>{wsEndpoint}</code>
								</div>
							</div>
						) : null}
					</div>
					<label id="graphqlToggleSwitch" className="toggle-switch">
						<input
							id="graphqlEnabledToggle"
							type="checkbox"
							checked={enabled}
							disabled={saving || loadingConfig || !connected.value}
							onChange={(e: Event) => onToggle((e.target as HTMLInputElement).checked)}
						/>
						<span className="toggle-slider" />
					</label>
				</div>
				{saving ? (
					<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
						Applying...
					</div>
				) : null}
				{msg ? (
					<div className="text-xs text-[var(--ok)]" style={{ marginTop: "8px" }}>
						{msg}
					</div>
				) : null}
				{err ? (
					<div className="text-xs text-[var(--error)]" style={{ marginTop: "8px" }}>
						{err}
					</div>
				) : null}
			</div>

			{enabled ? (
				<div className="flex-1 min-h-0 overflow-hidden rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)]">
					<iframe
						src="/graphql"
						className="h-full w-full border-0"
						title="GraphiQL Playground"
						allow="clipboard-write"
					/>
				</div>
			) : null}
		</div>
	);
}

function ConfigSection(): VNode {
	const [toml, setToml] = useState("");
	const [configPath, setConfigPath] = useState("");
	const [configLoading, setConfigLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [testing, setTesting] = useState(false);
	const [resettingTemplate, setResettingTemplate] = useState(false);
	const [restarting, setRestarting] = useState(false);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);
	const [warnings, setWarnings] = useState<string[]>([]);

	function fetchConfig(): void {
		setConfigLoading(true);
		rerender();
		fetch("/api/config")
			.then((r) => {
				if (!r.ok) {
					return r.text().then((text) => {
						try {
							const json = JSON.parse(text);
							return { error: json.error || `HTTP ${r.status}: ${r.statusText}` };
						} catch (_e) {
							return { error: `HTTP ${r.status}: ${r.statusText}` };
						}
					});
				}
				return r.json().catch(() => ({ error: "Invalid JSON response from server" }));
			})
			.then((d: { error?: string; toml?: string; path?: string }) => {
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
			.catch((fetchErr: Error) => {
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server. Please check if moltis is running.";
				}
				setErr(errMsg);
				setConfigLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		fetchConfig();
	}, []);

	function onTest(e: Event): void {
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
			.then((r) => r.json().catch(() => ({ error: "Invalid JSON response" })))
			.then((d: { valid?: boolean; error?: string; warnings?: string[] }) => {
				setTesting(false);
				if (d.valid) {
					setMsg("Configuration is valid.");
					setWarnings(d.warnings || []);
				} else {
					setErr(d.error || "Invalid configuration");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setTesting(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onSave(e: Event): void {
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
			.then((r) => r.json().catch(() => ({ error: "Invalid JSON response" })))
			.then((d: { ok?: boolean; error?: string }) => {
				setSaving(false);
				if (d.ok) {
					setMsg("Configuration saved. Restart required for changes to take effect.");
				} else {
					setErr(d.error || "Failed to save");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setSaving(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	function onRestart(): void {
		setRestarting(true);
		setMsg("Restarting moltis...");
		setErr(null);
		rerender();

		fetch("/api/restart", { method: "POST" })
			.then((r) =>
				r
					.json()
					.catch(() => ({}))
					.then((d: { error?: string }) => ({ status: r.status, data: d })),
			)
			.then(({ status, data }: { status: number; data: { error?: string } }) => {
				if (status >= 400 && data.error) {
					setRestarting(false);
					setErr(data.error);
					setMsg(null);
					rerender();
				} else {
					setTimeout(waitForRestart, 1000);
				}
			})
			.catch(() => {
				setTimeout(waitForRestart, 1000);
			});
	}

	function waitForRestart(): void {
		let attempts = 0;
		const maxAttempts = 30;

		function check(): void {
			attempts++;
			fetch("/api/gon", { method: "GET" })
				.then((r) => {
					if (r.ok) {
						window.location.reload();
					} else if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr("Server did not come back up. Check if moltis is running.");
						rerender();
					}
				})
				.catch(() => {
					if (attempts < maxAttempts) {
						setTimeout(check, 1000);
					} else {
						setRestarting(false);
						setErr("Server did not come back up. Check if moltis is running.");
						rerender();
					}
				});
		}

		check();
	}

	function onReset(): void {
		fetchConfig();
		setMsg(null);
		setErr(null);
		setWarnings([]);
	}

	function onResetToTemplate(): void {
		if (
			!confirm(
				"Replace current config with the default template?\n\nThis will show all available options with documentation. Your current values will be lost unless you copy them first.",
			)
		) {
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
					return { error: `HTTP ${r.status}: Failed to load template` };
				}
				return r.json().catch(() => ({ error: "Invalid JSON response" }));
			})
			.then((d: { error?: string; toml?: string }) => {
				setResettingTemplate(false);
				if (d.error) {
					setErr(d.error);
				} else {
					setToml(d.toml || "");
					setMsg("Loaded default template with all options. Review and save when ready.");
				}
				rerender();
			})
			.catch((fetchErr: Error) => {
				setResettingTemplate(false);
				let errMsg = fetchErr.message || "Network error";
				if (errMsg.includes("pattern")) {
					errMsg = "Failed to connect to server";
				}
				setErr(errMsg);
				rerender();
			});
	}

	if (configLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Configuration</h2>
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Configuration</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "700px", margin: 0 }}>
				Edit the full moltis configuration. This includes server, tools, LLM providers, auth, and all other settings.
				Test your changes before saving. Changes require a restart to take effect.{" "}
				<a
					href="https://docs.moltis.org/configuration.html"
					target="_blank"
					rel="noopener"
					style={{ color: "var(--accent)", textDecoration: "underline" }}
				>
					View documentation {"\u2197"}
				</a>
			</p>
			{configPath ? (
				<div className="text-xs text-[var(--muted)]" style={{ fontFamily: "var(--font-mono)" }}>
					<span style={{ opacity: 0.7 }}>File:</span> {configPath}
				</div>
			) : null}

			<form onSubmit={onSave} style={{ maxWidth: "800px" }}>
				<div style={{ marginBottom: "12px" }}>
					<textarea
						className="provider-key-input"
						rows={20}
						style={{
							width: "100%",
							minHeight: "320px",
							resize: "vertical",
							fontFamily: "var(--font-mono)",
							fontSize: ".78rem",
							lineHeight: 1.5,
							whiteSpace: "pre",
							overflowWrap: "normal",
							overflowX: "auto",
						}}
						value={toml}
						onInput={(e: Event) => {
							setToml((e.target as HTMLTextAreaElement).value);
							setMsg(null);
							setErr(null);
							setWarnings([]);
						}}
						spellcheck={false}
					/>
				</div>

				{warnings.length > 0 ? (
					<div
						style={{
							marginBottom: "12px",
							padding: "10px 12px",
							background: "color-mix(in srgb, orange 10%, transparent)",
							border: "1px solid orange",
							borderRadius: "6px",
						}}
					>
						<div className="text-xs font-medium" style={{ color: "orange", marginBottom: "6px" }}>
							Warnings:
						</div>
						<ul style={{ margin: 0, paddingLeft: "16px" }}>
							{warnings.map((w, i) => (
								<li key={i} className="text-xs text-[var(--muted)]" style={{ margin: "4px 0" }}>
									{w}
								</li>
							))}
						</ul>
					</div>
				) : null}

				<div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onTest}
						disabled={testing || saving || resettingTemplate || restarting}
					>
						{testing ? "Testing\u2026" : "Test"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onReset}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						Reload
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={onResetToTemplate}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{resettingTemplate ? "Resetting\u2026" : "Reset to defaults"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-danger"
						onClick={onRestart}
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{restarting ? "Restarting\u2026" : "Restart"}
					</button>
					<div style={{ flex: 1 }} />
					<button
						type="submit"
						className="provider-btn"
						disabled={saving || testing || resettingTemplate || restarting}
					>
						{saving ? "Saving\u2026" : "Save"}
					</button>
				</div>

				{msg ? (
					<div className="text-xs" style={{ marginTop: "8px", color: "var(--accent)" }}>
						{msg}
					</div>
				) : null}
				{err ? (
					<div
						className="text-xs"
						style={{ marginTop: "8px", color: "var(--error)", whiteSpace: "pre-wrap", fontFamily: "var(--font-mono)" }}
					>
						{err}
					</div>
				) : null}
				{restarting ? (
					<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
						The page will reload automatically when the server is back up.
					</div>
				) : null}
			</form>

			<div style={{ maxWidth: "800px", marginTop: "8px", paddingTop: "16px", borderTop: "1px solid var(--border)" }}>
				<p className="text-xs text-[var(--muted)] leading-relaxed">
					<strong>Tip:</strong> Click "Load Template" to see all available configuration options with documentation.
					This replaces the editor content with a fully documented template - copy your current values first if needed.
				</p>
			</div>
		</div>
	);
}

// ── Remote access section ────────────────────────────────────

function renderLinkedText(text: string): (string | VNode)[] {
	return String(text || "")
		.split(/(https?:\/\/[^\s]+)/g)
		.filter(Boolean)
		.map((part, index) =>
			/^https?:\/\//.test(part) ? (
				<a key={index} href={part} target="_blank" rel="noopener" className="underline break-all">
					{part}
				</a>
			) : (
				part
			),
		);
}

/** Clone a hidden element from index.html by ID. */
function cloneHidden(id: string): HTMLElement | null {
	const el = document.getElementById(id);
	if (!el) return null;
	const clone = el.cloneNode(true) as HTMLElement;
	clone.removeAttribute("id");
	clone.style.display = "";
	return clone;
}

interface TailscaleStatus {
	installed?: boolean;
	version?: string;
	tailnet?: string;
	login_name?: string;
	tailscale_ip?: string;
	tailscale_up?: boolean;
	mode?: string;
	hostname?: string;
	url?: string;
	passkey_warning?: string;
}

interface NgrokStatus {
	enabled?: boolean;
	authtoken_source?: string;
	domain?: string;
	public_url?: string;
	passkey_warning?: string;
	error?: string;
}

interface NgrokForm {
	enabled: boolean;
	authtoken: string;
	clearAuthtoken: boolean;
	domain: string;
}

function RemoteAccessSection(): VNode {
	const [tsStatus, setTsStatus] = useState<TailscaleStatus | null>(null);
	const [tsError, setTsError] = useState<string | null>(null);
	const [tsWarning, setTsWarning] = useState<string | null>(null);
	const [tsLoading, setTsLoading] = useState(true);
	const [configuring, setConfiguring] = useState(false);
	const [configuringMode, setConfiguringMode] = useState<string | null>(null);
	const [ngStatus, setNgStatus] = useState<NgrokStatus | null>(null);
	const [ngError, setNgError] = useState<string | null>(null);
	const [ngLoading, setNgLoading] = useState(true);
	const [ngSaving, setNgSaving] = useState(false);
	const [ngMsg, setNgMsg] = useState<string | null>(null);
	const [ngForm, setNgForm] = useState<NgrokForm>({
		enabled: false,
		authtoken: "",
		clearAuthtoken: false,
		domain: "",
	});
	const [authReady, setAuthReady] = useState(false);

	function fetchTsStatus(): void {
		setTsLoading(true);
		rerender();
		fetch("/api/tailscale/status")
			.then((r) => {
				const ct = r.headers.get("content-type") || "";
				if (r.status === 404 || !ct.includes("application/json")) {
					setTsError("Tailscale feature is not enabled. Rebuild with --features tailscale.");
					setTsLoading(false);
					rerender();
					return null;
				}
				return r.json();
			})
			.then((data: TailscaleStatus | null) => {
				if (!data) return;
				if ((data as { error?: string }).error) {
					setTsError((data as { error?: string }).error || null);
				} else {
					setTsStatus(data);
					setTsError(null);
					setTsWarning(data.passkey_warning || null);
				}
				setTsLoading(false);
				rerender();
			})
			.catch((e: Error) => {
				setTsError(e.message);
				setTsLoading(false);
				rerender();
			});
	}

	function fetchNgrokStatus(): void {
		setNgLoading(true);
		rerender();
		fetch("/api/ngrok/status")
			.then((r) => {
				const ct = r.headers.get("content-type") || "";
				if (r.status === 404 || !ct.includes("application/json")) {
					setNgError("ngrok feature is not enabled. Rebuild with --features ngrok.");
					setNgStatus(null);
					setNgLoading(false);
					rerender();
					return null;
				}
				return r.json();
			})
			.then((data: NgrokStatus | null) => {
				if (!data) return;
				setNgStatus(data);
				setNgError(data.error || null);
				setNgLoading(false);
				setNgForm({
					enabled: Boolean(data.enabled),
					authtoken: "",
					clearAuthtoken: false,
					domain: data.domain || "",
				});
				rerender();
			})
			.catch((e: Error) => {
				setNgError(e.message);
				setNgLoading(false);
				rerender();
			});
	}

	function setMode(mode: string): void {
		setConfiguring(true);
		setTsError(null);
		setTsWarning(null);
		setConfiguringMode(mode);
		rerender();
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode }),
		})
			.then((r) => r.json())
			.then((data: { error?: string; passkey_warning?: string }) => {
				if (data.error) {
					setTsError(data.error);
				} else {
					setTsWarning(data.passkey_warning || null);
					fetchTsStatus();
				}
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			})
			.catch((e: Error) => {
				setTsError(e.message);
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			});
	}

	function persistNgrokConfig(nextForm: NgrokForm, successMessage: string): void {
		setNgSaving(true);
		setNgError(null);
		setNgMsg(null);
		rerender();

		fetch("/api/ngrok/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				enabled: nextForm.enabled,
				authtoken: nextForm.authtoken,
				clear_authtoken: nextForm.clearAuthtoken,
				domain: nextForm.domain,
			}),
		})
			.then((r) =>
				r
					.json()
					.catch(() => ({}))
					.then((data: { error?: string; status?: NgrokStatus }) => ({ ok: r.ok, data })),
			)
			.then(({ ok, data }: { ok: boolean; data: { error?: string; status?: NgrokStatus } }) => {
				setNgSaving(false);
				if (!ok || data.error) {
					setNgError(data.error || null);
				} else {
					setNgMsg(successMessage);
					if (data.status) {
						setNgStatus(data.status);
						setNgForm({
							enabled: Boolean(data.status.enabled),
							authtoken: "",
							clearAuthtoken: false,
							domain: data.status.domain || "",
						});
					} else {
						fetchNgrokStatus();
					}
				}
				rerender();
			})
			.catch((e: Error) => {
				setNgSaving(false);
				setNgError(e.message);
				rerender();
			});
	}

	function saveNgrokConfig(e: Event): void {
		e.preventDefault();
		persistNgrokConfig(ngForm, "ngrok settings applied.");
	}

	function toggleNgrokEnabled(): void {
		const nextForm = {
			...ngForm,
			enabled: !ngForm.enabled,
		};
		setNgForm(nextForm);
		persistNgrokConfig(nextForm, `ngrok ${nextForm.enabled ? "enabled" : "disabled"}.`);
	}

	function toggleNgrokTokenDeletion(): void {
		if (ngForm.clearAuthtoken) {
			setNgForm({
				...ngForm,
				clearAuthtoken: false,
			});
			return;
		}

		if (!window.confirm("Delete the current ngrok token from config when you save?")) {
			return;
		}

		setNgForm({
			...ngForm,
			authtoken: "",
			clearAuthtoken: true,
		});
	}

	useEffect(() => {
		fetchTsStatus();
		fetchNgrokStatus();
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d: { auth_disabled?: boolean; has_password?: boolean } | null) => {
				if (!d) return;
				const ready = d.auth_disabled ? false : d.has_password === true;
				setAuthReady(ready);
				rerender();
			})
			.catch(() => {
				/* ignore auth status fetch errors */
			});
	}, []);

	function renderTailscaleModeButton(mode: string, currentMode: string): VNode {
		const active = currentMode === mode && !configuring;
		const classes = active
			? "ts-mode-active"
			: "text-[var(--muted)] border-[var(--border)] bg-transparent hover:text-[var(--text)] hover:border-[var(--border-strong)]";
		return (
			<button
				type="button"
				className={`text-xs border px-3 py-1.5 rounded-md cursor-pointer transition-colors font-medium ${classes}${
					configuringMode === mode ? " ts-mode-configuring" : ""
				}`}
				disabled={configuring}
				onClick={() => setMode(mode)}
			>
				{configuringMode === mode ? <span className="ts-spinner" /> : null}
				{mode}
			</button>
		);
	}

	function renderTailscaleCard(): VNode {
		const currentMode = tsStatus?.mode || "off";
		const tsVaultBlocked = tsError === "vault is sealed";
		return (
			<section className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
				<div className="flex flex-col gap-1">
					<h3 className="text-base font-medium text-[var(--text-strong)]">Tailscale</h3>
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Expose the gateway via Tailscale Serve (tailnet-only HTTPS) or Funnel (public HTTPS). The gateway stays
						bound to localhost while Tailscale proxies traffic to it.
					</p>
				</div>

				{tsLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"} this can take a few seconds.</div>
				) : null}
				{tsStatus?.installed ? (
					<div className="info-bar">
						<span className="info-field">
							<span className="status-dot connected" />
							<span className="info-label">Installed</span>
							{tsStatus.version ? <span className="info-version">v{tsStatus.version.split("-")[0]}</span> : null}
						</span>
						{tsStatus.tailnet ? (
							<span className="info-field">
								<span className="info-label">Tailnet:</span>
								<span className="info-value-strong">{tsStatus.tailnet}</span>
							</span>
						) : null}
						{tsStatus.login_name ? (
							<span className="info-field">
								<span className="info-label">Account:</span>
								<span className="info-value">{tsStatus.login_name}</span>
							</span>
						) : null}
						{tsStatus.tailscale_ip ? (
							<span className="info-field">
								<span className="info-label">IP:</span>
								<span className="info-value-mono">{tsStatus.tailscale_ip}</span>
							</span>
						) : null}
					</div>
				) : null}
				{tsError ? (
					<div className="settings-alert-error whitespace-pre-line max-w-form">
						<span className="icon icon-lg icon-warn-triangle shrink-0 mt-0.5" />
						<span>{renderLinkedText(tsError)}</span>
					</div>
				) : null}
				{tsVaultBlocked ? (
					<button type="button" className="provider-btn self-start" onClick={() => navigate(settingsPath("vault"))}>
						Unlock in Encryption settings
					</button>
				) : null}
				{tsWarning ? <div className="alert-warning-text max-w-form">{tsWarning}</div> : null}

				{tsStatus?.installed === false ? (
					<div
						className="info-bar"
						style={{ justifyContent: "center", flexDirection: "column", gap: "12px", textAlign: "center" }}
					>
						<p className="text-sm text-[var(--text)]">
							The <code className="font-mono text-sm">tailscale</code> CLI was not found on this machine.
						</p>
						<div className="flex items-center justify-center gap-2 flex-wrap">
							<a
								href="https://tailscale.com/download"
								target="_blank"
								rel="noopener"
								className="provider-btn"
								style={{ display: "inline-block", textDecoration: "none" }}
							>
								Install Tailscale
							</a>
							<button type="button" className="provider-btn provider-btn-secondary" onClick={fetchTsStatus}>
								Re-check
							</button>
						</div>
					</div>
				) : null}

				{!tsLoading && tsStatus?.installed !== false ? (
					<div className="flex flex-col gap-4">
						{tsStatus?.tailscale_up === false ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> Tailscale is not running. Start it with{" "}
								<code className="font-mono">tailscale up</code> or open the Tailscale app.
							</div>
						) : null}

						<div className="max-w-form flex flex-col gap-2">
							<h4 className="text-sm font-medium text-[var(--text-strong)]">Mode</h4>
							<div className="flex gap-2 flex-wrap">
								{(["off", "serve", "funnel"] as const).map((mode) => renderTailscaleModeButton(mode, currentMode))}
							</div>
							{configuring ? (
								<div className="text-xs text-[var(--muted)]">
									Configuring tailscale {configuringMode}
									{"\u2026"} This can take up to 10 seconds.
								</div>
							) : null}
						</div>

						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Enabling Funnel exposes moltis to the public internet.
							This code has not been security-audited. Use at your own risk.
						</div>
						{authReady ? null : (
							<div className="flex flex-col gap-2 max-w-form">
								<div className="alert-warning-text">
									<span className="alert-label-warn">Warning:</span> Funnel can be enabled now, but remote visitors will
									see the setup-required page until authentication is configured.
								</div>
								<button
									type="button"
									className="provider-btn self-start"
									onClick={() => navigate(settingsPath("security"))}
								>
									Set up authentication
								</button>
							</div>
						)}

						{tsStatus?.hostname ? (
							<div className="max-w-form">
								<h4 className="text-sm font-medium text-[var(--text-strong)] mb-1">Hostname</h4>
								{tsStatus.url && currentMode !== "off" ? (
									<a
										href={tsStatus.url}
										target="_blank"
										rel="noopener"
										className="font-mono text-sm text-[var(--accent)] no-underline"
									>
										{tsStatus.hostname}
									</a>
								) : (
									<div className="font-mono text-sm">{tsStatus.hostname}</div>
								)}
							</div>
						) : null}
						{tsStatus?.url && currentMode !== "off" ? (
							<div className="max-w-form">
								<h4 className="text-sm font-medium text-[var(--text-strong)] mb-1">URL</h4>
								<a
									href={tsStatus.url}
									target="_blank"
									rel="noopener"
									className="font-mono text-sm text-[var(--accent)] no-underline break-all"
								>
									{tsStatus.url}
								</a>
							</div>
						) : null}
						{currentMode === "funnel" ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> Funnel exposes your gateway to the public internet.
								Make sure password authentication is configured.
							</div>
						) : null}
					</div>
				) : null}
			</section>
		);
	}

	function renderNgrokCard(): VNode {
		const authSourceLabel =
			ngStatus?.authtoken_source === "config"
				? "Stored in config"
				: ngStatus?.authtoken_source === "env"
					? "Using NGROK_AUTHTOKEN from the environment"
					: "No authtoken configured yet";
		const ngVaultBlocked = ngError === "vault is sealed";

		return (
			<section className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
				<div className="flex flex-col gap-1">
					<h3 className="text-base font-medium text-[var(--text-strong)]">ngrok</h3>
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Create a public HTTPS endpoint without installing an external binary. Changes apply immediately.
					</p>
				</div>

				{ngLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"} this can take a few seconds.</div>
				) : null}
				{ngError ? (
					<div className="settings-alert-error whitespace-pre-line max-w-form">
						<span className="icon icon-lg icon-warn-triangle shrink-0 mt-0.5" />
						<span>{renderLinkedText(ngError)}</span>
					</div>
				) : null}
				{ngVaultBlocked ? (
					<button type="button" className="provider-btn self-start" onClick={() => navigate(settingsPath("vault"))}>
						Unlock in Encryption settings
					</button>
				) : null}

				{ngLoading || ngError ? null : (
					<form className="flex flex-col gap-4" onSubmit={saveNgrokConfig}>
						<div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2.5 flex items-center justify-between gap-3">
							<div>
								<div className="text-sm font-medium text-[var(--text-strong)]">
									ngrok is {ngForm.enabled ? "enabled" : "disabled"}
								</div>
								<div className="text-xs text-[var(--muted)]">
									Public HTTPS endpoint for demos, shared testing, and team access.
								</div>
							</div>
							<button type="button" className="provider-btn" disabled={ngSaving} onClick={toggleNgrokEnabled}>
								{ngSaving ? "Saving\u2026" : ngForm.enabled ? "Disable ngrok" : "Enable ngrok"}
							</button>
						</div>

						<div className="flex flex-col gap-1">
							<label className="text-sm font-medium text-[var(--text-strong)]" htmlFor="ngrok-authtoken">
								Authtoken
							</label>
							<input
								id="ngrok-authtoken"
								type="password"
								className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-sm text-[var(--text)]"
								placeholder={
									ngStatus?.authtoken_source ? "Leave blank to keep the current token" : "Paste your ngrok authtoken"
								}
								value={ngForm.authtoken}
								onInput={(e: Event) => setNgForm({ ...ngForm, authtoken: (e.currentTarget as HTMLInputElement).value })}
							/>
							<div className="text-xs text-[var(--muted)]">{authSourceLabel}</div>
							<div className="text-xs text-[var(--muted)]">
								Create or copy an authtoken from{" "}
								<a
									href="https://dashboard.ngrok.com/get-started/your-authtoken"
									target="_blank"
									rel="noopener"
									className="text-[var(--accent)] no-underline hover:underline"
								>
									ngrok dashboard
								</a>
								.
							</div>
							{ngStatus?.authtoken_source === "config" ? (
								<div className="flex flex-col gap-1">
									<button
										type="button"
										className="text-xs text-[var(--accent)] self-start bg-transparent border-0 p-0 cursor-pointer hover:underline"
										onClick={toggleNgrokTokenDeletion}
									>
										{ngForm.clearAuthtoken ? "Keep current token" : "Delete current token"}
									</button>
									{ngForm.clearAuthtoken ? (
										<div className="text-xs text-[var(--muted)]">
											The saved config token will be deleted when you save.
										</div>
									) : null}
								</div>
							) : null}
						</div>

						<div className="flex flex-col gap-1">
							<label className="text-sm font-medium text-[var(--text-strong)]" htmlFor="ngrok-domain">
								Reserved domain
							</label>
							<input
								id="ngrok-domain"
								type="text"
								className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-sm text-[var(--text)]"
								placeholder="team-gateway.ngrok.app"
								value={ngForm.domain}
								onInput={(e: Event) => setNgForm({ ...ngForm, domain: (e.currentTarget as HTMLInputElement).value })}
							/>
							<div className="text-xs text-[var(--muted)]">
								Optional. Use a reserved domain if you want a stable passkey origin across restarts.
							</div>
						</div>

						{ngStatus?.public_url ? (
							<div className="flex flex-col gap-1">
								<h4 className="text-sm font-medium text-[var(--text-strong)]">Active public URL</h4>
								<a
									href={ngStatus.public_url}
									target="_blank"
									rel="noopener"
									className="font-mono text-sm text-[var(--accent)] no-underline break-all"
								>
									{ngStatus.public_url}
								</a>
							</div>
						) : null}
						{ngStatus?.passkey_warning ? (
							<div className="alert-warning-text max-w-form">{ngStatus.passkey_warning}</div>
						) : null}
						{ngForm.enabled && !authReady ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> ngrok can be enabled now, but remote visitors will
								see the setup-required page until authentication is configured.
							</div>
						) : null}
						{ngMsg ? <div className="text-xs text-[var(--ok)]">{ngMsg}</div> : null}

						<div className="flex flex-wrap gap-2">
							<button type="submit" className="provider-btn" disabled={ngSaving}>
								{ngSaving ? "Saving\u2026" : "Save ngrok settings"}
							</button>
						</div>
					</form>
				)}
			</section>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Remote Access</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed max-w-[60rem]" style={{ margin: 0 }}>
				Choose how moltis is exposed beyond localhost. Tailscale is the safer default for tailnet access and optional
				public Funnel, while ngrok gives you a managed public HTTPS URL for teams, demos, and shared endpoints.
			</p>
			<div className="flex flex-col gap-4">
				{renderTailscaleCard()}
				{renderNgrokCard()}
			</div>
		</div>
	);
}

// ── Voice section ────────────────────────────────────────────

// Voice section signals
const voiceShowAddModal = signal(false);
const voiceSelectedProvider = signal<string | null>(null);
const voiceSelectedProviderData = signal<VoiceProviderData | null>(null);

interface VoiceProviderData {
	id: string;
	name: string;
	description?: string;
	type?: string;
	category?: string;
	available?: boolean;
	enabled?: boolean;
	keySource?: string;
	settingsSummary?: string;
	binaryPath?: string;
	statusMessage?: string;
	keyPlaceholder?: string;
	keyUrl?: string;
	keyUrlLabel?: string;
	hint?: string;
	capabilities?: { baseUrl?: boolean };
	settings?: { baseUrl?: string; voiceId?: string; voice?: string; model?: string; languageCode?: string };
}

interface VoiceProviders {
	tts: VoiceProviderData[];
	stt: VoiceProviderData[];
}

interface VoiceTesting {
	id: string;
	type: string;
	phase: string;
}

interface VoiceTestResult {
	text?: string | null;
	success?: boolean;
	error?: string | null;
}

interface VoxtralRequirements {
	os?: string;
	arch?: string;
	compatible?: boolean;
	reasons?: string[];
	python?: { available?: boolean; version?: string };
	cuda?: { available?: boolean; gpu_name?: string; memory_mb?: number };
}

function VoiceSection(): VNode {
	const [allProviders, setAllProviders] = useState<VoiceProviders>({ tts: [], stt: [] });
	const [voiceLoading, setVoiceLoading] = useState(true);
	const [voxtralReqs, setVoxtralReqs] = useState<VoxtralRequirements | null>(null);
	const [savingProvider, setSavingProvider] = useState<string | null>(null);
	const [voiceMsg, setVoiceMsg] = useState<string | null>(null);
	const [voiceErr, setVoiceErr] = useState<string | null>(null);
	const [voiceTesting, setVoiceTesting] = useState<VoiceTesting | null>(null);
	const [activeRecorder, setActiveRecorder] = useState<MediaRecorder | null>(null);
	const [voiceTestResults, setVoiceTestResults] = useState<Record<string, VoiceTestResult>>({});

	function fetchVoiceStatus(options?: { silent?: boolean }): void {
		if (!options?.silent) {
			setVoiceLoading(true);
			rerender();
		}
		Promise.all([fetchVoiceProviders(), sendRpc("voice.config.voxtral_requirements", {})])
			.then(([providers, voxtral]) => {
				const provRes = providers as RpcResponse;
				const voxtralRes = voxtral as RpcResponse;
				if (provRes?.ok) setAllProviders((provRes.payload as VoiceProviders) || { tts: [], stt: [] });
				if (voxtralRes?.ok) setVoxtralReqs(voxtralRes.payload as VoxtralRequirements);
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

	function onToggleProvider(provider: VoiceProviderData, enabled: boolean, providerType: string): void {
		setVoiceErr(null);
		setVoiceMsg(null);
		setSavingProvider(provider.id);
		rerender();

		toggleVoiceProvider(provider.id, enabled, providerType)
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSavingProvider(null);
				if (res?.ok) {
					setVoiceMsg(`${provider.name} ${enabled ? "enabled" : "disabled"}.`);
					setTimeout(() => {
						setVoiceMsg(null);
						rerender();
					}, 2000);
					fetchVoiceStatus({ silent: true });
				} else {
					setVoiceErr((res?.error as { message?: string })?.message || "Failed to toggle provider");
				}
				rerender();
			})
			.catch((err: Error) => {
				setSavingProvider(null);
				setVoiceErr(err.message);
				rerender();
			});
	}

	function onConfigureProvider(providerId: string, providerData: VoiceProviderData): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = providerData || null;
		voiceShowAddModal.value = true;
	}

	function getUnconfiguredProviders(): VoiceProviderData[] {
		return [...allProviders.stt, ...allProviders.tts].filter((p) => !p.available);
	}

	// Stop active STT recording
	function stopSttRecording(): void {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	function humanizeMicError(err: { name?: string; message?: string }): string {
		if (err.name === "OverconstrainedError" || (err.message && /constraint/i.test(err.message))) {
			return "No compatible microphone found. Check your audio input device.";
		}
		if (err.name === "NotFoundError" || err.name === "NotAllowedError") {
			return "Microphone access denied or no microphone found. Check browser permissions.";
		}
		if (err.name === "NotReadableError") {
			return "Microphone is in use by another application.";
		}
		return err.message || "STT test failed";
	}

	// Test a voice provider (TTS or STT)
	async function testVoiceProvider(providerId: string, type: string): Promise<void> {
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
				const id = gon.get("identity") as { user_name?: string; name?: string } | undefined;
				const user = id?.user_name || "friend";
				const bot = id?.name || "Moltis";
				const ttsText = await fetchPhrase("settings", user, bot);
				const res = (await testTts(ttsText, providerId)) as RpcResponse;
				if (res?.ok && (res.payload as { audio?: string })?.audio) {
					const payload = res.payload as { audio: string; mimeType?: string; content_type?: string; format?: string };
					// Decode base64 audio and play it
					const bytes = decodeBase64Safe(payload.audio);
					const audioMime = payload.mimeType || payload.content_type || "audio/mpeg";
					console.log("[TTS] audio received: %d bytes, mime=%s, format=%s", bytes.length, audioMime, payload.format);
					const blob = new Blob([bytes as BlobPart], { type: audioMime });
					const url = URL.createObjectURL(blob);
					const audio = new Audio(url);
					audio.onerror = (e) => {
						console.error("[TTS] audio element error:", audio.error?.message || e);
						URL.revokeObjectURL(url);
					};
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e: Error) => console.error("[TTS] play() failed:", e));
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: true, error: null },
					}));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: false, error: (res?.error as { message?: string })?.message || "TTS test failed" },
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: (err as Error).message || "TTS test failed" },
				}));
			}
			setVoiceTesting(null);
		} else {
			// Test STT by recording audio and transcribing
			try {
				const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
				const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
					? "audio/webm;codecs=opus"
					: "audio/webm";
				const mediaRecorder = new MediaRecorder(stream, { mimeType });
				const audioChunks: Blob[] = [];

				mediaRecorder.ondataavailable = (e: BlobEvent) => {
					if (e.data.size > 0) audioChunks.push(e.data);
				};

				mediaRecorder.start();
				setActiveRecorder(mediaRecorder);
				setVoiceTesting({ id: providerId, type, phase: "recording" });
				rerender();

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (const track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });
					rerender();

					const audioBlob = new Blob(audioChunks, { type: mediaRecorder.mimeType || mimeType });

					try {
						const resp = await transcribeAudio(S.activeSessionKey, providerId, audioBlob);
						console.log("[STT] upload response: status=%d ok=%s", resp.status, resp.ok);
						if (resp.ok) {
							const sttRes = (await resp.json()) as {
								ok?: boolean;
								transcription?: { text?: string };
								transcriptionError?: string;
								error?: string;
							};

							if (sttRes.ok && typeof sttRes.transcription?.text === "string") {
								const transcriptText = sttRes.transcription.text.trim();
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
							const errBody = await resp.text();
							console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
							let errMsg = "STT test failed";
							try {
								errMsg = (JSON.parse(errBody) as { error?: string })?.error || errMsg;
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
							[providerId]: { text: null, error: (fetchErr as Error).message || "STT test failed" },
						}));
					}
					setVoiceTesting(null);
					rerender();
				};
			} catch (err) {
				setVoiceErr(humanizeMicError(err as { name?: string; message?: string }));
				setVoiceTesting(null);
			}
		}
		rerender();
	}

	if (voiceLoading || !connected.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>
				<div className="text-xs text-[var(--muted)]">{connected.value ? "Loading\u2026" : "Connecting\u2026"}</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Configure text-to-speech (TTS) and speech-to-text (STT) providers. STT lets you use the microphone button in
				chat to record voice input. TTS lets you hear responses as audio.
			</p>

			{voiceMsg ? <div className="text-xs text-[var(--accent)]">{voiceMsg}</div> : null}
			{voiceErr ? <div className="text-xs text-[var(--error)]">{voiceErr}</div> : null}

			<div style={{ maxWidth: "700px", display: "flex", flexDirection: "column", gap: "24px" }}>
				{/* STT Providers */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-3">Speech-to-Text (Voice Input)</h3>
					<div className="flex flex-col gap-2">
						{allProviders.stt.map((prov) => {
							const meta = prov;
							const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "stt" ? voiceTesting : null;
							const testResult = voiceTestResults[prov.id] || null;
							return (
								<VoiceProviderRow
									key={prov.id}
									provider={prov}
									meta={meta}
									type="stt"
									saving={savingProvider === prov.id}
									testState={testState}
									testResult={testResult}
									onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "stt")}
									onConfigure={() => onConfigureProvider(prov.id, prov)}
									onTest={() => testVoiceProvider(prov.id, "stt")}
								/>
							);
						})}
					</div>
				</div>

				{/* TTS Providers */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-3">Text-to-Speech (Audio Responses)</h3>
					<div className="flex flex-col gap-2">
						{allProviders.tts.map((prov) => {
							const meta = prov;
							const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "tts" ? voiceTesting : null;
							const testResult = voiceTestResults[prov.id] || null;
							return (
								<VoiceProviderRow
									key={prov.id}
									provider={prov}
									meta={meta}
									type="tts"
									saving={savingProvider === prov.id}
									testState={testState}
									testResult={testResult}
									onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "tts")}
									onConfigure={() => onConfigureProvider(prov.id, prov)}
									onTest={() => testVoiceProvider(prov.id, "tts")}
								/>
							);
						})}
					</div>
				</div>
			</div>

			<AddVoiceProviderModal
				unconfiguredProviders={getUnconfiguredProviders()}
				voxtralReqs={voxtralReqs}
				onSaved={() => {
					fetchVoiceStatus();
					voiceShowAddModal.value = false;
					voiceSelectedProvider.value = null;
					voiceSelectedProviderData.value = null;
				}}
			/>
		</div>
	);
}

// Individual provider row with enable toggle

interface VoiceProviderRowProps {
	provider: VoiceProviderData;
	meta: VoiceProviderData;
	type: string;
	saving: boolean;
	testState: VoiceTesting | null;
	testResult: VoiceTestResult | null;
	onToggle: (enabled: boolean) => void;
	onConfigure: () => void;
	onTest: () => void;
}

function VoiceProviderRow({
	provider,
	meta,
	type,
	saving,
	testState,
	testResult,
	onToggle,
	onConfigure,
	onTest,
}: VoiceProviderRowProps): VNode {
	const canEnable = provider.available;
	const keySourceLabel =
		provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";
	const showTestBtn = canEnable && provider.enabled;

	// Determine button text based on test state
	let buttonText = "Test";
	let buttonDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			buttonText = "Stop";
		} else if (testState.phase === "transcribing") {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		} else {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		}
	}

	return (
		<div
			className="provider-card"
			style={{ padding: "10px 14px", borderRadius: "8px", display: "flex", alignItems: "center", gap: "12px" }}
		>
			<div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "2px" }}>
				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<span className="text-sm text-[var(--text-strong)]">{meta.name}</span>
					{provider.category === "local" ? <span className="provider-item-badge">local</span> : null}
					{keySourceLabel ? <span className="text-xs text-[var(--muted)]">{keySourceLabel}</span> : null}
				</div>
				<span className="text-xs text-[var(--muted)]">{meta.description}</span>
				{provider.settingsSummary ? (
					<span className="text-xs text-[var(--muted)]">Voice: {provider.settingsSummary}</span>
				) : null}
				{provider.binaryPath ? (
					<span className="text-xs text-[var(--muted)]">Found at: {provider.binaryPath}</span>
				) : null}
				{!canEnable && provider.statusMessage ? (
					<span className="text-xs text-[var(--muted)]">{provider.statusMessage}</span>
				) : null}
				{testState?.phase === "recording" ? (
					<div className="voice-recording-hint">
						<span className="voice-recording-dot" />
						<span>Speak now, then click Stop when finished</span>
					</div>
				) : null}
				{testState?.phase === "transcribing" ? (
					<span className="text-xs text-[var(--muted)]">Transcribing...</span>
				) : null}
				{testState?.phase === "testing" && type === "tts" ? (
					<span className="text-xs text-[var(--muted)]">Playing audio...</span>
				) : null}
				{testResult?.text ? (
					<div className="voice-transcription-result">
						<span className="voice-transcription-label">Transcribed:</span>
						<span className="voice-transcription-text">"{testResult.text}"</span>
					</div>
				) : null}
				{testResult?.success === true ? (
					<div className="voice-success-result">
						<span className="icon icon-md icon-check-circle" />
						<span>Audio played successfully</span>
					</div>
				) : null}
				{testResult?.error ? (
					<div className="voice-error-result">
						<span className="icon icon-md icon-x-circle" />
						<span>{testResult.error}</span>
					</div>
				) : null}
			</div>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={onConfigure}>
					Configure
				</button>
				{showTestBtn ? (
					<button
						className="provider-btn provider-btn-secondary provider-btn-sm"
						onClick={onTest}
						disabled={buttonDisabled}
						title={type === "tts" ? "Test voice output" : "Test voice input"}
					>
						{buttonText}
					</button>
				) : null}
				{canEnable ? (
					<label className="toggle-switch">
						<input
							type="checkbox"
							checked={provider.enabled}
							disabled={saving}
							onChange={(e: Event) => onToggle((e.target as HTMLInputElement).checked)}
						/>
						<span className="toggle-slider" />
					</label>
				) : provider.category === "local" ? (
					<span className="text-xs text-[var(--muted)]">Install required</span>
				) : null}
			</div>
		</div>
	);
}

// Local provider instructions component (uses hidden HTML elements)

interface LocalProviderInstructionsProps {
	providerId: string;
	voxtralReqs: VoxtralRequirements | null;
}

function LocalProviderInstructions({ providerId, voxtralReqs }: LocalProviderInstructionsProps): VNode {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const container = ref.current;
		if (!container) return;
		while (container.firstChild) container.removeChild(container.firstChild);

		const templateId: Record<string, string> = {
			"whisper-cli": "voice-whisper-cli-instructions",
			"sherpa-onnx": "voice-sherpa-onnx-instructions",
			piper: "voice-piper-instructions",
			coqui: "voice-coqui-instructions",
			"voxtral-local": "voice-voxtral-instructions",
		};

		const tplId = templateId[providerId];
		if (!tplId) return;

		const el = cloneHidden(tplId);
		if (!el) return;

		// For voxtral-local, populate the requirements section
		if (providerId === "voxtral-local" && el.querySelector("[data-voxtral-requirements]")) {
			const reqsContainer = el.querySelector("[data-voxtral-requirements]") as HTMLElement;
			if (voxtralReqs) {
				let detected = `${voxtralReqs.os}/${voxtralReqs.arch}`;
				if (voxtralReqs.python?.available) detected += `, Python ${voxtralReqs.python.version}`;
				else detected += ", no Python";
				if (voxtralReqs.cuda?.available) {
					detected += `, ${voxtralReqs.cuda.gpu_name || "NVIDIA GPU"} (${Math.round((voxtralReqs.cuda.memory_mb || 0) / 1024)}GB)`;
				} else detected += ", no CUDA GPU";

				const reqEl = cloneHidden(
					voxtralReqs.compatible ? "voice-voxtral-requirements-ok" : "voice-voxtral-requirements-fail",
				);
				if (reqEl) {
					const detectedEl = reqEl.querySelector("[data-voxtral-detected]") as HTMLElement;
					if (detectedEl) detectedEl.textContent = detected;
					if (!voxtralReqs.compatible && voxtralReqs.reasons?.length) {
						const ul = reqEl.querySelector("[data-voxtral-reasons]") as HTMLElement;
						for (const r of voxtralReqs.reasons) {
							const li = document.createElement("li");
							li.style.margin = "2px 0";
							li.textContent = r;
							ul.appendChild(li);
						}
					}
					reqsContainer.appendChild(reqEl);
				}
			} else {
				const loadingEl = document.createElement("div");
				loadingEl.className = "text-xs text-[var(--muted)] mb-3";
				loadingEl.textContent = "Checking system requirements\u2026";
				reqsContainer.appendChild(loadingEl);
			}
		}

		container.appendChild(el);
	}, [providerId, voxtralReqs]);

	return <div ref={ref} />;
}

// Add Voice Provider Modal

interface AddVoiceProviderModalProps {
	unconfiguredProviders: VoiceProviderData[];
	voxtralReqs: VoxtralRequirements | null;
	onSaved: () => void;
}

interface ElevenlabsCatalog {
	voices: { id: string; name: string }[];
	models: { id: string; name: string }[];
	warning: string | null;
}

function AddVoiceProviderModal({ unconfiguredProviders, voxtralReqs, onSaved }: AddVoiceProviderModalProps): VNode {
	const [apiKey, setApiKey] = useState("");
	const [baseUrlValue, setBaseUrlValue] = useState("");
	const [voiceValue, setVoiceValue] = useState("");
	const [modelValue, setModelValue] = useState("");
	const [languageCodeValue, setLanguageCodeValue] = useState("");
	const [elevenlabsCatalog, setElevenlabsCatalog] = useState<ElevenlabsCatalog>({
		voices: [],
		models: [],
		warning: null,
	});
	const [elevenlabsCatalogLoading, setElevenlabsCatalogLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState("");

	const selectedProvider = voiceSelectedProvider.value;
	const providerMeta = selectedProvider
		? unconfiguredProviders.find((p) => p.id === selectedProvider) || voiceSelectedProviderData.value
		: null;
	const isElevenLabsProvider = selectedProvider === "elevenlabs" || selectedProvider === "elevenlabs-stt";
	const supportsTtsVoiceSettings = providerMeta?.type === "tts";
	const supportsBaseUrl = providerMeta?.capabilities?.baseUrl === true;

	function onClose(): void {
		voiceShowAddModal.value = false;
		voiceSelectedProvider.value = null;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	function onSaveKey(): void {
		const hasApiKey = apiKey.trim().length > 0;
		const trimmedBaseUrl = baseUrlValue.trim();
		const hadBaseUrl =
			typeof providerMeta?.settings?.baseUrl === "string" && providerMeta.settings.baseUrl.trim().length > 0;
		const hasBaseUrl = supportsBaseUrl && (trimmedBaseUrl.length > 0 || hadBaseUrl);
		const hasSettings =
			(supportsTtsVoiceSettings && (voiceValue.trim() || modelValue.trim() || languageCodeValue.trim())) || hasBaseUrl;
		if (!(hasApiKey || hasSettings)) {
			setError("Provide an API key, base URL, or at least one provider setting.");
			return;
		}
		setError("");
		setSaving(true);

		const voiceOpts = {
			baseUrl: hasBaseUrl ? trimmedBaseUrl : undefined,
			voice: supportsTtsVoiceSettings ? voiceValue.trim() || undefined : undefined,
			model: supportsTtsVoiceSettings ? modelValue.trim() || undefined : undefined,
			languageCode: supportsTtsVoiceSettings ? languageCodeValue.trim() || undefined : undefined,
		};
		const req = hasApiKey
			? saveVoiceKey(selectedProvider as string, apiKey.trim(), voiceOpts)
			: saveVoiceSettings(selectedProvider as string, voiceOpts);
		req
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSaving(false);
				if (res?.ok) {
					setApiKey("");
					onSaved();
				} else {
					setError((res?.error as { message?: string })?.message || "Failed to save key");
				}
			})
			.catch((err: Error) => {
				setSaving(false);
				setError(err.message);
			});
	}

	function onSelectProvider(providerId: string): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	useEffect(() => {
		const settings = voiceSelectedProviderData.value?.settings;
		if (!settings) return;
		setBaseUrlValue(settings.baseUrl || "");
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
			.then((res: RpcResponse) => {
				if (res?.ok) {
					const payload = res.payload as {
						voices?: { id: string; name: string }[];
						models?: { id: string; name: string }[];
						warning?: string;
					};
					setElevenlabsCatalog({
						voices: payload?.voices || [],
						models: payload?.models || [],
						warning: payload?.warning || null,
					});
				}
			})
			.catch(() => {
				setElevenlabsCatalog({ voices: [], models: [], warning: "Failed to fetch ElevenLabs voice catalog." });
			})
			.finally(() => {
				setElevenlabsCatalogLoading(false);
				rerender();
			});
	}, [selectedProvider, isElevenLabsProvider]);

	// Group providers by type and category
	const sttCloud = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "cloud");
	const sttLocal = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "local");
	const ttsProviders = unconfiguredProviders.filter((p) => p.type === "tts");

	// If a provider is selected, show its configuration form
	if (selectedProvider && providerMeta) {
		// Cloud provider - show API key form
		if (providerMeta.category === "cloud") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="mb-3 text-xs text-[var(--muted)]">{providerMeta.description}</div>

						<label className="text-xs text-[var(--muted)]">API Key</label>
						<input
							type="password"
							className="provider-key-input w-full"
							value={apiKey}
							onInput={(e: Event) => setApiKey((e.target as HTMLInputElement).value)}
							placeholder={providerMeta.keyPlaceholder || "Leave blank to keep existing key"}
						/>
						{providerMeta.keyUrl ? (
							<div className="text-xs text-[var(--muted)]">
								Get your API key at{" "}
								<a
									href={providerMeta.keyUrl}
									target="_blank"
									rel="noopener"
									className="hover:underline text-[var(--accent)]"
								>
									{providerMeta.keyUrlLabel}
								</a>
							</div>
						) : null}

						{supportsBaseUrl ? (
							<div className="mt-2 flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Base URL</label>
								<input
									type="text"
									className="provider-key-input w-full"
									data-field="baseUrl"
									value={baseUrlValue}
									onInput={(e: Event) => setBaseUrlValue((e.target as HTMLInputElement).value)}
									placeholder="http://localhost:8000/v1"
								/>
								<div className="text-xs text-[var(--muted)]">
									Use this for a local or OpenAI-compatible server. Leave the API key blank if your endpoint does not
									require one.
								</div>
							</div>
						) : null}

						{supportsTtsVoiceSettings ? (
							<div className="flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Voice</label>
								{isElevenLabsProvider && elevenlabsCatalogLoading ? (
									<div className="text-xs text-[var(--muted)]">Loading ElevenLabs voices...</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.warning ? (
									<div className="text-xs text-[var(--muted)]">{elevenlabsCatalog.warning}</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.voices.length > 0 ? (
									<select
										className="provider-key-input w-full"
										onChange={(e: Event) => setVoiceValue((e.target as HTMLSelectElement).value)}
									>
										<option value="">Pick a voice from your account...</option>
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name} ({v.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={voiceValue}
									onInput={(e: Event) => setVoiceValue((e.target as HTMLInputElement).value)}
									list={isElevenLabsProvider ? "elevenlabs-voice-options" : undefined}
									placeholder="voice id / name (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-voice-options">
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name}
											</option>
										))}
									</datalist>
								) : null}

								<label className="text-xs text-[var(--muted)]">Model</label>
								{isElevenLabsProvider && elevenlabsCatalog.models.length > 0 ? (
									<select
										className="provider-key-input w-full"
										onChange={(e: Event) => setModelValue((e.target as HTMLSelectElement).value)}
									>
										<option value="">Pick a model...</option>
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name} ({m.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={modelValue}
									onInput={(e: Event) => setModelValue((e.target as HTMLInputElement).value)}
									list={isElevenLabsProvider ? "elevenlabs-model-options" : undefined}
									placeholder="model (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-model-options">
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name}
											</option>
										))}
									</datalist>
								) : null}

								{selectedProvider === "google" || selectedProvider === "google-tts" ? (
									<div className="flex flex-col gap-2">
										<label className="text-xs text-[var(--muted)]">Language Code</label>
										<input
											type="text"
											className="provider-key-input w-full"
											value={languageCodeValue}
											onInput={(e: Event) => setLanguageCodeValue((e.target as HTMLInputElement).value)}
											placeholder="en-US (optional)"
										/>
									</div>
								) : null}
							</div>
						) : null}

						{providerMeta.hint ? (
							<div
								className="text-xs text-[var(--muted)]"
								style={{
									marginTop: "8px",
									padding: "8px",
									background: "var(--surface-alt)",
									borderRadius: "4px",
									fontStyle: "italic",
								}}
							>
								{providerMeta.hint}
							</div>
						) : null}

						{error ? (
							<div className="text-xs" style={{ color: "var(--error)" }}>
								{error}
							</div>
						) : null}

						<div style={{ display: "flex", gap: "8px", marginTop: "8px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
									setApiKey("");
									setError("");
								}}
							>
								Back
							</button>
							<button className="provider-btn" disabled={saving} onClick={onSaveKey}>
								{saving ? "Saving\u2026" : "Save"}
							</button>
						</div>
					</div>
				</Modal>
			);
		}

		// Local provider - show setup instructions
		if (providerMeta.category === "local") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "12px" }}>
							{providerMeta.description}
						</div>
						<LocalProviderInstructions providerId={selectedProvider} voxtralReqs={voxtralReqs} />
						<div style={{ display: "flex", gap: "8px", marginTop: "12px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
								}}
							>
								Back
							</button>
						</div>
					</div>
				</Modal>
			);
		}
	}

	// Show provider selection list
	return (
		<Modal show={voiceShowAddModal.value} onClose={onClose} title="Add Voice Provider">
			<div className="channel-form" style={{ gap: "16px" }}>
				{sttCloud.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Cloud)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							{sttCloud.map((p) => (
								<button
									key={p.id}
									className="provider-card"
									style={{
										padding: "10px 12px",
										borderRadius: "6px",
										cursor: "pointer",
										textAlign: "left",
										border: "1px solid var(--border)",
										background: "var(--surface)",
									}}
									onClick={() => onSelectProvider(p.id)}
								>
									<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
										<div style={{ flex: 1 }}>
											<div className="text-sm text-[var(--text-strong)]">{p.name}</div>
											<div className="text-xs text-[var(--muted)]">{p.description}</div>
										</div>
										<span className="icon icon-chevron-right" style={{ color: "var(--muted)" }} />
									</div>
								</button>
							))}
						</div>
					</div>
				) : null}

				{sttLocal.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Local)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							{sttLocal.map((p) => (
								<button
									key={p.id}
									className="provider-card"
									style={{
										padding: "10px 12px",
										borderRadius: "6px",
										cursor: "pointer",
										textAlign: "left",
										border: "1px solid var(--border)",
										background: "var(--surface)",
									}}
									onClick={() => onSelectProvider(p.id)}
								>
									<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
										<div style={{ flex: 1 }}>
											<div className="text-sm text-[var(--text-strong)]">{p.name}</div>
											<div className="text-xs text-[var(--muted)]">{p.description}</div>
										</div>
										<span className="icon icon-chevron-right" style={{ color: "var(--muted)" }} />
									</div>
								</button>
							))}
						</div>
					</div>
				) : null}

				{ttsProviders.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Text-to-Speech
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							{ttsProviders.map((p) => (
								<button
									key={p.id}
									className="provider-card"
									style={{
										padding: "10px 12px",
										borderRadius: "6px",
										cursor: "pointer",
										textAlign: "left",
										border: "1px solid var(--border)",
										background: "var(--surface)",
									}}
									onClick={() => onSelectProvider(p.id)}
								>
									<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
										<div style={{ flex: 1 }}>
											<div className="text-sm text-[var(--text-strong)]">{p.name}</div>
											<div className="text-xs text-[var(--muted)]">{p.description}</div>
										</div>
										<span className="icon icon-chevron-right" style={{ color: "var(--muted)" }} />
									</div>
								</button>
							))}
						</div>
					</div>
				) : null}

				{unconfiguredProviders.length === 0 ? (
					<div className="text-sm text-[var(--muted)]" style={{ textAlign: "center", padding: "20px 0" }}>
						All available providers are already configured.
					</div>
				) : null}
			</div>
		</Modal>
	);
}

// ── Memory section ────────────────────────────────────────────

interface MemoryStatus {
	total_files?: number;
	total_chunks?: number;
	embedding_model?: string;
	db_size_display?: string;
}

interface MemoryConfig {
	style?: string;
	agent_write_mode?: string;
	user_profile_write_mode?: string;
	backend?: string;
	provider?: string;
	citations?: string;
	llm_reranking?: boolean;
	search_merge_strategy?: string;
	session_export?: string;
	prompt_memory_mode?: string;
	qmd_feature_enabled?: boolean;
}

interface QmdStatus {
	available?: boolean;
	version?: string;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Large component managing memory settings with QMD integration
function MemorySection(): VNode {
	const [memStatus, setMemStatus] = useState<MemoryStatus | null>(null);
	const [memConfig, setMemConfig] = useState<MemoryConfig | null>(null);
	const [qmdStatus, setQmdStatus] = useState<QmdStatus | null>(null);
	const [memLoading, setMemLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [saved, setSaved] = useState(false);
	const [error, setError] = useState<string | null>(null);

	// Form state
	const [style, setStyle] = useState("hybrid");
	const [agentWriteMode, setAgentWriteMode] = useState("hybrid");
	const [userProfileWriteMode, setUserProfileWriteMode] = useState("explicit-and-auto");
	const [backend, setBackend] = useState("builtin");
	const [provider, setProvider] = useState("auto");
	const [citations, setCitations] = useState("auto");
	const [llmReranking, setLlmReranking] = useState(false);
	const [searchMergeStrategy, setSearchMergeStrategy] = useState("rrf");
	const [sessionExport, setSessionExport] = useState("on-new-or-reset");
	const [promptMemoryMode, setPromptMemoryMode] = useState("live-reload");

	useEffect(() => {
		// Fetch memory status, config, and QMD status
		Promise.all([sendRpc("memory.status", {}), sendRpc("memory.config.get", {}), sendRpc("memory.qmd.status", {})])
			.then(([statusRes, configRes, qmdRes]: [RpcResponse, RpcResponse, RpcResponse]) => {
				if (statusRes?.ok) {
					setMemStatus(statusRes.payload as MemoryStatus);
				}
				if (configRes?.ok) {
					const cfg = configRes.payload as MemoryConfig;
					setMemConfig(cfg);
					setStyle(cfg.style || "hybrid");
					setAgentWriteMode(cfg.agent_write_mode || "hybrid");
					setUserProfileWriteMode(cfg.user_profile_write_mode || "explicit-and-auto");
					setBackend(cfg.backend || "builtin");
					setProvider(cfg.provider || "auto");
					setCitations(cfg.citations || "auto");
					setLlmReranking(cfg.llm_reranking ?? false);
					setSearchMergeStrategy(cfg.search_merge_strategy || "rrf");
					setSessionExport(cfg.session_export || "on-new-or-reset");
					setPromptMemoryMode(cfg.prompt_memory_mode || "live-reload");
				}
				if (qmdRes?.ok) {
					setQmdStatus(qmdRes.payload as QmdStatus);
				}
				setMemLoading(false);
				rerender();
			})
			.catch(() => {
				setMemLoading(false);
				rerender();
			});
	}, []);

	function onSave(e: Event): void {
		e.preventDefault();
		setError(null);
		setSaving(true);
		setSaved(false);

		sendRpc("memory.config.update", {
			style,
			agent_write_mode: agentWriteMode,
			user_profile_write_mode: userProfileWriteMode,
			backend,
			provider,
			citations,
			llm_reranking: llmReranking,
			search_merge_strategy: searchMergeStrategy,
			session_export: sessionExport,
			prompt_memory_mode: promptMemoryMode,
		}).then((res: RpcResponse) => {
			setSaving(false);
			if (res?.ok) {
				setMemConfig(res.payload as MemoryConfig);
				setSaved(true);
				setTimeout(() => {
					setSaved(false);
					rerender();
				}, 2000);
			} else {
				setError((res?.error as { message?: string })?.message || "Failed to save");
			}
			rerender();
		});
	}

	if (memLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Memory</h2>
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	const qmdFeatureEnabled = memConfig?.qmd_feature_enabled !== false;
	const qmdAvailable = qmdStatus?.available === true;

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Memory</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed max-w-form" style={{ margin: 0 }}>
				Configure how the agent stores and retrieves long-term memory. Memory enables the agent to recall past
				conversations, notes, and context across sessions.
			</p>

			{/* Status */}
			{memStatus ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--bg)",
					}}
				>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Status
					</h3>
					<div style={{ display: "grid", gridTemplateColumns: "repeat(2,1fr)", gap: "8px 16px", fontSize: ".8rem" }}>
						<div>
							<span className="text-[var(--muted)]">Files:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.total_files || 0}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">Chunks:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.total_chunks || 0}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">Model:</span>
							<span
								className="text-[var(--text)]"
								style={{ marginLeft: "6px", fontFamily: "var(--font-mono)", fontSize: ".75rem" }}
							>
								{memStatus.embedding_model || "none"}
							</span>
						</div>
						<div>
							<span className="text-[var(--muted)]">DB Size:</span>
							<span className="text-[var(--text)]" style={{ marginLeft: "6px" }}>
								{memStatus.db_size_display || "0 B"}
							</span>
						</div>
					</div>
				</div>
			) : null}

			{/* Configuration */}
			<form onSubmit={onSave} style={{ maxWidth: "600px", display: "flex", flexDirection: "column", gap: "16px" }}>
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Memory Style
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose the high-level orchestration model. This controls whether prompt-visible <code>MEMORY.md</code> and
						memory tools are both active, one is active, or both are off.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "240px" }}
						value={style}
						onChange={(e: Event) => {
							setStyle((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="hybrid">Hybrid</option>
						<option value="prompt-only">Prompt-only</option>
						<option value="search-only">Search-only</option>
						<option value="off">Off</option>
					</select>
				</div>

				{/* Backend selection */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Backend
					</h3>

					{/* Comparison table */}
					<div
						style={{
							marginBottom: "12px",
							padding: "12px",
							borderRadius: "6px",
							border: "1px solid var(--border)",
							background: "var(--bg)",
							fontSize: ".75rem",
						}}
					>
						<table style={{ width: "100%", borderCollapse: "collapse" }}>
							<thead>
								<tr style={{ borderBottom: "1px solid var(--border)" }}>
									<th style={{ textAlign: "left", padding: "4px 8px 8px 0", color: "var(--muted)", fontWeight: 500 }}>
										Feature
									</th>
									<th style={{ textAlign: "center", padding: "4px 8px 8px", color: "var(--muted)", fontWeight: 500 }}>
										Built-in
									</th>
									<th style={{ textAlign: "center", padding: "4px 8px 8px", color: "var(--muted)", fontWeight: 500 }}>
										QMD
									</th>
								</tr>
							</thead>
							<tbody>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Search type</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>FTS5 + vector</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>
										BM25 + vector + LLM
									</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>External dependency</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>None</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Node.js/Bun</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Embedding cache</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>{"\u2713"}</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>OpenAI batch API</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>
										{"\u2713"} (50% cheaper)
									</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Provider fallback</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>{"\u2713"}</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>{"\u2717"}</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>LLM reranking</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Optional</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--accent)" }}>Built-in</td>
								</tr>
								<tr>
									<td style={{ padding: "6px 8px 6px 0", color: "var(--text)" }}>Best for</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Most users</td>
									<td style={{ padding: "6px 8px", textAlign: "center", color: "var(--muted)" }}>Power users</td>
								</tr>
							</tbody>
						</table>
					</div>

					<div style={{ display: "flex", gap: "8px" }}>
						<button
							type="button"
							className={`provider-btn ${backend === "builtin" ? "" : "provider-btn-secondary"}`}
							onClick={() => {
								setBackend("builtin");
								rerender();
							}}
						>
							Built-in (Recommended)
						</button>
						<button
							type="button"
							className={`provider-btn ${backend === "qmd" ? "" : "provider-btn-secondary"}`}
							disabled={!qmdFeatureEnabled}
							onClick={() => {
								setBackend("qmd");
								rerender();
							}}
						>
							QMD
						</button>
					</div>

					{qmdFeatureEnabled ? null : (
						<div className="text-xs text-[var(--error)]" style={{ marginTop: "8px" }}>
							QMD feature is not enabled. Rebuild moltis with{" "}
							<code style={{ fontFamily: "var(--font-mono)", fontSize: ".7rem" }}>--features qmd</code>
						</div>
					)}

					{backend === "qmd" ? (
						<div
							style={{
								marginTop: "12px",
								padding: "12px",
								borderRadius: "6px",
								border: "1px solid var(--border)",
								background: "var(--bg)",
							}}
						>
							<h4 className="text-xs font-medium text-[var(--text-strong)]" style={{ margin: "0 0 8px" }}>
								QMD Status
							</h4>
							{qmdAvailable ? (
								<div
									className="text-xs"
									style={{ color: "var(--accent)", display: "flex", alignItems: "center", gap: "6px" }}
								>
									<span>{"\u2713"}</span> QMD is installed{" "}
									{qmdStatus?.version ? <span className="text-[var(--muted)]">({qmdStatus.version})</span> : null}
								</div>
							) : (
								<div>
									<div className="text-xs" style={{ color: "var(--error)", marginBottom: "8px" }}>
										{"\u2717"} QMD is not installed or not found in PATH
									</div>
									<div className="text-xs text-[var(--muted)]" style={{ lineHeight: 1.6 }}>
										<strong style={{ color: "var(--text)" }}>Installation:</strong>
										<br />
										<code
											style={{
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											npm install -g @tobilu/qmd
										</code>
										<span style={{ margin: "0 4px" }}>or</span>
										<code
											style={{
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											bun install -g @tobilu/qmd
										</code>
										<br />
										<br />
										Verify the CLI is available:
										<code
											style={{
												display: "block",
												marginTop: "4px",
												fontFamily: "var(--font-mono)",
												fontSize: ".7rem",
												background: "var(--surface)",
												padding: "2px 4px",
												borderRadius: "3px",
											}}
										>
											qmd --version
										</code>
										<br />
										<a
											href="https://github.com/tobi/qmd"
											target="_blank"
											rel="noopener"
											style={{ color: "var(--accent)" }}
										>
											View documentation {"\u2192"}
										</a>
									</div>
								</div>
							)}
						</div>
					) : null}
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Prompt Memory Mode
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						When prompt memory is enabled, choose whether <code>MEMORY.md</code> is reread on every turn or frozen when
						the session starts.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "260px" }}
						value={promptMemoryMode}
						disabled={style === "search-only" || style === "off"}
						onChange={(e: Event) => {
							setPromptMemoryMode((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="live-reload">Live reload</option>
						<option value="frozen-at-session-start">Frozen at session start</option>
					</select>
					{style === "search-only" || style === "off" ? (
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
							Prompt memory is disabled by the current memory style, so this setting will only matter after you
							re-enable prompt memory.
						</div>
					) : null}
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Agent Memory Writes
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Control where agent-authored memory writes can land. This affects <code>memory_save</code> and silent
						compaction memory flushes.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={agentWriteMode}
						onChange={(e: Event) => {
							setAgentWriteMode((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="hybrid">Hybrid (MEMORY.md and memory/*.md)</option>
						<option value="prompt-only">Prompt-only (MEMORY.md only)</option>
						<option value="search-only">Search-only (memory/*.md only)</option>
						<option value="off">Off</option>
					</select>
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						USER.md Writes
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Control whether Moltis mirrors your profile into <code>USER.md</code>, and whether browser or channel
						timezone/location signals can update it silently.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "250px" }}
						value={userProfileWriteMode}
						onChange={(e: Event) => {
							setUserProfileWriteMode((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="explicit-and-auto">Explicit and auto</option>
						<option value="explicit-only">Explicit only</option>
						<option value="off">Off (moltis.toml only)</option>
					</select>
				</div>

				{/* Citations */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Embedding Provider
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Select which embedding provider the built-in memory backend should use for RAG. QMD manages retrieval
						separately, so this setting is ignored while the QMD backend is active.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={provider}
						disabled={backend === "qmd"}
						onChange={(e: Event) => {
							setProvider((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="auto">Auto-detect</option>
						<option value="local">Local GGUF</option>
						<option value="ollama">Ollama</option>
						<option value="openai">OpenAI</option>
						<option value="custom">Custom OpenAI-compatible</option>
					</select>
					{backend === "qmd" ? (
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "8px" }}>
							This setting is kept for when you switch back to the built-in backend.
						</div>
					) : null}
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Citations
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Include source file and line number with search results to help track where information comes from.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "150px" }}
						value={citations}
						onChange={(e: Event) => {
							setCitations((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="auto">Auto (multi-file only)</option>
						<option value="on">Always</option>
						<option value="off">Never</option>
					</select>
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Search Merge Strategy
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Choose how Moltis blends vector and keyword memory hits before optional reranking.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "180px" }}
						value={searchMergeStrategy}
						onChange={(e: Event) => {
							setSearchMergeStrategy((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="rrf">RRF</option>
						<option value="linear">Linear</option>
					</select>
				</div>

				{/* LLM Reranking */}
				<div>
					<label style={{ display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
						<input
							type="checkbox"
							checked={llmReranking}
							onChange={(e: Event) => {
								setLlmReranking((e.target as HTMLInputElement).checked);
								rerender();
							}}
						/>
						<div>
							<span className="text-sm font-medium text-[var(--text-strong)]">LLM Reranking</span>
							<p className="text-xs text-[var(--muted)]" style={{ margin: "2px 0 0" }}>
								Use the LLM to rerank search results for better relevance (slower but more accurate).
							</p>
						</div>
					</label>
				</div>

				{/* Session Export */}
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
						Session Export
					</h3>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "0 0 8px" }}>
						Export session transcripts into searchable memory when a session is rolled over.
					</p>
					<select
						className="provider-key-input"
						style={{ width: "auto", minWidth: "220px" }}
						value={sessionExport}
						onChange={(e: Event) => {
							setSessionExport((e.target as HTMLSelectElement).value);
							rerender();
						}}
					>
						<option value="on-new-or-reset">On /new and /reset</option>
						<option value="off">Off</option>
					</select>
				</div>

				<div
					style={{
						display: "flex",
						alignItems: "center",
						gap: "8px",
						paddingTop: "8px",
						borderTop: "1px solid var(--border)",
					}}
				>
					<button type="submit" className="provider-btn" disabled={saving}>
						{saving ? "Saving\u2026" : "Save"}
					</button>
					{saved ? (
						<span className="text-xs" style={{ color: "var(--accent)" }}>
							Saved
						</span>
					) : null}
					{error ? (
						<span className="text-xs" style={{ color: "var(--error)" }}>
							{error}
						</span>
					) : null}
				</div>
			</form>
		</div>
	);
}

// ── Notifications section ─────────────────────────────────────

interface PushSubscription {
	endpoint: string;
	device?: string;
	ip?: string;
	created_at?: string;
}

interface PushServerStatus {
	subscription_count?: number;
	subscriptions?: PushSubscription[];
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Notifications section handles multiple states and conditions
function NotificationsSection(): VNode {
	const [supported, setSupported] = useState(false);
	const [permission, setPermission] = useState("default");
	const [subscribed, setSubscribed] = useState(false);
	const [isLoading, setIsLoading] = useState(true);
	const [toggling, setToggling] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [serverStatus, setServerStatus] = useState<PushServerStatus | null>(null);

	async function checkStatus(): Promise<void> {
		setIsLoading(true);
		rerender();

		const pushSupported = push.isPushSupported();
		setSupported(pushSupported);

		if (pushSupported) {
			setPermission(push.getPermissionState());
			await push.initPushState();
			setSubscribed(push.isSubscribed());

			// Check server status
			const status = await push.getPushStatus();
			setServerStatus(status as PushServerStatus);
		}

		setIsLoading(false);
		rerender();
	}

	async function refreshStatus(): Promise<void> {
		const status = await push.getPushStatus();
		setServerStatus(status as PushServerStatus);
		rerender();
	}

	async function onRemoveSubscription(endpoint: string): Promise<void> {
		const result = await push.removeSubscription(endpoint);
		if (!result.success) {
			setError(result.error || "Failed to remove subscription");
			rerender();
		}
		// The WebSocket event will trigger refreshStatus automatically
	}

	useEffect(() => {
		checkStatus();
		// Listen for subscription changes via WebSocket
		const off = onEvent("push.subscriptions", () => {
			refreshStatus();
		});
		return off;
	}, []);

	async function onToggle(): Promise<void> {
		setError(null);
		setToggling(true);
		rerender();

		const result = subscribed ? await push.unsubscribeFromPush() : await push.subscribeToPush();

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
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	if (!supported) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0 }}>
						Push notifications are not supported in this browser.
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Try using Safari, Chrome, or Firefox on a device that supports web push.
					</p>
				</div>
			</div>
		);
	}

	if (serverStatus === null) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0 }}>
						Push notifications are not configured on the server.
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						The server was built without the{" "}
						<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>push-notifications</code> feature.
					</p>
				</div>
			</div>
		);
	}

	// Check if running as installed PWA - push notifications require installation on Safari
	const standalone = isStandalone();
	const needsInstall = !standalone && /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Notifications</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Receive push notifications when the agent completes a task or needs your attention.
			</p>

			{/* Push notifications toggle */}
			<div style={{ maxWidth: "600px" }}>
				<div className="provider-item" style={{ marginBottom: 0 }}>
					<div style={{ flex: 1, minWidth: 0 }}>
						<div className="provider-item-name" style={{ fontSize: ".9rem" }}>
							Push Notifications
						</div>
						<div style={{ fontSize: ".75rem", color: "var(--muted)", marginTop: "2px" }}>
							{needsInstall
								? "Add this app to your Dock to enable notifications."
								: subscribed
									? "You will receive notifications on this device."
									: permission === "denied"
										? "Notifications are blocked. Enable them in browser settings."
										: "Enable to receive notifications on this device."}
						</div>
					</div>
					<button
						className={`provider-btn ${subscribed ? "provider-btn-danger" : ""}`}
						onClick={onToggle}
						disabled={toggling || permission === "denied" || needsInstall}
					>
						{toggling ? "\u2026" : subscribed ? "Disable" : "Enable"}
					</button>
				</div>
				{error ? (
					<div className="text-xs" style={{ marginTop: "8px", color: "var(--error)" }}>
						{error}
					</div>
				) : null}
			</div>

			{/* Install required notice */}
			{needsInstall ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0, fontWeight: 500 }}>
						Installation required
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						On Safari, push notifications are only available for installed apps. Add moltis to your Dock using{" "}
						<strong>File {"\u2192"} Add to Dock</strong> (or Share {"\u2192"} Add to Dock on iOS), then open it from
						there.
					</p>
				</div>
			) : null}

			{/* Permission status */}
			{permission === "denied" && !needsInstall ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<p className="text-sm" style={{ color: "var(--error)", margin: 0, fontWeight: 500 }}>
						Notifications are blocked
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						You previously blocked notifications for this site. To enable them, you'll need to update your browser's
						site settings and allow notifications for this origin.
					</p>
				</div>
			) : null}

			{/* Subscribed devices */}
			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px", marginTop: "8px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]" style={{ marginBottom: "8px" }}>
					Subscribed Devices ({serverStatus?.subscription_count || 0})
				</h3>
				{(serverStatus?.subscriptions?.length || 0) > 0 ? (
					<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
						{serverStatus?.subscriptions?.map((sub) => (
							<div className="provider-item" style={{ marginBottom: 0 }} key={sub.endpoint}>
								<div style={{ flex: 1, minWidth: 0 }}>
									<div className="provider-item-name" style={{ fontSize: ".85rem" }}>
										{sub.device}
									</div>
									<div
										style={{
											fontSize: ".7rem",
											color: "var(--muted)",
											marginTop: "2px",
											display: "flex",
											gap: "12px",
											flexWrap: "wrap",
										}}
									>
										{sub.ip ? <span style={{ fontFamily: "var(--font-mono)" }}>{sub.ip}</span> : null}
										<time dateTime={sub.created_at}>{new Date(sub.created_at || "").toLocaleDateString()}</time>
									</div>
								</div>
								<button className="provider-btn provider-btn-danger" onClick={() => onRemoveSubscription(sub.endpoint)}>
									Remove
								</button>
							</div>
						))}
					</div>
				) : (
					<div className="text-xs text-[var(--muted)]" style={{ padding: "4px 0" }}>
						No devices subscribed yet.
					</div>
				)}
			</div>
		</div>
	);
}

// ── Page-section init/teardown map ──────────────────────────

interface PageSectionHandler {
	init: (container: HTMLElement, subPath?: string | null) => void;
	teardown: () => void;
}

const pageSectionHandlers: Record<string, PageSectionHandler> = {
	crons: {
		init: (container: HTMLElement) => initCrons(container, null),
		teardown: teardownCrons,
	},
	heartbeat: {
		init: (container: HTMLElement) => initCrons(container, "heartbeat"),
		teardown: teardownCrons,
	},
	webhooks: { init: initWebhooks, teardown: teardownWebhooks },
	providers: { init: initProviders, teardown: teardownProviders },
	channels: { init: initChannels, teardown: teardownChannels },
	mcp: { init: initMcp, teardown: teardownMcp },
	nodes: { init: initNodes, teardown: teardownNodes },
	projects: { init: initProjects, teardown: teardownProjects },
	hooks: { init: initHooks, teardown: teardownHooks },
	skills: { init: initSkills, teardown: teardownSkills },
	agents: { init: initAgents, teardown: teardownAgents },
	terminal: { init: initTerminal, teardown: teardownTerminal },
	sandboxes: { init: initImages, teardown: teardownImages },
	monitoring: {
		init: (container: HTMLElement) => initMonitoring(container, null, { syncPath: false }),
		teardown: teardownMonitoring,
	},
	logs: { init: initLogs, teardown: teardownLogs },
	"network-audit": { init: initNetworkAudit, teardown: teardownNetworkAudit },
};

/** Wrapper that mounts a page init/teardown pair into a ref div. */

interface PageSectionProps {
	initFn: (container: HTMLElement, subPath?: string) => void;
	teardownFn: (() => void) | null;
	subPath?: string;
}

function PageSection({ initFn, teardownFn, subPath }: PageSectionProps): VNode {
	const ref = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (ref.current) initFn(ref.current, subPath);
		return () => {
			if (teardownFn) teardownFn();
		};
	}, [initFn, teardownFn, subPath]);
	return <div ref={ref} className="flex-1 flex flex-col min-w-0 overflow-hidden" />;
}

// ── Main layout ──────────────────────────────────────────────

function SettingsPage(): VNode {
	useEffect(() => {
		fetchIdentity();
	}, []);

	const section = activeSection.value;
	const subPath = activeSubPath.value;
	const ps = pageSectionHandlers[section];
	const mobile = isMobileViewport();
	const showSidebar = !mobile || mobileSidebarVisible.value;
	const showContent = !(mobile && showSidebar);
	const mobileSectionsLabel = showSidebar ? "Hide Sections" : "Sections";

	return (
		<div className={`settings-layout ${mobile && !showSidebar ? "settings-layout-mobile-collapsed" : ""}`}>
			{showSidebar ? <SettingsSidebar /> : null}
			{showContent ? (
				<div className="settings-content-wrap">
					{mobile ? (
						<div className="settings-mobile-controls">
							<button className="settings-mobile-chat-btn" type="button" onClick={() => navigate(routes.chats!)}>
								<span className="icon icon-chat" />
								<span>Back to Chats</span>
							</button>
							<button
								className="settings-mobile-menu-btn"
								type="button"
								onClick={() => {
									mobileSidebarVisible.value = !mobileSidebarVisible.value;
									rerender();
								}}
							>
								<span className="icon icon-burger" />
								<span>{mobileSectionsLabel}</span>
							</button>
						</div>
					) : null}
					{ps ? (
						section === "terminal" && gon.get("terminal_enabled") !== true ? (
							<div className="flex-1 flex flex-col min-w-0 p-4 gap-3 overflow-y-auto">
								<h2 className="text-base font-medium text-[var(--text-strong)]">Terminal</h2>
								<div className="text-xs text-[var(--muted)] max-w-form">
									The host terminal has been disabled by the server administrator. To re-enable it, set{" "}
									<code>terminal_enabled = true</code> under <code>[server]</code> in the configuration file, or remove
									the <code>MOLTIS_TERMINAL_DISABLED</code> environment variable if it is set.
								</div>
							</div>
						) : (
							<PageSection key={`${section}:${subPath}`} initFn={ps.init} teardownFn={ps.teardown} subPath={subPath} />
						)
					) : null}
					{section === "identity" ? <IdentitySection /> : null}
					{section === "memory" ? <MemorySection /> : null}
					{section === "environment" ? <EnvironmentSection /> : null}
					{section === "tools" ? <ToolsSection /> : null}
					{section === "security" ? <SecuritySection /> : null}
					{section === "vault" ? <VaultSection /> : null}
					{section === "ssh" ? <SshSection /> : null}
					{section === "remote-access" ? <RemoteAccessSection /> : null}
					{section === "voice" ? (
						gon.get("voice_enabled") === true ? (
							<VoiceSection />
						) : (
							<div className="flex-1 flex flex-col min-w-0 p-4 gap-3 overflow-y-auto">
								<h2 className="text-base font-medium text-[var(--text-strong)]">Voice</h2>
								<div className="text-xs text-[var(--muted)] max-w-form">
									Voice settings are unavailable in this build. Start a binary with the voice feature enabled to
									configure STT/TTS providers.
								</div>
							</div>
						)
					) : null}
					{section === "notifications" ? <NotificationsSection /> : null}
					{section === "import" ? <OpenClawImportSection /> : null}
					{section === "graphql" ? <GraphqlSection /> : null}
					{section === "config" ? <ConfigSection /> : null}
				</div>
			) : null}
		</div>
	);
}

const DEFAULT_SECTION = "identity";

registerPrefix(
	routes.settings!,
	(container: HTMLElement, param?: string | null) => {
		mounted = true;
		containerRef = container;
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		const parts = (param || "").replace(/:/g, "/").split("/").filter(Boolean);
		const requestedSection = parts[0] || "";
		const requestedSectionAlias = requestedSection === "tailscale" ? "remote-access" : requestedSection;
		const subPath = parts.slice(1).join("/");
		const isValidSection = requestedSectionAlias && getSectionItems().some((s) => s.id === requestedSectionAlias);
		const section = isValidSection ? requestedSectionAlias : DEFAULT_SECTION;
		activeSection.value = section;
		activeSubPath.value = isValidSection ? subPath : "";
		mobileSidebarVisible.value = !isMobileViewport();
		if (!isValidSection || requestedSectionAlias !== requestedSection) {
			history.replaceState(null, "", settingsPath(section));
		}
		render(<SettingsPage />, container);
		fetchIdentity();
	},
	() => {
		mounted = false;
		if (containerRef) render(null, containerRef);
		containerRef = null;
		identity.value = null;
		loading.value = true;
		activeSection.value = DEFAULT_SECTION;
		activeSubPath.value = "";
		mobileSidebarVisible.value = true;
	},
);
