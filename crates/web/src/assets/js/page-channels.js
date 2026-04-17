// ── Channels page (Preact + HTM + Signals) ──────────────────

import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
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
} from "./channel-utils.js";
import { onEvent } from "./events.js";
import { get as getGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { connected } from "./signals.js";
import * as S from "./state.js";
import { models as modelsSig } from "./stores/model-store.js";
import { ConfirmDialog, Modal, ModelSelect, requestConfirm, showToast } from "./ui.js";

var channels = signal([]);

export function prefetchChannels() {
	return fetchChannelStatus().then((res) => {
		if (res?.ok) {
			var ch = res.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
		}
	});
}
var senders = signal([]);
var activeTab = signal("channels");
var showAddTelegram = signal(false);
var showAddTeams = signal(false);
var showAddDiscord = signal(false);
var showAddWhatsApp = signal(false);
var showAddSlack = signal(false);
var showAddMatrix = signal(false);
var showAddNostr = signal(false);
var editingChannel = signal(null);
var sendersAccount = signal("");

// Track WhatsApp pairing state (updated by WebSocket events).
var waQrData = signal(null);
var waQrSvg = signal(null);
var waPairingAccountId = signal(null);
var waPairingError = signal(null);

function channelType(type) {
	return type || "telegram";
}

function channelLabel(type) {
	var t = channelType(type);
	if (t === "msteams") return "Microsoft Teams";
	if (t === "discord") return "Discord";
	if (t === "whatsapp") return "WhatsApp";
	if (t === "slack") return "Slack";
	if (t === "matrix") return "Matrix";
	if (t === "nostr") return "Nostr";
	return "Telegram";
}

function channelDescriptor(type) {
	var descs = getGon("channel_descriptors") || [];
	return descs.find((d) => d.channel_type === channelType(type)) || null;
}

var MODE_LABELS = {
	none: "Send only",
	polling: "Polling",
	gateway_loop: "Gateway",
	socket_mode: "Socket Mode",
	webhook: "Webhook",
};

var MODE_HINTS = {
	webhook: "Requires a publicly reachable URL. Configure your platform to send events to the endpoint shown below.",
	polling: "Connects automatically via long-polling. No public URL needed.",
	gateway_loop: "Maintains a persistent connection. No public URL needed.",
	socket_mode: "Connects via Socket Mode. No public URL needed.",
	none: "This channel is send-only and cannot receive inbound messages.",
};

function ConnectionModeHint({ type }) {
	var desc = channelDescriptor(type);
	if (!desc) return null;
	var hint = MODE_HINTS[desc.capabilities.inbound_mode];
	if (!hint) return null;
	return html`<div class="text-xs text-[var(--muted)] mt-1 flex items-center gap-1">
		<span class="tier-badge">${MODE_LABELS[desc.capabilities.inbound_mode]}</span>
		<span>${hint}</span>
	</div>`;
}

function ChannelStorageNotice({ compact = false }) {
	return html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)] ${compact ? "" : "max-w-3xl"}">
		<span class="font-medium text-[var(--text-strong)]">Storage note.</span> ${channelStorageNote()}
	</div>`;
}

function prettyConfigJson(value) {
	try {
		return JSON.stringify(value || {}, null, 2);
	} catch (_error) {
		return "{}";
	}
}

function copyToClipboard(value, successMessage) {
	var text = String(value || "").trim();
	if (!text) return;
	navigator.clipboard.writeText(text).then(() => showToast(successMessage));
}

function MatrixInfoRow({ label, value, copyLabel = null }) {
	var text = String(value || "").trim();
	return html`<div class="flex items-center justify-between gap-3">
		<div class="min-w-0">
			<div class="text-[11px] uppercase tracking-wide text-sky-700">${label}</div>
			<div class="truncate font-mono text-sky-900">${text || "\u2014"}</div>
		</div>
		${
			text &&
			html`<button
				type="button"
				class="provider-btn provider-btn-sm provider-btn-secondary"
				onClick=${() => copyToClipboard(text, copyLabel || `${label} copied`)}>
				Copy
			</button>`
		}
	</div>`;
}

function MatrixOwnershipCard({ channel, matrixStatus }) {
	var retryingOwnership = useSignal(false);
	var retryOwnershipError = useSignal("");
	var ownershipMode = normalizeMatrixOwnershipMode(matrixStatus?.ownership_mode);
	var authMode = normalizeMatrixAuthMode(matrixStatus?.auth_mode);
	var recoveryState = String(matrixStatus?.recovery_state || "unknown");
	var deviceVerified = !!matrixStatus?.device_verified_by_owner;
	var ownershipError = String(matrixStatus?.ownership_error || "").trim();
	var approvalMatch = ownershipError.match(/https?:\/\/\S+/);
	var ownershipIssue =
		ownershipMode !== "moltis_owned" || ownershipError.length === 0
			? "none"
			: ownershipError.includes("requires browser approval to reset cross-signing")
				? "approval_required"
				: ownershipError.includes("incomplete secret storage")
					? "incomplete_secret_storage"
					: "generic_blocked";
	var modeTitle =
		ownershipIssue === "approval_required"
			? "Ownership approval required"
			: ownershipIssue !== "none"
				? "Moltis ownership blocked"
				: ownershipMode === "moltis_owned"
					? "Managed by Moltis"
					: "User-managed in Element";
	var modeText =
		ownershipIssue === "approval_required"
			? "This existing Matrix account can already chat, but Matrix needs one browser approval before Moltis can take over encryption ownership. Open the approval page, approve the reset, then retry ownership setup."
			: ownershipIssue === "incomplete_secret_storage"
				? "This account already has partial Matrix secure-backup state. Finish or repair it in Element, or switch this channel to user-managed mode."
				: ownershipIssue === "generic_blocked"
					? "Moltis could not take ownership of this Matrix account automatically. Repair the account in Element or switch this channel to user-managed mode."
					: authMode === "password" || authMode === "oidc"
						? matrixOwnershipModeGuidance(authMode, ownershipMode)
						: "Access token auth is always user-managed. If you want encrypted Matrix chats, reconnect this channel with OIDC or password auth so Moltis can create its own device.";
	var detailTitle =
		ownershipIssue === "approval_required"
			? "Browser approval pending"
			: ownershipError
				? "Ownership setup needs attention"
				: "";
	var detailText =
		ownershipIssue === "approval_required"
			? `Approve the reset while signed into ${matrixStatus?.user_id || "this Matrix account"} in the browser, then use the retry button here so Moltis can finish taking ownership.`
			: ownershipError;
	var approvalUrl = approvalMatch ? approvalMatch[0].replace(/[;),.]+$/, "") : "";
	var verificationText = deviceVerified ? "Device verified by owner" : "Device not yet verified by owner";
	var hasAccountDetails =
		!!String(channel.config?.homeserver || "").trim() ||
		!!String(matrixStatus?.user_id || "").trim() ||
		!!String(matrixStatus?.device_id || "").trim() ||
		!!String(matrixStatus?.device_display_name || channel.config?.device_display_name || "").trim();

	function retryOwnershipSetup() {
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
				(res?.error && (res.error.message || res.error.detail)) || "Failed to retry Matrix ownership setup.";
		});
	}

	return html`<div class="rounded-md border border-sky-600/30 bg-sky-50 px-3 py-2 text-xs text-sky-900">
		<div class="flex items-center gap-2">
			<div class="font-medium text-sky-800">${modeTitle}</div>
			<span class="provider-item-badge ${deviceVerified ? "configured" : "oauth"}">${verificationText}</span>
		</div>
		<div class="mt-1 text-sky-900">${modeText}</div>
		<div class="mt-2 text-sky-900">
			Cross-signing: <span class="font-medium">${matrixStatus?.cross_signing_complete ? "ready" : "not ready"}</span>.
			Recovery: <span class="font-medium">${recoveryState}</span>.
		</div>
		${
			hasAccountDetails &&
			html`<details class="mt-2 rounded-md border border-sky-600/20 bg-sky-100/50 px-3 py-2">
				<summary class="cursor-pointer text-[11px] font-medium uppercase tracking-wide text-sky-800">
					Matrix account details
				</summary>
				<div class="mt-2 grid gap-2">
					<${MatrixInfoRow} label="Homeserver" value=${channel.config?.homeserver || ""} copyLabel="Homeserver copied" />
					<${MatrixInfoRow} label="Matrix user" value=${matrixStatus?.user_id || ""} copyLabel="Matrix user ID copied" />
					<${MatrixInfoRow} label="Device ID" value=${matrixStatus?.device_id || ""} copyLabel="Matrix device ID copied" />
					<${MatrixInfoRow}
						label="Device name"
						value=${matrixStatus?.device_display_name || channel.config?.device_display_name || ""}
						copyLabel="Matrix device name copied" />
				</div>
			</details>`
		}
		${
			ownershipIssue === "approval_required" &&
			approvalUrl &&
			html`<div class="mt-2">
				<div class="flex flex-wrap gap-2">
					<a
						href=${approvalUrl}
						target="_blank"
						rel="noreferrer"
						class="provider-btn provider-btn-sm"
						aria-label=${`Open approval page for ${matrixStatus?.user_id || "this Matrix account"}`}>
						Open approval page for ${matrixStatus?.user_id || "this account"}
					</a>
					<button
						type="button"
						class="provider-btn provider-btn-sm"
						onClick=${retryOwnershipSetup}
						disabled=${retryingOwnership.value}>
						${retryingOwnership.value ? "Retrying ownership setup..." : "Click here once you reset the account"}
					</button>
				</div>
				<div class="mt-2 text-[11px] text-sky-800">Make sure the browser page is signed into <span class="font-mono text-sky-800">${matrixStatus?.user_id || "the Matrix bot account"}</span>.</div>
				${
					retryOwnershipError.value &&
					html`<div class="mt-2 rounded-md border border-amber-600/30 bg-amber-50 px-3 py-2 text-amber-900">
						${retryOwnershipError.value}
					</div>`
				}
			</div>`
		}
		${
			detailTitle &&
			html`<div class="mt-2 rounded-md border border-amber-600/30 bg-amber-50 px-3 py-2 text-amber-900">
				<div class="font-medium text-amber-800">${detailTitle}</div>
				<div class="mt-1">${detailText}</div>
			</div>`
		}
		${
			ownershipIssue === "none" && ownershipMode === "moltis_owned" && !deviceVerified &&
			html`<div class="mt-2 flex flex-wrap gap-2">
				<button
					type="button"
					class="provider-btn provider-btn-sm"
					onClick=${retryOwnershipSetup}
					disabled=${retryingOwnership.value}>
					${retryingOwnership.value ? "Verifying\u2026" : "Retry device verification"}
				</button>
				${retryOwnershipError.value && html`<div class="text-xs text-[var(--error)]">${retryOwnershipError.value}</div>`}
			</div>`
		}
	</div>`;
}

function AdvancedConfigPatchField({ value, onInput, currentConfig = null }) {
	return html`<details class="channel-card">
		<summary class="cursor-pointer text-xs font-medium text-[var(--text-strong)]">Advanced Config JSON</summary>
		<div class="mt-2 flex flex-col gap-3">
			<div class="text-xs text-[var(--muted)]">
				Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not have dedicated fields yet.
			</div>
			${
				currentConfig &&
				html`<div class="flex flex-col gap-1">
					<label class="text-xs text-[var(--muted)]">Current stored config (read-only)</label>
					<textarea class="channel-input min-h-[160px] font-mono text-xs" readOnly value=${prettyConfigJson(currentConfig)} />
				</div>`
			}
			<div class="flex flex-col gap-1">
				<label class="text-xs text-[var(--muted)]">Advanced config JSON patch (optional)</label>
				<textarea
					data-field="advancedConfigPatch"
					class="channel-input min-h-[140px] font-mono text-xs"
					value=${value}
					onInput=${(e) => {
						onInput(e.target.value);
					}}
					placeholder='{"reply_to_message": true}'></textarea>
			</div>
		</div>
	</details>`;
}

function senderSelectionKey(ch) {
	return `${channelType(ch.type)}::${ch.account_id}`;
}

function parseSenderSelectionKey(key) {
	var idx = key.indexOf("::");
	if (idx < 0) return { type: "telegram", account_id: key };
	return {
		type: key.slice(0, idx) || "telegram",
		account_id: key.slice(idx + 2),
	};
}

function loadChannels() {
	fetchChannelStatus().then((res) => {
		if (res?.ok) {
			var ch = res.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
			updateNavCount("channels", ch.length);
		}
	});
}

function loadSenders() {
	var selected = sendersAccount.value;
	if (!selected) {
		senders.value = [];
		return;
	}
	var parsed = parseSenderSelectionKey(selected);
	sendRpc("channels.senders.list", { type: parsed.type, account_id: parsed.account_id }).then((res) => {
		if (res?.ok) senders.value = res.payload?.senders || [];
	});
}

function ChannelIcon({ type }) {
	var t = channelType(type);
	if (t === "msteams") return html`<span class="icon icon-msteams"></span>`;
	if (t === "discord") return html`<span class="icon icon-discord"></span>`;
	if (t === "whatsapp") return html`<span class="icon icon-whatsapp"></span>`;
	if (t === "slack") return html`<span class="icon icon-slack"></span>`;
	if (t === "matrix") return html`<span class="icon icon-matrix"></span>`;
	return html`<span class="icon icon-telegram"></span>`;
}

// ── Channel card ─────────────────────────────────────────────
function ChannelCard(props) {
	var ch = props.channel;

	function onRemove() {
		requestConfirm(`Remove ${ch.name || ch.account_id}?`).then((yes) => {
			if (!yes) return;
			sendRpc("channels.remove", { type: channelType(ch.type), account_id: ch.account_id }).then((r) => {
				if (r?.ok) loadChannels();
			});
		});
	}

	var statusClass = ch.status === "connected" ? "configured" : "oauth";
	var sessionLine = "";
	if (ch.sessions && ch.sessions.length > 0) {
		var active = ch.sessions.filter((s) => s.active);
		sessionLine =
			active.length > 0
				? active.map((s) => `${s.label || s.key} (${s.messageCount} msgs)`).join(", ")
				: "No active session";
	}
	var desc = channelDescriptor(ch.type);
	var modeLabel = desc ? MODE_LABELS[desc.capabilities.inbound_mode] || desc.capabilities.inbound_mode : null;
	var matrixStatus = ch.extra?.matrix || null;
	var pendingVerifications = Array.isArray(matrixStatus?.pending_verifications)
		? matrixStatus.pending_verifications
		: [];
	var verificationStateLabel = matrixStatus?.verification_state || null;
	var showOwnershipCard =
		channelType(ch.type) === "matrix" &&
		(matrixStatus?.user_id || matrixStatus?.device_id || ch.config?.homeserver || matrixStatus?.ownership_error);

	return html`<div class="provider-card p-3 rounded-lg mb-2">
    <div class="flex items-center gap-2.5">
	      <span class="inline-flex items-center justify-center w-7 h-7 rounded-md bg-[var(--surface2)]">
	        <${ChannelIcon} type=${ch.type} />
	      </span>
	      <div class="flex flex-col gap-0.5">
	        <span class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id || channelLabel(ch.type)}</span>
        ${ch.details && html`<span class="text-xs text-[var(--muted)]">${ch.details}</span>`}
        ${sessionLine && html`<span class="text-xs text-[var(--muted)]">${sessionLine}</span>`}
        ${
					channelType(ch.type) === "matrix" &&
					verificationStateLabel &&
					html`<span class="text-xs text-[var(--muted)]">Encryption device state: ${verificationStateLabel}</span>`
				}
        ${channelType(ch.type) === "telegram" && ch.account_id && html`<a href="https://t.me/${ch.account_id}" target="_blank" class="text-xs text-[var(--accent)] underline">t.me/${ch.account_id}</a>`}
      </div>
      <span class="provider-item-badge ${statusClass}">${ch.status || "unknown"}</span>
      ${modeLabel && html`<span class="tier-badge">${modeLabel}</span>`}
    </div>
    ${
			channelType(ch.type) === "matrix" &&
			pendingVerifications.length > 0 &&
			html`<div class="rounded-md border border-emerald-600/30 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
		      <div class="font-medium text-emerald-800">Verification pending</div>
		      ${pendingVerifications.map(
						(prompt) => html`<div class="mt-1">
							<div>With ${prompt.other_user_id}</div>
							<div class="text-emerald-200/90">Send <span class="font-mono">verify yes</span>, <span class="font-mono">verify no</span>, <span class="font-mono">verify show</span>, or <span class="font-mono">verify cancel</span> as a normal message in that same Matrix chat.</div>
						</div>`,
					)}
		    </div>`
		}
    ${showOwnershipCard && html`<${MatrixOwnershipCard} channel=${ch} matrixStatus=${matrixStatus} />`}
    <div class="flex gap-2">
      <button class="provider-btn provider-btn-sm provider-btn-secondary" title="Edit ${ch.account_id || "channel"}"
        onClick=${() => {
					editingChannel.value = ch;
				}}>Edit</button>
      <button class="provider-btn provider-btn-sm provider-btn-danger" title="Remove ${ch.account_id || "channel"}"
        onClick=${onRemove}>Remove</button>
    </div>
  </div>`;
}

// ── Connect channel buttons ──────────────────────────────────
function ConnectButtons() {
	var offered = new Set(getGon("channels_offered") || ["telegram", "whatsapp", "discord", "slack", "matrix"]);
	return html`<div class="flex gap-2 flex-wrap">
		${
			offered.has("telegram") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddTelegram.value = true;
			}}>
			<span class="icon icon-telegram"></span> Connect Telegram
		</button>`
		}
		${
			offered.has("msteams") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddTeams.value = true;
			}}>
			<span class="icon icon-msteams"></span> Connect Microsoft Teams
		</button>`
		}
		${
			offered.has("discord") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddDiscord.value = true;
			}}>
			<span class="icon icon-discord"></span> Connect Discord
		</button>`
		}
		${
			offered.has("slack") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddSlack.value = true;
			}}>
			<span class="icon icon-slack"></span> Connect Slack
		</button>`
		}
		${
			offered.has("matrix") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddMatrix.value = true;
			}}>
			<span class="icon icon-matrix"></span> Connect Matrix
		</button>`
		}
		${
			offered.has("whatsapp") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddWhatsApp.value = true;
			}}>
			<span class="icon icon-whatsapp"></span> Connect WhatsApp
		</button>`
		}
		${
			offered.has("nostr") &&
			html`<button class="provider-btn provider-btn-secondary inline-flex items-center gap-1.5"
			onClick=${() => {
				if (connected.value) showAddNostr.value = true;
			}}>
			<span class="icon icon-nostr"></span> Connect Nostr
		</button>`
		}
	</div>`;
}

// ── Channels tab ─────────────────────────────────────────────
function ChannelsTab() {
	if (channels.value.length === 0) {
		return html`<div class="text-center py-10">
	      <div class="text-sm text-[var(--muted)] mb-4">No channels connected.</div>
	      <div class="flex justify-center"><${ConnectButtons} /></div>
	    </div>`;
	}
	return html`${channels.value.map((ch) => html`<${ChannelCard} key=${senderSelectionKey(ch)} channel=${ch} />`)}`;
}

// ── Sender row renderer ─────────────────────────────────────
function renderSenderRow(s, onAction) {
	var identifier = s.username || s.peer_id;
	var lastSeenMs = s.last_seen ? s.last_seen * 1000 : 0;
	var usernameLabel = s.username ? (String(s.username).startsWith("@") ? s.username : `@${s.username}`) : "\u2014";
	var statusBadge = s.otp_pending
		? html`<span class="provider-item-badge cursor-pointer select-none" style="background:var(--warning-bg, #fef3c7);color:var(--warning-text, #92400e);" onClick=${() => {
				navigator.clipboard.writeText(s.otp_pending.code).then(() => showToast("OTP code copied"));
			}}>OTP: <code class="text-xs">${s.otp_pending.code}</code></span>`
		: html`<span class="provider-item-badge ${s.allowed ? "configured" : "oauth"}">${s.allowed ? "Allowed" : "Denied"}</span>`;
	var actionBtn = s.allowed
		? html`<button class="provider-btn provider-btn-sm provider-btn-danger" onClick=${() => onAction(identifier, "deny")}>Deny</button>`
		: html`<button class="provider-btn provider-btn-sm" onClick=${() => onAction(identifier, "approve")}>Approve</button>`;
	return html`<tr key=${s.peer_id}>
    <td class="senders-td">${s.sender_name || s.peer_id}</td>
    <td class="senders-td" style="color:var(--muted);">${usernameLabel}</td>
    <td class="senders-td">${s.message_count}</td>
    <td class="senders-td" style="color:var(--muted);font-size:12px;">${lastSeenMs ? html`<time data-epoch-ms="${lastSeenMs}">${new Date(lastSeenMs).toISOString()}</time>` : "\u2014"}</td>
    <td class="senders-td">${statusBadge}</td>
    <td class="senders-td">${actionBtn}</td>
  </tr>`;
}

// ── Senders tab ──────────────────────────────────────────────
function SendersTab() {
	useEffect(() => {
		if (channels.value.length > 0 && !sendersAccount.value) {
			sendersAccount.value = senderSelectionKey(channels.value[0]);
		}
	}, [channels.value]);

	useEffect(() => {
		loadSenders();
	}, [sendersAccount.value]);

	if (channels.value.length === 0) {
		return html`<div class="text-sm text-[var(--muted)]">No channels configured.</div>`;
	}

	function onAction(identifier, action) {
		var rpc = action === "approve" ? "channels.senders.approve" : "channels.senders.deny";
		var parsed = parseSenderSelectionKey(sendersAccount.value);
		sendRpc(rpc, {
			type: parsed.type,
			account_id: parsed.account_id,
			identifier: identifier,
		}).then(() => {
			loadSenders();
			loadChannels();
		});
	}

	return html`<div>
    <div style="margin-bottom:12px;">
      <label class="text-xs text-[var(--muted)]" style="margin-right:6px;">Account:</label>
	      <select style="background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:4px 8px;font-size:12px;"
	        value=${sendersAccount.value} onChange=${(e) => {
						sendersAccount.value = e.target.value;
					}}>
	        ${channels.value.map(
						(ch) =>
							html`<option key=${senderSelectionKey(ch)} value=${senderSelectionKey(ch)}>${ch.name || ch.account_id}</option>`,
					)}
	      </select>
    </div>
    ${senders.value.length === 0 && html`<div class="text-sm text-[var(--muted)] senders-empty">No messages received yet for this account.</div>`}
    ${
			senders.value.length > 0 &&
			html`<table class="senders-table">
      <thead><tr>
        <th class="senders-th">Sender</th><th class="senders-th">Username</th>
        <th class="senders-th">Messages</th><th class="senders-th">Last Seen</th>
        <th class="senders-th">Status</th><th class="senders-th">Action</th>
      </tr></thead>
      <tbody>
        ${senders.value.map((s) => renderSenderRow(s, onAction))}
      </tbody>
    </table>`
		}
  </div>`;
}

// ── Tag-style allowlist input ────────────────────────────────
function AllowlistInput({ value, onChange, preserveAt }) {
	var input = useSignal("");

	function addTag(raw) {
		var tag = preserveAt ? raw.trim() : raw.trim().replace(/^@/, "");
		if (tag && !value.includes(tag)) onChange([...value, tag]);
		input.value = "";
	}

	function removeTag(tag) {
		onChange(value.filter((t) => t !== tag));
	}

	function onKeyDown(e) {
		if ((e.key === "Enter" || e.key === ",") && !e.isComposing) {
			e.preventDefault();
			if (input.value.trim()) addTag(input.value);
		} else if (e.key === "Backspace" && !input.value && value.length > 0) {
			onChange(value.slice(0, -1));
		}
	}

	return html`<div class="flex flex-wrap items-center gap-1.5 rounded border border-[var(--border)] bg-[var(--surface2)] px-2 py-1.5"
    style="min-height:38px;cursor:text;"
    onClick=${(e) => e.currentTarget.querySelector("input")?.focus()}>
    ${value.map(
			(tag) => html`<span key=${tag}
        class="inline-flex items-center gap-1 rounded-full bg-[var(--accent)]/10 px-2 py-0.5 text-xs text-[var(--accent)]">
        ${tag}
        <button type="button" class="inline-flex items-center text-[var(--muted)] hover:text-[var(--accent)]"
          style="line-height:1;font-size:14px;padding:0;background:none;border:none;cursor:pointer;"
          onClick=${(e) => {
						e.stopPropagation();
						removeTag(tag);
					}}>\u00d7</button>
      </span>`,
		)}
    <input type="text" value=${input.value}
      onInput=${(e) => {
				input.value = e.target.value;
			}}
      onKeyDown=${onKeyDown}
      placeholder=${value.length === 0 ? "Type a username and press Enter" : ""}
      class="flex-1 bg-transparent text-[var(--text)] text-sm outline-none border-none"
      style="min-width:80px;padding:2px 0;font-family:var(--font-body);" />
  </div>`;
}

// ── Shared form fields (DM policy, mention mode, model, allowlist) ───
function SharedChannelFields({ addModel, allowlistItems }) {
	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return html`
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" class="channel-select">
        <option value="allowlist">Allowlist only</option>
        <option value="open">Open (anyone)</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
      <select data-field="mentionMode" class="channel-select">
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="none">Don't respond in groups</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
        onChange=${(v) => {
					addModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">DM Allowlist</label>
      <${AllowlistInput} value=${allowlistItems.value} onChange=${(v) => {
				allowlistItems.value = v;
			}} />
  `;
}

// ── Add Telegram modal ───────────────────────────────────────
function AddTelegramModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var allowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var advancedConfigPatch = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = accountDraft.value.trim();
		var credential = form.querySelector("[data-field=credential]").value.trim();
		var v = validateChannelFields("telegram", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var addConfig = {
			token: credential,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("telegram", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddTelegram.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.";
			}
		});
	}

	return html`<${Modal} show=${showAddTelegram.value} onClose=${() => {
		showAddTelegram.value = false;
	}}
	    title="Connect Telegram">
	    <div class="channel-form">
	      <div class="channel-card">
	        <div>
	          <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
	          <div class="text-xs text-[var(--muted)] channel-help">1. Open <a href="https://t.me/BotFather" target="_blank" class="text-[var(--accent)] underline">@BotFather</a> in Telegram</div>
	          <div class="text-xs text-[var(--muted)]">2. Send /newbot and follow the prompts to choose a name and username</div>
	          <div class="text-xs text-[var(--muted)]">3. Copy the bot token and paste it below</div>
	        </div>
	      </div>
	      <${ConnectionModeHint} type="telegram" />
	      <label class="text-xs text-[var(--muted)]">Bot username</label>
	      <input data-field="accountId" type="text" placeholder="e.g. my_assistant_bot"
	        value=${accountDraft.value}
	        onInput=${(e) => {
						accountDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">Bot Token (from @BotFather)</label>
	      <input data-field="credential" type="password" placeholder="123456:ABC-DEF..." class="channel-input"
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
	        name="telegram_bot_token" />
	      ${
					accountDraft.value.trim() &&
					html`<div class="flex items-center gap-1.5 text-xs py-1">
	        <span class="text-[var(--muted)]">Chat with your bot:</span>
	        <a href="https://t.me/${accountDraft.value.trim()}" target="_blank" class="text-[var(--accent)] underline">t.me/${accountDraft.value.trim()}</a>
	      </div>`
				}
	      <${SharedChannelFields} addModel=${addModel} allowlistItems=${allowlistItems} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn" onClick=${onSubmit} disabled=${saving.value}>
	        ${saving.value ? "Connecting\u2026" : "Connect Telegram"}
	      </button>
	    </div>
	  </${Modal}>`;
}

// ── Add Microsoft Teams modal ────────────────────────────────
function AddTeamsModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var allowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var webhookSecret = useSignal("");
	var baseUrlDraft = useSignal(defaultTeamsBaseUrl());
	var bootstrapEndpoint = useSignal("");
	var tsStatus = useSignal(null);
	var tsLoading = useSignal(true);
	var enablingFunnel = useSignal(false);
	var advancedConfigPatch = useSignal("");

	// Fetch Tailscale status on mount.
	useEffect(() => {
		fetch("/api/tailscale/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((data) => {
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

	function onEnableFunnel() {
		enablingFunnel.value = true;
		error.value = "";
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode: "funnel" }),
		})
			.then((r) => r.json())
			.then((data) => {
				enablingFunnel.value = false;
				if (data?.ok !== false && data?.url) {
					baseUrlDraft.value = data.url.replace(/\/$/, "");
					tsStatus.value = data;
					refreshBootstrapEndpoint();
				} else {
					error.value = data?.error || "Failed to enable Tailscale Funnel.";
				}
			})
			.catch((e) => {
				enablingFunnel.value = false;
				error.value = `Tailscale error: ${e.message}`;
			});
	}

	function refreshBootstrapEndpoint() {
		if (!bootstrapEndpoint.value) return;
		bootstrapEndpoint.value = buildTeamsEndpoint(baseUrlDraft.value, accountDraft.value, webhookSecret.value);
	}

	function onBootstrapTeams() {
		var accountId = accountDraft.value.trim();
		if (!accountId) {
			error.value = "Enter App ID / Account ID first.";
			return;
		}
		var secret = webhookSecret.value.trim();
		if (!secret) {
			secret = generateWebhookSecretHex();
			webhookSecret.value = secret;
		}
		var endpoint = buildTeamsEndpoint(baseUrlDraft.value, accountId, secret);
		if (!endpoint) {
			error.value = "Enter a valid public base URL (example: https://bot.example.com).";
			return;
		}
		bootstrapEndpoint.value = endpoint;
		error.value = "";
		showToast("Teams endpoint generated");
	}

	function copyBootstrapEndpoint() {
		if (!bootstrapEndpoint.value) return;
		if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
			showToast("Clipboard is unavailable");
			return;
		}
		navigator.clipboard.writeText(bootstrapEndpoint.value).then(() => {
			showToast("Messaging endpoint copied");
		});
	}

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = accountDraft.value.trim();
		var credential = form.querySelector("[data-field=credential]").value.trim();
		var v = validateChannelFields("msteams", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var addConfig = {
			app_id: accountId,
			app_password: credential,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: allowlistItems.value,
		};
		if (webhookSecret.value.trim()) addConfig.webhook_secret = webhookSecret.value.trim();
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("msteams", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
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
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.";
			}
		});
	}

	return html`<${Modal} show=${showAddTeams.value} onClose=${() => {
		showAddTeams.value = false;
	}}
	    title="Connect Microsoft Teams">
	    <div class="channel-form">
	      ${
					!(tsLoading.value || (tsStatus.value?.mode === "funnel" && tsStatus.value?.url)) &&
					html`
	        <div class="rounded-md border border-amber-600/30 bg-amber-50 p-3 text-xs flex flex-col gap-2">
	          <span class="font-medium text-[var(--text-strong)]">Public URL required</span>
	          <span class="text-[var(--muted)]">Teams sends messages to your server via webhook. Your Moltis instance must be reachable over HTTPS.</span>
	          ${
							tsStatus.value?.installed && tsStatus.value?.tailscale_up
								? html`<div class="flex flex-col gap-2">
	              <span class="text-[var(--muted)]">Tailscale is connected. Enable <strong>Funnel</strong> to make it publicly reachable:</span>
	              <button type="button" class="provider-btn provider-btn-sm" onClick=${onEnableFunnel} disabled=${enablingFunnel.value}>
	                ${enablingFunnel.value ? "Enabling\u2026" : "Enable Tailscale Funnel"}
	              </button>
	            </div>`
								: html`<span class="text-[var(--muted)]">Enable <strong>Tailscale Funnel</strong> in Settings, or use <a href="https://ngrok.com/" target="_blank" class="text-[var(--accent)] underline">ngrok</a> / <a href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/" target="_blank" class="text-[var(--accent)] underline">Cloudflare Tunnel</a>.</span>`
						}
	        </div>
	      `
				}
	      ${
					tsStatus.value?.mode === "funnel" &&
					tsStatus.value?.url &&
					html`
	        <div class="rounded-md border border-green-500/30 bg-green-500/5 p-3 text-xs flex items-center gap-2">
	          <span class="text-green-600">\u2713</span>
	          <span class="text-[var(--muted)]">Tailscale Funnel active \u2014 publicly reachable at <strong>${tsStatus.value.url}</strong></span>
	        </div>
	      `
				}
	      <div class="channel-card">
	        <div class="flex flex-col gap-1">
	          <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Teams bot</span>
	          <span class="text-xs font-medium text-[var(--text-strong)] opacity-70" style="font-size:10px">Option A: Teams Developer Portal (easiest)</span>
	          <div class="text-xs text-[var(--muted)]">1. Open <a href="https://dev.teams.microsoft.com/bots" target="_blank" class="text-[var(--accent)] underline">Teams Developer Portal \u2192 Bot Management</a></div>
	          <div class="text-xs text-[var(--muted)]">2. Click <strong>+ New Bot</strong>, give it a name, copy the <strong>Bot ID</strong> (App ID)</div>
	          <div class="text-xs text-[var(--muted)]">3. Under <strong>Client secrets</strong>, add a secret and copy the value (App Password)</div>
	          <span class="text-xs font-medium text-[var(--text-strong)] opacity-70" style="font-size:10px;margin-top:4px">Option B: Azure Portal</span>
	          <div class="text-xs text-[var(--muted)]">1. <a href="https://portal.azure.com/#create/Microsoft.AzureBot" target="_blank" class="text-[var(--accent)] underline">Create an Azure Bot</a>, then find App ID in Configuration</div>
	          <div class="text-xs text-[var(--muted)]">2. Click <strong>Manage Password</strong> \u2192 <strong>New client secret</strong> for the App Password</div>
	          <div class="text-xs text-[var(--muted)]" style="margin-top:4px">Then generate the endpoint below and paste it as the <strong>Messaging endpoint</strong> in your bot settings. <a href="https://docs.moltis.org/teams.html" target="_blank" class="text-[var(--accent)] underline">Full guide \u2192</a></div>
	        </div>
	      </div>
	      <${ConnectionModeHint} type="msteams" />
	      <label class="text-xs text-[var(--muted)]">App ID (Bot ID from Azure)</label>
	      <input data-field="accountId" type="text" placeholder="e.g. 12345678-abcd-efgh-ijkl-000000000000"
	        value=${accountDraft.value}
	        onInput=${(e) => {
						accountDraft.value = e.target.value;
						refreshBootstrapEndpoint();
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">App Password (client secret from Azure)</label>
	      <input data-field="credential" type="password" placeholder="Client secret value" class="channel-input"
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
	        name="teams_app_password" />
	      <div>
	        <label class="text-xs text-[var(--muted)]">Webhook Secret <span class="opacity-60">(optional \u2014 auto-generated if blank)</span></label>
	        <input type="text" placeholder="Leave blank to auto-generate" class="channel-input"
	          value=${webhookSecret.value}
	          onInput=${(e) => {
							webhookSecret.value = e.target.value;
							refreshBootstrapEndpoint();
						}} />
	        <label class="text-xs text-[var(--muted)] mt-2">Public Base URL <span class="opacity-60">(your server\u2019s HTTPS address)</span></label>
	        <input type="text" placeholder="https://bot.example.com" class="channel-input"
	          value=${baseUrlDraft.value}
	          onInput=${(e) => {
							baseUrlDraft.value = e.target.value;
							refreshBootstrapEndpoint();
						}} />
	        <div class="flex gap-2 mt-2">
	          <button type="button" class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${onBootstrapTeams}>
	            Bootstrap Teams
	          </button>
	          ${
							bootstrapEndpoint.value &&
							html`<button type="button" class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${copyBootstrapEndpoint}>
	            Copy Endpoint
	          </button>`
						}
	        </div>
	        ${
						bootstrapEndpoint.value &&
						html`<div class="mt-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2">
	          <div class="text-xs text-[var(--muted)] mb-1">Messaging endpoint \u2014 paste this into your bot\u2019s configuration:</div>
	          <code class="text-xs block break-all select-all">${bootstrapEndpoint.value}</code>
	        </div>`
					}
	        <div class="text-[10px] text-[var(--muted)] mt-1 opacity-70">Teams requires HTTPS. For local dev, use <a href="https://ngrok.com/" target="_blank" class="text-[var(--accent)] underline">ngrok</a> or <a href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/" target="_blank" class="text-[var(--accent)] underline">Cloudflare Tunnel</a>.</div>
	      </div>
	      <${SharedChannelFields} addModel=${addModel} allowlistItems=${allowlistItems} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn" onClick=${onSubmit} disabled=${saving.value}>
	        ${saving.value ? "Connecting\u2026" : "Connect Microsoft Teams"}
	      </button>
	    </div>
	  </${Modal}>`;
}

// ── Discord invite URL helper ─────────────────────────────────
function discordInviteUrl(token) {
	if (!token) return "";
	var parts = token.split(".");
	if (parts.length < 3) return "";
	try {
		var id = atob(parts[0]);
		if (!/^\d+$/.test(id)) return "";
		return `https://discord.com/oauth2/authorize?client_id=${id}&scope=bot&permissions=100352`;
	} catch {
		return "";
	}
}

// ── Add Discord modal ─────────────────────────────────────────
function AddDiscordModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var allowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var tokenDraft = useSignal("");
	var advancedConfigPatch = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = accountDraft.value.trim();
		var credential = tokenDraft.value.trim();
		var v = validateChannelFields("discord", accountId, credential);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var addConfig = {
			token: credential,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("discord", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddDiscord.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				tokenDraft.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.";
			}
		});
	}

	var inviteUrl = discordInviteUrl(tokenDraft.value);

	return html`<${Modal} show=${showAddDiscord.value} onClose=${() => {
		showAddDiscord.value = false;
	}}
	    title="Connect Discord">
	    <div class="channel-form">
	      <div class="channel-card">
	        <div>
	          <span class="text-xs font-medium text-[var(--text-strong)]">How to set up a Discord bot</span>
	          <div class="text-xs text-[var(--muted)] channel-help">1. Go to the <a href="https://discord.com/developers/applications" target="_blank" class="text-[var(--accent)] underline">Discord Developer Portal</a></div>
	          <div class="text-xs text-[var(--muted)]">2. Create a new Application \u2192 Bot tab \u2192 copy the bot token</div>
	          <div class="text-xs text-[var(--muted)]">3. Enable "Message Content Intent" under Privileged Gateway Intents</div>
	          <div class="text-xs text-[var(--muted)]">4. Paste the token below \u2014 an invite link will be generated automatically</div>
	          <div class="text-xs text-[var(--muted)]">5. You can also DM the bot directly without adding it to a server</div>
	        </div>
	      </div>
	      <${ConnectionModeHint} type="discord" />
	      <label class="text-xs text-[var(--muted)]">Account ID</label>
	      <input data-field="accountId" type="text" placeholder="e.g. my-discord-bot"
	        value=${accountDraft.value}
	        onInput=${(e) => {
						accountDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">Bot Token</label>
	      <input data-field="credential" type="password" placeholder="Discord bot token" class="channel-input"
	        value=${tokenDraft.value}
	        onInput=${(e) => {
						tokenDraft.value = e.target.value;
					}}
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
	        name="discord_bot_token" />
	      ${
					inviteUrl &&
					html`<div class="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 flex flex-col gap-1">
	        <span class="text-xs font-medium text-[var(--text-strong)]">Invite bot to a server</span>
	        <span class="text-xs text-[var(--muted)]">Open this link to add the bot (Send Messages, Attach Files, Read Message History):</span>
	        <a href=${inviteUrl} target="_blank" class="text-xs text-[var(--accent)] underline break-all">${inviteUrl}</a>
	      </div>`
				}
	      <${SharedChannelFields} addModel=${addModel} allowlistItems=${allowlistItems} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn" onClick=${onSubmit} disabled=${saving.value}>
	        ${saving.value ? "Connecting\u2026" : "Connect Discord"}
	      </button>
	    </div>
	  </${Modal}>`;
}

// ── Add Slack modal ──────────────────────────────────────────
function AddSlackModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var allowlistItems = useSignal([]);
	var channelAllowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var botTokenDraft = useSignal("");
	var appTokenDraft = useSignal("");
	var connectionMode = useSignal("socket_mode");
	var signingSecretDraft = useSignal("");
	var advancedConfigPatch = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = accountDraft.value.trim();
		var botToken = botTokenDraft.value.trim();
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
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var addConfig = {
			bot_token: botToken,
			app_token: appTokenDraft.value.trim(),
			connection_mode: connectionMode.value,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			group_policy: form.querySelector("[data-field=groupPolicy]")?.value || "open",
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: allowlistItems.value,
			channel_allowlist: channelAllowlistItems.value,
		};
		if (connectionMode.value === "events_api") {
			addConfig.signing_secret = signingSecretDraft.value.trim();
		}
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("slack", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
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
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect Slack.";
			}
		});
	}

	return html`<${Modal} show=${showAddSlack.value} onClose=${() => {
		showAddSlack.value = false;
	}}
	    title="Connect Slack">
	    <div class="channel-form">
	      <div class="channel-card">
	        <div>
	          <span class="text-xs font-medium text-[var(--text-strong)]">How to set up a Slack bot</span>
	          <div class="text-xs text-[var(--muted)] channel-help">1. Go to <a href="https://api.slack.com/apps" target="_blank" class="text-[var(--accent)] underline">api.slack.com/apps</a> and create a new app</div>
	          <div class="text-xs text-[var(--muted)]">2. Under OAuth & Permissions, add bot scopes: <code class="text-[var(--accent)]">chat:write</code>, <code class="text-[var(--accent)]">channels:history</code>, <code class="text-[var(--accent)]">im:history</code>, <code class="text-[var(--accent)]">app_mentions:read</code></div>
	          <div class="text-xs text-[var(--muted)]">3. Install the app to your workspace and copy the Bot User OAuth Token</div>
	          <div class="text-xs text-[var(--muted)]">4. For Socket Mode: enable Socket Mode and generate an App-Level Token with <code class="text-[var(--accent)]">connections:write</code> scope</div>
	          <div class="text-xs text-[var(--muted)]">5. For Events API: set the Request URL to your server\u2019s webhook endpoint</div>
	        </div>
	      </div>
	      <${ConnectionModeHint} type="slack" />
	      <label class="text-xs text-[var(--muted)]">Account ID</label>
	      <input data-field="accountId" type="text" placeholder="e.g. my-slack-bot"
	        value=${accountDraft.value}
	        onInput=${(e) => {
						accountDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">Bot Token (xoxb-...)</label>
	      <input data-field="botToken" type="password" placeholder="xoxb-..." class="channel-input"
	        value=${botTokenDraft.value}
	        onInput=${(e) => {
						botTokenDraft.value = e.target.value;
					}}
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false" />
	      <label class="text-xs text-[var(--muted)]">Connection Mode</label>
	      <select data-field="connectionMode" class="channel-select"
	        value=${connectionMode.value}
	        onChange=${(e) => {
						connectionMode.value = e.target.value;
					}}>
	        <option value="socket_mode">Socket Mode (recommended)</option>
	        <option value="events_api">Events API (HTTP webhook)</option>
	      </select>
	      ${
					connectionMode.value === "socket_mode" &&
					html`
	        <label class="text-xs text-[var(--muted)]">App Token (xapp-...)</label>
	        <input data-field="appToken" type="password" placeholder="xapp-..." class="channel-input"
	          value=${appTokenDraft.value}
	          onInput=${(e) => {
							appTokenDraft.value = e.target.value;
						}}
	          autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false" />
	      `
				}
	      ${
					connectionMode.value === "events_api" &&
					html`
	        <label class="text-xs text-[var(--muted)]">Signing Secret</label>
	        <input data-field="signingSecret" type="password" placeholder="Signing secret from Basic Information" class="channel-input"
	          value=${signingSecretDraft.value}
	          onInput=${(e) => {
							signingSecretDraft.value = e.target.value;
						}}
	          autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false" />
	      `
				}
	      <label class="text-xs text-[var(--muted)]">Group/Channel Policy</label>
	      <select data-field="groupPolicy" class="channel-select">
	        <option value="open">Open (respond in any channel)</option>
	        <option value="allowlist">Channel allowlist only</option>
	        <option value="disabled">Disabled (no channel messages)</option>
	      </select>
	      <${SharedChannelFields} addModel=${addModel} allowlistItems=${allowlistItems} />
	      <label class="text-xs text-[var(--muted)]">Channel Allowlist (Slack channel IDs)</label>
	      <${AllowlistInput} value=${channelAllowlistItems.value}
	        onChange=${(items) => {
						channelAllowlistItems.value = items;
					}} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn" onClick=${onSubmit} disabled=${saving.value}>
	        ${saving.value ? "Connecting\u2026" : "Connect Slack"}
	      </button>
	    </div>
	  </${Modal}>`;
}

// ── Add Matrix modal ─────────────────────────────────────────
function AddMatrixModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var userAllowlistItems = useSignal([]);
	var roomAllowlistItems = useSignal([]);
	var homeserverDraft = useSignal(MATRIX_DEFAULT_HOMESERVER);
	var authModeDraft = useSignal("oidc");
	var userIdDraft = useSignal("");
	var credentialDraft = useSignal("");
	var deviceDisplayNameDraft = useSignal("");
	var ownershipModeDraft = useSignal("moltis_owned");
	var oidcWaiting = useSignal(false);
	var oidcPollRef = useRef(null);
	var otpSelfApprovalDraft = useSignal(true);
	var otpCooldownDraft = useSignal("300");
	var advancedConfigPatch = useSignal("");

	function resetForm() {
		if (oidcPollRef.current) {
			clearInterval(oidcPollRef.current);
			oidcPollRef.current = null;
		}
		addModel.value = "";
		userAllowlistItems.value = [];
		roomAllowlistItems.value = [];
		homeserverDraft.value = MATRIX_DEFAULT_HOMESERVER;
		authModeDraft.value = "oidc";
		userIdDraft.value = "";
		credentialDraft.value = "";
		deviceDisplayNameDraft.value = "";
		ownershipModeDraft.value = "moltis_owned";
		otpSelfApprovalDraft.value = true;
		otpCooldownDraft.value = "300";
		advancedConfigPatch.value = "";
		oidcWaiting.value = false;
	}

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var authMode = normalizeMatrixAuthMode(authModeDraft.value);
		var credential = credentialDraft.value.trim();
		var homeserver = homeserverDraft.value.trim();
		var userId = userIdDraft.value.trim();
		var accountId = deriveMatrixAccountId({ userId, homeserver });
		var v = validateChannelFields("matrix", accountId, credential, {
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
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;

		// OIDC flow: start browser-based auth instead of direct connect.
		if (authMode === "oidc") {
			var redirectUri = window.location.origin + "/auth/callback";
			var oidcConfig = {
				homeserver: homeserver,
				ownership_mode: normalizeMatrixOwnershipMode(ownershipModeDraft.value),
				dm_policy: form.querySelector("[data-field=dmPolicy]").value,
				room_policy: form.querySelector("[data-field=roomPolicy]").value,
				mention_mode: form.querySelector("[data-field=mentionMode]").value,
				auto_join: form.querySelector("[data-field=autoJoin]").value,
				user_allowlist: userAllowlistItems.value,
				room_allowlist: roomAllowlistItems.value,
				otp_self_approval: otpSelfApprovalDraft.value,
				otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldownDraft.value),
			};
			if (deviceDisplayNameDraft.value.trim()) oidcConfig.device_display_name = deviceDisplayNameDraft.value.trim();
			if (addModel.value) {
				oidcConfig.model = addModel.value;
				var found = modelsSig.value.find((x) => x.id === addModel.value);
				if (found?.provider) oidcConfig.model_provider = found.provider;
			}
			Object.assign(oidcConfig, advancedPatch.value);
			sendRpc("channels.oauth_start", {
				account_id: accountId,
				homeserver: homeserver,
				redirect_uri: redirectUri,
				config: oidcConfig,
			}).then((res) => {
				if (res?.ok && res.payload?.auth_url) {
					oidcWaiting.value = true;
					saving.value = false;
					window.open(res.payload.auth_url, "_blank", "noopener");
					// Poll for the channel to appear.
					var pollCount = 0;
					oidcPollRef.current = setInterval(() => {
						pollCount++;
						if (pollCount > 120) { // 2 min timeout
							clearInterval(oidcPollRef.current);
							oidcPollRef.current = null;
							oidcWaiting.value = false;
							error.value = "OIDC authentication timed out. Please try again.";
							return;
						}
						fetchChannelStatus().then((statusRes) => {
							if (!statusRes?.ok) return;
							var channels = statusRes.payload?.channels || [];
							if (channels.some((ch) => ch.account_id === accountId && ch.status === "connected")) {
								clearInterval(oidcPollRef.current);
								oidcPollRef.current = null;
								oidcWaiting.value = false;
								showAddMatrix.value = false;
								resetForm();
								loadChannels();
							}
						});
					}, 1000);
				} else {
					saving.value = false;
					error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to start OIDC login.";
				}
			});
			return;
		}

		var addConfig = {
			homeserver: homeserver,
			ownership_mode: authMode === "password" || authMode === "oidc" ? normalizeMatrixOwnershipMode(ownershipModeDraft.value) : "user_managed",
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			room_policy: form.querySelector("[data-field=roomPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			auto_join: form.querySelector("[data-field=autoJoin]").value,
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
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("matrix", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddMatrix.value = false;
				resetForm();
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect Matrix.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return html`<${Modal} show=${showAddMatrix.value} onClose=${() => {
		resetForm();
		showAddMatrix.value = false;
	}}
	    title="Connect Matrix">
	    <div class="channel-form">
	      <div class="channel-card">
	        <div>
	          <span class="text-xs font-medium text-[var(--text-strong)]">Connect a Matrix bot user</span>
	          <div class="text-xs text-[var(--muted)] channel-help">1. Leave the homeserver as <span class="font-mono">${MATRIX_DEFAULT_HOMESERVER}</span> for matrix.org accounts</div>
	          <div class="text-xs text-[var(--muted)]">2. OIDC is the default for modern homeservers. Use Password for encrypted chats on older servers, or Access Token for plain traffic</div>
	          <div class="text-xs text-[var(--muted)]">3. Moltis generates the local account ID automatically from the Matrix user or homeserver</div>
	        </div>
	      </div>
	      <div class="rounded-md border border-emerald-600/30 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
	        <div class="font-medium text-emerald-800">Encrypted chats require OIDC or Password auth</div>
	        <div>${MATRIX_ENCRYPTION_GUIDANCE}</div>
	      </div>
	      <${ConnectionModeHint} type="matrix" />
	      <label class="text-xs text-[var(--muted)]">Homeserver URL</label>
	      <input data-field="homeserver" type="text" placeholder=${MATRIX_DEFAULT_HOMESERVER}
	        value=${homeserverDraft.value}
	        onInput=${(e) => {
						homeserverDraft.value = e.target.value;
					}}
	        class="channel-input"
	        autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false"
	        autofocus />
	      <label class="text-xs text-[var(--muted)]">Authentication</label>
	      <select data-field="authMode" class="channel-select"
	        value=${authModeDraft.value}
	        onChange=${(e) => {
						authModeDraft.value = normalizeMatrixAuthMode(e.target.value);
					}}>
	        <option value="oidc">OIDC (recommended)</option>
	        <option value="password">Password</option>
	        <option value="access_token">Access token</option>
	      </select>
	      <div class="text-xs text-[var(--muted)]">${matrixAuthModeGuidance(authModeDraft.value)}</div>
	      ${oidcWaiting.value && html`
	        <div class="rounded-md border border-blue-600/30 bg-blue-50 px-3 py-3 text-xs text-blue-900 flex items-center gap-2">
	          <span class="animate-spin inline-block w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full"></span>
	          <span>Waiting for OIDC authentication... Complete the login in the browser window that opened.</span>
	        </div>
	      `}
	      ${
					authModeDraft.value === "password" || authModeDraft.value === "oidc"
						? html`
	        <label class="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2">
	          <input
	            type="checkbox"
	            aria-label="Let Moltis own this Matrix account"
	            checked=${normalizeMatrixOwnershipMode(ownershipModeDraft.value) === "moltis_owned"}
	            onChange=${(e) => {
								ownershipModeDraft.value = e.target.checked ? "moltis_owned" : "user_managed";
							}} />
	          <span class="flex flex-col gap-1">
	            <span class="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
	            <span class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(
								authModeDraft.value,
								ownershipModeDraft.value,
							)}</span>
	          </span>
	        </label>`
						: html`<div class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(
								authModeDraft.value,
								"user_managed",
							)}</div>`
				}
	      ${authModeDraft.value !== "oidc" && html`
	      <label class="text-xs text-[var(--muted)]">Matrix User ID${authModeDraft.value === "password" ? " (required)" : " (optional)"}</label>
	      <input data-field="userId" type="text" placeholder="@bot:example.com"
	        value=${userIdDraft.value}
	        onInput=${(e) => {
						userIdDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">${matrixCredentialLabel(authModeDraft.value)}</label>
	      <input data-field="credential" type="password" placeholder=${matrixCredentialPlaceholder(authModeDraft.value)}
	        value=${credentialDraft.value}
	        onInput=${(e) => {
						credentialDraft.value = e.target.value;
					}}
	        class="channel-input"
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false" />
	      <div class="text-xs text-[var(--muted)]">
	        ${
						authModeDraft.value === "password"
							? html`Use the password for the dedicated Matrix bot account. This is the required mode for encrypted Matrix chats because Moltis needs to create and persist its own Matrix device keys.`
							: html`Get the access token in Element: <span class="font-mono">Settings -> Help & About -> Advanced -> Access Token</span>. Access token mode does <span class="font-medium">not</span> support encrypted Matrix chats because Moltis cannot import that existing device's private encryption keys.`
					}
	        ${" "}
	        <a href=${MATRIX_DOCS_URL} target="_blank" rel="noreferrer" class="text-[var(--accent)] underline">Matrix setup docs</a>
	      </div>
	      `}
	      <label class="text-xs text-[var(--muted)]">Device Display Name (optional)</label>
	      <input data-field="deviceDisplayName" type="text" placeholder="Moltis Matrix Bot"
	        value=${deviceDisplayNameDraft.value}
	        onInput=${(e) => {
						deviceDisplayNameDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">DM Policy</label>
	      <select data-field="dmPolicy" class="channel-select">
	        <option value="allowlist">Allowlist only</option>
	        <option value="open">Open (anyone)</option>
	        <option value="disabled">Disabled</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">Room Policy</label>
	      <select data-field="roomPolicy" class="channel-select">
	        <option value="allowlist">Room allowlist only</option>
	        <option value="open">Open (any joined room)</option>
	        <option value="disabled">Disabled</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">Room Mention Mode</label>
	      <select data-field="mentionMode" class="channel-select">
	        <option value="mention">Must mention bot</option>
	        <option value="always">Always respond</option>
	        <option value="none">Never respond in rooms</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">Invite Auto-Join</label>
	      <select data-field="autoJoin" class="channel-select">
	        <option value="always">Always join invites</option>
	        <option value="allowlist">Only when inviter or room is allowlisted</option>
	        <option value="off">Do not auto-join</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">Unknown DM Approval</label>
	      <select data-field="otpSelfApproval" class="channel-select"
	        value=${otpSelfApprovalDraft.value ? "on" : "off"}
	        onChange=${(e) => {
						otpSelfApprovalDraft.value = e.target.value !== "off";
					}}>
	        <option value="on">PIN challenge enabled (recommended)</option>
	        <option value="off">Reject unknown DMs without a PIN</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">PIN Cooldown Seconds</label>
	      <input data-field="otpCooldown" type="number" min="1" step="1" class="channel-input"
	        value=${otpCooldownDraft.value}
	        onInput=${(e) => {
						otpCooldownDraft.value = e.target.value;
					}} />
	      <div class="text-xs text-[var(--muted)]">With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.</div>
	      <label class="text-xs text-[var(--muted)]">Default Model</label>
	      <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
	        onChange=${(v) => {
						addModel.value = v;
					}}
	        placeholder=${defaultPlaceholder} />
	      <label class="text-xs text-[var(--muted)]">DM Allowlist (Matrix user IDs)</label>
	      <${AllowlistInput} value=${userAllowlistItems.value} preserveAt=${true} onChange=${(items) => {
					userAllowlistItems.value = items;
				}} />
	      <label class="text-xs text-[var(--muted)]">Room Allowlist (room IDs or aliases)</label>
	      <${AllowlistInput} value=${roomAllowlistItems.value} preserveAt=${true} onChange=${(items) => {
					roomAllowlistItems.value = items;
				}} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button key=${`mx-${saving.value}-${oidcWaiting.value}`} class="provider-btn" onClick=${onSubmit} disabled=${saving.value || oidcWaiting.value}>${saving.value ? "Connecting\u2026" : oidcWaiting.value ? "Waiting for OIDC\u2026" : authModeDraft.value === "oidc" ? "Authenticate with OIDC" : "Connect Matrix"}</button>
	    </div>
	  </${Modal}>`;
}

// ── QR code display (WhatsApp pairing) ───────────────────────
function qrSvgObjectUrl(svg) {
	if (!svg) return null;
	try {
		return URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
	} catch (_err) {
		return null;
	}
}

function QrCodeDisplay({ data, svg }) {
	var [svgUrl, setSvgUrl] = useState(null);

	useEffect(() => {
		var nextUrl = qrSvgObjectUrl(svg);
		setSvgUrl(nextUrl);
		return () => {
			if (nextUrl) URL.revokeObjectURL(nextUrl);
		};
	}, [svg]);

	if (!data)
		return html`<div class="flex items-center justify-center p-8 text-[var(--muted)] text-sm">Waiting for QR code...</div>`;

	return html`<div class="flex flex-col items-center gap-3 p-4">
    <div class="rounded-lg bg-white p-3" style="width:200px;height:200px;display:flex;align-items:center;justify-content:center;">
      ${
				svgUrl
					? html`<img src=${svgUrl} alt="WhatsApp pairing QR code" style="width:100%;height:100%;display:block;" />`
					: html`<div class="text-center text-xs text-gray-600">
        <div style="font-family:monospace;font-size:9px;word-break:break-all;max-height:180px;overflow:hidden;">${data.substring(0, 200)}</div>
      </div>`
			}
    </div>
    <div class="text-xs text-[var(--muted)] text-center">
      Scan this QR code in your terminal output,<br/>or open WhatsApp > Settings > Linked Devices > Link a Device.
    </div>
  </div>`;
}

// ── Add Nostr modal ──────────────────────────────────────────
function AddNostrModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var allowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var secretKeyDraft = useSignal("");
	var relaysDraft = useSignal("wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol");
	var advancedConfigPatch = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = accountDraft.value.trim();
		var secretKey = secretKeyDraft.value.trim();
		if (!accountId) {
			error.value = "Account ID is required.";
			return;
		}
		if (!secretKey) {
			error.value = "Secret key is required.";
			return;
		}
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var relays = relaysDraft.value
			.split(",")
			.map((r) => r.trim())
			.filter(Boolean);
		var addConfig = {
			secret_key: secretKey,
			relays: relays,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			allowed_pubkeys: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("nostr", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddNostr.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				accountDraft.value = "";
				secretKeyDraft.value = "";
				relaysDraft.value = "wss://relay.damus.io, wss://relay.nostr.band, wss://nos.lol";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect channel.";
			}
		});
	}

	return html`<${Modal} show=${showAddNostr.value} onClose=${() => {
		showAddNostr.value = false;
	}}
	    title="Connect Nostr">
	    <div class="channel-form">
	      <div class="channel-card">
	        <div>
	          <span class="text-xs font-medium text-[var(--text-strong)]">How to set up Nostr DMs</span>
	          <div class="text-xs text-[var(--muted)] channel-help">1. Generate or use an existing Nostr secret key (nsec1... or hex)</div>
	          <div class="text-xs text-[var(--muted)]">2. Configure relay URLs (defaults are provided)</div>
	          <div class="text-xs text-[var(--muted)]">3. Add allowed public keys (npub1... or hex) to the allowlist</div>
	          <div class="text-xs text-[var(--muted)]">4. Send a DM to the bot's public key from any Nostr client</div>
	        </div>
	      </div>
	      <${ConnectionModeHint} type="nostr" />
	      <label class="text-xs text-[var(--muted)]">Account ID</label>
	      <input data-field="accountId" type="text" placeholder="e.g. my-nostr-bot"
	        value=${accountDraft.value}
	        onInput=${(e) => {
						accountDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">Secret Key</label>
	      <input data-field="credential" type="password" placeholder="nsec1... or 64-char hex" class="channel-input"
	        value=${secretKeyDraft.value}
	        onInput=${(e) => {
						secretKeyDraft.value = e.target.value;
					}}
	        autocomplete="new-password" autocapitalize="none" autocorrect="off" spellcheck="false"
	        name="nostr_secret_key" />
	      <label class="text-xs text-[var(--muted)]">Relays (comma-separated)</label>
	      <input data-field="relays" type="text" placeholder="wss://relay.damus.io, wss://nos.lol"
	        value=${relaysDraft.value}
	        onInput=${(e) => {
						relaysDraft.value = e.target.value;
					}}
	        class="channel-input" />
	      <label class="text-xs text-[var(--muted)]">DM Policy</label>
	      <select data-field="dmPolicy" class="channel-select">
	        <option value="allowlist">Allowlist only</option>
	        <option value="open">Open (anyone)</option>
	        <option value="disabled">Disabled</option>
	      </select>
	      <label class="text-xs text-[var(--muted)]">Default Model</label>
	      <${ModelSelect} models=${modelsSig.value} value=${addModel.value} onChange=${(v) => {
					addModel.value = v;
				}} />
	      <label class="text-xs text-[var(--muted)]">Allowed Public Keys</label>
	      <${AllowlistInput} value=${allowlistItems.value} onChange=${(v) => {
					allowlistItems.value = v;
				}} />
	      <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
	      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn" onClick=${onSubmit} disabled=${saving.value}>
	        ${saving.value ? "Connecting\u2026" : "Connect Nostr"}
	      </button>
	    </div>
	  </${Modal}>`;
}

// ── Add WhatsApp modal ───────────────────────────────────────
function AddWhatsAppModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");
	var pairingStarted = useSignal(false);
	var allowlistItems = useSignal([]);
	var accountDraft = useSignal("");
	var advancedConfigPatch = useSignal("");
	var qrPollRef = useRef(null);

	function onStartPairing(e) {
		e.preventDefault();
		var accountId = accountDraft.value.trim() || "main";
		var form = e.target.closest(".channel-form");
		var advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
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

		var addConfig = {
			dm_policy: form.querySelector("[data-field=dmPolicy]")?.value || "open",
			allowlist: allowlistItems.value,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel("whatsapp", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				pairingStarted.value = true;
				// Poll channels.status as fallback for QR display and connection detection.
				if (qrPollRef.current) clearInterval(qrPollRef.current);
				qrPollRef.current = setInterval(async () => {
					try {
						var st = await sendRpc("channels.status");
						if (!st?.ok) return;
						var ch = (st.payload?.channels || []).find((c) => c.type === "whatsapp" && c.account_id === accountId);
						if (!ch) return;
						if (ch.status === "connected" || (waQrData.value && !ch.extra?.qr_data)) {
							clearInterval(qrPollRef.current);
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
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to start pairing.";
			}
		});
	}

	function onClose() {
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

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return html`<${Modal} show=${showAddWhatsApp.value} onClose=${onClose} title="Connect WhatsApp">
    <div class="channel-form">
      ${
				pairingStarted.value
					? html`
        <div class="flex flex-col items-center gap-4">
          ${
						waPairingError.value
							? html`<div class="text-sm text-[var(--error)]">${waPairingError.value}</div>`
							: html`<${QrCodeDisplay} data=${waQrData.value} svg=${waQrSvg.value} />`
					}
          <div class="text-xs text-[var(--muted)]">QR code refreshes automatically. Keep this window open.<br/>Only new messages will be processed — past conversations are not synced.</div>
        </div>
      `
					: html`
        <div class="channel-card">
          <div>
            <span class="text-xs font-medium text-[var(--text-strong)]">Link your WhatsApp</span>
            <div class="text-xs text-[var(--muted)] channel-help">1. Choose an account ID below (any name you like)</div>
            <div class="text-xs text-[var(--muted)]">2. Click "Start Pairing" to generate a QR code</div>
            <div class="text-xs text-[var(--muted)]">3. Open WhatsApp on your phone > Settings > Linked Devices > Link a Device</div>
            <div class="text-xs text-[var(--muted)]">4. Scan the QR code to connect</div>
          </div>
        </div>
        <${ConnectionModeHint} type="whatsapp" />
        <label class="text-xs text-[var(--muted)]">Account ID</label>
        <input data-field="accountId" type="text" placeholder="main" class="channel-input"
          value=${accountDraft.value}
          onInput=${(e) => {
						accountDraft.value = e.target.value;
					}} />
        <label class="text-xs text-[var(--muted)]">DM Policy</label>
        <select data-field="dmPolicy" class="channel-select">
          <option value="open">Open (anyone)</option>
          <option value="allowlist">Allowlist only</option>
          <option value="disabled">Disabled</option>
        </select>
        <label class="text-xs text-[var(--muted)]">Default Model</label>
        <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
          onChange=${(v) => {
						addModel.value = v;
					}}
          placeholder=${defaultPlaceholder} />
        <label class="text-xs text-[var(--muted)]">DM Allowlist</label>
        <${AllowlistInput} value=${allowlistItems.value} onChange=${(v) => {
					allowlistItems.value = v;
				}} />
        <${AdvancedConfigPatchField} value=${advancedConfigPatch.value} onInput=${(value) => {
					advancedConfigPatch.value = value;
				}} />
        ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
        <button class="provider-btn" onClick=${onStartPairing} disabled=${saving.value}>
          ${saving.value ? "Starting\u2026" : "Start Pairing"}
        </button>
      `
			}
    </div>
  </${Modal}>`;
}

// ── Edit channel modal ───────────────────────────────────────
function EditChannelModal() {
	var ch = editingChannel.value;
	var error = useSignal("");
	var saving = useSignal(false);
	var editModel = useSignal("");
	var allowlistItems = useSignal([]);
	var roomAllowlistItems = useSignal([]);
	var editCredential = useSignal("");
	var editWebhookSecret = useSignal("");
	var editStreamMode = useSignal("edit_in_place");
	var editReplyStyle = useSignal("top_level");
	var editWelcomeCard = useSignal(true);
	var editBotName = useSignal("");
	var editMatrixAuthMode = useSignal("access_token");
	var editMatrixDeviceDisplayName = useSignal("");
	var editMatrixOwnershipMode = useSignal("user_managed");
	var editMatrixOtpSelfApproval = useSignal(true);
	var editMatrixOtpCooldown = useSignal("300");
	var editAdvancedConfigPatch = useSignal("");
	useEffect(() => {
		editModel.value = ch?.config?.model || "";
		allowlistItems.value = ch?.config?.allowlist || ch?.config?.user_allowlist || ch?.config?.allowed_pubkeys || [];
		roomAllowlistItems.value = ch?.config?.room_allowlist || [];
		editCredential.value = "";
		editWebhookSecret.value = ch?.config?.webhook_secret || "";
		editStreamMode.value = ch?.config?.stream_mode || "edit_in_place";
		editReplyStyle.value = ch?.config?.reply_style || "top_level";
		editWelcomeCard.value = ch?.config?.welcome_card !== false;
		editBotName.value = ch?.config?.bot_name || "";
		editMatrixAuthMode.value = ch?.config?.password ? "password" : "access_token";
		editMatrixDeviceDisplayName.value = ch?.config?.device_display_name || "";
		editMatrixOwnershipMode.value = normalizeMatrixOwnershipMode(
			ch?.config?.ownership_mode || (ch?.config?.password ? "moltis_owned" : "user_managed"),
		);
		editMatrixOtpSelfApproval.value = ch?.config?.otp_self_approval !== false;
		editMatrixOtpCooldown.value = String(ch?.config?.otp_cooldown_secs || 300);
		editAdvancedConfigPatch.value = "";
	}, [ch]);
	if (!ch) return null;
	var cfg = ch.config || {};
	var chType = channelType(ch.type);
	var isTeams = chType === "msteams";
	var isDiscord = chType === "discord";
	var isWhatsApp = chType === "whatsapp";
	var isTelegram = chType === "telegram";
	var isMatrix = chType === "matrix";
	var isNostr = chType === "nostr";

	function addModelToConfig(config) {
		if (!editModel.value) return;
		config.model = editModel.value;
		var found = modelsSig.value.find((x) => x.id === editModel.value);
		if (found?.provider) config.model_provider = found.provider;
	}

	function addChannelCredentials(config, form) {
		if (isTeams) {
			config.app_id = cfg.app_id || ch.account_id;
			config.app_password = editCredential.value || cfg.app_password || "";
			if (editWebhookSecret.value.trim()) config.webhook_secret = editWebhookSecret.value.trim();
		} else if (isDiscord) {
			config.token = editCredential.value || cfg.token || "";
		} else if (isTelegram) {
			config.token = cfg.token || "";
		} else if (isNostr) {
			config.secret_key = editCredential.value || cfg.secret_key || "";
			var relaysVal = form.querySelector("[data-field=relays]")?.value || "";
			config.relays = relaysVal
				.split(",")
				.map((r) => r.trim())
				.filter(Boolean);
		} else if (isMatrix) {
			config.homeserver = form.querySelector("[data-field=homeserver]")?.value || cfg.homeserver || "";
			config.user_id = form.querySelector("[data-field=userId]")?.value || cfg.user_id || "";
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

	function buildUpdateConfig(form) {
		var updateConfig = {};
		updateConfig.dm_policy = form.querySelector("[data-field=dmPolicy]")?.value || "open";
		updateConfig.allowlist = allowlistItems.value;
		if (isMatrix) {
			updateConfig.user_allowlist = allowlistItems.value;
			updateConfig.room_policy = form.querySelector("[data-field=roomPolicy]")?.value || cfg.room_policy || "allowlist";
			updateConfig.auto_join = form.querySelector("[data-field=autoJoin]")?.value || cfg.auto_join || "always";
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
			updateConfig.mention_mode = form.querySelector("[data-field=mentionMode]")?.value || "mention";
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

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var advancedPatch = parseChannelConfigPatch(editAdvancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;
		var updateConfig = buildUpdateConfig(form);
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
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to update channel.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return html`<${Modal} show=${true} onClose=${() => {
		editingChannel.value = null;
	}} title=${`Edit ${channelLabel(ch.type)} Channel`}>
	    <div class="channel-form">
	      <div class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id}</div>
	      ${isTelegram && ch.account_id && html`<a href="https://t.me/${ch.account_id}" target="_blank" class="text-xs text-[var(--accent)] underline">t.me/${ch.account_id}</a>`}
	      ${
					isTeams &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">App Password (optional: leave blank to keep existing)</label>
				        <input type="password" class="channel-input w-full" value=${editCredential.value}
				          onInput=${(e) => {
										editCredential.value = e.target.value;
									}} />
				      </div>`
				}
	      ${
					isTeams &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Webhook Secret</label>
				        <input type="text" class="channel-input w-full" value=${editWebhookSecret.value}
				          onInput=${(e) => {
										editWebhookSecret.value = e.target.value;
									}} />
				      </div>
				      <div class="flex gap-3">
				        <div class="flex-1">
				          <label class="text-xs text-[var(--muted)]">Streaming</label>
				          <select class="channel-select" value=${editStreamMode.value}
				            onChange=${(e) => {
											editStreamMode.value = e.target.value;
										}}>
				            <option value="edit_in_place">Edit-in-place (live updates)</option>
				            <option value="off">Off (send once complete)</option>
				          </select>
				        </div>
				        <div class="flex-1">
				          <label class="text-xs text-[var(--muted)]">Reply Style</label>
				          <select class="channel-select" value=${editReplyStyle.value}
				            onChange=${(e) => {
											editReplyStyle.value = e.target.value;
										}}>
				            <option value="top_level">Top-level message</option>
				            <option value="thread">Reply in thread</option>
				          </select>
				        </div>
				      </div>
				      <div class="flex gap-3 items-end">
				        <div class="flex-1">
				          <label class="text-xs text-[var(--muted)]">Bot Name (for welcome card)</label>
				          <input type="text" class="channel-input" value=${editBotName.value}
				            onInput=${(e) => {
											editBotName.value = e.target.value;
										}}
				            placeholder="Moltis" />
				        </div>
				        <label class="flex items-center gap-2 text-xs text-[var(--muted)] pb-2 cursor-pointer">
				          <input type="checkbox" checked=${editWelcomeCard.value}
				            onChange=${(e) => {
											editWelcomeCard.value = e.target.checked;
										}} />
				          Welcome card
				        </label>
				      </div>`
				}
	      ${
					isDiscord &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Bot Token (optional: leave blank to keep existing)</label>
				        <input type="password" class="channel-input w-full" value=${editCredential.value}
				          onInput=${(e) => {
										editCredential.value = e.target.value;
									}} />
				      </div>`
				}
	      ${
					isNostr &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Secret Key (optional: leave blank to keep existing)</label>
				        <input type="password" class="channel-input w-full" value=${editCredential.value}
				          onInput=${(e) => {
										editCredential.value = e.target.value;
									}}
				          autocomplete="new-password" />
				      </div>
				      <div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Relays (comma-separated)</label>
				        <input data-field="relays" type="text" class="channel-input w-full"
				          defaultValue=${(cfg.relays || []).join(", ")} />
				      </div>`
				}
	      ${
					isMatrix &&
					html`<div class="rounded-md border border-emerald-600/30 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
				        <div class="font-medium text-emerald-800">Encrypted chats require OIDC or Password auth</div>
				        <div>${MATRIX_ENCRYPTION_GUIDANCE}</div>
				      </div>`
				}
	      ${
					isMatrix &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Authentication</label>
				        <select class="channel-select w-full" value=${editMatrixAuthMode.value}
				          onChange=${(e) => {
										editMatrixAuthMode.value = normalizeMatrixAuthMode(e.target.value);
									}}>
				          <option value="oidc">OIDC</option>
				          <option value="access_token">Access token</option>
				          <option value="password">Password</option>
				        </select>
				        <div class="text-xs text-[var(--muted)]">${matrixAuthModeGuidance(editMatrixAuthMode.value)}</div>
				      </div>`
				}
	      ${
					isMatrix &&
					html`
	        <div class="flex flex-col gap-1">
	          ${
							editMatrixAuthMode.value === "password"
								? html`
					<label class="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2">
					  <input
					    type="checkbox"
					    aria-label="Let Moltis own this Matrix account"
					    checked=${normalizeMatrixOwnershipMode(editMatrixOwnershipMode.value) === "moltis_owned"}
					    onChange=${(e) => {
								editMatrixOwnershipMode.value = e.target.checked ? "moltis_owned" : "user_managed";
							}} />
					  <span class="flex flex-col gap-1">
					    <span class="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
					    <span class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(
								editMatrixAuthMode.value,
								editMatrixOwnershipMode.value,
							)}</span>
					  </span>
					</label>`
								: html`<div class="text-xs text-[var(--muted)]">${matrixOwnershipModeGuidance(
										editMatrixAuthMode.value,
										"user_managed",
									)}</div>`
						}
	        </div>`
				}
	      ${
					isMatrix &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Homeserver URL</label>
				        <input data-field="homeserver" type="text" class="channel-input w-full" defaultValue=${cfg.homeserver || ""} />
				      </div>`
				}
	      ${
					isMatrix &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Matrix User ID${editMatrixAuthMode.value === "password" ? " (required)" : " (optional)"}</label>
				        <input data-field="userId" type="text" class="channel-input w-full" defaultValue=${cfg.user_id || ""} />
				      </div>`
				}
	      ${
					isMatrix &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">${matrixCredentialLabel(editMatrixAuthMode.value)} (optional: leave blank to keep existing)</label>
				        <input type="password" class="channel-input w-full" value=${editCredential.value}
				          onInput=${(e) => {
										editCredential.value = e.target.value;
									}}
				          placeholder=${matrixCredentialPlaceholder(editMatrixAuthMode.value)} />
				        <div class="text-xs text-[var(--muted)]">
				          ${
										editMatrixAuthMode.value === "password"
											? html`Password auth is required for encrypted Matrix chats because Moltis needs its own Matrix device keys.`
											: html`Access token mode does <span class="font-medium">not</span> support encrypted Matrix chats because Moltis cannot import the existing device's private encryption keys.`
									}
				          ${" "}
				          <a href=${MATRIX_DOCS_URL} target="_blank" rel="noreferrer" class="text-[var(--accent)] underline">Matrix setup docs</a>
				        </div>
				      </div>`
				}
	      ${
					isMatrix &&
					html`<div class="flex flex-col gap-1">
				        <label class="text-xs text-[var(--muted)]">Device Display Name (optional)</label>
				        <input type="text" class="channel-input w-full" value=${editMatrixDeviceDisplayName.value}
				          onInput=${(e) => {
										editMatrixDeviceDisplayName.value = e.target.value;
									}} />
				      </div>`
				}
	      <label class="text-xs text-[var(--muted)]">DM Policy</label>
	      <select data-field="dmPolicy" class="channel-select" value=${cfg.dm_policy || (isWhatsApp ? "open" : "allowlist")}>
	        ${isWhatsApp && html`<option value="open">Open (anyone)</option>`}
	        <option value="allowlist">Allowlist only</option>
	        ${!isWhatsApp && html`<option value="open">Open (anyone)</option>`}
        <option value="disabled">Disabled</option>
      </select>
      ${
				!isWhatsApp &&
				html`
        <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
        <select data-field="mentionMode" class="channel-select" value=${cfg.mention_mode || "mention"}>
          <option value="mention">Must @mention bot</option>
          <option value="always">Always respond</option>
          <option value="none">Don't respond in groups</option>
        </select>
      `
			}
      ${
				isMatrix &&
				html`
        <label class="text-xs text-[var(--muted)]">Unknown DM Approval</label>
        <select class="channel-select" value=${editMatrixOtpSelfApproval.value ? "on" : "off"}
          onChange=${(e) => {
						editMatrixOtpSelfApproval.value = e.target.value !== "off";
					}}>
          <option value="on">PIN challenge enabled (recommended)</option>
          <option value="off">Reject unknown DMs without a PIN</option>
        </select>
        <label class="text-xs text-[var(--muted)]">PIN Cooldown Seconds</label>
        <input type="number" min="1" step="1" class="channel-input"
          value=${editMatrixOtpCooldown.value}
          onInput=${(e) => {
						editMatrixOtpCooldown.value = e.target.value;
					}} />
        <div class="text-xs text-[var(--muted)]">With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.</div>
        <label class="text-xs text-[var(--muted)]">Room Policy</label>
        <select data-field="roomPolicy" class="channel-select" value=${cfg.room_policy || "allowlist"}>
          <option value="allowlist">Room allowlist only</option>
          <option value="open">Open (any joined room)</option>
          <option value="disabled">Disabled</option>
        </select>
        <label class="text-xs text-[var(--muted)]">Invite Auto-Join</label>
        <select data-field="autoJoin" class="channel-select" value=${cfg.auto_join || "always"}>
          <option value="always">Always join invites</option>
          <option value="allowlist">Only when inviter or room is allowlisted</option>
          <option value="off">Do not auto-join</option>
        </select>
      `
			}
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${editModel.value}
        onChange=${(v) => {
					editModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">DM Allowlist</label>
      <${AllowlistInput} value=${allowlistItems.value} preserveAt=${isMatrix} onChange=${(v) => {
				allowlistItems.value = v;
			}} />
      ${
				isMatrix &&
				html`
        <label class="text-xs text-[var(--muted)]">Room Allowlist</label>
        <${AllowlistInput} value=${roomAllowlistItems.value} preserveAt=${true} onChange=${(v) => {
					roomAllowlistItems.value = v;
				}} />
      `
			}
	      <${AdvancedConfigPatchField} value=${editAdvancedConfigPatch.value}
	        onInput=${(value) => {
						editAdvancedConfigPatch.value = value;
					}}
	        currentConfig=${cfg} />
      ${error.value && html`<div class="text-xs text-[var(--error)] py-1">${error.value}</div>`}
	      <button class="provider-btn"
	        onClick=${onSave} disabled=${saving.value}>
	        ${saving.value ? "Saving\u2026" : "Save Changes"}
	      </button>
    </div>
  </${Modal}>`;
}

// ── Channel event handlers ───────────────────────────────────
function handleWhatsAppPairingEvent(p) {
	if (p.kind === "pairing_qr_code" && p.account_id === waPairingAccountId.value) {
		waQrData.value = p.qr_data;
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

function handleChannelEvent(p) {
	if (p.kind === "otp_resolved") {
		loadChannels();
	}
	handleWhatsAppPairingEvent(p);
	if (p.kind === "pairing_complete" || p.kind === "account_disabled" || p.kind === "status_changed") {
		loadChannels();
	}
	var selected = parseSenderSelectionKey(sendersAccount.value || "");
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
function ChannelsPage() {
	useEffect(() => {
		S.setRefreshChannelsPage(loadChannels);
		// Use prefetched cache for instant render
		if (S.cachedChannels !== null) channels.value = S.cachedChannels;
		if (connected.value) loadChannels();

		var unsub = onEvent("channel", handleChannelEvent);
		S.setChannelEventUnsub(unsub);

		return () => {
			S.setRefreshChannelsPage(null);
			if (unsub) unsub();
			S.setChannelEventUnsub(null);
		};
	}, [connected.value]);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3 flex-wrap">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Channels</h2>
        <div style="display:flex;gap:4px;margin-left:12px;">
          <button class="session-action-btn" style=${activeTab.value === "channels" ? "font-weight:600;" : ""}
            onClick=${() => {
							activeTab.value = "channels";
						}}>Channels</button>
          <button class="session-action-btn" style=${activeTab.value === "senders" ? "font-weight:600;" : ""}
            onClick=${() => {
							activeTab.value = "senders";
						}}>Senders</button>
        </div>
        ${activeTab.value === "channels" && channels.value.length > 0 && html`<${ConnectButtons} />`}
      </div>
      ${activeTab.value === "channels" && html`<${ChannelStorageNotice} />`}
      ${activeTab.value === "channels" ? html`<${ChannelsTab} />` : html`<${SendersTab} />`}
    </div>
    <${AddTelegramModal} />
    <${AddTeamsModal} />
    <${AddDiscordModal} />
    <${AddSlackModal} />
    <${AddMatrixModal} />
    <${AddNostrModal} />
    <${AddWhatsAppModal} />
    <${EditChannelModal} />
    <${ConfirmDialog} />
  `;
}

var _channelsContainer = null;

export function initChannels(container) {
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
	render(html`<${ChannelsPage} />`, container);
}

export function teardownChannels() {
	S.setRefreshChannelsPage(null);
	if (S.channelEventUnsub) {
		S.channelEventUnsub();
		S.setChannelEventUnsub(null);
	}
	if (_channelsContainer) render(null, _channelsContainer);
	_channelsContainer = null;
}
