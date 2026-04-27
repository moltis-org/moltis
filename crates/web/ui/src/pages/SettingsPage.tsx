// ── Settings page (Preact + Signals) ───────────────────

import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import * as gon from "../gon";
import { navigate, registerPrefix } from "../router";
import { routes, settingsPath } from "../routes";
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

// ── Section components (extracted into ./sections/) ──────

import type { SectionItem } from "./sections/_shared";
import {
	activeSection,
	activeSubPath,
	fetchIdentity,
	getContainerRef,
	identity,
	isMobileViewport,
	loading,
	mobileSidebarVisible,
	setContainerRef,
	setMounted,
	setRerenderFn,
} from "./sections/_shared";
import { ConfigSection, GraphqlSection } from "./sections/ConfigSection";
import { EnvironmentSection } from "./sections/EnvironmentSection";
import { IdentitySection } from "./sections/IdentitySection";
import { MemorySection } from "./sections/MemorySection";
import { NotificationsSection } from "./sections/NotificationsSection";
import { OpenClawImportSection } from "./sections/OpenClawImportSection";
import { RemoteAccessSection } from "./sections/RemoteAccessSection";
import { SecuritySection } from "./sections/SecuritySection";
import { SshSection } from "./sections/SshSection";
import { ToolsSection } from "./sections/ToolsSection";
import { VaultSection } from "./sections/VaultSection";
import { VoiceSection } from "./sections/VoiceSection";

// ── Sidebar navigation items ─────────────────────────────────

const sections: SectionItem[] = [
	{ group: "General" },
	{
		id: "profile",
		label: "User Profile",
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

// ── Rerender wiring ──────────────────────────────────────────

function rerender(): void {
	const ref = getContainerRef();
	if (ref) render(<SettingsPage />, ref);
}

// ── Sidebar ──────────────────────────────────────────────────

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
					{section === "profile" ? <IdentitySection /> : null}
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

const DEFAULT_SECTION = "profile";

registerPrefix(
	routes.settings!,
	(container: HTMLElement, param?: string | null) => {
		setMounted(true);
		setContainerRef(container);
		setRerenderFn(rerender);
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		const parts = (param || "").replace(/:/g, "/").split("/").filter(Boolean);
		const requestedSection = parts[0] || "";
		const requestedSectionAlias =
			requestedSection === "tailscale"
				? "remote-access"
				: requestedSection === "identity"
					? "profile"
					: requestedSection;
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
		setMounted(false);
		const ref = getContainerRef();
		if (ref) render(null, ref);
		setContainerRef(null);
		identity.value = null;
		loading.value = true;
		activeSection.value = DEFAULT_SECTION;
		activeSubPath.value = "";
		mobileSidebarVisible.value = true;
	},
);
