// ── SessionHeader Preact component ───────────────────────────
//
// Replaces the imperative updateChatSessionHeader() with a reactive
// Preact component reading sessionStore.activeSession.

import { html } from "htm/preact";
import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import { onEvent } from "../events.js";
import * as gon from "../gon.js";
import { parseAgentsListPayload, sendRpc } from "../helpers.js";
import {
	clearActiveSession,
	fetchSessions,
	isArchivableSession,
	removeSessionFromClientState,
	setSessionActiveRunId,
	setSessionReplying,
	switchSession,
} from "../sessions.js";
import { sessionStore } from "../stores/session-store.js";
import { ComboSelect, confirmDialog, shareLinkDialog, shareVisibilityDialog, showToast } from "../ui.js";

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

function isSshTargetNode(node) {
	return node?.platform === "ssh" || String(node?.nodeId || "").startsWith("ssh:");
}

function nodeOptionLabel(node) {
	if (!node) return "Local";
	if (node.displayName) return node.displayName;
	if (isSshTargetNode(node)) {
		var target = String(node.nodeId || "").replace(/^ssh:/, "");
		return `SSH: ${target}`;
	}
	return node.nodeId;
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

export function SessionHeader({
	showSelectors = true,
	showName = true,
	showShare = true,
	showFork = true,
	showStop = true,
	showClear = true,
	showDelete = true,
	showArchive = true,
	nameOwnLine = false,
	showRenameButton = false,
	actionButtonClass = "chat-session-btn",
	onBeforeShare = null,
	onBeforeArchive = null,
	onBeforeDelete = null,
} = {}) {
	var session = sessionStore.activeSession.value;
	var sessionDataVersion = session?.dataVersion.value || 0;
	var currentKey = sessionStore.activeSessionKey.value;
	var gonAgentsPayload = parseAgentsListPayload(gon.get("agents"));
	var initialAgentOptions = Array.isArray(gonAgentsPayload?.agents) ? gonAgentsPayload.agents : [];
	var initialDefaultAgentId = typeof gonAgentsPayload?.defaultId === "string" ? gonAgentsPayload.defaultId : "main";

	var [renaming, setRenaming] = useState(false);
	var [clearing, setClearing] = useState(false);
	var [stopping, setStopping] = useState(false);
	var [switchingAgent, setSwitchingAgent] = useState(false);
	var [agentOptions, setAgentOptions] = useState(initialAgentOptions);
	var [defaultAgentId, setDefaultAgentId] = useState(initialDefaultAgentId);
	var [agentOptionsLoaded, setAgentOptionsLoaded] = useState(initialAgentOptions.length > 0);
	var [nodeOptions, setNodeOptions] = useState([]);
	var [switchingNode, setSwitchingNode] = useState(false);
	var inputRef = useRef(null);

	var fullName = session ? session.label || session.key : currentKey;
	var displayName = nameOwnLine ? fullName : fullName.length > 20 ? `${fullName.slice(0, 20)}\u2026` : fullName;
	var replying = session?.replying.value;
	var activeRunId = session?.activeRunId.value || null;

	var isMain = currentKey === "main";
	var isCron = currentKey.startsWith("cron:");
	var canRename = !(isMain || isCron);
	var canStop = !isCron && replying;
	var canArchive = !!session && isArchivableSession(session);
	var showArchivedSessions = sessionStore.showArchivedSessions.value;
	var currentAgentId = session?.agent_id || defaultAgentId || "main";
	var currentNodeId = session?.node_id || "";

	useEffect(() => {
		var cancelled = false;
		sendRpc("agents.list", {}).then((res) => {
			if (cancelled) return;
			if (!res?.ok) {
				setAgentOptionsLoaded(true);
				return;
			}
			var parsed = parseAgentsListPayload(res.payload);
			setDefaultAgentId(parsed.defaultId);
			setAgentOptions(parsed.agents);
			setAgentOptionsLoaded(true);
		});
		return () => {
			cancelled = true;
		};
	}, [currentKey]);

	// Fetch connected nodes and subscribe to presence updates.
	useEffect(() => {
		var cancelled = false;
		var fetchNodes = () => {
			sendRpc("node.list", {}).then((res) => {
				if (cancelled || !res?.ok) return;
				setNodeOptions(Array.isArray(res.payload) ? res.payload : []);
			});
		};
		fetchNodes();
		var unsub = onEvent("presence", () => {
			if (!cancelled) fetchNodes();
		});
		return () => {
			cancelled = true;
			unsub();
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
			if (e.key === "Enter" && !e.isComposing) {
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
		if (typeof onBeforeDelete === "function") {
			onBeforeDelete();
		}
		var currentSession = sessionStore.getByKey(currentKey);
		var msgCount = currentSession ? currentSession.messageCount || 0 : 0;
		var nextKey = nextSessionKey(currentKey);
		var canOptimisticallyDelete = !(currentSession && currentSession.worktree_branch);
		var applyDeletedState = () => {
			removeSessionFromClientState(currentKey, { nextKey: nextKey });
			switchSession(nextKey);
		};
		var runDelete = (force) => {
			var request = { key: currentKey };
			if (force) request.force = true;
			var optimisticApplied = false;
			if (canOptimisticallyDelete && !force) {
				applyDeletedState();
				optimisticApplied = true;
			}
			sendRpc("sessions.delete", request).then((res) => {
				var err = res?.error?.message || res?.error || "";
				if (res && !res.ok && typeof err === "string" && err.indexOf("uncommitted changes") !== -1) {
					fetchSessions();
					confirmDialog("Worktree has uncommitted changes. Force delete?").then((yes) => {
						if (!yes) return;
						runDelete(true);
					});
					return;
				}
				if (res && !res.ok) {
					showToast(err || "Failed to delete session", "error");
					fetchSessions();
					return;
				}
				if (!optimisticApplied) {
					applyDeletedState();
				}
				fetchSessions();
			});
		};
		var isUnmodifiedFork = currentSession && currentSession.forkPoint != null && msgCount <= currentSession.forkPoint;
		if (msgCount > 0 && !isUnmodifiedFork) {
			confirmDialog("Delete this session?").then((yes) => {
				if (yes) runDelete(false);
			});
		} else {
			runDelete(false);
		}
	}, [currentKey, onBeforeDelete, sessionDataVersion]);

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
		if (typeof onBeforeShare === "function") {
			onBeforeShare();
		}
		shareVisibilityDialog().then((visibility) => {
			if (!visibility) return;
			void shareSnapshot(visibility);
		});
	}, [onBeforeShare, shareSnapshot]);

	var onArchive = useCallback(() => {
		if (!(session && canArchive)) return;
		if (typeof onBeforeArchive === "function") {
			onBeforeArchive();
		}
		var nextArchived = !session.archived;
		sendRpc("sessions.patch", { key: currentKey, archived: nextArchived }).then((res) => {
			if (!res?.ok) {
				showToast(res?.error?.message || "Failed to update archive state", "error");
				return;
			}
			if (session) {
				session.archived = nextArchived;
				session.dataVersion.value++;
			}
			if (nextArchived && !showArchivedSessions) {
				switchSession("main");
			}
			fetchSessions();
		});
	}, [canArchive, currentKey, onBeforeArchive, session, showArchivedSessions]);

	var onAgentChange = useCallback(
		(nextAgentId) => {
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

	var onNodeChange = useCallback(
		(nextNodeId) => {
			if (switchingNode) return;
			setSwitchingNode(true);
			sendRpc("nodes.set_session", {
				session_key: currentKey,
				node_id: nextNodeId || null,
			})
				.then((res) => {
					if (!res?.ok) {
						showToast(res?.error?.message || "Failed to switch node", "error");
						return;
					}
					if (session) {
						session.node_id = nextNodeId || null;
						session.dataVersion.value++;
					}
					fetchSessions();
				})
				.finally(() => {
					setSwitchingNode(false);
				});
		},
		[currentKey, session, switchingNode],
	);

	var agentSelectValue = currentAgentId;
	var hasCurrentAgentOption = agentOptions.some((agent) => agent.id === agentSelectValue);
	var agentSelectOptions = agentOptions.map((agent) => {
		var prefix = agent.emoji ? `${agent.emoji} ` : "";
		var suffix = agent.id === defaultAgentId ? " (default)" : "";
		return {
			value: agent.id,
			label: `${prefix}${agent.name}${suffix}`,
		};
	});
	if (!hasCurrentAgentOption && agentSelectValue && (switchingAgent || agentOptionsLoaded)) {
		agentSelectOptions = [
			{
				value: agentSelectValue,
				label: switchingAgent ? "Switching…" : `agent:${agentSelectValue}`,
			},
			...agentSelectOptions,
		];
	}
	var agentSelectDisabled = switchingAgent || agentSelectOptions.length === 0;
	var shouldShowAgentPicker = !isCron && agentOptionsLoaded && (agentOptions.length > 1 || !hasCurrentAgentOption);

	var shouldShowNodePicker = !isCron && (nodeOptions.length > 0 || Boolean(currentNodeId));
	var hasCurrentNodeOption = currentNodeId === "" || nodeOptions.some((node) => node.nodeId === currentNodeId);
	var nodeSelectOptions = [
		{ value: "", label: "Local" },
		...nodeOptions.map((node) => ({
			value: node.nodeId,
			label: nodeOptionLabel(node),
		})),
	];
	if (!hasCurrentNodeOption && currentNodeId) {
		var fallbackLabel = currentNodeId.startsWith("ssh:") ? `SSH: ${currentNodeId.slice(4)}` : `node:${currentNodeId}`;
		nodeSelectOptions = [
			{
				value: currentNodeId,
				label: switchingNode ? "Switching…" : fallbackLabel,
			},
			...nodeSelectOptions,
		];
	}

	var nameStyle = { cursor: canRename ? "pointer" : "default" };
	if (nameOwnLine) {
		nameStyle.color = "var(--text-strong)";
		nameStyle.wordBreak = "break-word";
	}
	var renameInputStyle = nameOwnLine ? { maxWidth: "none", width: "100%" } : undefined;

	var nameControl =
		showName &&
		(renaming
			? html`<input
				ref=${inputRef}
				class="chat-session-rename-input"
				style=${renameInputStyle}
				onBlur=${commitRename}
				onKeyDown=${onKeyDown}
			/>`
			: html`<span
				class="chat-session-name"
				style=${nameStyle}
				title=${canRename ? "Click to rename" : ""}
				onClick=${startRename}
			>${displayName}</span>`);

	var renameCta =
		showName &&
		showRenameButton &&
		canRename &&
		!renaming &&
		html`<button class=${actionButtonClass} onClick=${startRename} title="Rename session">
			Rename
		</button>`;

	return html`
			<div class=${nameOwnLine ? "flex flex-col gap-2 w-full" : "flex items-center gap-2"}>
				${
					nameOwnLine &&
					showName &&
					html`<div class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2 w-full">
						<div class="min-w-0">${nameControl}</div>
						<div class="justify-self-end">${renameCta}</div>
					</div>`
				}
			<div class=${nameOwnLine ? "flex flex-wrap items-center gap-2" : "flex items-center gap-2"}>
			${
				showSelectors &&
				shouldShowAgentPicker &&
				html`
				<${ComboSelect}
					options=${agentSelectOptions}
					value=${agentSelectValue}
					onChange=${onAgentChange}
					placeholder="Session agent"
					searchable=${false}
					allowEmpty=${false}
					fullWidth=${false}
					disabled=${agentSelectDisabled}
				/>
			`
			}
			${
				showSelectors &&
				shouldShowNodePicker &&
				html`
				<${ComboSelect}
					options=${nodeSelectOptions}
					value=${currentNodeId}
					onChange=${onNodeChange}
					placeholder="Session node"
					searchable=${false}
					allowEmpty=${false}
					fullWidth=${false}
					disabled=${switchingNode}
				/>
			`
			}
			${!nameOwnLine && showName && nameControl}
			${!nameOwnLine && renameCta}
				${
					showArchive &&
					canArchive &&
					html`
					<button class=${actionButtonClass} onClick=${onArchive} title=${session?.archived ? "Unarchive session" : "Archive session"}>
						${session?.archived ? "Unarchive" : "Archive"}
					</button>
				`
				}
				${
					showDelete &&
					!isMain &&
					html`
					<button
						class=${`${actionButtonClass} chat-session-btn-danger inline-flex items-center gap-1.5`}
						onClick=${onDelete}
						title="Delete session"
						style=${{ background: "var(--error)", borderColor: "var(--error)", color: "#fff" }}
					>
						<span class="icon icon-sm icon-x-circle shrink-0"></span>
						Delete
					</button>
				`
				}
				${
					showFork &&
					!isCron &&
					html`
					<button class=${`${actionButtonClass} inline-flex items-center gap-1.5`} onClick=${onFork} title="Fork session">
						<span class="icon icon-sm icon-layers shrink-0"></span>
						Fork
					</button>
				`
				}
				${
					showShare &&
					!isCron &&
					html`
					<button class=${`${actionButtonClass} inline-flex items-center gap-1.5`} onClick=${onShare} title="Share snapshot">
						<span class="icon icon-sm icon-share shrink-0"></span>
						Share
					</button>
				`
				}
				${
					showStop &&
					canStop &&
					html`
					<button class=${actionButtonClass} onClick=${onStop} title="Stop generation" disabled=${stopping}>
						${stopping ? "Stopping\u2026" : "Stop"}
					</button>
				`
				}
				${
					showClear &&
					isMain &&
					html`
					<button class=${actionButtonClass} onClick=${onClear} title="Clear session" disabled=${clearing}>
						${clearing ? "Clearing\u2026" : "Clear"}
					</button>
				`
				}
			</div>
		</div>
	`;
}
