// ── Channels page (Preact + Signals) ──────────────────────────

import type { Signal } from "@preact/signals";
import { computed, signal, useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import {
	channelStorageNote,
	fetchChannelStatus,
	matrixOwnershipModeGuidance,
	normalizeMatrixAuthMode,
	normalizeMatrixOwnershipMode,
} from "../channel-utils";
import { TabBar } from "../components/forms/Tabs";
import { onEvent } from "../events";
import { get as getGon } from "../gon";
import { sendRpc } from "../helpers";
import { updateNavCount } from "../nav-counts";
import { connected } from "../signals";
import * as S from "../state";
import { ConfirmDialog, requestConfirm, showToast } from "../ui";
import { AddDiscordModal } from "./channels/modals/AddDiscordModal";
import { AddMatrixModal } from "./channels/modals/AddMatrixModal";
import { AddNostrModal } from "./channels/modals/AddNostrModal";
import { AddSlackModal } from "./channels/modals/AddSlackModal";
import { AddTeamsModal } from "./channels/modals/AddTeamsModal";
// ── Sub-module imports (modals + shared fields) ──────────────
import { AddTelegramModal } from "./channels/modals/AddTelegramModal";
import { AddWhatsAppModal } from "./channels/modals/AddWhatsAppModal";
import { EditChannelModal } from "./channels/modals/EditChannelModal";

// ── Types ────────────────────────────────────────────────────

interface ChannelSession {
	key: string;
	label?: string;
	active?: boolean;
	messageCount?: number;
}

interface MatrixVerificationPrompt {
	other_user_id: string;
}

interface MatrixStatus {
	user_id?: string;
	device_id?: string;
	device_display_name?: string;
	ownership_mode?: string;
	auth_mode?: string;
	recovery_state?: string;
	device_verified_by_owner?: boolean;
	ownership_error?: string;
	cross_signing_complete?: boolean;
	verification_state?: string;
	pending_verifications?: MatrixVerificationPrompt[];
}

interface ChannelExtra {
	matrix?: MatrixStatus;
	qr_data?: string;
	qr_svg?: string;
}

/** Channel config fields (union of all channel types). */
export interface ChannelConfig {
	// Common
	token?: string;
	dm_policy?: string;
	mention_mode?: string;
	allowlist?: string[];
	model?: string;
	model_provider?: string;
	// Teams
	app_id?: string;
	app_password?: string;
	webhook_secret?: string;
	stream_mode?: string;
	reply_style?: string;
	welcome_card?: boolean;
	bot_name?: string;
	// Slack
	bot_token?: string;
	app_token?: string;
	connection_mode?: string;
	group_policy?: string;
	signing_secret?: string;
	channel_allowlist?: string[];
	// Matrix
	homeserver?: string;
	user_id?: string;
	password?: string | null;
	access_token?: string;
	device_id?: string;
	device_display_name?: string | null;
	ownership_mode?: string;
	room_policy?: string;
	auto_join?: string;
	user_allowlist?: string[];
	room_allowlist?: string[];
	otp_self_approval?: boolean;
	otp_cooldown_secs?: number;
	// Nostr
	secret_key?: string;
	relays?: string[];
	allowed_pubkeys?: string[];
	// Advanced config patch pass-through
	[key: string]: unknown;
}

export interface Channel {
	type: string;
	account_id: string;
	name?: string;
	details?: string;
	status?: string;
	config?: ChannelConfig;
	sessions?: ChannelSession[];
	extra?: ChannelExtra;
}

interface ChannelDescriptor {
	channel_type: string;
	capabilities: {
		inbound_mode: string;
	};
}

export interface TailscaleStatus {
	mode?: string;
	url?: string;
	installed?: boolean;
	tailscale_up?: boolean;
}

interface SenderEntry {
	peer_id: string;
	sender_name?: string;
	username?: string;
	message_count: number;
	last_seen?: number;
	allowed?: boolean;
	otp_pending?: { code: string };
}

interface ChannelEvent {
	kind: string;
	account_id?: string;
	channel_type?: string;
	qr_data?: string;
	qr_svg?: string;
	reason?: string;
}

// ── Module-level signals ─────────────────────────────────────

const channels: Signal<Channel[]> = signal([]);

export function prefetchChannels(): Promise<void> {
	return fetchChannelStatus().then((res: unknown) => {
		const r = res as { ok?: boolean; payload?: { channels?: Channel[] } } | undefined;
		if (r?.ok) {
			const ch = r.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
		}
	});
}

const senders: Signal<SenderEntry[]> = signal([]);
const activeTab: Signal<string> = signal("channels");
export const showAddTelegram: Signal<boolean> = signal(false);
export const showAddTeams: Signal<boolean> = signal(false);
export const showAddDiscord: Signal<boolean> = signal(false);
export const showAddWhatsApp: Signal<boolean> = signal(false);
export const showAddSlack: Signal<boolean> = signal(false);
export const showAddMatrix: Signal<boolean> = signal(false);
export const showAddNostr: Signal<boolean> = signal(false);
export const editingChannel: Signal<Channel | null> = signal(null);
const sendersAccount: Signal<string> = signal("");

// Track WhatsApp pairing state (updated by WebSocket events).
export const waQrData: Signal<string | null> = signal(null);
export const waQrSvg: Signal<string | null> = signal(null);
export const waPairingAccountId: Signal<string | null> = signal(null);
export const waPairingError: Signal<string | null> = signal(null);

// ── Helpers ──────────────────────────────────────────────────

export function channelType(type: string | undefined): string {
	return type || "telegram";
}

export function channelLabel(type: string | undefined): string {
	const t = channelType(type);
	if (t === "msteams") return "Microsoft Teams";
	if (t === "discord") return "Discord";
	if (t === "whatsapp") return "WhatsApp";
	if (t === "slack") return "Slack";
	if (t === "matrix") return "Matrix";
	if (t === "nostr") return "Nostr";
	return "Telegram";
}

function channelDescriptor(type: string | undefined): ChannelDescriptor | null {
	const descs = (getGon("channel_descriptors") || []) as ChannelDescriptor[];
	return descs.find((d) => d.channel_type === channelType(type)) || null;
}

const MODE_LABELS: Record<string, string> = {
	none: "Send only",
	polling: "Polling",
	gateway_loop: "Gateway",
	socket_mode: "Socket Mode",
	webhook: "Webhook",
};

const MODE_HINTS: Record<string, string> = {
	webhook: "Requires a publicly reachable URL. Configure your platform to send events to the endpoint shown below.",
	polling: "Connects automatically via long-polling. No public URL needed.",
	gateway_loop: "Maintains a persistent connection. No public URL needed.",
	socket_mode: "Connects via Socket Mode. No public URL needed.",
	none: "This channel is send-only and cannot receive inbound messages.",
};

// ── Small sub-components ─────────────────────────────────────

interface ConnectionModeHintProps {
	type: string;
}

export function ConnectionModeHint({ type }: ConnectionModeHintProps): VNode | null {
	const desc = channelDescriptor(type);
	if (!desc) return null;
	const hint = MODE_HINTS[desc.capabilities.inbound_mode];
	if (!hint) return null;
	return (
		<div className="text-xs text-[var(--muted)] mt-1 flex items-center gap-1">
			<span className="tier-badge">{MODE_LABELS[desc.capabilities.inbound_mode]}</span>
			<span>{hint}</span>
		</div>
	);
}

interface ChannelStorageNoticeProps {
	compact?: boolean;
}

function ChannelStorageNotice({ compact = false }: ChannelStorageNoticeProps): VNode {
	return (
		<div
			className={`rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)] ${compact ? "" : "max-w-3xl"}`}
		>
			<span className="font-medium text-[var(--text-strong)]">Storage note.</span> {channelStorageNote()}
		</div>
	);
}

function copyToClipboard(value: unknown, successMessage: string): void {
	const text = String(value || "").trim();
	if (!text) return;
	navigator.clipboard.writeText(text).then(() => showToast(successMessage));
}

// ── Matrix info row ──────────────────────────────────────────

interface MatrixInfoRowProps {
	label: string;
	value: unknown;
	copyLabel?: string | null;
}

function MatrixInfoRow({ label, value, copyLabel = null }: MatrixInfoRowProps): VNode {
	const text = String(value || "").trim();
	return (
		<div className="flex items-center justify-between gap-3">
			<div className="min-w-0">
				<div className="text-[11px] uppercase tracking-wide text-sky-700">{label}</div>
				<div className="truncate font-mono text-sky-900">{text || "\u2014"}</div>
			</div>
			{text && (
				<button
					type="button"
					className="provider-btn provider-btn-sm provider-btn-secondary"
					onClick={() => copyToClipboard(text, copyLabel || `${label} copied`)}
				>
					Copy
				</button>
			)}
		</div>
	);
}

// ── Matrix ownership card ────────────────────────────────────

interface MatrixOwnershipCardProps {
	channel: Channel;
	matrixStatus: MatrixStatus;
}

function MatrixOwnershipCard({ channel, matrixStatus }: MatrixOwnershipCardProps): VNode {
	const retryingOwnership = useSignal(false);
	const retryOwnershipError = useSignal("");
	const ownershipMode = normalizeMatrixOwnershipMode(matrixStatus?.ownership_mode);
	const authMode = normalizeMatrixAuthMode(matrixStatus?.auth_mode);
	const recoveryState = String(matrixStatus?.recovery_state || "unknown");
	const deviceVerified = !!matrixStatus?.device_verified_by_owner;
	const ownershipError = String(matrixStatus?.ownership_error || "").trim();
	const approvalMatch = ownershipError.match(/https?:\/\/\S+/);
	const ownershipIssue =
		ownershipMode !== "moltis_owned" || ownershipError.length === 0
			? "none"
			: ownershipError.includes("requires browser approval to reset cross-signing")
				? "approval_required"
				: ownershipError.includes("incomplete secret storage")
					? "incomplete_secret_storage"
					: "generic_blocked";
	const modeTitle =
		ownershipIssue === "approval_required"
			? "Ownership approval required"
			: ownershipIssue !== "none"
				? "Moltis ownership blocked"
				: ownershipMode === "moltis_owned"
					? "Managed by Moltis"
					: "User-managed in Element";
	const modeText =
		ownershipIssue === "approval_required"
			? "This existing Matrix account can already chat, but Matrix needs one browser approval before Moltis can take over encryption ownership. Open the approval page, approve the reset, then retry ownership setup."
			: ownershipIssue === "incomplete_secret_storage"
				? "This account already has partial Matrix secure-backup state. Finish or repair it in Element, or switch this channel to user-managed mode."
				: ownershipIssue === "generic_blocked"
					? "Moltis could not take ownership of this Matrix account automatically. Repair the account in Element or switch this channel to user-managed mode."
					: authMode === "password" || authMode === "oidc"
						? matrixOwnershipModeGuidance(authMode, ownershipMode)
						: "Access token auth is always user-managed. If you want encrypted Matrix chats, reconnect this channel with OIDC or password auth so Moltis can create its own device.";
	const detailTitle =
		ownershipIssue === "approval_required"
			? "Browser approval pending"
			: ownershipError
				? "Ownership setup needs attention"
				: "";
	const detailText =
		ownershipIssue === "approval_required"
			? `Approve the reset while signed into ${matrixStatus?.user_id || "this Matrix account"} in the browser, then use the retry button here so Moltis can finish taking ownership.`
			: ownershipError;
	const approvalUrl = approvalMatch ? approvalMatch[0].replace(/[;),.]+$/, "") : "";
	const verificationText = deviceVerified ? "Device verified by owner" : "Device not yet verified by owner";
	const hasAccountDetails =
		!!String(channel.config?.homeserver || "").trim() ||
		!!String(matrixStatus?.user_id || "").trim() ||
		!!String(matrixStatus?.device_id || "").trim() ||
		!!String(matrixStatus?.device_display_name || channel.config?.device_display_name || "").trim();

	function retryOwnershipSetup(): void {
		retryingOwnership.value = true;
		retryOwnershipError.value = "";
		sendRpc("channels.retry_ownership", {
			type: channelType(channel.type),
			account_id: channel.account_id,
		}).then((res) => {
			retryingOwnership.value = false;
			if (res?.ok) {
				showToast("Retrying Matrix ownership setup");
				loadChannels();
				return;
			}
			retryOwnershipError.value =
				(res?.error as { message?: string; detail?: string })?.message ||
				(res?.error as { detail?: string })?.detail ||
				"Failed to retry Matrix ownership setup.";
		});
	}

	return (
		<div className="rounded-md border border-sky-600/30 bg-sky-50 px-3 py-2 text-xs text-sky-900">
			<div className="flex items-center gap-2">
				<div className="font-medium text-sky-800">{modeTitle}</div>
				<span className={`provider-item-badge ${deviceVerified ? "configured" : "oauth"}`}>{verificationText}</span>
			</div>
			<div className="mt-1 text-sky-900">{modeText}</div>
			<div className="mt-2 text-sky-900">
				Cross-signing:{" "}
				<span className="font-medium">{matrixStatus?.cross_signing_complete ? "ready" : "not ready"}</span>. Recovery:{" "}
				<span className="font-medium">{recoveryState}</span>.
			</div>
			{hasAccountDetails && (
				<details className="mt-2 rounded-md border border-sky-600/20 bg-sky-100/50 px-3 py-2">
					<summary className="cursor-pointer text-[11px] font-medium uppercase tracking-wide text-sky-800">
						Matrix account details
					</summary>
					<div className="mt-2 grid gap-2">
						<MatrixInfoRow label="Homeserver" value={channel.config?.homeserver || ""} copyLabel="Homeserver copied" />
						<MatrixInfoRow label="Matrix user" value={matrixStatus?.user_id || ""} copyLabel="Matrix user ID copied" />
						<MatrixInfoRow
							label="Device ID"
							value={matrixStatus?.device_id || ""}
							copyLabel="Matrix device ID copied"
						/>
						<MatrixInfoRow
							label="Device name"
							value={matrixStatus?.device_display_name || channel.config?.device_display_name || ""}
							copyLabel="Matrix device name copied"
						/>
					</div>
				</details>
			)}
			{ownershipIssue === "approval_required" && approvalUrl && (
				<div className="mt-2">
					<div className="flex flex-wrap gap-2">
						<a
							href={approvalUrl}
							target="_blank"
							rel="noreferrer"
							className="provider-btn provider-btn-sm"
							aria-label={`Open approval page for ${matrixStatus?.user_id || "this Matrix account"}`}
						>
							Open approval page for {matrixStatus?.user_id || "this account"}
						</a>
						<button
							type="button"
							className="provider-btn provider-btn-sm"
							onClick={retryOwnershipSetup}
							disabled={retryingOwnership.value}
						>
							{retryingOwnership.value ? "Retrying ownership setup..." : "Click here once you reset the account"}
						</button>
					</div>
					<div className="mt-2 text-[11px] text-sky-800">
						Make sure the browser page is signed into{" "}
						<span className="font-mono text-sky-800">{matrixStatus?.user_id || "the Matrix bot account"}</span>.
					</div>
					{retryOwnershipError.value && (
						<div className="mt-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-amber-100">
							{retryOwnershipError.value}
						</div>
					)}
				</div>
			)}
			{detailTitle && (
				<div className="mt-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-amber-100">
					<div className="font-medium text-amber-50">{detailTitle}</div>
					<div className="mt-1">{detailText}</div>
				</div>
			)}
		</div>
	);
}

// ── Sender selection helpers ─────────────────────────────────

function senderSelectionKey(ch: Channel): string {
	return `${channelType(ch.type)}::${ch.account_id}`;
}

function parseSenderSelectionKey(key: string): { type: string; account_id: string } {
	const idx = key.indexOf("::");
	if (idx < 0) return { type: "telegram", account_id: key };
	return {
		type: key.slice(0, idx) || "telegram",
		account_id: key.slice(idx + 2),
	};
}

// ── Data loaders ─────────────────────────────────────────────

export function loadChannels(): void {
	fetchChannelStatus().then((res: unknown) => {
		const r = res as { ok?: boolean; payload?: { channels?: Channel[] } } | undefined;
		if (r?.ok) {
			const ch = r.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
			updateNavCount("channels", ch.length);
		}
	});
}

function loadSenders(): void {
	const selected = sendersAccount.value;
	if (!selected) {
		senders.value = [];
		return;
	}
	const parsed = parseSenderSelectionKey(selected);
	sendRpc<{ senders?: SenderEntry[] }>("channels.senders.list", {
		type: parsed.type,
		account_id: parsed.account_id,
	}).then((res) => {
		if (res?.ok) senders.value = (res.payload?.senders || []) as SenderEntry[];
	});
}

// ── Channel icon ─────────────────────────────────────────────

interface ChannelIconProps {
	type: string;
}

function ChannelIcon({ type }: ChannelIconProps): VNode {
	const t = channelType(type);
	if (t === "msteams") return <span className="icon icon-msteams" />;
	if (t === "discord") return <span className="icon icon-discord" />;
	if (t === "whatsapp") return <span className="icon icon-whatsapp" />;
	if (t === "slack") return <span className="icon icon-slack" />;
	if (t === "matrix") return <span className="icon icon-matrix" />;
	return <span className="icon icon-telegram" />;
}

// ── Channel card ─────────────────────────────────────────────

interface ChannelCardProps {
	channel: Channel;
}

function ChannelCard({ channel: ch }: ChannelCardProps): VNode {
	function onRemove(): void {
		requestConfirm(`Remove ${ch.name || ch.account_id}?`).then((yes) => {
			if (!yes) return;
			sendRpc("channels.remove", { type: channelType(ch.type), account_id: ch.account_id }).then((r) => {
				if (r?.ok) loadChannels();
			});
		});
	}

	const statusClass = ch.status === "connected" ? "configured" : "oauth";
	let sessionLine = "";
	if (ch.sessions && ch.sessions.length > 0) {
		const active = ch.sessions.filter((s) => s.active);
		sessionLine =
			active.length > 0
				? active.map((s) => `${s.label || s.key} (${s.messageCount} msgs)`).join(", ")
				: "No active session";
	}
	const desc = channelDescriptor(ch.type);
	const modeLabel = desc ? MODE_LABELS[desc.capabilities.inbound_mode] || desc.capabilities.inbound_mode : null;
	const matrixStatus = ch.extra?.matrix || null;
	const pendingVerifications = Array.isArray(matrixStatus?.pending_verifications)
		? matrixStatus.pending_verifications
		: [];
	const verificationStateLabel = matrixStatus?.verification_state || null;
	const showOwnershipCard =
		channelType(ch.type) === "matrix" &&
		!!(matrixStatus?.user_id || matrixStatus?.device_id || ch.config?.homeserver || matrixStatus?.ownership_error);

	return (
		<div className="provider-card p-3 rounded-lg mb-2">
			<div className="flex items-center gap-2.5">
				<span className="inline-flex items-center justify-center w-7 h-7 rounded-md bg-[var(--surface2)]">
					<ChannelIcon type={ch.type} />
				</span>
				<div className="flex flex-col gap-0.5">
					<span className="text-sm text-[var(--text-strong)]">{ch.name || ch.account_id || channelLabel(ch.type)}</span>
					{ch.details && <span className="text-xs text-[var(--muted)]">{ch.details}</span>}
					{sessionLine && <span className="text-xs text-[var(--muted)]">{sessionLine}</span>}
					{channelType(ch.type) === "matrix" && verificationStateLabel && (
						<span className="text-xs text-[var(--muted)]">Encryption device state: {verificationStateLabel}</span>
					)}
					{channelType(ch.type) === "telegram" && ch.account_id && (
						<a
							href={`https://t.me/${ch.account_id}`}
							target="_blank"
							className="text-xs text-[var(--accent)] underline"
						>
							t.me/{ch.account_id}
						</a>
					)}
				</div>
				<span className={`provider-item-badge ${statusClass}`}>{ch.status || "unknown"}</span>
				{modeLabel && <span className="tier-badge">{modeLabel}</span>}
			</div>
			{channelType(ch.type) === "matrix" && pendingVerifications.length > 0 && (
				<div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-100">
					<div className="font-medium text-sky-900">Verification pending</div>
					{pendingVerifications.map((prompt, i) => (
						<div key={i} className="mt-1">
							<div>With {prompt.other_user_id}</div>
							<div className="text-emerald-200/90">
								Send <span className="font-mono">verify yes</span>, <span className="font-mono">verify no</span>,{" "}
								<span className="font-mono">verify show</span>, or <span className="font-mono">verify cancel</span> as a
								normal message in that same Matrix chat.
							</div>
						</div>
					))}
				</div>
			)}
			{showOwnershipCard && matrixStatus && <MatrixOwnershipCard channel={ch} matrixStatus={matrixStatus} />}
			<div className="flex gap-2">
				<button
					className="provider-btn provider-btn-sm provider-btn-secondary"
					title={`Edit ${ch.account_id || "channel"}`}
					onClick={() => {
						editingChannel.value = ch;
					}}
				>
					Edit
				</button>
				<button
					className="provider-btn provider-btn-sm provider-btn-danger"
					title={`Remove ${ch.account_id || "channel"}`}
					onClick={onRemove}
				>
					Remove
				</button>
			</div>
		</div>
	);
}

// ── Connect channel buttons ──────────────────────────────────

function ConnectButtons(): VNode {
	const offered = new Set(
		(getGon("channels_offered") || ["telegram", "whatsapp", "discord", "slack", "matrix"]) as string[],
	);
	return (
		<div className="flex gap-2 flex-wrap">
			{offered.has("telegram") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddTelegram.value = true;
					}}
				>
					<span className="icon icon-telegram" /> Connect Telegram
				</button>
			)}
			{offered.has("msteams") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddTeams.value = true;
					}}
				>
					<span className="icon icon-msteams" /> Connect Microsoft Teams
				</button>
			)}
			{offered.has("discord") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddDiscord.value = true;
					}}
				>
					<span className="icon icon-discord" /> Connect Discord
				</button>
			)}
			{offered.has("slack") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddSlack.value = true;
					}}
				>
					<span className="icon icon-slack" /> Connect Slack
				</button>
			)}
			{offered.has("matrix") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddMatrix.value = true;
					}}
				>
					<span className="icon icon-matrix" /> Connect Matrix
				</button>
			)}
			{offered.has("whatsapp") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddWhatsApp.value = true;
					}}
				>
					<span className="icon icon-whatsapp" /> Connect WhatsApp
				</button>
			)}
			{offered.has("nostr") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => {
						if (connected.value) showAddNostr.value = true;
					}}
				>
					<span className="icon icon-nostr" /> Connect Nostr
				</button>
			)}
		</div>
	);
}

// ── Channels tab ─────────────────────────────────────────────

function ChannelsTab(): VNode {
	if (channels.value.length === 0) {
		return (
			<div className="text-center py-10">
				<div className="text-sm text-[var(--muted)] mb-4">No channels connected.</div>
				<div className="flex justify-center">
					<ConnectButtons />
				</div>
			</div>
		);
	}
	return (
		<>
			{channels.value.map((ch) => (
				<ChannelCard key={senderSelectionKey(ch)} channel={ch} />
			))}
		</>
	);
}

// ── Sender row renderer ──────────────────────────────────────

function renderSenderRow(s: SenderEntry, onAction: (identifier: string, action: string) => void): VNode {
	const identifier = s.username || s.peer_id;
	const lastSeenMs = s.last_seen ? s.last_seen * 1000 : 0;
	const usernameLabel = s.username ? (String(s.username).startsWith("@") ? s.username : `@${s.username}`) : "\u2014";
	const statusBadge = s.otp_pending ? (
		<span
			className="provider-item-badge cursor-pointer select-none"
			style={{ background: "var(--warning-bg, #fef3c7)", color: "var(--warning-text, #92400e)" }}
			onClick={() => {
				navigator.clipboard.writeText(s.otp_pending?.code ?? "").then(() => showToast("OTP code copied"));
			}}
		>
			OTP: <code className="text-xs">{s.otp_pending.code}</code>
		</span>
	) : (
		<span className={`provider-item-badge ${s.allowed ? "configured" : "oauth"}`}>
			{s.allowed ? "Allowed" : "Denied"}
		</span>
	);
	const actionBtn = s.allowed ? (
		<button className="provider-btn provider-btn-sm provider-btn-danger" onClick={() => onAction(identifier, "deny")}>
			Deny
		</button>
	) : (
		<button className="provider-btn provider-btn-sm" onClick={() => onAction(identifier, "approve")}>
			Approve
		</button>
	);
	return (
		<tr key={s.peer_id}>
			<td className="senders-td">{s.sender_name || s.peer_id}</td>
			<td className="senders-td" style={{ color: "var(--muted)" }}>
				{usernameLabel}
			</td>
			<td className="senders-td">{s.message_count}</td>
			<td className="senders-td" style={{ color: "var(--muted)", fontSize: "12px" }}>
				{lastSeenMs ? <time data-epoch-ms={String(lastSeenMs)}>{new Date(lastSeenMs).toISOString()}</time> : "\u2014"}
			</td>
			<td className="senders-td">{statusBadge}</td>
			<td className="senders-td">{actionBtn}</td>
		</tr>
	);
}

// ── Senders tab ──────────────────────────────────────────────

function SendersTab(): VNode {
	useEffect(() => {
		if (channels.value.length > 0 && !sendersAccount.value) {
			sendersAccount.value = senderSelectionKey(channels.value[0]);
		}
	}, [channels.value]);

	useEffect(() => {
		loadSenders();
	}, [sendersAccount.value]);

	if (channels.value.length === 0) {
		return <div className="text-sm text-[var(--muted)]">No channels configured.</div>;
	}

	function onAction(identifier: string, action: string): void {
		const rpc = action === "approve" ? "channels.senders.approve" : "channels.senders.deny";
		const parsed = parseSenderSelectionKey(sendersAccount.value);
		sendRpc(rpc, {
			type: parsed.type,
			account_id: parsed.account_id,
			identifier,
		}).then(() => {
			loadSenders();
			loadChannels();
		});
	}

	return (
		<div>
			<div style={{ marginBottom: "12px" }}>
				<label className="text-xs text-[var(--muted)]" style={{ marginRight: "6px" }}>
					Account:
				</label>
				<select
					style={{
						background: "var(--surface2)",
						color: "var(--text)",
						border: "1px solid var(--border)",
						borderRadius: "4px",
						padding: "4px 8px",
						fontSize: "12px",
					}}
					value={sendersAccount.value}
					onChange={(e) => {
						sendersAccount.value = (e.target as HTMLSelectElement).value;
					}}
				>
					{channels.value.map((ch) => (
						<option key={senderSelectionKey(ch)} value={senderSelectionKey(ch)}>
							{ch.name || ch.account_id}
						</option>
					))}
				</select>
			</div>
			{senders.value.length === 0 && (
				<div className="text-sm text-[var(--muted)] senders-empty">No messages received yet for this account.</div>
			)}
			{senders.value.length > 0 && (
				<table className="senders-table">
					<thead>
						<tr>
							<th className="senders-th">Sender</th>
							<th className="senders-th">Username</th>
							<th className="senders-th">Messages</th>
							<th className="senders-th">Last Seen</th>
							<th className="senders-th">Status</th>
							<th className="senders-th">Action</th>
						</tr>
					</thead>
					<tbody>{senders.value.map((s) => renderSenderRow(s, onAction))}</tbody>
				</table>
			)}
		</div>
	);
}

// ── Channel event handlers ───────────────────────────────────

function handleWhatsAppPairingEvent(p: ChannelEvent): void {
	if (p.kind === "pairing_qr_code" && p.account_id === waPairingAccountId.value) {
		waQrData.value = p.qr_data || null;
		waQrSvg.value = p.qr_svg || null;
	}
	if (p.kind === "pairing_complete" && p.account_id === waPairingAccountId.value) {
		showToast("WhatsApp connected!");
		showAddWhatsApp.value = false;
		waPairingAccountId.value = null;
		waQrData.value = null;
		waQrSvg.value = null;
		loadChannels();
	}
	if (p.kind === "pairing_failed" && p.account_id === waPairingAccountId.value) {
		waPairingError.value = p.reason || "Pairing failed";
	}
}

function handleChannelEvent(_payload: unknown): void {
	const p = _payload as ChannelEvent;
	if (p.kind === "otp_resolved") {
		loadChannels();
	}
	handleWhatsAppPairingEvent(p);
	if (p.kind === "pairing_complete" || p.kind === "account_disabled") {
		loadChannels();
	}
	const selected = parseSenderSelectionKey(sendersAccount.value || "");
	if (
		activeTab.value === "senders" &&
		selected.account_id === p.account_id &&
		selected.type === channelType(p.channel_type) &&
		(p.kind === "inbound_message" || p.kind === "otp_challenge" || p.kind === "otp_resolved")
	) {
		loadSenders();
	}
}

// ── Main page component ──────────────────────────────────────

function ChannelsPageComponent(): VNode {
	useEffect(() => {
		S.setRefreshChannelsPage(loadChannels);
		// Use prefetched cache for instant render
		if (S.cachedChannels !== null) channels.value = S.cachedChannels as Channel[];
		if (connected.value) loadChannels();

		const unsub = onEvent("channel", handleChannelEvent);
		S.setChannelEventUnsub(unsub);

		return () => {
			S.setRefreshChannelsPage(null);
			if (unsub) unsub();
			S.setChannelEventUnsub(null);
		};
	}, [connected.value]);

	const channelsTabs = computed(() => [
		{ id: "channels", label: "Channels", badge: channels.value.length || undefined },
		{ id: "senders", label: "Senders", badge: senders.value.length || undefined },
	]);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3 flex-wrap">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Channels</h2>
				{activeTab.value === "channels" && channels.value.length > 0 && <ConnectButtons />}
			</div>
			<TabBar
				tabs={channelsTabs.value}
				active={activeTab.value}
				onChange={(id) => {
					activeTab.value = id;
				}}
			/>
			{activeTab.value === "channels" && <ChannelStorageNotice />}
			{activeTab.value === "channels" ? <ChannelsTab /> : <SendersTab />}
			<AddTelegramModal />
			<AddTeamsModal />
			<AddDiscordModal />
			<AddSlackModal />
			<AddMatrixModal />
			<AddNostrModal />
			<AddWhatsAppModal />
			<EditChannelModal />
			<ConfirmDialog />
		</div>
	);
}

// ── Mount / unmount exports ──────────────────────────────────

let _channelsContainer: HTMLElement | null = null;

export function initChannels(container: HTMLElement): void {
	_channelsContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	activeTab.value = "channels";
	showAddTelegram.value = false;
	showAddTeams.value = false;
	showAddDiscord.value = false;
	showAddSlack.value = false;
	showAddMatrix.value = false;
	showAddNostr.value = false;
	showAddWhatsApp.value = false;
	editingChannel.value = null;
	sendersAccount.value = "";
	senders.value = [];
	render(<ChannelsPageComponent />, container);
}

export function teardownChannels(): void {
	S.setRefreshChannelsPage(null);
	if (S.channelEventUnsub) {
		S.channelEventUnsub();
		S.setChannelEventUnsub(null);
	}
	if (_channelsContainer) render(null, _channelsContainer);
	_channelsContainer = null;
}
