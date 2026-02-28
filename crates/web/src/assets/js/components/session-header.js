// ── SessionHeader Preact component ───────────────────────────
//
// Replaces the imperative updateChatSessionHeader() with a reactive
// Preact component reading sessionStore.activeSession.

import { html } from "htm/preact";
import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import { parseAgentsListPayload, sendRpc } from "../helpers.js";
import {
	clearActiveSession,
	fetchSessions,
	setSessionActiveRunId,
	setSessionReplying,
	switchSession,
} from "../sessions.js";
import { sessionStore } from "../stores/session-store.js";
import { confirmDialog, shareLinkDialog, shareVisibilityDialog, showToast } from "../ui.js";

function nextSessionKey(currentKey) {
	var allSessions = sessionStore.sessions.value;
	var s = allSessions.find((x) => x.key === currentKey);
	if (s?.parentSessionKey) return s.parentSessionKey;
	var idx = allSessions.findIndex((x) => x.key === currentKey);
	if (idx >= 0 && idx + 1 < allSessions.length) return allSessions[idx + 1].key;
	if (idx > 0) return allSessions[idx - 1].key;
	return "main";
}

function buildShareUrl(payload) {
	var url = `${window.location.origin}${payload.path}`;
	if (payload.accessKey) {
		url += `?k=${encodeURIComponent(payload.accessKey)}`;
	}
	return url;
}

async function copyShareUrl(url, visibility) {
	try {
		if (navigator.clipboard?.writeText) {
			await navigator.clipboard.writeText(url);
			showToast("Share link copied", "success");
			return;
		}
	} catch (_err) {
		// Clipboard APIs can fail on some browsers/permissions.
	}
	await shareLinkDialog(url, visibility);
}

export function SessionHeader() {
	var session = sessionStore.activeSession.value;
	var currentKey = sessionStore.activeSessionKey.value;

	var [renaming, setRenaming] = useState(false);
	var [clearing, setClearing] = useState(false);
	var [stopping, setStopping] = useState(false);
	var [switchingAgent, setSwitchingAgent] = useState(false);
	var [agentOptions, setAgentOptions] = useState([]);
	var [defaultAgentId, setDefaultAgentId] = useState("main");
	var inputRef = useRef(null);

	var fullName = session ? session.label || session.key : currentKey;
	var displayName = fullName.length > 20 ? `${fullName.slice(0, 20)}\u2026` : fullName;
	var replying = session?.replying.value;
	var activeRunId = session?.activeRunId.value || null;

	var isMain = currentKey === "main";
	var isChannel = session?.channelBinding || currentKey.startsWith("telegram:") || currentKey.startsWith("msteams:");
	var isCron = currentKey.startsWith("cron:");
	var canRename = !(isMain || isChannel || isCron);
	var canStop = !isCron && replying;
	var currentAgentId = session?.agent_id || defaultAgentId || "main";

	useEffect(() => {
		var cancelled = false;
		sendRpc("agents.list", {}).then((res) => {
			if (cancelled || !res?.ok) return;
			var parsed = parseAgentsListPayload(res.payload);
			setDefaultAgentId(parsed.defaultId);
			setAgentOptions(parsed.agents);
		});
		return () => {
			cancelled = true;
		};
	}, [currentKey]);

	var startRename = useCallback(() => {
		if (!canRename) return;
		setRenaming(true);
		requestAnimationFrame(() => {
			if (inputRef.current) {
				inputRef.current.value = fullName;
				inputRef.current.focus();
				inputRef.current.select();
			}
		});
	}, [canRename, fullName]);

	var commitRename = useCallback(() => {
		var val = inputRef.current?.value.trim() || "";
		setRenaming(false);
		if (val && val !== fullName) {
			sendRpc("sessions.patch", { key: currentKey, label: val }).then((res) => {
				if (res?.ok) fetchSessions();
			});
		}
	}, [currentKey, fullName]);

	var onKeyDown = useCallback(
		(e) => {
			if (e.key === "Enter") {
				e.preventDefault();
				commitRename();
			}
			if (e.key === "Escape") {
				setRenaming(false);
			}
		},
		[commitRename],
	);

	var onFork = useCallback(() => {
		sendRpc("sessions.fork", { key: currentKey }).then((res) => {
			if (res?.ok && res.payload?.sessionKey) {
				fetchSessions();
				switchSession(res.payload.sessionKey);
			}
		});
	}, [currentKey]);

	var onDelete = useCallback(() => {
		var msgCount = session ? session.messageCount || 0 : 0;
		var nextKey = nextSessionKey(currentKey);
		var doDelete = () => {
			sendRpc("sessions.delete", { key: currentKey }).then((res) => {
				if (res && !res.ok && res.error && res.error.indexOf("uncommitted changes") !== -1) {
					confirmDialog("Worktree has uncommitted changes. Force delete?").then((yes) => {
						if (!yes) return;
						sendRpc("sessions.delete", { key: currentKey, force: true }).then(() => {
							switchSession(nextKey);
							fetchSessions();
						});
					});
					return;
				}
				switchSession(nextKey);
				fetchSessions();
			});
		};
		var isUnmodifiedFork = session && session.forkPoint != null && msgCount <= session.forkPoint;
		if (msgCount > 0 && !isUnmodifiedFork) {
			confirmDialog("Delete this session?").then((yes) => {
				if (yes) doDelete();
			});
		} else {
			doDelete();
		}
	}, [currentKey, session]);

	var onClear = useCallback(() => {
		if (clearing) return;
		setClearing(true);
		clearActiveSession().finally(() => {
			setClearing(false);
		});
	}, [clearing]);

	var onStop = useCallback(() => {
		if (stopping) return;
		var params = { sessionKey: currentKey };
		if (activeRunId) params.runId = activeRunId;
		setStopping(true);
		sendRpc("chat.abort", params)
			.then((res) => {
				if (!res?.ok) {
					showToast(res?.error?.message || "Failed to stop response", "error");
					return;
				}
				setSessionActiveRunId(currentKey, null);
				setSessionReplying(currentKey, false);
			})
			.finally(() => {
				setStopping(false);
			});
	}, [activeRunId, currentKey, stopping]);

	var shareSnapshot = useCallback(
		async (visibility) => {
			var res = await sendRpc("sessions.share.create", { key: currentKey, visibility: visibility });
			if (!(res?.ok && res.payload?.path)) {
				showToast(res?.error?.message || "Failed to create share link", "error");
				return;
			}

			var url = buildShareUrl(res.payload);
			await copyShareUrl(url, visibility);

			if (visibility === "private") {
				showToast("Private link includes a key, share it only with trusted people", "success");
			}

			// Reload the active session so the snapshot cutoff notice appears.
			switchSession(currentKey);
			fetchSessions();
		},
		[currentKey],
	);

	var onShare = useCallback(() => {
		shareVisibilityDialog().then((visibility) => {
			if (!visibility) return;
			void shareSnapshot(visibility);
		});
	}, [shareSnapshot]);

	var onAgentChange = useCallback(
		(event) => {
			var nextAgentId = event.target.value;
			if (!nextAgentId || nextAgentId === currentAgentId || switchingAgent) {
				return;
			}
			setSwitchingAgent(true);
			sendRpc("agents.set_session", {
				session_key: currentKey,
				agent_id: nextAgentId,
			})
				.then((res) => {
					if (!res?.ok) {
						showToast(res?.error?.message || "Failed to switch agent", "error");
						return;
					}
					if (session) {
						session.agent_id = nextAgentId;
						session.dataVersion.value++;
					}
					fetchSessions();
				})
				.finally(() => {
					setSwitchingAgent(false);
				});
		},
		[currentAgentId, currentKey, session, switchingAgent],
	);

	var agentSelectValue = currentAgentId;
	var hasCurrentAgentOption = agentOptions.some((agent) => agent.id === agentSelectValue);
	var selectDisabled = switchingAgent || agentOptions.length === 0;

	return html`
		<div class="flex items-center gap-2">
			${
				!isCron &&
				html`
				<select
					class="chat-session-btn"
					value=${agentSelectValue}
					onChange=${onAgentChange}
					disabled=${selectDisabled}
					title="Session agent"
					style="max-width:180px;text-overflow:ellipsis;"
				>
					${
						!hasCurrentAgentOption &&
						html`
						<option value=${agentSelectValue}>
							${switchingAgent ? "Switching…" : `agent:${agentSelectValue}`}
						</option>
					`
					}
					${agentOptions.map((agent) => {
						var prefix = agent.emoji ? `${agent.emoji} ` : "";
						var suffix = agent.id === defaultAgentId ? " (default)" : "";
						return html`
							<option key=${agent.id} value=${agent.id}>
								${`${prefix}${agent.name}${suffix}`}
							</option>
						`;
					})}
				</select>
			`
			}
			${
				renaming
					? html`<input
						ref=${inputRef}
						class="chat-session-rename-input"
						onBlur=${commitRename}
						onKeyDown=${onKeyDown}
					/>`
					: html`<span
						class="chat-session-name"
						style=${{ cursor: canRename ? "pointer" : "default" }}
						title=${canRename ? "Click to rename" : ""}
						onClick=${startRename}
					>${displayName}</span>`
			}
			${
				!isCron &&
				html`
				<button class="chat-session-btn" onClick=${onFork} title="Fork session">
					Fork
				</button>
				<button class="chat-session-btn" onClick=${onShare} title="Share snapshot">
					Share
				</button>
			`
			}
			${
				canStop &&
				html`
				<button class="chat-session-btn" onClick=${onStop} title="Stop generation" disabled=${stopping}>
					${stopping ? "Stopping\u2026" : "Stop"}
				</button>
			`
			}
			${
				isMain &&
				html`
				<button class="chat-session-btn" onClick=${onClear} title="Clear session" disabled=${clearing}>
					${clearing ? "Clearing\u2026" : "Clear"}
				</button>
			`
			}
			${
				!(isMain || isCron) &&
				html`
				<button class="chat-session-btn chat-session-btn-danger" onClick=${onDelete} title="Delete session">
					Delete
				</button>
			`
			}
		</div>
	`;
}
