// ── SessionList Preact component ─────────────────────────────
//
// Replaces the imperative renderSessionList() with a reactive Preact
// component that auto-rerenders from sessionStore signals.

import { html } from "htm/preact";
import { useEffect, useRef } from "preact/hooks";
import { makeBranchIcon, makeChatIcon, makeCronIcon, makeProjectIcon, makeTelegramIcon } from "../icons.js";
import { currentPrefix, navigate, sessionPath } from "../router.js";
import { switchSession } from "../sessions.js";
import { projectStore } from "../stores/project-store.js";
import { sessionStore } from "../stores/session-store.js";

// ── Braille spinner ─────────────────────────────────────────
var spinnerFrames = [
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

// ── Helpers ──────────────────────────────────────────────────

function isTelegramSession(s) {
	var key = s.key || "";
	if (key.startsWith("telegram:")) return true;
	var binding = s.channelBinding || null;
	if (!binding) return false;
	try {
		return JSON.parse(binding).channel_type === "telegram";
	} catch (_e) {
		return false;
	}
}

function formatHHMM(epochMs) {
	return new Date(epochMs).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

// ── Icon component (renders SVG icon into a ref) ────────────
function SessionIcon({ session, isBranch }) {
	var iconRef = useRef(null);
	useEffect(() => {
		if (!iconRef.current) return;
		iconRef.current.textContent = "";
		var key = session.key || "";
		var icon;
		if (isBranch) icon = makeBranchIcon();
		else if (key.startsWith("cron:")) icon = makeCronIcon();
		else if (isTelegramSession(session)) icon = makeTelegramIcon();
		else icon = makeChatIcon();
		iconRef.current.appendChild(icon);
	}, [session.key, isBranch]);

	var telegram = isTelegramSession(session);
	var iconStyle = {};
	if (telegram) {
		iconStyle.color = session.activeChannel ? "var(--accent)" : "var(--muted)";
		iconStyle.opacity = session.activeChannel ? "1" : "0.5";
	} else {
		iconStyle.color = "var(--muted)";
	}
	var title = telegram ? (session.activeChannel ? "Active Telegram session" : "Telegram session (inactive)") : "";

	// Read the reactive signal — auto-subscribes for badge updates.
	var count = session.badgeCount.value;

	return html`
		<span class="session-icon" style=${iconStyle} title=${title}>
			<span ref=${iconRef}></span>
			<span class="session-spinner"></span>
			${
				count > 0 &&
				html`
				<span class="session-badge" data-session-key=${session.key}>
					${count > 99 ? "99+" : String(count)}
				</span>
			`
			}
		</span>
	`;
}

// ── Session meta (fork, worktree, project) ──────────────────
function SessionMeta({ session }) {
	var ref = useRef(null);

	useEffect(() => {
		if (!ref.current) return;
		ref.current.textContent = "";

		var parts = [];
		if (session.forkPoint != null) parts.push(`fork@${session.forkPoint}`);
		var branch = session.worktree_branch || "";
		if (branch) parts.push(`\u2387 ${branch}`);

		var projId = session.projectId || "";
		var proj = projId ? projectStore.getById(projId) : null;

		if (parts.length === 0 && !proj) return;

		ref.current.textContent = parts.join(" \u00b7 ");
		if (proj) {
			if (parts.length > 0) ref.current.appendChild(document.createTextNode(" \u00b7 "));
			var icon = makeProjectIcon();
			icon.style.display = "inline";
			icon.style.verticalAlign = "-1px";
			icon.style.marginRight = "2px";
			icon.style.opacity = "0.7";
			ref.current.appendChild(icon);
			ref.current.appendChild(document.createTextNode(proj.label || proj.id));
		}
	}, [session.projectId, session.forkPoint, session.worktree_branch]);

	return html`<div class="session-meta" data-session-key=${session.key} ref=${ref}></div>`;
}

// ── SessionItem component ───────────────────────────────────
function SessionItem({ session, activeKey, depth, keyMap }) {
	var isBranch = depth > 0;
	var active = session.key === activeKey;
	// Read per-session signals — auto-subscribes for re-render.
	// dataVersion triggers re-render when plain properties (preview,
	// updatedAt, label) change. Badge updates come from badgeCount
	// signal read inside SessionIcon.
	var replying = session.replying.value;
	session.dataVersion.value;
	// Unread tint: true when not viewing this session and there are messages
	// beyond what we last saw (badgeCount is reactive, triggers re-render).
	var badge = session.badgeCount.value;
	var unread = session.localUnread.value || (!active && badge > (session.lastSeenMessageCount || 0));

	var className = "session-item";
	if (active) className += " active";
	if (unread) className += " unread";
	if (replying) className += " replying";

	var style = isBranch ? { paddingLeft: `${12 + depth * 16}px` } : {};

	var rawPreview = session.preview || "";
	var parentPreview =
		session.parentSessionKey && keyMap[session.parentSessionKey] ? keyMap[session.parentSessionKey].preview || "" : "";
	var preview = rawPreview && rawPreview === parentPreview ? "" : rawPreview;
	var ts = session.updatedAt || 0;

	function onClick() {
		if (currentPrefix !== "/chats") {
			navigate(sessionPath(session.key));
		} else {
			switchSession(session.key);
		}
	}

	return html`
		<div class=${className} data-session-key=${session.key} style=${style} onClick=${onClick}>
			<div class="session-info">
				<div class="session-label">
					<${SessionIcon} session=${session} isBranch=${isBranch} />
					<span data-label-text>${session.label || session.key}</span>
					${
						ts > 0 &&
						html`
						<span class="session-time" title=${new Date(ts).toLocaleString()}>
							${formatHHMM(ts)}
						</span>
					`
					}
				</div>
				${preview && html`<div class="session-preview">${preview}</div>`}
				<${SessionMeta} session=${session} />
			</div>
		</div>
	`;
}

// ── SessionList component ───────────────────────────────────
export function SessionList() {
	var allSessions = sessionStore.sessions.value;
	var activeKey = sessionStore.activeSessionKey.value;
	var filterId = projectStore.projectFilterId.value;

	var filtered = filterId ? allSessions.filter((s) => s.projectId === filterId) : allSessions;

	// Build parent→children map for tree rendering
	var childrenMap = {};
	var keyMap = {};
	filtered.forEach((s) => {
		keyMap[s.key] = s;
		if (s.parentSessionKey) {
			if (!childrenMap[s.parentSessionKey]) childrenMap[s.parentSessionKey] = [];
			childrenMap[s.parentSessionKey].push(s);
		}
	});
	var roots = filtered.filter((s) => !(s.parentSessionKey && keyMap[s.parentSessionKey]));

	// Spinner animation via setInterval
	var spinnersRef = useRef(null);
	useEffect(() => {
		var idx = 0;
		var timer = setInterval(() => {
			idx = (idx + 1) % spinnerFrames.length;
			if (!spinnersRef.current) return;
			var els = spinnersRef.current.querySelectorAll(".session-item.replying .session-spinner");
			for (var el of els) el.textContent = spinnerFrames[idx];
		}, 80);
		return () => clearInterval(timer);
	}, []);

	function renderTree(session, depth) {
		var children = childrenMap[session.key] || [];
		return html`
			<${SessionItem}
				key=${session.key}
				session=${session}
				activeKey=${activeKey}
				depth=${depth}
				keyMap=${keyMap}
			/>
			${children.map((child) => renderTree(child, depth + 1))}
		`;
	}

	return html`
		<div ref=${spinnersRef}>
			${roots.map((s) => renderTree(s, 0))}
		</div>
	`;
}
