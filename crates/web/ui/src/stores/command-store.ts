// ── Command palette store ────────────────────────────────────
//
// Signal-driven state for the command palette. Commands are built
// dynamically from gon-injected routes so they stay in sync with
// what the server exposes.

import { signal } from "@preact/signals";
import * as gon from "../gon";
import { navigate } from "../router";
import { routes, settingsPath } from "../routes";
import { applyTheme } from "../theme";

// ── Types ────────────────────────────────────────────────────

export interface Command {
	id: string;
	label: string;
	group: "navigation" | "settings" | "actions";
	icon?: string;
	shortcut?: string;
	keywords?: string[];
	action: () => void;
}

// ── Signals ──────────────────────────────────────────────────

export const paletteOpen = signal(false);

export function togglePalette(): void {
	paletteOpen.value = !paletteOpen.value;
}

export function openPalette(): void {
	paletteOpen.value = true;
}

export function closePalette(): void {
	paletteOpen.value = false;
}

// ── Command registry ─────────────────────────────────────────

function nav(path: string | undefined): () => void {
	return () => {
		if (path) navigate(path);
	};
}

function settingsNav(id: string): () => void {
	return () => navigate(settingsPath(id));
}

export function buildCommands(): Command[] {
	const cmds: Command[] = [];

	// ── Navigation ───────────────────────────────────────────
	cmds.push(
		{
			id: "nav-chats",
			label: "Chats",
			group: "navigation",
			icon: "icon-chat",
			keywords: ["sessions", "conversation"],
			action: nav(routes.chats),
		},
		{
			id: "nav-providers",
			label: "Providers",
			group: "navigation",
			icon: "icon-layers",
			keywords: ["llm", "model", "api"],
			action: nav(routes.providers),
		},
		{
			id: "nav-projects",
			label: "Projects",
			group: "navigation",
			icon: "icon-folder",
			keywords: ["workspace"],
			action: nav(routes.projects),
		},
		{
			id: "nav-skills",
			label: "Skills",
			group: "navigation",
			icon: "icon-sparkles",
			keywords: ["ability", "tool"],
			action: nav(routes.skills),
		},
		{
			id: "nav-crons",
			label: "Crons",
			group: "navigation",
			icon: "icon-cron",
			keywords: ["schedule", "job", "timer"],
			action: nav(routes.crons),
		},
		{
			id: "nav-logs",
			label: "Logs",
			group: "navigation",
			icon: "icon-document",
			keywords: ["log", "output"],
			action: nav(routes.logs),
		},
		{
			id: "nav-metrics",
			label: "Metrics",
			group: "navigation",
			icon: "icon-activity",
			keywords: ["monitoring", "dashboard", "stats"],
			action: nav(routes.monitoring),
		},
	);

	// ── Settings sections ────────────────────────────────────
	cmds.push(
		{
			id: "set-profile",
			label: "User Profile",
			group: "settings",
			icon: "icon-person",
			keywords: ["identity", "name", "avatar"],
			action: settingsNav("profile"),
		},
		{
			id: "set-agents",
			label: "Agents",
			group: "settings",
			icon: "icon-users",
			keywords: ["agent", "preset"],
			action: settingsNav("agents"),
		},
		{
			id: "set-nodes",
			label: "Nodes",
			group: "settings",
			icon: "icon-nodes",
			keywords: ["node", "cluster"],
			action: settingsNav("nodes"),
		},
		{
			id: "set-environment",
			label: "Environment",
			group: "settings",
			icon: "icon-terminal",
			keywords: ["env", "variable"],
			action: settingsNav("environment"),
		},
		{
			id: "set-memory",
			label: "Memory",
			group: "settings",
			icon: "icon-database",
			keywords: ["knowledge", "context"],
			action: settingsNav("memory"),
		},
		{
			id: "set-notifications",
			label: "Notifications",
			group: "settings",
			icon: "icon-bell",
			keywords: ["alert", "notify"],
			action: settingsNav("notifications"),
		},
		{
			id: "set-webhooks",
			label: "Webhooks",
			group: "settings",
			icon: "icon-webhooks",
			keywords: ["webhook", "hook", "callback"],
			action: settingsNav("webhooks"),
		},
		{
			id: "set-heartbeat",
			label: "Heartbeat",
			group: "settings",
			icon: "icon-heart",
			keywords: ["pulse", "health"],
			action: settingsNav("heartbeat"),
		},
		{
			id: "set-security",
			label: "Authentication",
			group: "settings",
			icon: "icon-key",
			keywords: ["password", "passkey", "auth", "security"],
			action: settingsNav("security"),
		},
		{
			id: "set-vault",
			label: "Encryption",
			group: "settings",
			icon: "icon-lock",
			keywords: ["vault", "encrypt", "secret"],
			action: settingsNav("vault"),
		},
		{
			id: "set-ssh",
			label: "SSH",
			group: "settings",
			icon: "icon-ssh",
			keywords: ["key", "remote"],
			action: settingsNav("ssh"),
		},
		{
			id: "set-remote",
			label: "Remote Access",
			group: "settings",
			icon: "icon-share",
			keywords: ["tailscale", "tunnel"],
			action: settingsNav("remote-access"),
		},
		{
			id: "set-sandboxes",
			label: "Sandboxes",
			group: "settings",
			icon: "icon-cube",
			keywords: ["container", "docker"],
			action: settingsNav("sandboxes"),
		},
		{
			id: "set-channels",
			label: "Channels",
			group: "settings",
			icon: "icon-channels",
			keywords: ["telegram", "whatsapp", "slack", "discord"],
			action: settingsNav("channels"),
		},
		{
			id: "set-hooks",
			label: "Hooks",
			group: "settings",
			icon: "icon-wrench",
			keywords: ["hook", "automation"],
			action: settingsNav("hooks"),
		},
		{
			id: "set-mcp",
			label: "MCP Servers",
			group: "settings",
			icon: "icon-link",
			keywords: ["mcp", "server", "protocol"],
			action: settingsNav("mcp"),
		},
		{
			id: "set-tools",
			label: "Tools",
			group: "settings",
			icon: "icon-settings-gear",
			keywords: ["tool", "function"],
			action: settingsNav("tools"),
		},
		{
			id: "set-voice",
			label: "Voice",
			group: "settings",
			icon: "icon-microphone",
			keywords: ["speech", "audio", "transcribe"],
			action: settingsNav("voice"),
		},
		{
			id: "set-config",
			label: "Configuration",
			group: "settings",
			icon: "icon-document",
			keywords: ["toml", "config", "file"],
			action: settingsNav("config"),
		},
	);

	// Conditional settings
	if (gon.get("terminal_enabled")) {
		cmds.push({
			id: "set-terminal",
			label: "Terminal",
			group: "settings",
			icon: "icon-terminal",
			keywords: ["shell", "bash", "console"],
			action: settingsNav("terminal"),
		});
	}
	if (gon.get("graphql_enabled")) {
		cmds.push({
			id: "set-graphql",
			label: "GraphQL",
			group: "settings",
			icon: "icon-graphql",
			keywords: ["query", "playground"],
			action: settingsNav("graphql"),
		});
	}

	// ── Actions ──────────────────────────────────────────────
	cmds.push(
		{
			id: "act-new-session",
			label: "New Session",
			group: "actions",
			icon: "icon-chat",
			keywords: ["create", "start", "new"],
			action: () => {
				const btn = document.getElementById("newSessionBtn");
				if (btn) btn.click();
			},
		},
		{
			id: "act-theme",
			label: "Toggle Theme",
			group: "actions",
			icon: "icon-sun",
			keywords: ["dark", "light", "theme", "mode"],
			action: () => {
				const current = document.documentElement.getAttribute("data-theme");
				const next = current === "dark" ? "light" : "dark";
				localStorage.setItem("moltis-theme", next);
				applyTheme(next);
			},
		},
		{
			id: "act-logout",
			label: "Sign Out",
			group: "actions",
			icon: "icon-logout",
			keywords: ["logout", "signout", "disconnect"],
			action: () => {
				const btn = document.getElementById("logoutBtn");
				if (btn) btn.click();
			},
		},
	);

	return cmds;
}
