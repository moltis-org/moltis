// ── WebSocket ─────────────────────────────────────────────────

import { chatAddMsg } from "./chat-ui";
import { eventListeners } from "./events";
import { sendRpc } from "./helpers";
import { clearLogsAlert, updateLogsAlert } from "./logs-alert";
import { fetchModels } from "./models";
import { prefetchChannels } from "./pages/ChannelsPage";
import { fetchProjects } from "./projects";
import { currentPage, currentPrefix, mount } from "./router";
import { fetchSessions } from "./sessions";
import * as S from "./state";
import { sessionStore } from "./stores/session-store";
import type { ConnectOptions } from "./ws-connect";
import { connectWs, forceReconnect, subscribeEvents } from "./ws-connect";

// ── Handler imports from ws/ sub-modules ─────────────────────

import { handleChatEvent } from "./ws/chat-handlers";
import {
	handleBrowserImagePull,
	handleLocalLlmDownload,
	handleLocalLlmLifecycle,
	handleSandboxHostProvision,
	handleSandboxImageBuild,
	handleSandboxImageProvision,
	handleSandboxPrepare,
} from "./ws/sandbox-handlers";
import {
	handleApprovalEvent,
	handleAuthCredentialsChanged,
	handleLocationRequest,
	handleLogEntry,
	handleModelsUpdated,
	handleNetworkAuditEntry,
	handleWsError,
} from "./ws/system-handlers";

// ── Types ────────────────────────────────────────────────────

import type { StreamMeta, WsFrame } from "./types/ws-events";

// Extend Window for moltis-specific global
declare global {
	interface Window {
		__moltisSuppressNextPasswordChangedRedirect?: boolean;
	}
}

// ── Connection state ────────────────────────────────────────

let hasConnectedOnce = false;

// ── Event handler map and dispatcher ────────────────────────

const eventHandlers: Record<string, (payload: Record<string, unknown>, streamMeta?: StreamMeta | null) => void> = {
	chat: handleChatEvent as (payload: Record<string, unknown>) => void,
	error: handleWsError as (payload: Record<string, unknown>) => void,
	"auth.credentials_changed": handleAuthCredentialsChanged as (payload: Record<string, unknown>) => void,
	"exec.approval.requested": handleApprovalEvent as unknown as (payload: Record<string, unknown>) => void,
	"logs.entry": handleLogEntry as (payload: Record<string, unknown>) => void,
	"sandbox.prepare": handleSandboxPrepare as (payload: Record<string, unknown>) => void,
	"sandbox.image.build": handleSandboxImageBuild as (payload: Record<string, unknown>) => void,
	"sandbox.image.provision": handleSandboxImageProvision as (payload: Record<string, unknown>) => void,
	"sandbox.host.provision": handleSandboxHostProvision as (payload: Record<string, unknown>) => void,
	"browser.image.pull": handleBrowserImagePull as (payload: Record<string, unknown>) => void,
	"local-llm.download": handleLocalLlmDownload as (payload: Record<string, unknown>) => void,
	"local-llm.lifecycle": handleLocalLlmLifecycle,
	"models.updated": handleModelsUpdated as (payload: Record<string, unknown>) => void,
	"location.request": handleLocationRequest as (payload: Record<string, unknown>) => void,
	"network.audit.entry": handleNetworkAuditEntry as (payload: Record<string, unknown>) => void,
};

function dispatchFrame(frame: WsFrame): void {
	if (frame.type !== "event") return;
	const streamMeta: StreamMeta | null =
		frame.stream != null || frame.done != null
			? { stream: frame.stream, done: frame.done, channel: frame.channel }
			: null;
	const listeners = eventListeners[frame.event || ""] || [];
	listeners.forEach((h) => {
		h(frame.payload || {});
	});
	const handler = eventHandlers[frame.event || ""];
	if (handler) handler(frame.payload || {}, streamMeta);
}

// ── Connect ──────────────────────────────────────────────────

const connectOpts: ConnectOptions = {
	onFrame: dispatchFrame as ConnectOptions["onFrame"],
	onConnected: async (hello) => {
		const isReconnect = hasConnectedOnce;
		hasConnectedOnce = true;
		setStatus("connected", "");
		const now = new Date();
		const ts = now.toLocaleTimeString([], {
			hour: "2-digit",
			minute: "2-digit",
			second: "2-digit",
		});
		chatAddMsg("system", `Connected to moltis gateway v${hello.server.version} at ${ts}`);
		if ((S.sandboxInfo as Record<string, unknown> | null)?.image_building) {
			handleSandboxImageBuild({ phase: "start" });
		}
		// Subscribe to all needed events (v4 protocol).
		// Await so that events are not lost to a race between subscribe and
		// the first broadcast after connect.
		S.setSubscribed(false);
		await subscribeEvents(
			Object.keys(eventHandlers).concat([
				"tick",
				"shutdown",
				"auth.credentials_changed",
				"exec.approval.requested",
				"exec.approval.resolved",
				"device.pair.requested",
				"device.pair.resolved",
				"node.pair.requested",
				"node.pair.resolved",
				"node.invoke.request",
				"session",
				"update.available",
				"hooks.status",
				"push.subscriptions",
				"channel",
				"metrics.update",
				"skills.install.progress",
				"mcp.status",
			]),
		);
		S.setSubscribed(true);
		// Keep initial hydration authoritative via app bootstrap/gon.
		// On reconnect, force a fresh snapshot in case realtime events were missed.
		if (isReconnect) {
			fetchModels();
			fetchSessions();
			fetchProjects();
			prefetchChannels();
		}
		sendRpc("logs.status", {}).then((res) => {
			if (res?.ok) {
				const p = (res.payload || {}) as Record<string, unknown>;
				S.setUnseenErrors((p.unseen_errors as number) || 0);
				S.setUnseenWarns((p.unseen_warns as number) || 0);
				if (currentPage === "/logs") clearLogsAlert();
				else updateLogsAlert();
			}
		});
		if (currentPage === "/chats" || currentPrefix === "/chats") mount(currentPage || "");
	},
	onHandshakeFailed: (frame) => {
		setStatus("", "handshake failed");
		const reason = frame.error?.message || "unknown error";
		chatAddMsg("error", `Handshake failed: ${reason}`);
	},
	onDisconnected: (wasConnected: boolean) => {
		if (wasConnected) {
			setStatus("", "disconnected \u2014 reconnecting\u2026");
		}
		// Reset active session's stream state
		const activeS = sessionStore.activeSession.value;
		if (activeS) activeS.resetStreamState();
		S.setStreamEl(null);
		S.setStreamText("");
	},
};

export function connect(): void {
	setStatus("connecting", "connecting...");
	connectWs(connectOpts);
}

function setStatus(state: string, text: string): void {
	const dot = S.$("statusDot");
	const sText = S.$("statusText");
	if (dot) dot.className = `status-dot ${state}`;
	if (sText) {
		sText.textContent = text;
		sText.classList.toggle("status-text-live", state === "connected");
	}
	const sendBtn = S.$<HTMLButtonElement>("sendBtn");
	if (sendBtn) sendBtn.disabled = state !== "connected";
}

document.addEventListener("visibilitychange", () => {
	if (!(document.hidden || S.connected)) {
		forceReconnect(connectOpts);
	}
});
