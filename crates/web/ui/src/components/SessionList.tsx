// ── SessionList Preact component ─────���───────────────────────
//
// Replaces the imperative renderSessionList() with a reactive Preact
// component that auto-rerenders from sessionStore signals.

import type { VNode } from "preact";
import { useEffect, useRef } from "preact/hooks";
import {
	makeBranchIcon,
	makeChatIcon,
	makeCronIcon,
	makeDiscordIcon,
	makeMatrixIcon,
	makeProjectIcon,
	makeSlackIcon,
	makeTeamsIcon,
	makeTelegramIcon,
} from "../icons";
import { currentPrefix, navigate, sessionPath } from "../router";
import { switchSession } from "../sessions";
import * as projectStore from "../stores/project-store";
import { type Session, sessionStore } from "../stores/session-store";
import { ChannelType } from "../types";

// ── Braille spinner ───────────��─────────────────────────────
const spinnerFrames: string[] = [
	"\u280B",
	"\u2819",
	"\u2839",
	"\u2838",
	"\u283C",
	"\u2834",
	"\u2826",
	"\u2827",
	"\u2807",
	"\u280F",
];

// ── Helpers ─────────���────────────────────────────────────────

function channelSessionType(s: Session): ChannelType | null {
	const key = s.key || "";
	if (key.startsWith(`${ChannelType.Telegram}:`)) return ChannelType.Telegram;
	if (key.startsWith(`${ChannelType.MsTeams}:`)) return ChannelType.MsTeams;
	if (key.startsWith(`${ChannelType.Discord}:`)) return ChannelType.Discord;
	if (key.startsWith(`${ChannelType.Slack}:`)) return ChannelType.Slack;
	if (key.startsWith(`${ChannelType.Matrix}:`)) return ChannelType.Matrix;
	const binding = s.channelBinding || null;
	if (!binding) return null;
	try {
		const parsed = typeof binding === "string" ? JSON.parse(binding) : binding;
		return (parsed.channel_type as ChannelType) || null;
	} catch (_e) {
		return null;
	}
}

function formatHHMM(epochMs: number): string {
	return new Date(epochMs).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

// ── Icon component (renders SVG icon into a ref) ────────────

interface SessionIconProps {
	session: Session;
	isBranch: boolean;
}

function SessionIcon({ session, isBranch }: SessionIconProps): VNode {
	const iconRef = useRef<HTMLSpanElement>(null);
	useEffect(() => {
		if (!iconRef.current) return;
		iconRef.current.textContent = "";
		const key = session.key || "";
		let icon: SVGElement | HTMLElement;
		const channelType = channelSessionType(session);
		if (isBranch) icon = makeBranchIcon();
		else if (key.startsWith("cron:")) icon = makeCronIcon();
		else if (channelType === ChannelType.Telegram) icon = makeTelegramIcon();
		else if (channelType === ChannelType.MsTeams) icon = makeTeamsIcon();
		else if (channelType === ChannelType.Discord) icon = makeDiscordIcon();
		else if (channelType === ChannelType.Slack) icon = makeSlackIcon();
		else if (channelType === ChannelType.Matrix) icon = makeMatrixIcon();
		else icon = makeChatIcon();
		iconRef.current.appendChild(icon);
	}, [session.key, isBranch]);

	const channelType = channelSessionType(session);
	const channelBound = Boolean(channelType);
	const iconStyle: Record<string, string> = {};
	if (channelBound) {
		iconStyle.color = session.activeChannel ? "var(--accent)" : "var(--muted)";
		iconStyle.opacity = session.activeChannel ? "1" : "0.5";
	} else {
		iconStyle.color = "var(--muted)";
	}
	const channelLabel =
		channelType === ChannelType.MsTeams
			? "Microsoft Teams"
			: channelType === ChannelType.Discord
				? "Discord"
				: channelType === ChannelType.Slack
					? "Slack"
					: channelType === ChannelType.Matrix
						? "Matrix"
						: "Telegram";
	const title = channelBound
		? session.activeChannel
			? `Active ${channelLabel} session`
			: `${channelLabel} session (inactive)`
		: "";

	// Read the reactive signal — auto-subscribes for badge updates.
	const count = session.badgeCount.value;

	return (
		<span className="session-icon" style={iconStyle} title={title}>
			<span ref={iconRef} />
			<span className="session-spinner" />
			{count > 0 && (
				<span className="session-badge" data-session-key={session.key}>
					{count > 99 ? "99+" : String(count)}
				</span>
			)}
		</span>
	);
}

// ── Session meta (fork, worktree, project) ──────────────────

interface SessionMetaProps {
	session: Session;
}

function SessionMeta({ session }: SessionMetaProps): VNode {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (!ref.current) return;
		ref.current.textContent = "";

		const parts: string[] = [];
		if (session.forkPoint != null) parts.push(`fork@${session.forkPoint}`);
		const branch = session.worktree_branch || "";
		if (branch) parts.push(`\u2387 ${branch}`);

		const projId = session.projectId || "";
		const proj = projId ? projectStore.getById(projId) : null;

		if (parts.length === 0 && !proj) return;

		ref.current.textContent = parts.join(" \u00b7 ");
		if (proj) {
			if (parts.length > 0) ref.current.appendChild(document.createTextNode(" \u00b7 "));
			const icon = makeProjectIcon();
			icon.style.display = "inline";
			icon.style.verticalAlign = "-1px";
			icon.style.marginRight = "2px";
			icon.style.opacity = "0.7";
			ref.current.appendChild(icon);
			ref.current.appendChild(document.createTextNode((proj.label as string) || proj.id));
		}
	}, [session.projectId, session.forkPoint, session.worktree_branch]);

	return <div className="session-meta" data-session-key={session.key} ref={ref} />;
}

// ── SessionItem component ───────────���───────────────────────

interface KeyMap {
	[key: string]: Session;
}

interface SessionItemProps {
	session: Session;
	activeKey: string;
	depth: number;
	keyMap: KeyMap;
	refreshing: boolean;
}

function SessionItem({ session, activeKey, depth, keyMap, refreshing }: SessionItemProps): VNode {
	const isBranch = depth > 0;
	const active = session.key === activeKey;
	// Read per-session signals — auto-subscribes for re-render.
	// dataVersion triggers re-render when plain properties (preview,
	// updatedAt, label) change. Badge updates come from badgeCount
	// signal read inside SessionIcon.
	const replying = session.replying.value;
	void session.dataVersion.value;
	// Unread tint: true when not viewing this session and there are messages
	// beyond what we last saw (badgeCount is reactive, triggers re-render).
	const badge = session.badgeCount.value;
	const unread = session.localUnread.value || (!active && badge > (session.lastSeenMessageCount || 0));

	let className = "session-item";
	if (active) className += " active";
	if (unread) className += " unread";
	if (replying) className += " replying";
	if (refreshing) className += " loading";

	const style = isBranch ? { paddingLeft: `${12 + depth * 16}px` } : {};

	const rawPreview = session.preview || "";
	const parentPreview =
		session.parentSessionKey && keyMap[session.parentSessionKey] ? keyMap[session.parentSessionKey].preview || "" : "";
	const preview = rawPreview && rawPreview === parentPreview ? "" : rawPreview;
	const ts = session.updatedAt || 0;
	const agentId = session.agent_id || "main";
	const showAgentBadge = !!agentId && agentId !== "main";

	const href = sessionPath(session.key);

	function onClick(event: MouseEvent): void {
		if (event.defaultPrevented) return;
		if (event.button !== 0) return;
		if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
		event.preventDefault();
		if (currentPrefix !== "/chats") {
			navigate(href);
		} else {
			switchSession(session.key);
		}
	}

	return (
		<a href={href} className={className} data-session-key={session.key} style={style} onClick={onClick}>
			<div className="session-info">
				<div className="session-label">
					<SessionIcon session={session} isBranch={isBranch} />
					<span data-label-text>{session.label || session.key}</span>
					{showAgentBadge && (
						<span
							className="text-[10px] text-[var(--muted)] border border-[var(--border)] rounded px-1 py-0 ml-1"
							title={`Agent: ${agentId}`}
						>
							@{agentId}
						</span>
					)}
					{ts > 0 && (
						<span className="session-time" title={new Date(ts).toLocaleString()}>
							{formatHHMM(ts)}
						</span>
					)}
				</div>
				{preview && <div className="session-preview">{preview}</div>}
				<SessionMeta session={session} />
			</div>
		</a>
	);
}

// ── SessionList component ──────────────────���────────────────
export function SessionList(): VNode {
	const allSessions = sessionStore.sessions.value;
	const activeKey = sessionStore.activeSessionKey.value;
	const refreshingKey = sessionStore.refreshInProgressKey.value;
	const filterId = projectStore.projectFilterId.value;
	const tab = sessionStore.sessionListTab.value;
	const showArchived = sessionStore.showArchivedSessions.value;

	// Spinner animation via setInterval
	const spinnersRef = useRef<HTMLDivElement>(null);
	useEffect(() => {
		let idx = 0;
		const timer = setInterval(() => {
			idx = (idx + 1) % spinnerFrames.length;
			if (!spinnersRef.current) return;
			const els = spinnersRef.current.querySelectorAll(
				".session-item.replying .session-spinner, .session-item.loading .session-spinner",
			);
			for (const el of els) el.textContent = spinnerFrames[idx];
		}, 80);
		return () => clearInterval(timer);
	}, []);

	let filtered = filterId ? allSessions.filter((s) => s.projectId === filterId) : allSessions;
	if (tab === "sessions") {
		filtered = filtered.filter((s) => !(s.key || "").startsWith("cron:") && (showArchived || !s.archived));
	} else if (tab === "cron") {
		filtered = filtered.filter((s) => (s.key || "").startsWith("cron:"));
	}

	// Build parent→children map for tree rendering
	const childrenMap: Record<string, Session[]> = {};
	const keyMap: KeyMap = {};
	filtered.forEach((s) => {
		keyMap[s.key] = s;
		if (s.parentSessionKey) {
			if (!childrenMap[s.parentSessionKey]) childrenMap[s.parentSessionKey] = [];
			childrenMap[s.parentSessionKey].push(s);
		}
	});
	const roots = filtered.filter((s) => !(s.parentSessionKey && keyMap[s.parentSessionKey]));

	function renderTree(session: Session, depth: number): VNode {
		const children = childrenMap[session.key] || [];
		return (
			<>
				<SessionItem
					key={session.key}
					session={session}
					activeKey={activeKey}
					depth={depth}
					keyMap={keyMap}
					refreshing={session.key === refreshingKey}
				/>
				{children.map((child) => renderTree(child, depth + 1))}
			</>
		);
	}

	return <div ref={spinnersRef}>{roots.map((s) => renderTree(s, 0))}</div>;
}
