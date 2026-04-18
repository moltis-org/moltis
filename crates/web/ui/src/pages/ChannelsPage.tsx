// ── Channels page (Preact + Signals) ──────────────────────────

import { signal, useSignal } from "@preact/signals";
import type { Signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import {
	addChannel,
	buildTeamsEndpoint,
	channelStorageNote,
	defaultTeamsBaseUrl,
	deriveMatrixAccountId,
	fetchChannelStatus,
	generateWebhookSecretHex,
	MATRIX_DEFAULT_HOMESERVER,
	MATRIX_DOCS_URL,
	MATRIX_ENCRYPTION_GUIDANCE,
	matrixAuthModeGuidance,
	matrixCredentialLabel,
	matrixCredentialPlaceholder,
	matrixOwnershipModeGuidance,
	normalizeMatrixAuthMode,
	normalizeMatrixOtpCooldown,
	normalizeMatrixOwnershipMode,
	parseChannelConfigPatch,
	validateChannelFields,
} from "../channel-utils";
import { onEvent } from "../events";
import { get as getGon } from "../gon";
import { sendRpc } from "../helpers";
import { updateNavCount } from "../nav-counts";
import { connected } from "../signals";
import * as S from "../state";
import { models as modelsSig } from "../stores/model-store";
import { ConfirmDialog, Modal, ModelSelect, requestConfirm, showToast } from "../ui";

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
interface ChannelConfig {
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

interface Channel {
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

interface TailscaleStatus {
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
const showAddTelegram: Signal<boolean> = signal(false);
const showAddTeams: Signal<boolean> = signal(false);
const showAddDiscord: Signal<boolean> = signal(false);
const showAddWhatsApp: Signal<boolean> = signal(false);
const showAddSlack: Signal<boolean> = signal(false);
const showAddMatrix: Signal<boolean> = signal(false);
const showAddNostr: Signal<boolean> = signal(false);
const editingChannel: Signal<Channel | null> = signal(null);
const sendersAccount: Signal<string> = signal("");

// Track WhatsApp pairing state (updated by WebSocket events).
const waQrData: Signal<string | null> = signal(null);
const waQrSvg: Signal<string | null> = signal(null);
const waPairingAccountId: Signal<string | null> = signal(null);
const waPairingError: Signal<string | null> = signal(null);

// ── Helpers ──────────────────────────────────────────────────

function channelType(type: string | undefined): string {
	return type || "telegram";
}

function channelLabel(type: string | undefined): string {
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

function ConnectionModeHint({ type }: ConnectionModeHintProps): VNode | null {
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
		<div className={`rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)] ${compact ? "" : "max-w-3xl"}`}>
			<span className="font-medium text-[var(--text-strong)]">Storage note.</span> {channelStorageNote()}
		</div>
	);
}

function prettyConfigJson(value: unknown): string {
	try {
		return JSON.stringify(value || {}, null, 2);
	} catch (_error) {
		return "{}";
	}
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
				<div className="text-[11px] uppercase tracking-wide text-emerald-200/70">{label}</div>
				<div className="truncate font-mono text-emerald-50">{text || "\u2014"}</div>
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
					: authMode === "password"
						? matrixOwnershipModeGuidance(authMode, ownershipMode)
						: "Access token auth is always user-managed. If you want encrypted Matrix chats, reconnect this channel with password auth so Moltis can create its own device.";
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
				((res?.error as { message?: string; detail?: string })?.message ||
					(res?.error as { detail?: string })?.detail) ||
				"Failed to retry Matrix ownership setup.";
		});
	}

	return (
		<div className="rounded-md border border-sky-500/30 bg-sky-500/10 px-3 py-2 text-xs text-sky-100">
			<div className="flex items-center gap-2">
				<div className="font-medium text-sky-50">{modeTitle}</div>
				<span className={`provider-item-badge ${deviceVerified ? "configured" : "oauth"}`}>{verificationText}</span>
			</div>
			<div className="mt-1 text-sky-100/90">{modeText}</div>
			<div className="mt-2 text-sky-100/90">
				Cross-signing: <span className="font-medium">{matrixStatus?.cross_signing_complete ? "ready" : "not ready"}</span>.
				{" "}Recovery: <span className="font-medium">{recoveryState}</span>.
			</div>
			{hasAccountDetails && (
				<details className="mt-2 rounded-md border border-sky-500/20 bg-sky-500/5 px-3 py-2">
					<summary className="cursor-pointer text-[11px] font-medium uppercase tracking-wide text-sky-100/80">
						Matrix account details
					</summary>
					<div className="mt-2 grid gap-2">
						<MatrixInfoRow label="Homeserver" value={channel.config?.homeserver || ""} copyLabel="Homeserver copied" />
						<MatrixInfoRow label="Matrix user" value={matrixStatus?.user_id || ""} copyLabel="Matrix user ID copied" />
						<MatrixInfoRow label="Device ID" value={matrixStatus?.device_id || ""} copyLabel="Matrix device ID copied" />
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
					<div className="mt-2 text-[11px] text-sky-100/80">
						Make sure the browser page is signed into <span className="font-mono text-sky-50">{matrixStatus?.user_id || "the Matrix bot account"}</span>.
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

// ── Advanced config patch field ──────────────────────────────

interface AdvancedConfigPatchFieldProps {
	value: string;
	onInput: (value: string) => void;
	currentConfig?: ChannelConfig | null;
}

function AdvancedConfigPatchField({ value, onInput, currentConfig = null }: AdvancedConfigPatchFieldProps): VNode {
	return (
		<details className="channel-card">
			<summary className="cursor-pointer text-xs font-medium text-[var(--text-strong)]">Advanced Config JSON</summary>
			<div className="mt-2 flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]">
					Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not have dedicated fields yet.
				</div>
				{currentConfig && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Current stored config (read-only)</label>
						<textarea className="channel-input min-h-[160px] font-mono text-xs" readOnly value={prettyConfigJson(currentConfig)} />
					</div>
				)}
				<div className="flex flex-col gap-1">
					<label className="text-xs text-[var(--muted)]">Advanced config JSON patch (optional)</label>
					<textarea
						data-field="advancedConfigPatch"
						className="channel-input min-h-[140px] font-mono text-xs"
						value={value}
						onInput={(e) => {
							onInput((e.target as HTMLTextAreaElement).value);
						}}
						placeholder='{"reply_to_message": true}'
					></textarea>
				</div>
			</div>
		</details>
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

function loadChannels(): void {
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
	sendRpc<{ senders?: SenderEntry[] }>("channels.senders.list", { type: parsed.type, account_id: parsed.account_id }).then((res) => {
		if (res?.ok) senders.value = (res.payload?.senders || []) as SenderEntry[];
	});
}

// ── Channel icon ─────────────────────────────────────────────

interface ChannelIconProps {
	type: string;
}

function ChannelIcon({ type }: ChannelIconProps): VNode {
	const t = channelType(type);
	if (t === "msteams") return <span className="icon icon-msteams"></span>;
	if (t === "discord") return <span className="icon icon-discord"></span>;
	if (t === "whatsapp") return <span className="icon icon-whatsapp"></span>;
	if (t === "slack") return <span className="icon icon-slack"></span>;
	if (t === "matrix") return <span className="icon icon-matrix"></span>;
	return <span className="icon icon-telegram"></span>;
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
						<a href={`https://t.me/${ch.account_id}`} target="_blank" className="text-xs text-[var(--accent)] underline">t.me/{ch.account_id}</a>
					)}
				</div>
				<span className={`provider-item-badge ${statusClass}`}>{ch.status || "unknown"}</span>
				{modeLabel && <span className="tier-badge">{modeLabel}</span>}
			</div>
			{channelType(ch.type) === "matrix" && pendingVerifications.length > 0 && (
				<div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-100">
					<div className="font-medium text-emerald-50">Verification pending</div>
					{pendingVerifications.map((prompt, i) => (
						<div key={i} className="mt-1">
							<div>With {prompt.other_user_id}</div>
							<div className="text-emerald-200/90">
								Send <span className="font-mono">verify yes</span>, <span className="font-mono">verify no</span>, <span className="font-mono">verify show</span>, or <span className="font-mono">verify cancel</span> as a normal message in that same Matrix chat.
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
	const offered = new Set((getGon("channels_offered") || ["telegram", "whatsapp", "discord", "slack", "matrix"]) as string[]);
	return (
		<div className="flex gap-2 flex-wrap">
			{offered.has("telegram") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddTelegram.value = true; }}
				>
					<span className="icon icon-telegram"></span> Connect Telegram
				</button>
			)}
			{offered.has("msteams") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddTeams.value = true; }}
				>
					<span className="icon icon-msteams"></span> Connect Microsoft Teams
				</button>
			)}
			{offered.has("discord") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddDiscord.value = true; }}
				>
					<span className="icon icon-discord"></span> Connect Discord
				</button>
			)}
			{offered.has("slack") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddSlack.value = true; }}
				>
					<span className="icon icon-slack"></span> Connect Slack
				</button>
			)}
			{offered.has("matrix") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddMatrix.value = true; }}
				>
					<span className="icon icon-matrix"></span> Connect Matrix
				</button>
			)}
			{offered.has("whatsapp") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddWhatsApp.value = true; }}
				>
					<span className="icon icon-whatsapp"></span> Connect WhatsApp
				</button>
			)}
			{offered.has("nostr") && (
				<button
					className="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
					onClick={() => { if (connected.value) showAddNostr.value = true; }}
				>
					<span className="icon icon-nostr"></span> Connect Nostr
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
				<div className="flex justify-center"><ConnectButtons /></div>
			</div>
		);
	}
	return <>{channels.value.map((ch) => <ChannelCard key={senderSelectionKey(ch)} channel={ch} />)}</>;
}

// ── Sender row renderer ──────────────────────────────────────

function renderSenderRow(s: SenderEntry, onAction: (identifier: string, action: string) => void): VNode {
	const identifier = s.username || s.peer_id;
	const lastSeenMs = s.last_seen ? s.last_seen * 1000 : 0;
	const usernameLabel = s.username ? (String(s.username).startsWith("@") ? s.username : `@${s.username}`) : "\u2014";
	const statusBadge = s.otp_pending
		? (
			<span
				className="provider-item-badge cursor-pointer select-none"
				style={{ background: "var(--warning-bg, #fef3c7)", color: "var(--warning-text, #92400e)" }}
				onClick={() => {
					navigator.clipboard.writeText(s.otp_pending!.code).then(() => showToast("OTP code copied"));
				}}
			>
				OTP: <code className="text-xs">{s.otp_pending.code}</code>
			</span>
		)
		: <span className={`provider-item-badge ${s.allowed ? "configured" : "oauth"}`}>{s.allowed ? "Allowed" : "Denied"}</span>;
	const actionBtn = s.allowed
		? <button className="provider-btn provider-btn-sm provider-btn-danger" onClick={() => onAction(identifier, "deny")}>Deny</button>
		: <button className="provider-btn provider-btn-sm" onClick={() => onAction(identifier, "approve")}>Approve</button>;
	return (
		<tr key={s.peer_id}>
			<td className="senders-td">{s.sender_name || s.peer_id}</td>
			<td className="senders-td" style={{ color: "var(--muted)" }}>{usernameLabel}</td>
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
				<label className="text-xs text-[var(--muted)]" style={{ marginRight: "6px" }}>Account:</label>
				<select
					style={{ background: "var(--surface2)", color: "var(--text)", border: "1px solid var(--border)", borderRadius: "4px", padding: "4px 8px", fontSize: "12px" }}
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
					<tbody>
						{senders.value.map((s) => renderSenderRow(s, onAction))}
					</tbody>
				</table>
			)}
		</div>
	);
}

// ── Tag-style allowlist input ────────────────────────────────

interface AllowlistInputProps {
	value: string[];
	onChange: (items: string[]) => void;
	preserveAt?: boolean;
}

function AllowlistInput({ value, onChange, preserveAt }: AllowlistInputProps): VNode {
	const input = useSignal("");

	function addTag(raw: string): void {
		const tag = preserveAt ? raw.trim() : raw.trim().replace(/^@/, "");
		if (tag && !value.includes(tag)) onChange([...value, tag]);
		input.value = "";
	}

	function removeTag(tag: string): void {
		onChange(value.filter((t) => t !== tag));
	}

	function onKeyDown(e: KeyboardEvent): void {
		if ((e.key === "Enter" || e.key === ",") && !e.isComposing) {
			e.preventDefault();
			if (input.value.trim()) addTag(input.value);
		} else if (e.key === "Backspace" && !input.value && value.length > 0) {
			onChange(value.slice(0, -1));
		}
	}

	return (
		<div
			className="flex flex-wrap items-center gap-1.5 rounded border border-[var(--border)] bg-[var(--surface2)] px-2 py-1.5"
			style={{ minHeight: "38px", cursor: "text" }}
			onClick={(e) => (e.currentTarget as HTMLElement).querySelector("input")?.focus()}
		>
			{value.map((tag) => (
				<span
					key={tag}
					className="inline-flex items-center gap-1 rounded-full bg-[var(--accent)]/10 px-2 py-0.5 text-xs text-[var(--accent)]"
				>
					{tag}
					<button
						type="button"
						className="inline-flex items-center text-[var(--muted)] hover:text-[var(--accent)]"
						style={{ lineHeight: 1, fontSize: "14px", padding: 0, background: "none", border: "none", cursor: "pointer" }}
						onClick={(e) => {
							e.stopPropagation();
							removeTag(tag);
						}}
					>
						{"\u00d7"}
					</button>
				</span>
			))}
			<input
				type="text"
				value={input.value}
				onInput={(e) => {
					input.value = (e.target as HTMLInputElement).value;
				}}
				onKeyDown={onKeyDown}
				placeholder={value.length === 0 ? "Type a username and press Enter" : ""}
				className="flex-1 bg-transparent text-[var(--text)] text-sm outline-none border-none"
				style={{ minWidth: "80px", padding: "2px 0", fontFamily: "var(--font-body)" }}
			/>
		</div>
	);
}

// ── Shared form fields ───────────────────────────────────────

interface SharedChannelFieldsProps {
	addModel: Signal<string>;
	allowlistItems: Signal<string[]>;
}

function SharedChannelFields({ addModel, allowlistItems }: SharedChannelFieldsProps): VNode {
	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<>
			<label className="text-xs text-[var(--muted)]">DM Policy</label>
			<select data-field="dmPolicy" className="channel-select">
				<option value="allowlist">Allowlist only</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
			<label className="text-xs text-[var(--muted)]">Group Mention Mode</label>
			<select data-field="mentionMode" className="channel-select">
				<option value="mention">Must @mention bot</option>
				<option value="always">Always respond</option>
				<option value="none">Don't respond in groups</option>
			</select>
			<label className="text-xs text-[var(--muted)]">Default Model</label>
			<ModelSelect
				models={modelsSig.value}
				value={addModel.value}
				onChange={(v: string) => {
					addModel.value = v;
				}}
				placeholder={defaultPlaceholder}
			/>
			<label className="text-xs text-[var(--muted)]">DM Allowlist</label>
			<AllowlistInput
				value={allowlistItems.value}
				onChange={(v) => {
					allowlistItems.value = v;
				}}
			/>
		</>
	);
}

// ── Add Telegram modal ───────────────────────────────────────

function AddTelegramModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const credential = (form.querySelector("[data-field=credential]") as HTMLInputElement).value.trim();
		const v = validateChannelFields("telegram", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const addConfig: ChannelConfig = {
			token: credential,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("telegram", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddTelegram.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal show={showAddTelegram.value} onClose={() => { showAddTelegram.value = false; }} title="Connect Telegram">
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">1. Open <a href="https://t.me/BotFather" target="_blank" className="text-[var(--accent)] underline">@BotFather</a> in Telegram</div>
						<div className="text-xs text-[var(--muted)]">2. Send /newbot and follow the prompts to choose a name and username</div>
						<div className="text-xs text-[var(--muted)]">3. Copy the bot token and paste it below</div>
					</div>
				</div>
				<ConnectionModeHint type="telegram" />
				<label className="text-xs text-[var(--muted)]">Bot username</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my_assistant_bot"
					value={accountDraft.value}
					onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Bot Token (from @BotFather)</label>
				<input
					data-field="credential"
					type="password"
					placeholder="123456:ABC-DEF..."
					className="channel-input"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="telegram_bot_token"
				/>
				{accountDraft.value.trim() && (
					<div className="flex items-center gap-1.5 text-xs py-1">
						<span className="text-[var(--muted)]">Chat with your bot:</span>
						<a href={`https://t.me/${accountDraft.value.trim()}`} target="_blank" className="text-[var(--accent)] underline">t.me/{accountDraft.value.trim()}</a>
					</div>
				)}
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Telegram"}
				</button>
			</div>
		</Modal>
	);
}

// ── Add Microsoft Teams modal ────────────────────────────────

function AddTeamsModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const webhookSecret = useSignal("");
	const baseUrlDraft = useSignal(defaultTeamsBaseUrl());
	const bootstrapEndpoint = useSignal("");
	const tsStatus = useSignal<TailscaleStatus | null>(null);
	const tsLoading = useSignal(true);
	const enablingFunnel = useSignal(false);
	const advancedConfigPatch = useSignal("");

	// Fetch Tailscale status on mount.
	useEffect(() => {
		fetch("/api/tailscale/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((data: TailscaleStatus | null) => {
				tsStatus.value = data;
				tsLoading.value = false;
				if (data?.mode === "funnel" && data?.url) {
					baseUrlDraft.value = data.url.replace(/\/$/, "");
				}
			})
			.catch(() => {
				tsLoading.value = false;
			});
	}, []);

	function onEnableFunnel(): void {
		enablingFunnel.value = true;
		error.value = "";
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode: "funnel" }),
		})
			.then((r) => r.json())
			.then((data: TailscaleStatus & { ok?: boolean; error?: string }) => {
				enablingFunnel.value = false;
				if (data?.ok !== false && data?.url) {
					baseUrlDraft.value = data.url.replace(/\/$/, "");
					tsStatus.value = data;
					refreshBootstrapEndpoint();
				} else {
					error.value = data?.error || "Failed to enable Tailscale Funnel.";
				}
			})
			.catch((e: Error) => {
				enablingFunnel.value = false;
				error.value = `Tailscale error: ${e.message}`;
			});
	}

	function refreshBootstrapEndpoint(): void {
		if (!bootstrapEndpoint.value) return;
		bootstrapEndpoint.value = buildTeamsEndpoint(baseUrlDraft.value, accountDraft.value, webhookSecret.value);
	}

	function onBootstrapTeams(): void {
		const accountId = accountDraft.value.trim();
		if (!accountId) {
			error.value = "Enter App ID / Account ID first.";
			return;
		}
		let secret = webhookSecret.value.trim();
		if (!secret) {
			secret = generateWebhookSecretHex();
			webhookSecret.value = secret;
		}
		const endpoint = buildTeamsEndpoint(baseUrlDraft.value, accountId, secret);
		if (!endpoint) {
			error.value = "Enter a valid public base URL (example: https://bot.example.com).";
			return;
		}
		bootstrapEndpoint.value = endpoint;
		error.value = "";
		showToast("Teams endpoint generated");
	}

	function copyBootstrapEndpoint(): void {
		if (!bootstrapEndpoint.value) return;
		if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
			showToast("Clipboard is unavailable");
			return;
		}
		navigator.clipboard.writeText(bootstrapEndpoint.value).then(() => {
			showToast("Messaging endpoint copied");
		});
	}

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const credential = (form.querySelector("[data-field=credential]") as HTMLInputElement).value.trim();
		const v = validateChannelFields("msteams", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const addConfig: ChannelConfig = {
			app_id: accountId,
			app_password: credential,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			allowlist: allowlistItems.value,
		};
		if (webhookSecret.value.trim()) addConfig.webhook_secret = webhookSecret.value.trim();
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("msteams", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddTeams.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				webhookSecret.value = "";
				baseUrlDraft.value = defaultTeamsBaseUrl();
				bootstrapEndpoint.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal show={showAddTeams.value} onClose={() => { showAddTeams.value = false; }} title="Connect Microsoft Teams">
			<div className="channel-form">
				{!(tsLoading.value || (tsStatus.value?.mode === "funnel" && tsStatus.value?.url)) && (
					<div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs flex flex-col gap-2">
						<span className="font-medium text-[var(--text-strong)]">Public URL required</span>
						<span className="text-[var(--muted)]">Teams sends messages to your server via webhook. Your Moltis instance must be reachable over HTTPS.</span>
						{tsStatus.value?.installed && tsStatus.value?.tailscale_up ? (
							<div className="flex flex-col gap-2">
								<span className="text-[var(--muted)]">Tailscale is connected. Enable <strong>Funnel</strong> to make it publicly reachable:</span>
								<button type="button" className="provider-btn provider-btn-sm" onClick={onEnableFunnel} disabled={enablingFunnel.value}>
									{enablingFunnel.value ? "Enabling\u2026" : "Enable Tailscale Funnel"}
								</button>
							</div>
						) : (
							<span className="text-[var(--muted)]">Enable <strong>Tailscale Funnel</strong> in Settings, or use <a href="https://ngrok.com/" target="_blank" className="text-[var(--accent)] underline">ngrok</a> / <a href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/" target="_blank" className="text-[var(--accent)] underline">Cloudflare Tunnel</a>.</span>
						)}
					</div>
				)}
				{tsStatus.value?.mode === "funnel" && tsStatus.value?.url && (
					<div className="rounded-md border border-green-500/30 bg-green-500/5 p-3 text-xs flex items-center gap-2">
						<span className="text-green-600">{"\u2713"}</span>
						<span className="text-[var(--muted)]">Tailscale Funnel active &mdash; publicly reachable at <strong>{tsStatus.value.url}</strong></span>
					</div>
				)}
				<div className="channel-card">
					<div className="flex flex-col gap-1">
						<span className="text-xs font-medium text-[var(--text-strong)]">How to create a Teams bot</span>
						<span className="text-xs font-medium text-[var(--text-strong)] opacity-70" style={{ fontSize: "10px" }}>Option A: Teams Developer Portal (easiest)</span>
						<div className="text-xs text-[var(--muted)]">1. Open <a href="https://dev.teams.microsoft.com/bots" target="_blank" className="text-[var(--accent)] underline">Teams Developer Portal &rarr; Bot Management</a></div>
						<div className="text-xs text-[var(--muted)]">2. Click <strong>+ New Bot</strong>, give it a name, copy the <strong>Bot ID</strong> (App ID)</div>
						<div className="text-xs text-[var(--muted)]">3. Under <strong>Client secrets</strong>, add a secret and copy the value (App Password)</div>
						<span className="text-xs font-medium text-[var(--text-strong)] opacity-70" style={{ fontSize: "10px", marginTop: "4px" }}>Option B: Azure Portal</span>
						<div className="text-xs text-[var(--muted)]">1. <a href="https://portal.azure.com/#create/Microsoft.AzureBot" target="_blank" className="text-[var(--accent)] underline">Create an Azure Bot</a>, then find App ID in Configuration</div>
						<div className="text-xs text-[var(--muted)]">2. Click <strong>Manage Password</strong> &rarr; <strong>New client secret</strong> for the App Password</div>
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "4px" }}>Then generate the endpoint below and paste it as the <strong>Messaging endpoint</strong> in your bot settings. <a href="https://docs.moltis.org/teams.html" target="_blank" className="text-[var(--accent)] underline">Full guide &rarr;</a></div>
					</div>
				</div>
				<ConnectionModeHint type="msteams" />
				<label className="text-xs text-[var(--muted)]">App ID (Bot ID from Azure)</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. 12345678-abcd-efgh-ijkl-000000000000"
					value={accountDraft.value}
					onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; refreshBootstrapEndpoint(); }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">App Password (client secret from Azure)</label>
				<input
					data-field="credential"
					type="password"
					placeholder="Client secret value"
					className="channel-input"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="teams_app_password"
				/>
				<div>
					<label className="text-xs text-[var(--muted)]">Webhook Secret <span className="opacity-60">(optional &mdash; auto-generated if blank)</span></label>
					<input
						type="text"
						placeholder="Leave blank to auto-generate"
						className="channel-input"
						value={webhookSecret.value}
						onInput={(e) => { webhookSecret.value = (e.target as HTMLInputElement).value; refreshBootstrapEndpoint(); }}
					/>
					<label className="text-xs text-[var(--muted)] mt-2">Public Base URL <span className="opacity-60">(your server's HTTPS address)</span></label>
					<input
						type="text"
						placeholder="https://bot.example.com"
						className="channel-input"
						value={baseUrlDraft.value}
						onInput={(e) => { baseUrlDraft.value = (e.target as HTMLInputElement).value; refreshBootstrapEndpoint(); }}
					/>
					<div className="flex gap-2 mt-2">
						<button type="button" className="provider-btn provider-btn-sm provider-btn-secondary" onClick={onBootstrapTeams}>
							Bootstrap Teams
						</button>
						{bootstrapEndpoint.value && (
							<button type="button" className="provider-btn provider-btn-sm provider-btn-secondary" onClick={copyBootstrapEndpoint}>
								Copy Endpoint
							</button>
						)}
					</div>
					{bootstrapEndpoint.value && (
						<div className="mt-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2">
							<div className="text-xs text-[var(--muted)] mb-1">Messaging endpoint &mdash; paste this into your bot's configuration:</div>
							<code className="text-xs block break-all select-all">{bootstrapEndpoint.value}</code>
						</div>
					)}
					<div className="text-[10px] text-[var(--muted)] mt-1 opacity-70">
						Teams requires HTTPS. For local dev, use <a href="https://ngrok.com/" target="_blank" className="text-[var(--accent)] underline">ngrok</a> or <a href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/" target="_blank" className="text-[var(--accent)] underline">Cloudflare Tunnel</a>.
					</div>
				</div>
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Microsoft Teams"}
				</button>
			</div>
		</Modal>
	);
}

// ── Discord invite URL helper ────────────────────────────────

function discordInviteUrl(token: string): string {
	if (!token) return "";
	const parts = token.split(".");
	if (parts.length < 3) return "";
	try {
		const id = atob(parts[0]);
		if (!/^\d+$/.test(id)) return "";
		return `https://discord.com/oauth2/authorize?client_id=${id}&scope=bot&permissions=100352`;
	} catch {
		return "";
	}
}

// ── Add Discord modal ────────────────────────────────────────

function AddDiscordModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const tokenDraft = useSignal("");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const credential = tokenDraft.value.trim();
		const v = validateChannelFields("discord", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const addConfig: ChannelConfig = {
			token: credential,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("discord", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddDiscord.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				tokenDraft.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect channel.";
			}
		});
	}

	const inviteUrl = discordInviteUrl(tokenDraft.value);

	return (
		<Modal show={showAddDiscord.value} onClose={() => { showAddDiscord.value = false; }} title="Connect Discord">
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up a Discord bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">1. Go to the <a href="https://discord.com/developers/applications" target="_blank" className="text-[var(--accent)] underline">Discord Developer Portal</a></div>
						<div className="text-xs text-[var(--muted)]">2. Create a new Application &rarr; Bot tab &rarr; copy the bot token</div>
						<div className="text-xs text-[var(--muted)]">3. Enable "Message Content Intent" under Privileged Gateway Intents</div>
						<div className="text-xs text-[var(--muted)]">4. Paste the token below &mdash; an invite link will be generated automatically</div>
						<div className="text-xs text-[var(--muted)]">5. You can also DM the bot directly without adding it to a server</div>
					</div>
				</div>
				<ConnectionModeHint type="discord" />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-discord-bot"
					value={accountDraft.value}
					onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Bot Token</label>
				<input
					data-field="credential"
					type="password"
					placeholder="Discord bot token"
					className="channel-input"
					value={tokenDraft.value}
					onInput={(e) => { tokenDraft.value = (e.target as HTMLInputElement).value; }}
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="discord_bot_token"
				/>
				{inviteUrl && (
					<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 flex flex-col gap-1">
						<span className="text-xs font-medium text-[var(--text-strong)]">Invite bot to a server</span>
						<span className="text-xs text-[var(--muted)]">Open this link to add the bot (Send Messages, Attach Files, Read Message History):</span>
						<a href={inviteUrl} target="_blank" className="text-xs text-[var(--accent)] underline break-all">{inviteUrl}</a>
					</div>
				)}
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Discord"}
				</button>
			</div>
		</Modal>
	);
}

// ── Add Slack modal ──────────────────────────────────────────

function AddSlackModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const channelAllowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const botTokenDraft = useSignal("");
	const appTokenDraft = useSignal("");
	const connectionMode = useSignal("socket_mode");
	const signingSecretDraft = useSignal("");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const botToken = botTokenDraft.value.trim();
		if (!accountId) {
			error.value = "Account ID is required.";
			return;
		}
		if (!botToken) {
			error.value = "Bot Token is required.";
			return;
		}
		if (connectionMode.value === "socket_mode" && !appTokenDraft.value.trim()) {
			error.value = "App Token is required for Socket Mode.";
			return;
		}
		if (connectionMode.value === "events_api" && !signingSecretDraft.value.trim()) {
			error.value = "Signing Secret is required for Events API mode.";
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const addConfig: ChannelConfig = {
			bot_token: botToken,
			app_token: appTokenDraft.value.trim(),
			connection_mode: connectionMode.value,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			group_policy: (form.querySelector("[data-field=groupPolicy]") as HTMLSelectElement)?.value || "open",
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			allowlist: allowlistItems.value,
			channel_allowlist: channelAllowlistItems.value,
		};
		if (connectionMode.value === "events_api") {
			addConfig.signing_secret = signingSecretDraft.value.trim();
		}
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("slack", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddSlack.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				channelAllowlistItems.value = [];
				accountDraft.value = "";
				botTokenDraft.value = "";
				appTokenDraft.value = "";
				signingSecretDraft.value = "";
				connectionMode.value = "socket_mode";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect Slack.";
			}
		});
	}

	return (
		<Modal show={showAddSlack.value} onClose={() => { showAddSlack.value = false; }} title="Connect Slack">
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up a Slack bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">1. Go to <a href="https://api.slack.com/apps" target="_blank" className="text-[var(--accent)] underline">api.slack.com/apps</a> and create a new app</div>
						<div className="text-xs text-[var(--muted)]">2. Under OAuth & Permissions, add bot scopes: <code className="text-[var(--accent)]">chat:write</code>, <code className="text-[var(--accent)]">channels:history</code>, <code className="text-[var(--accent)]">im:history</code>, <code className="text-[var(--accent)]">app_mentions:read</code></div>
						<div className="text-xs text-[var(--muted)]">3. Install the app to your workspace and copy the Bot User OAuth Token</div>
						<div className="text-xs text-[var(--muted)]">4. For Socket Mode: enable Socket Mode and generate an App-Level Token with <code className="text-[var(--accent)]">connections:write</code> scope</div>
						<div className="text-xs text-[var(--muted)]">5. For Events API: set the Request URL to your server's webhook endpoint</div>
					</div>
				</div>
				<ConnectionModeHint type="slack" />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-slack-bot"
					value={accountDraft.value}
					onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Bot Token (xoxb-...)</label>
				<input
					data-field="botToken"
					type="password"
					placeholder="xoxb-..."
					className="channel-input"
					value={botTokenDraft.value}
					onInput={(e) => { botTokenDraft.value = (e.target as HTMLInputElement).value; }}
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
				/>
				<label className="text-xs text-[var(--muted)]">Connection Mode</label>
				<select
					data-field="connectionMode"
					className="channel-select"
					value={connectionMode.value}
					onChange={(e) => { connectionMode.value = (e.target as HTMLSelectElement).value; }}
				>
					<option value="socket_mode">Socket Mode (recommended)</option>
					<option value="events_api">Events API (HTTP webhook)</option>
				</select>
				{connectionMode.value === "socket_mode" && (
					<>
						<label className="text-xs text-[var(--muted)]">App Token (xapp-...)</label>
						<input
							data-field="appToken"
							type="password"
							placeholder="xapp-..."
							className="channel-input"
							value={appTokenDraft.value}
							onInput={(e) => { appTokenDraft.value = (e.target as HTMLInputElement).value; }}
							autoComplete="new-password"
							autoCapitalize="none"
							autoCorrect="off"
							spellcheck={false}
						/>
					</>
				)}
				{connectionMode.value === "events_api" && (
					<>
						<label className="text-xs text-[var(--muted)]">Signing Secret</label>
						<input
							data-field="signingSecret"
							type="password"
							placeholder="Signing secret from Basic Information"
							className="channel-input"
							value={signingSecretDraft.value}
							onInput={(e) => { signingSecretDraft.value = (e.target as HTMLInputElement).value; }}
							autoComplete="new-password"
							autoCapitalize="none"
							autoCorrect="off"
							spellcheck={false}
						/>
					</>
				)}
				<label className="text-xs text-[var(--muted)]">Group/Channel Policy</label>
				<select data-field="groupPolicy" className="channel-select">
					<option value="open">Open (respond in any channel)</option>
					<option value="allowlist">Channel allowlist only</option>
					<option value="disabled">Disabled (no channel messages)</option>
				</select>
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<label className="text-xs text-[var(--muted)]">Channel Allowlist (Slack channel IDs)</label>
				<AllowlistInput
					value={channelAllowlistItems.value}
					onChange={(items) => { channelAllowlistItems.value = items; }}
				/>
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Slack"}
				</button>
			</div>
		</Modal>
	);
}

// ── Add Matrix modal ─────────────────────────────────────────

function AddMatrixModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const userAllowlistItems = useSignal<string[]>([]);
	const roomAllowlistItems = useSignal<string[]>([]);
	const homeserverDraft = useSignal(MATRIX_DEFAULT_HOMESERVER);
	const authModeDraft = useSignal("password");
	const userIdDraft = useSignal("");
	const credentialDraft = useSignal("");
	const deviceDisplayNameDraft = useSignal("");
	const ownershipModeDraft = useSignal("moltis_owned");
	const otpSelfApprovalDraft = useSignal(true);
	const otpCooldownDraft = useSignal("300");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const authMode = normalizeMatrixAuthMode(authModeDraft.value);
		const credential = credentialDraft.value.trim();
		const homeserver = homeserverDraft.value.trim();
		const userId = userIdDraft.value.trim();
		const accountId = deriveMatrixAccountId({ userId, homeserver });
		const v = validateChannelFields("matrix", accountId, credential, {
			matrixAuthMode: authMode,
			matrixUserId: userId,
		});
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		if (!homeserver) {
			error.value = "Homeserver URL is required.";
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const addConfig: ChannelConfig = {
			homeserver,
			ownership_mode: authMode === "password" ? normalizeMatrixOwnershipMode(ownershipModeDraft.value) : "user_managed",
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			room_policy: (form.querySelector("[data-field=roomPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			auto_join: (form.querySelector("[data-field=autoJoin]") as HTMLSelectElement).value,
			user_allowlist: userAllowlistItems.value,
			room_allowlist: roomAllowlistItems.value,
			otp_self_approval: otpSelfApprovalDraft.value,
			otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldownDraft.value),
		};
		if (authMode === "password") {
			addConfig.password = credential;
		} else {
			addConfig.access_token = credential;
		}
		if (userId) addConfig.user_id = userId;
		if (deviceDisplayNameDraft.value.trim()) addConfig.device_display_name = deviceDisplayNameDraft.value.trim();
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("matrix", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddMatrix.value = false;
				addModel.value = "";
				userAllowlistItems.value = [];
				roomAllowlistItems.value = [];
				homeserverDraft.value = MATRIX_DEFAULT_HOMESERVER;
				authModeDraft.value = "password";
				userIdDraft.value = "";
				credentialDraft.value = "";
				deviceDisplayNameDraft.value = "";
				ownershipModeDraft.value = "moltis_owned";
				otpSelfApprovalDraft.value = true;
				otpCooldownDraft.value = "300";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect Matrix.";
			}
		});
	}

	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<Modal show={showAddMatrix.value} onClose={() => { showAddMatrix.value = false; }} title="Connect Matrix">
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">Connect a Matrix bot user</span>
						<div className="text-xs text-[var(--muted)] channel-help">1. Leave the homeserver as <span className="font-mono">{MATRIX_DEFAULT_HOMESERVER}</span> for matrix.org accounts</div>
						<div className="text-xs text-[var(--muted)]">2. Password is the default because it supports encrypted Matrix chats. Access token auth is only for plain Matrix traffic</div>
						<div className="text-xs text-[var(--muted)]">3. Moltis generates the local account ID automatically from the Matrix user or homeserver</div>
					</div>
				</div>
				<div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-100">
					<div className="font-medium text-emerald-50">Encrypted chats require password auth</div>
					<div>{MATRIX_ENCRYPTION_GUIDANCE}</div>
				</div>
				<ConnectionModeHint type="matrix" />
				<label className="text-xs text-[var(--muted)]">Homeserver URL</label>
				<input
					data-field="homeserver"
					type="text"
					placeholder={MATRIX_DEFAULT_HOMESERVER}
					value={homeserverDraft.value}
					onInput={(e) => { homeserverDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					autoFocus
				/>
				<label className="text-xs text-[var(--muted)]">Authentication</label>
				<select
					data-field="authMode"
					className="channel-select"
					value={authModeDraft.value}
					onChange={(e) => { authModeDraft.value = normalizeMatrixAuthMode((e.target as HTMLSelectElement).value); }}
				>
					<option value="password">Password</option>
					<option value="access_token">Access token</option>
				</select>
				<div className="text-xs text-[var(--muted)]">{matrixAuthModeGuidance(authModeDraft.value)}</div>
				{authModeDraft.value === "password" ? (
					<label className="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2">
						<input
							type="checkbox"
							aria-label="Let Moltis own this Matrix account"
							checked={normalizeMatrixOwnershipMode(ownershipModeDraft.value) === "moltis_owned"}
							onChange={(e) => { ownershipModeDraft.value = (e.target as HTMLInputElement).checked ? "moltis_owned" : "user_managed"; }}
						/>
						<span className="flex flex-col gap-1">
							<span className="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
							<span className="text-xs text-[var(--muted)]">{matrixOwnershipModeGuidance(authModeDraft.value, ownershipModeDraft.value)}</span>
						</span>
					</label>
				) : (
					<div className="text-xs text-[var(--muted)]">{matrixOwnershipModeGuidance(authModeDraft.value, "user_managed")}</div>
				)}
				<label className="text-xs text-[var(--muted)]">Matrix User ID{authModeDraft.value === "password" ? " (required)" : " (optional)"}</label>
				<input
					data-field="userId"
					type="text"
					placeholder="@bot:example.com"
					value={userIdDraft.value}
					onInput={(e) => { userIdDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">{matrixCredentialLabel(authModeDraft.value)}</label>
				<input
					data-field="credential"
					type="password"
					placeholder={matrixCredentialPlaceholder(authModeDraft.value)}
					value={credentialDraft.value}
					onInput={(e) => { credentialDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
				/>
				<div className="text-xs text-[var(--muted)]">
					{authModeDraft.value === "password"
						? "Use the password for the dedicated Matrix bot account. This is the required mode for encrypted Matrix chats because Moltis needs to create and persist its own Matrix device keys."
						: <>Get the access token in Element: <span className="font-mono">Settings -&gt; Help & About -&gt; Advanced -&gt; Access Token</span>. Access token mode does <span className="font-medium">not</span> support encrypted Matrix chats because Moltis cannot import that existing device's private encryption keys.</>
					}
					{" "}
					<a href={MATRIX_DOCS_URL} target="_blank" rel="noreferrer" className="text-[var(--accent)] underline">Matrix setup docs</a>
				</div>
				<label className="text-xs text-[var(--muted)]">Device Display Name (optional)</label>
				<input
					data-field="deviceDisplayName"
					type="text"
					placeholder="Moltis Matrix Bot"
					value={deviceDisplayNameDraft.value}
					onInput={(e) => { deviceDisplayNameDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select data-field="dmPolicy" className="channel-select">
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Room Policy</label>
				<select data-field="roomPolicy" className="channel-select">
					<option value="allowlist">Room allowlist only</option>
					<option value="open">Open (any joined room)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Room Mention Mode</label>
				<select data-field="mentionMode" className="channel-select">
					<option value="mention">Must mention bot</option>
					<option value="always">Always respond</option>
					<option value="none">Never respond in rooms</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Invite Auto-Join</label>
				<select data-field="autoJoin" className="channel-select">
					<option value="always">Always join invites</option>
					<option value="allowlist">Only when inviter or room is allowlisted</option>
					<option value="off">Do not auto-join</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Unknown DM Approval</label>
				<select
					data-field="otpSelfApproval"
					className="channel-select"
					value={otpSelfApprovalDraft.value ? "on" : "off"}
					onChange={(e) => { otpSelfApprovalDraft.value = (e.target as HTMLSelectElement).value !== "off"; }}
				>
					<option value="on">PIN challenge enabled (recommended)</option>
					<option value="off">Reject unknown DMs without a PIN</option>
				</select>
				<label className="text-xs text-[var(--muted)]">PIN Cooldown Seconds</label>
				<input
					data-field="otpCooldown"
					type="number"
					min={1}
					step={1}
					className="channel-input"
					value={otpCooldownDraft.value}
					onInput={(e) => { otpCooldownDraft.value = (e.target as HTMLInputElement).value; }}
				/>
				<div className="text-xs text-[var(--muted)]">With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.</div>
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={addModel.value}
					onChange={(v: string) => { addModel.value = v; }}
					placeholder={defaultPlaceholder}
				/>
				<label className="text-xs text-[var(--muted)]">DM Allowlist (Matrix user IDs)</label>
				<AllowlistInput value={userAllowlistItems.value} preserveAt={true} onChange={(items) => { userAllowlistItems.value = items; }} />
				<label className="text-xs text-[var(--muted)]">Room Allowlist (room IDs or aliases)</label>
				<AllowlistInput value={roomAllowlistItems.value} preserveAt={true} onChange={(items) => { roomAllowlistItems.value = items; }} />
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Matrix"}
				</button>
			</div>
		</Modal>
	);
}

// ── QR code display (WhatsApp pairing) ───────────────────────

function qrSvgObjectUrl(svg: string | null): string | null {
	if (!svg) return null;
	try {
		return URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
	} catch (_err) {
		return null;
	}
}

interface QrCodeDisplayProps {
	data: string | null;
	svg: string | null;
}

function QrCodeDisplay({ data, svg }: QrCodeDisplayProps): VNode {
	const [svgUrl, setSvgUrl] = useState<string | null>(null);

	useEffect(() => {
		const nextUrl = qrSvgObjectUrl(svg);
		setSvgUrl(nextUrl);
		return () => {
			if (nextUrl) URL.revokeObjectURL(nextUrl);
		};
	}, [svg]);

	if (!data)
		return <div className="flex items-center justify-center p-8 text-[var(--muted)] text-sm">Waiting for QR code...</div>;

	return (
		<div className="flex flex-col items-center gap-3 p-4">
			<div className="rounded-lg bg-white p-3" style={{ width: "200px", height: "200px", display: "flex", alignItems: "center", justifyContent: "center" }}>
				{svgUrl ? (
					<img src={svgUrl} alt="WhatsApp pairing QR code" style={{ width: "100%", height: "100%", display: "block" }} />
				) : (
					<div className="text-center text-xs text-gray-600">
						<div style={{ fontFamily: "monospace", fontSize: "9px", wordBreak: "break-all", maxHeight: "180px", overflow: "hidden" }}>{data.substring(0, 200)}</div>
					</div>
				)}
			</div>
			<div className="text-xs text-[var(--muted)] text-center">
				Scan this QR code in your terminal output,<br />or open WhatsApp &gt; Settings &gt; Linked Devices &gt; Link a Device.
			</div>
		</div>
	);
}

// ── Add Nostr modal ──────────────────────────────────────────

function AddNostrModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const secretKeyDraft = useSignal("");
	const relaysDraft = useSignal("wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const secretKey = secretKeyDraft.value.trim();
		if (!accountId) {
			error.value = "Account ID is required.";
			return;
		}
		if (!secretKey) {
			error.value = "Secret key is required.";
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		const relays = relaysDraft.value
			.split(",")
			.map((r) => r.trim())
			.filter(Boolean);
		const addConfig: ChannelConfig = {
			secret_key: secretKey,
			relays,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			allowed_pubkeys: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("nostr", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddNostr.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				secretKeyDraft.value = "";
				relaysDraft.value = "wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal show={showAddNostr.value} onClose={() => { showAddNostr.value = false; }} title="Connect Nostr">
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up Nostr DMs</span>
						<div className="text-xs text-[var(--muted)] channel-help">1. Generate or use an existing Nostr secret key (nsec1... or hex)</div>
						<div className="text-xs text-[var(--muted)]">2. Configure relay URLs (defaults are provided)</div>
						<div className="text-xs text-[var(--muted)]">3. Add allowed public keys (npub1... or hex) to the allowlist</div>
						<div className="text-xs text-[var(--muted)]">4. Send a DM to the bot's public key from any Nostr client</div>
					</div>
				</div>
				<ConnectionModeHint type="nostr" />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-nostr-bot"
					value={accountDraft.value}
					onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Secret Key</label>
				<input
					data-field="credential"
					type="password"
					placeholder="nsec1... or 64-char hex"
					className="channel-input"
					value={secretKeyDraft.value}
					onInput={(e) => { secretKeyDraft.value = (e.target as HTMLInputElement).value; }}
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="nostr_secret_key"
				/>
				<label className="text-xs text-[var(--muted)]">Relays (comma-separated)</label>
				<input
					data-field="relays"
					type="text"
					placeholder="wss://relay.damus.io, wss://nos.lol"
					value={relaysDraft.value}
					onInput={(e) => { relaysDraft.value = (e.target as HTMLInputElement).value; }}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select data-field="dmPolicy" className="channel-select">
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect models={modelsSig.value} value={addModel.value} onChange={(v: string) => { addModel.value = v; }} />
				<label className="text-xs text-[var(--muted)]">Allowed Public Keys</label>
				<AllowlistInput value={allowlistItems.value} onChange={(v) => { allowlistItems.value = v; }} />
				<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Nostr"}
				</button>
			</div>
		</Modal>
	);
}

// ── Add WhatsApp modal ───────────────────────────────────────

function AddWhatsAppModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const pairingStarted = useSignal(false);
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const advancedConfigPatch = useSignal("");
	const qrPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

	function onStartPairing(e: Event): void {
		e.preventDefault();
		const accountId = accountDraft.value.trim() || "main";
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		waQrData.value = null;
		waQrSvg.value = null;
		waPairingError.value = null;
		waPairingAccountId.value = accountId;

		const addConfig: ChannelConfig = {
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement)?.value || "open",
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("whatsapp", accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				pairingStarted.value = true;
				// Poll channels.status as fallback for QR display and connection detection.
				if (qrPollRef.current) clearInterval(qrPollRef.current);
				qrPollRef.current = setInterval(async () => {
					try {
						const st = await sendRpc<{ channels?: Channel[] }>("channels.status", {});
						if (!st?.ok) return;
						const ch = (st.payload?.channels || []).find((c) => c.type === "whatsapp" && c.account_id === accountId);
						if (!ch) return;
						if (ch.status === "connected" || (waQrData.value && !ch.extra?.qr_data)) {
							clearInterval(qrPollRef.current!);
							qrPollRef.current = null;
							showToast("WhatsApp connected!");
							showAddWhatsApp.value = false;
							waPairingAccountId.value = null;
							waQrData.value = null;
							waQrSvg.value = null;
							loadChannels();
							return;
						}
						if (ch.extra?.qr_data) {
							waQrData.value = ch.extra.qr_data;
							if (ch.extra.qr_svg) waQrSvg.value = ch.extra.qr_svg;
						}
					} catch (_e) {
						/* ignore */
					}
				}, 2000);
			} else {
				error.value = (r?.error?.message || r?.error?.detail) || "Failed to start pairing.";
			}
		});
	}

	function onClose(): void {
		if (qrPollRef.current) {
			clearInterval(qrPollRef.current);
			qrPollRef.current = null;
		}
		showAddWhatsApp.value = false;
		pairingStarted.value = false;
		waQrData.value = null;
		waQrSvg.value = null;
		waPairingError.value = null;
		waPairingAccountId.value = null;
		allowlistItems.value = [];
		accountDraft.value = "";
		advancedConfigPatch.value = "";
		loadChannels();
	}

	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<Modal show={showAddWhatsApp.value} onClose={onClose} title="Connect WhatsApp">
			<div className="channel-form">
				{pairingStarted.value ? (
					<div className="flex flex-col items-center gap-4">
						{waPairingError.value ? (
							<div className="text-sm text-[var(--error)]">{waPairingError.value}</div>
						) : (
							<QrCodeDisplay data={waQrData.value} svg={waQrSvg.value} />
						)}
						<div className="text-xs text-[var(--muted)]">QR code refreshes automatically. Keep this window open.<br />Only new messages will be processed &mdash; past conversations are not synced.</div>
					</div>
				) : (
					<>
						<div className="channel-card">
							<div>
								<span className="text-xs font-medium text-[var(--text-strong)]">Link your WhatsApp</span>
								<div className="text-xs text-[var(--muted)] channel-help">1. Choose an account ID below (any name you like)</div>
								<div className="text-xs text-[var(--muted)]">2. Click "Start Pairing" to generate a QR code</div>
								<div className="text-xs text-[var(--muted)]">3. Open WhatsApp on your phone &gt; Settings &gt; Linked Devices &gt; Link a Device</div>
								<div className="text-xs text-[var(--muted)]">4. Scan the QR code to connect</div>
							</div>
						</div>
						<ConnectionModeHint type="whatsapp" />
						<label className="text-xs text-[var(--muted)]">Account ID</label>
						<input
							data-field="accountId"
							type="text"
							placeholder="main"
							className="channel-input"
							value={accountDraft.value}
							onInput={(e) => { accountDraft.value = (e.target as HTMLInputElement).value; }}
						/>
						<label className="text-xs text-[var(--muted)]">DM Policy</label>
						<select data-field="dmPolicy" className="channel-select">
							<option value="open">Open (anyone)</option>
							<option value="allowlist">Allowlist only</option>
							<option value="disabled">Disabled</option>
						</select>
						<label className="text-xs text-[var(--muted)]">Default Model</label>
						<ModelSelect
							models={modelsSig.value}
							value={addModel.value}
							onChange={(v: string) => { addModel.value = v; }}
							placeholder={defaultPlaceholder}
						/>
						<label className="text-xs text-[var(--muted)]">DM Allowlist</label>
						<AllowlistInput value={allowlistItems.value} onChange={(v) => { allowlistItems.value = v; }} />
						<AdvancedConfigPatchField value={advancedConfigPatch.value} onInput={(value) => { advancedConfigPatch.value = value; }} />
						{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
						<button className="provider-btn" onClick={onStartPairing} disabled={saving.value}>
							{saving.value ? "Starting\u2026" : "Start Pairing"}
						</button>
					</>
				)}
			</div>
		</Modal>
	);
}

// ── Edit channel modal ───────────────────────────────────────

function EditChannelModal(): VNode | null {
	const ch = editingChannel.value;
	const error = useSignal("");
	const saving = useSignal(false);
	const editModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const roomAllowlistItems = useSignal<string[]>([]);
	const editCredential = useSignal("");
	const editWebhookSecret = useSignal("");
	const editStreamMode = useSignal("edit_in_place");
	const editReplyStyle = useSignal("top_level");
	const editWelcomeCard = useSignal(true);
	const editBotName = useSignal("");
	const editMatrixAuthMode = useSignal("access_token");
	const editMatrixDeviceDisplayName = useSignal("");
	const editMatrixOwnershipMode = useSignal("user_managed");
	const editMatrixOtpSelfApproval = useSignal(true);
	const editMatrixOtpCooldown = useSignal("300");
	const editAdvancedConfigPatch = useSignal("");

	useEffect(() => {
		editModel.value = (ch?.config?.model as string) || "";
		allowlistItems.value = (ch?.config?.allowlist || ch?.config?.user_allowlist || ch?.config?.allowed_pubkeys || []) as string[];
		roomAllowlistItems.value = (ch?.config?.room_allowlist || []) as string[];
		editCredential.value = "";
		editWebhookSecret.value = (ch?.config?.webhook_secret as string) || "";
		editStreamMode.value = (ch?.config?.stream_mode as string) || "edit_in_place";
		editReplyStyle.value = (ch?.config?.reply_style as string) || "top_level";
		editWelcomeCard.value = ch?.config?.welcome_card !== false;
		editBotName.value = (ch?.config?.bot_name as string) || "";
		editMatrixAuthMode.value = ch?.config?.password ? "password" : "access_token";
		editMatrixDeviceDisplayName.value = (ch?.config?.device_display_name as string) || "";
		editMatrixOwnershipMode.value = normalizeMatrixOwnershipMode(
			(ch?.config?.ownership_mode as string) || (ch?.config?.password ? "moltis_owned" : "user_managed"),
		);
		editMatrixOtpSelfApproval.value = ch?.config?.otp_self_approval !== false;
		editMatrixOtpCooldown.value = String(ch?.config?.otp_cooldown_secs || 300);
		editAdvancedConfigPatch.value = "";
	}, [ch]);

	if (!ch) return null;

	const cfg = ch.config || {};
	const chType = channelType(ch.type);
	const isTeams = chType === "msteams";
	const isDiscord = chType === "discord";
	const isWhatsApp = chType === "whatsapp";
	const isTelegram = chType === "telegram";
	const isMatrix = chType === "matrix";
	const isNostr = chType === "nostr";

	function addModelToConfig(config: ChannelConfig): void {
		if (!editModel.value) return;
		config.model = editModel.value;
		const found = modelsSig.value.find((x) => x.id === editModel.value);
		if (found?.provider) config.model_provider = found.provider;
	}

	function addChannelCredentials(config: ChannelConfig, form: HTMLElement): void {
		if (isTeams) {
			config.app_id = cfg.app_id || ch?.account_id;
			config.app_password = editCredential.value || cfg.app_password || "";
			if (editWebhookSecret.value.trim()) config.webhook_secret = editWebhookSecret.value.trim();
		} else if (isDiscord) {
			config.token = editCredential.value || cfg.token || "";
		} else if (isTelegram) {
			config.token = cfg.token || "";
		} else if (isNostr) {
			config.secret_key = editCredential.value || cfg.secret_key || "";
			const relaysVal = (form.querySelector("[data-field=relays]") as HTMLInputElement)?.value || "";
			config.relays = relaysVal.split(",").map((r) => r.trim()).filter(Boolean);
		} else if (isMatrix) {
			config.homeserver = (form.querySelector("[data-field=homeserver]") as HTMLInputElement)?.value || cfg.homeserver || "";
			config.user_id = (form.querySelector("[data-field=userId]") as HTMLInputElement)?.value || cfg.user_id || "";
			config.device_id = cfg.device_id || undefined;
			config.device_display_name = editMatrixDeviceDisplayName.value.trim() || null;
			config.ownership_mode =
				normalizeMatrixAuthMode(editMatrixAuthMode.value) === "password"
					? normalizeMatrixOwnershipMode(editMatrixOwnershipMode.value)
					: "user_managed";
			if (normalizeMatrixAuthMode(editMatrixAuthMode.value) === "password") {
				config.password = editCredential.value || cfg.password || "";
				config.access_token = "";
			} else {
				config.access_token = editCredential.value || cfg.access_token || "";
				config.password = null;
			}
		}
	}

	function buildUpdateConfig(form: HTMLElement): ChannelConfig {
		const updateConfig: ChannelConfig = {};
		updateConfig.dm_policy = (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement)?.value || "open";
		updateConfig.allowlist = allowlistItems.value;
		if (isMatrix) {
			updateConfig.user_allowlist = allowlistItems.value;
			updateConfig.room_policy = (form.querySelector("[data-field=roomPolicy]") as HTMLSelectElement)?.value || cfg.room_policy || "allowlist";
			updateConfig.auto_join = (form.querySelector("[data-field=autoJoin]") as HTMLSelectElement)?.value || cfg.auto_join || "always";
			updateConfig.room_allowlist = roomAllowlistItems.value;
			updateConfig.otp_self_approval = editMatrixOtpSelfApproval.value;
			updateConfig.otp_cooldown_secs = normalizeMatrixOtpCooldown(editMatrixOtpCooldown.value);
		}
		if (isNostr) {
			updateConfig.allowed_pubkeys = allowlistItems.value;
			// Preserve OTP settings that have no dedicated UI fields yet.
			updateConfig.otp_self_approval = cfg.otp_self_approval !== false;
			updateConfig.otp_cooldown_secs = cfg.otp_cooldown_secs ?? 300;
		}
		if (!(isWhatsApp || isNostr)) {
			updateConfig.mention_mode = (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement)?.value || "mention";
		}
		addChannelCredentials(updateConfig, form);
		addModelToConfig(updateConfig);
		if (isTeams) {
			updateConfig.stream_mode = editStreamMode.value;
			updateConfig.reply_style = editReplyStyle.value;
			updateConfig.welcome_card = editWelcomeCard.value;
			if (editBotName.value.trim()) updateConfig.bot_name = editBotName.value.trim();
		}
		return updateConfig;
	}

	function onSave(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const advancedPatch = parseChannelConfigPatch(editAdvancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		if (!ch) return;
		saving.value = true;
		const updateConfig = buildUpdateConfig(form);
		Object.assign(updateConfig, advancedPatch.value);
		sendRpc("channels.update", {
			type: channelType(ch.type),
			account_id: ch.account_id,
			config: updateConfig,
		}).then((res) => {
			saving.value = false;
			if (res?.ok) {
				editingChannel.value = null;
				loadChannels();
			} else {
				error.value = ((res?.error as { message?: string; detail?: string })?.message || (res?.error as { detail?: string })?.detail) || "Failed to update channel.";
			}
		});
	}

	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<Modal show={true} onClose={() => { editingChannel.value = null; }} title={`Edit ${channelLabel(ch.type)} Channel`}>
			<div className="channel-form">
				<div className="text-sm text-[var(--text-strong)]">{ch.name || ch.account_id}</div>
				{isTelegram && ch.account_id && (
					<a href={`https://t.me/${ch.account_id}`} target="_blank" className="text-xs text-[var(--accent)] underline">t.me/{ch.account_id}</a>
				)}
				{isTeams && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">App Password (optional: leave blank to keep existing)</label>
						<input
							type="password"
							className="channel-input w-full"
							value={editCredential.value}
							onInput={(e) => { editCredential.value = (e.target as HTMLInputElement).value; }}
						/>
					</div>
				)}
				{isTeams && (
					<>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Webhook Secret</label>
							<input
								type="text"
								className="channel-input w-full"
								value={editWebhookSecret.value}
								onInput={(e) => { editWebhookSecret.value = (e.target as HTMLInputElement).value; }}
							/>
						</div>
						<div className="flex gap-3">
							<div className="flex-1">
								<label className="text-xs text-[var(--muted)]">Streaming</label>
								<select
									className="channel-select"
									value={editStreamMode.value}
									onChange={(e) => { editStreamMode.value = (e.target as HTMLSelectElement).value; }}
								>
									<option value="edit_in_place">Edit-in-place (live updates)</option>
									<option value="off">Off (send once complete)</option>
								</select>
							</div>
							<div className="flex-1">
								<label className="text-xs text-[var(--muted)]">Reply Style</label>
								<select
									className="channel-select"
									value={editReplyStyle.value}
									onChange={(e) => { editReplyStyle.value = (e.target as HTMLSelectElement).value; }}
								>
									<option value="top_level">Top-level message</option>
									<option value="thread">Reply in thread</option>
								</select>
							</div>
						</div>
						<div className="flex gap-3 items-end">
							<div className="flex-1">
								<label className="text-xs text-[var(--muted)]">Bot Name (for welcome card)</label>
								<input
									type="text"
									className="channel-input"
									value={editBotName.value}
									onInput={(e) => { editBotName.value = (e.target as HTMLInputElement).value; }}
									placeholder="Moltis"
								/>
							</div>
							<label className="flex items-center gap-2 text-xs text-[var(--muted)] pb-2 cursor-pointer">
								<input
									type="checkbox"
									checked={editWelcomeCard.value}
									onChange={(e) => { editWelcomeCard.value = (e.target as HTMLInputElement).checked; }}
								/>
								Welcome card
							</label>
						</div>
					</>
				)}
				{isDiscord && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Bot Token (optional: leave blank to keep existing)</label>
						<input
							type="password"
							className="channel-input w-full"
							value={editCredential.value}
							onInput={(e) => { editCredential.value = (e.target as HTMLInputElement).value; }}
						/>
					</div>
				)}
				{isNostr && (
					<>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Secret Key (optional: leave blank to keep existing)</label>
							<input
								type="password"
								className="channel-input w-full"
								value={editCredential.value}
								onInput={(e) => { editCredential.value = (e.target as HTMLInputElement).value; }}
								autoComplete="new-password"
							/>
						</div>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Relays (comma-separated)</label>
							<input
								data-field="relays"
								type="text"
								className="channel-input w-full"
								defaultValue={(cfg.relays as string[] || []).join(", ")}
							/>
						</div>
					</>
				)}
				{isMatrix && (
					<div className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-100">
						<div className="font-medium text-emerald-50">Encrypted chats require password auth</div>
						<div>{MATRIX_ENCRYPTION_GUIDANCE}</div>
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Authentication</label>
						<select
							className="channel-select w-full"
							value={editMatrixAuthMode.value}
							onChange={(e) => { editMatrixAuthMode.value = normalizeMatrixAuthMode((e.target as HTMLSelectElement).value); }}
						>
							<option value="access_token">Access token</option>
							<option value="password">Password</option>
						</select>
						<div className="text-xs text-[var(--muted)]">{matrixAuthModeGuidance(editMatrixAuthMode.value)}</div>
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						{editMatrixAuthMode.value === "password" ? (
							<label className="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2">
								<input
									type="checkbox"
									aria-label="Let Moltis own this Matrix account"
									checked={normalizeMatrixOwnershipMode(editMatrixOwnershipMode.value) === "moltis_owned"}
									onChange={(e) => { editMatrixOwnershipMode.value = (e.target as HTMLInputElement).checked ? "moltis_owned" : "user_managed"; }}
								/>
								<span className="flex flex-col gap-1">
									<span className="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
									<span className="text-xs text-[var(--muted)]">{matrixOwnershipModeGuidance(editMatrixAuthMode.value, editMatrixOwnershipMode.value)}</span>
								</span>
							</label>
						) : (
							<div className="text-xs text-[var(--muted)]">{matrixOwnershipModeGuidance(editMatrixAuthMode.value, "user_managed")}</div>
						)}
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Homeserver URL</label>
						<input data-field="homeserver" type="text" className="channel-input w-full" defaultValue={(cfg.homeserver as string) || ""} />
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Matrix User ID{editMatrixAuthMode.value === "password" ? " (required)" : " (optional)"}</label>
						<input data-field="userId" type="text" className="channel-input w-full" defaultValue={(cfg.user_id as string) || ""} />
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">{matrixCredentialLabel(editMatrixAuthMode.value)} (optional: leave blank to keep existing)</label>
						<input
							type="password"
							className="channel-input w-full"
							value={editCredential.value}
							onInput={(e) => { editCredential.value = (e.target as HTMLInputElement).value; }}
							placeholder={matrixCredentialPlaceholder(editMatrixAuthMode.value)}
						/>
						<div className="text-xs text-[var(--muted)]">
							{editMatrixAuthMode.value === "password"
								? "Password auth is required for encrypted Matrix chats because Moltis needs its own Matrix device keys."
								: <>Access token mode does <span className="font-medium">not</span> support encrypted Matrix chats because Moltis cannot import the existing device's private encryption keys.</>
							}
							{" "}
							<a href={MATRIX_DOCS_URL} target="_blank" rel="noreferrer" className="text-[var(--accent)] underline">Matrix setup docs</a>
						</div>
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Device Display Name (optional)</label>
						<input
							type="text"
							className="channel-input w-full"
							value={editMatrixDeviceDisplayName.value}
							onInput={(e) => { editMatrixDeviceDisplayName.value = (e.target as HTMLInputElement).value; }}
						/>
					</div>
				)}
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select data-field="dmPolicy" className="channel-select" value={(cfg.dm_policy as string) || (isWhatsApp ? "open" : "allowlist")}>
					{isWhatsApp && <option value="open">Open (anyone)</option>}
					<option value="allowlist">Allowlist only</option>
					{!isWhatsApp && <option value="open">Open (anyone)</option>}
					<option value="disabled">Disabled</option>
				</select>
				{!isWhatsApp && (
					<>
						<label className="text-xs text-[var(--muted)]">Group Mention Mode</label>
						<select data-field="mentionMode" className="channel-select" value={(cfg.mention_mode as string) || "mention"}>
							<option value="mention">Must @mention bot</option>
							<option value="always">Always respond</option>
							<option value="none">Don't respond in groups</option>
						</select>
					</>
				)}
				{isMatrix && (
					<>
						<label className="text-xs text-[var(--muted)]">Unknown DM Approval</label>
						<select
							className="channel-select"
							value={editMatrixOtpSelfApproval.value ? "on" : "off"}
							onChange={(e) => { editMatrixOtpSelfApproval.value = (e.target as HTMLSelectElement).value !== "off"; }}
						>
							<option value="on">PIN challenge enabled (recommended)</option>
							<option value="off">Reject unknown DMs without a PIN</option>
						</select>
						<label className="text-xs text-[var(--muted)]">PIN Cooldown Seconds</label>
						<input
							type="number"
							min={1}
							step={1}
							className="channel-input"
							value={editMatrixOtpCooldown.value}
							onInput={(e) => { editMatrixOtpCooldown.value = (e.target as HTMLInputElement).value; }}
						/>
						<div className="text-xs text-[var(--muted)]">With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.</div>
						<label className="text-xs text-[var(--muted)]">Room Policy</label>
						<select data-field="roomPolicy" className="channel-select" value={(cfg.room_policy as string) || "allowlist"}>
							<option value="allowlist">Room allowlist only</option>
							<option value="open">Open (any joined room)</option>
							<option value="disabled">Disabled</option>
						</select>
						<label className="text-xs text-[var(--muted)]">Invite Auto-Join</label>
						<select data-field="autoJoin" className="channel-select" value={(cfg.auto_join as string) || "always"}>
							<option value="always">Always join invites</option>
							<option value="allowlist">Only when inviter or room is allowlisted</option>
							<option value="off">Do not auto-join</option>
						</select>
					</>
				)}
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={editModel.value}
					onChange={(v: string) => { editModel.value = v; }}
					placeholder={defaultPlaceholder}
				/>
				<label className="text-xs text-[var(--muted)]">DM Allowlist</label>
				<AllowlistInput value={allowlistItems.value} preserveAt={isMatrix} onChange={(v) => { allowlistItems.value = v; }} />
				{isMatrix && (
					<>
						<label className="text-xs text-[var(--muted)]">Room Allowlist</label>
						<AllowlistInput value={roomAllowlistItems.value} preserveAt={true} onChange={(v) => { roomAllowlistItems.value = v; }} />
					</>
				)}
				<AdvancedConfigPatchField
					value={editAdvancedConfigPatch.value}
					onInput={(value) => { editAdvancedConfigPatch.value = value; }}
					currentConfig={cfg}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSave} disabled={saving.value}>
					{saving.value ? "Saving\u2026" : "Save Changes"}
				</button>
			</div>
		</Modal>
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

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3 flex-wrap">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Channels</h2>
				<div style={{ display: "flex", gap: "4px", marginLeft: "12px" }}>
					<button
						className="session-action-btn"
						style={activeTab.value === "channels" ? { fontWeight: 600 } : undefined}
						onClick={() => { activeTab.value = "channels"; }}
					>
						Channels
					</button>
					<button
						className="session-action-btn"
						style={activeTab.value === "senders" ? { fontWeight: 600 } : undefined}
						onClick={() => { activeTab.value = "senders"; }}
					>
						Senders
					</button>
				</div>
				{activeTab.value === "channels" && channels.value.length > 0 && <ConnectButtons />}
			</div>
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
