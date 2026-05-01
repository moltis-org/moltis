// ── SessionHeader Preact component ───────────────────────────
//
// Replaces the imperative updateChatSessionHeader() with a reactive
// Preact component reading sessionStore.activeSession.

import type { VNode } from "preact";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { onEvent } from "../events";
import * as gon from "../gon";
import { parseAgentsListPayload, sendRpc } from "../helpers";
import {
	clearActiveSession,
	fetchSessions,
	isArchivableSession,
	removeSessionFromClientState,
	setSessionActiveRunId,
	setSessionReplying,
	switchSession,
} from "../sessions";
import { sessionStore } from "../stores/session-store";
import { ComboSelect, confirmDialog, shareLinkDialog, shareVisibilityDialog, showToast } from "../ui";

// ── Types ────────────────────────────────────────────────────

interface NodeInfo {
	nodeId: string;
	displayName?: string;
	platform?: string;
	[key: string]: unknown;
}

interface AgentOption {
	id: string;
	name: string;
	emoji?: string;
	is_default?: boolean;
	[key: string]: unknown;
}

interface SelectOption {
	value: string;
	label: string;
}

interface SharePayload {
	path?: string;
	accessKey?: string;
}

export interface SessionHeaderProps {
	showSelectors?: boolean;
	showName?: boolean;
	showShare?: boolean;
	showFork?: boolean;
	showStop?: boolean;
	showClear?: boolean;
	showDelete?: boolean;
	showArchive?: boolean;
	nameOwnLine?: boolean;
	showRenameButton?: boolean;
	actionButtonClass?: string;
	onBeforeShare?: (() => void) | null;
	onBeforeArchive?: (() => void) | null;
	onBeforeDelete?: (() => void) | null;
}

// ── Helpers ──────────────────────────────────────────────────

function nextSessionKey(currentKey: string): string {
	const allSessions = sessionStore.sessions.value;
	const s = allSessions.find((x) => x.key === currentKey);
	if (s?.parentSessionKey) return s.parentSessionKey;
	const idx = allSessions.findIndex((x) => x.key === currentKey);
	if (idx >= 0 && idx + 1 < allSessions.length) return allSessions[idx + 1].key;
	if (idx > 0) return allSessions[idx - 1].key;
	return "main";
}

function buildShareUrl(payload: SharePayload): string {
	let url = `${window.location.origin}${payload.path}`;
	if (payload.accessKey) {
		url += `?k=${encodeURIComponent(payload.accessKey)}`;
	}
	return url;
}

function isSshTargetNode(node: NodeInfo | null): boolean {
	return node?.platform === "ssh" || String(node?.nodeId || "").startsWith("ssh:");
}

function nodeOptionLabel(node: NodeInfo | null): string {
	if (!node) return "Local";
	if (node.displayName) return node.displayName;
	if (isSshTargetNode(node)) {
		const target = String(node.nodeId || "").replace(/^ssh:/, "");
		return `SSH: ${target}`;
	}
	return node.nodeId;
}

async function copyShareUrl(url: string, visibility: string): Promise<void> {
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

// ── Component ────────────────────────────────────────────────

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
}: SessionHeaderProps = {}): VNode {
	const session = sessionStore.activeSession.value;
	const sessionDataVersion = session?.dataVersion.value || 0;
	const currentKey = sessionStore.activeSessionKey.value;
	const gonAgentsPayload = parseAgentsListPayload(gon.get("agents") as never);
	const initialAgentOptions: AgentOption[] = Array.isArray(gonAgentsPayload?.agents)
		? (gonAgentsPayload.agents as AgentOption[])
		: [];
	const initialDefaultAgentId = typeof gonAgentsPayload?.defaultId === "string" ? gonAgentsPayload.defaultId : "main";

	const [renaming, setRenaming] = useState(false);
	const [clearing, setClearing] = useState(false);
	const [stopping, setStopping] = useState(false);
	const [switchingAgent, setSwitchingAgent] = useState(false);
	const [agentOptions, setAgentOptions] = useState<AgentOption[]>(initialAgentOptions);
	const [defaultAgentId, setDefaultAgentId] = useState(initialDefaultAgentId);
	const [agentOptionsLoaded, setAgentOptionsLoaded] = useState(initialAgentOptions.length > 0);
	const [nodeOptions, setNodeOptions] = useState<NodeInfo[]>([]);
	const [switchingNode, setSwitchingNode] = useState(false);
	const inputRef = useRef<HTMLInputElement>(null);

	const fullName = session ? session.label || session.key : currentKey;
	const displayName = nameOwnLine ? fullName : fullName.length > 20 ? `${fullName.slice(0, 20)}\u2026` : fullName;
	const replying = session?.replying.value;
	const activeRunId = session?.activeRunId.value || null;

	const isMain = currentKey === "main";
	const isCron = currentKey.startsWith("cron:");
	const canRename = !(isMain || isCron);
	const canStop = !isCron && replying;
	const canArchive = !!session && isArchivableSession(session.toMeta());
	const showArchivedSessions = sessionStore.showArchivedSessions.value;
	const currentAgentId = session?.agent_id || defaultAgentId || "main";
	const currentNodeId = session?.node_id || "";

	useEffect(() => {
		let cancelled = false;
		sendRpc("agents.list", {}).then((res) => {
			if (cancelled) return;
			if (!res?.ok) {
				setAgentOptionsLoaded(true);
				return;
			}
			const parsed = parseAgentsListPayload(res.payload as never);
			setDefaultAgentId(parsed.defaultId);
			setAgentOptions(parsed.agents as AgentOption[]);
			setAgentOptionsLoaded(true);
		});
		return () => {
			cancelled = true;
		};
	}, [currentKey]);

	// Fetch connected nodes and subscribe to presence updates.
	useEffect(() => {
		let cancelled = false;
		const fetchNodes = (): void => {
			sendRpc<NodeInfo[]>("node.list", {}).then((res) => {
				if (cancelled || !res?.ok) return;
				setNodeOptions(Array.isArray(res.payload) ? res.payload : []);
			});
		};
		fetchNodes();
		const unsub = onEvent("presence", () => {
			if (!cancelled) fetchNodes();
		});
		return () => {
			cancelled = true;
			unsub();
		};
	}, [currentKey]);

	const startRename = useCallback(() => {
		if (!canRename) return;
		setRenaming(true);
	}, [canRename]);

	// Populate, focus, and select the rename input synchronously after
	// render (useLayoutEffect) so there is no rAF race with Playwright
	// or other async interactions that could blur the input.
	useLayoutEffect(() => {
		if (renaming && inputRef.current) {
			inputRef.current.value = fullName;
			inputRef.current.focus();
			inputRef.current.select();
		}
	}, [renaming, fullName]);

	const commitRename = useCallback(() => {
		if (!inputRef.current) return;
		const val = inputRef.current.value.trim() || "";
		setRenaming(false);
		if (val && val !== fullName) {
			sendRpc("sessions.patch", { key: currentKey, label: val }).then((res) => {
				if (res?.ok) fetchSessions();
			});
		}
	}, [currentKey, fullName]);

	const onKeyDown = useCallback(
		(e: KeyboardEvent) => {
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

	const [generatingTitle, setGeneratingTitle] = useState(false);
	const onGenerateTitle = useCallback(() => {
		setGeneratingTitle(true);
		sendRpc<{ label?: string }>("sessions.generate_title", { key: currentKey })
			.then((res) => {
				if (res?.ok) fetchSessions();
			})
			.finally(() => setGeneratingTitle(false));
	}, [currentKey]);

	const onFork = useCallback(() => {
		sendRpc<{ sessionKey?: string }>("sessions.fork", { key: currentKey }).then((res) => {
			if (res?.ok && res.payload?.sessionKey) {
				fetchSessions();
				switchSession(res.payload.sessionKey);
			}
		});
	}, [currentKey]);

	const onDelete = useCallback(() => {
		if (typeof onBeforeDelete === "function") {
			onBeforeDelete();
		}
		const currentSession = sessionStore.getByKey(currentKey);
		const msgCount = currentSession ? currentSession.messageCount || 0 : 0;
		const nextKey = nextSessionKey(currentKey);
		const canOptimisticallyDelete = !currentSession?.worktree_branch;
		const applyDeletedState = (): void => {
			removeSessionFromClientState(currentKey, { nextKey });
			switchSession(nextKey);
		};
		const runDelete = (force: boolean): void => {
			const request: Record<string, unknown> = { key: currentKey };
			if (force) request.force = true;
			let optimisticApplied = false;
			if (canOptimisticallyDelete && !force) {
				applyDeletedState();
				optimisticApplied = true;
			}
			sendRpc("sessions.delete", request).then((res) => {
				const err = res?.error?.message || (typeof res?.error === "string" ? String(res.error) : "") || "";
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
		const isUnmodifiedFork = currentSession && currentSession.forkPoint != null && msgCount <= currentSession.forkPoint;
		if (msgCount > 0 && !isUnmodifiedFork) {
			confirmDialog("Delete this session?").then((yes) => {
				if (yes) runDelete(false);
			});
		} else {
			runDelete(false);
		}
	}, [currentKey, onBeforeDelete, sessionDataVersion]);

	const onClear = useCallback(() => {
		if (clearing) return;
		setClearing(true);
		clearActiveSession().finally(() => {
			setClearing(false);
		});
	}, [clearing]);

	const onStop = useCallback(() => {
		if (stopping) return;
		const params: Record<string, unknown> = { sessionKey: currentKey };
		if (activeRunId) params.runId = activeRunId;
		setStopping(true);
		sendRpc("chat.abort", params)
			.then((res) => {
				if (!res?.ok) {
					showToast((res?.error as { message?: string })?.message || "Failed to stop response", "error");
					return;
				}
				setSessionActiveRunId(currentKey, null);
				setSessionReplying(currentKey, false);
			})
			.finally(() => {
				setStopping(false);
			});
	}, [activeRunId, currentKey, stopping]);

	const shareSnapshot = useCallback(
		async (visibility: string) => {
			const res = await sendRpc<SharePayload>("sessions.share.create", { key: currentKey, visibility });
			if (!(res?.ok && res.payload?.path)) {
				showToast((res?.error as { message?: string })?.message || "Failed to create share link", "error");
				return;
			}

			const url = buildShareUrl(res.payload);
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

	const onShare = useCallback(() => {
		if (typeof onBeforeShare === "function") {
			onBeforeShare();
		}
		shareVisibilityDialog().then((visibility) => {
			if (!visibility) return;
			void shareSnapshot(visibility);
		});
	}, [onBeforeShare, shareSnapshot]);

	const onArchive = useCallback(() => {
		if (!(session && canArchive)) return;
		if (typeof onBeforeArchive === "function") {
			onBeforeArchive();
		}
		const nextArchived = !session.archived;
		sendRpc("sessions.patch", { key: currentKey, archived: nextArchived }).then((res) => {
			if (!res?.ok) {
				showToast((res?.error as { message?: string })?.message || "Failed to update archive state", "error");
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

	const onAgentChange = useCallback(
		(nextAgentId: string) => {
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
						showToast((res?.error as { message?: string })?.message || "Failed to switch agent", "error");
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

	const onNodeChange = useCallback(
		(nextNodeId: string) => {
			if (switchingNode) return;
			setSwitchingNode(true);
			sendRpc("nodes.set_session", {
				session_key: currentKey,
				node_id: nextNodeId || null,
			})
				.then((res) => {
					if (!res?.ok) {
						showToast((res?.error as { message?: string })?.message || "Failed to switch node", "error");
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

	const agentSelectValue = currentAgentId;
	const hasCurrentAgentOption = agentOptions.some((agent) => agent.id === agentSelectValue);
	let agentSelectOptions: SelectOption[] = agentOptions.map((agent) => {
		const prefix = agent.emoji ? `${agent.emoji} ` : "";
		const suffix = agent.id === defaultAgentId ? " (default)" : "";
		return {
			value: agent.id,
			label: `${prefix}${agent.name}${suffix}`,
		};
	});
	if (!hasCurrentAgentOption && agentSelectValue && (switchingAgent || agentOptionsLoaded)) {
		agentSelectOptions = [
			{
				value: agentSelectValue,
				label: switchingAgent ? "Switching\u2026" : `agent:${agentSelectValue}`,
			},
			...agentSelectOptions,
		];
	}
	const agentSelectDisabled = switchingAgent || agentSelectOptions.length === 0;
	const shouldShowAgentPicker = !isCron && agentOptionsLoaded && (agentOptions.length > 1 || !hasCurrentAgentOption);

	const shouldShowNodePicker = !isCron && (nodeOptions.length > 0 || Boolean(currentNodeId));
	const hasCurrentNodeOption = currentNodeId === "" || nodeOptions.some((node) => node.nodeId === currentNodeId);
	let nodeSelectOptions: SelectOption[] = [
		{ value: "", label: "Local" },
		...nodeOptions.map((node) => ({
			value: node.nodeId,
			label: nodeOptionLabel(node),
		})),
	];
	if (!hasCurrentNodeOption && currentNodeId) {
		const fallbackLabel = currentNodeId.startsWith("ssh:") ? `SSH: ${currentNodeId.slice(4)}` : `node:${currentNodeId}`;
		nodeSelectOptions = [
			{
				value: currentNodeId,
				label: switchingNode ? "Switching\u2026" : fallbackLabel,
			},
			...nodeSelectOptions,
		];
	}

	const nameStyle: Record<string, string> = { cursor: canRename ? "pointer" : "default" };
	if (nameOwnLine) {
		nameStyle.color = "var(--text-strong)";
		nameStyle.wordBreak = "break-word";
	}
	const renameInputStyle = nameOwnLine ? { maxWidth: "none", width: "100%" } : undefined;

	const nameControl =
		showName &&
		(renaming ? (
			<input
				ref={inputRef}
				className="chat-session-rename-input"
				style={renameInputStyle}
				onBlur={commitRename}
				onKeyDown={onKeyDown}
			/>
		) : (
			<span
				className="chat-session-name"
				style={nameStyle}
				title={canRename ? "Click to rename" : ""}
				onClick={startRename}
			>
				{displayName}
			</span>
		));

	const renameCta = showName && showRenameButton && canRename && !renaming && (
		<div className="flex items-center gap-1">
			<button className={actionButtonClass} onClick={startRename} title="Rename session">
				Rename
			</button>
			<button
				className={actionButtonClass}
				onClick={onGenerateTitle}
				disabled={generatingTitle}
				title="Auto-generate title from conversation"
			>
				{generatingTitle ? "..." : "Auto-title"}
			</button>
		</div>
	);

	return (
		<div className={nameOwnLine ? "flex flex-col gap-2 w-full" : "flex items-center gap-2"}>
			{nameOwnLine && showName && (
				<div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2 w-full">
					<div className="min-w-0">{nameControl}</div>
					<div className="justify-self-end">{renameCta}</div>
				</div>
			)}
			<div className={nameOwnLine ? "flex flex-wrap items-center gap-2" : "flex items-center gap-2"}>
				{showSelectors && shouldShowAgentPicker && (
					<ComboSelect
						options={agentSelectOptions}
						value={agentSelectValue}
						onChange={onAgentChange}
						placeholder="Session agent"
						searchable={false}
						allowEmpty={false}
						fullWidth={false}
						disabled={agentSelectDisabled}
					/>
				)}
				{showSelectors && shouldShowNodePicker && (
					<ComboSelect
						options={nodeSelectOptions}
						value={currentNodeId}
						onChange={onNodeChange}
						placeholder="Session node"
						searchable={false}
						allowEmpty={false}
						fullWidth={false}
						disabled={switchingNode}
					/>
				)}
				{!nameOwnLine && showName && nameControl}
				{!nameOwnLine && renameCta}
				{showArchive && canArchive && (
					<button
						className={actionButtonClass}
						onClick={onArchive}
						title={session?.archived ? "Unarchive session" : "Archive session"}
					>
						{session?.archived ? "Unarchive" : "Archive"}
					</button>
				)}
				{showFork && !isCron && (
					<button
						className={`${actionButtonClass} inline-flex items-center gap-1.5`}
						onClick={onFork}
						title="Fork session"
					>
						<span className="icon icon-sm icon-layers shrink-0" />
						Fork
					</button>
				)}
				{showShare && !isCron && (
					<button
						className={`${actionButtonClass} inline-flex items-center gap-1.5`}
						onClick={onShare}
						title="Share snapshot"
					>
						<span className="icon icon-sm icon-share shrink-0" />
						Share
					</button>
				)}
				{showDelete && !isMain && (
					<button
						className={`${actionButtonClass} chat-session-btn-danger inline-flex items-center gap-1.5`}
						onClick={onDelete}
						title="Delete session"
						style={{ background: "var(--error)", borderColor: "var(--error)", color: "#fff" }}
					>
						<span className="icon icon-sm icon-x-circle shrink-0" />
						Delete
					</button>
				)}
				{showStop && canStop && (
					<button className={actionButtonClass} onClick={onStop} title="Stop generation" disabled={stopping}>
						{stopping ? "Stopping\u2026" : "Stop"}
					</button>
				)}
				{showClear && isMain && (
					<button className={actionButtonClass} onClick={onClear} title="Clear session" disabled={clearing}>
						{clearing ? "Clearing\u2026" : "Clear"}
					</button>
				)}
			</div>
		</div>
	);
}
