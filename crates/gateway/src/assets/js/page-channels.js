// ── Channels page (Preact + HTM + Signals) ──────────────────

import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { addChannel, fetchChannelStatus, validateChannelFields } from "./channel-utils.js";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { navigate, registerPrefix } from "./router.js";
import { connected } from "./signals.js";
import * as S from "./state.js";
import { models as modelsSig } from "./stores/model-store.js";
import { ConfirmDialog, Modal, ModelSelect, requestConfirm, showToast } from "./ui.js";

var channels = signal([]);
var containerRef = null;

export function prefetchChannels() {
	fetchChannelStatus().then((res) => {
		if (res?.ok) {
			var ch = res.payload?.channels || [];
			channels.value = ch;
			S.setCachedChannels(ch);
		}
	});
}
var senders = signal([]);
var activeSection = signal("list");
var showAddModal = signal(null); // null, "telegram", "slack", or "discord"
var editingChannel = signal(null);
var sendersAccount = signal("");

// ── Sidebar navigation ───────────────────────────────────────
var sections = [
	{
		id: "list",
		label: "Channels",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M8.625 12a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Zm0 0H8.25m4.125 0a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Zm0 0H12m4.125 0a.375.375 0 1 1-.75 0 .375.375 0 0 1 .75 0Zm0 0h-.375M21 12c0 4.556-4.03 8.25-9 8.25a9.764 9.764 0 0 1-2.555-.337A5.972 5.972 0 0 1 5.41 20.97a5.969 5.969 0 0 1-.474-.065 4.48 4.48 0 0 0 .978-2.025c.09-.457-.133-.901-.467-1.226C3.93 16.178 3 14.189 3 12c0-4.556 4.03-8.25 9-8.25s9 3.694 9 8.25Z"/></svg>`,
	},
	{
		id: "senders",
		label: "Senders",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M15 19.128a9.38 9.38 0 0 0 2.625.372 9.337 9.337 0 0 0 4.121-.952 4.125 4.125 0 0 0-7.533-2.493M15 19.128v-.003c0-1.113-.285-2.16-.786-3.07M15 19.128v.106A12.318 12.318 0 0 1 8.624 21c-2.331 0-4.512-.645-6.374-1.766l-.001-.109a6.375 6.375 0 0 1 11.964-3.07M12 6.375a3.375 3.375 0 1 1-6.75 0 3.375 3.375 0 0 1 6.75 0Zm8.25 2.25a2.625 2.625 0 1 1-5.25 0 2.625 2.625 0 0 1 5.25 0Z"/></svg>`,
	},
];

function ChannelsSidebar() {
	return html`<div class="settings-sidebar">
		<div class="settings-sidebar-nav">
			${sections.map(
				(s) => html`
				<button
					key=${s.id}
					class="settings-nav-item ${activeSection.value === s.id ? "active" : ""}"
					onClick=${() => {
						navigate(`/channels/${s.id}`);
					}}
				>
					${s.icon}
					${s.label}
				</button>
			`,
			)}
		</div>
	</div>`;
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
	var accountId = sendersAccount.value;
	if (!accountId) {
		senders.value = [];
		return;
	}
	sendRpc("channels.senders.list", { account_id: accountId }).then((res) => {
		if (res?.ok) senders.value = res.payload?.senders || [];
	});
}

// ── Channel icons ────────────────────────────────────────────
function TelegramIcon() {
	return html`<span class="icon icon-telegram"></span>`;
}

function SlackIcon() {
	return html`<svg width="16" height="16" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="1.5">
    <path d="M14.5 10c-.83 0-1.5-.67-1.5-1.5v-5c0-.83.67-1.5 1.5-1.5s1.5.67 1.5 1.5v5c0 .83-.67 1.5-1.5 1.5z" />
    <path d="M20.5 10H19V8.5c0-.83.67-1.5 1.5-1.5s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z" />
    <path d="M9.5 14c.83 0 1.5.67 1.5 1.5v5c0 .83-.67 1.5-1.5 1.5S8 21.33 8 20.5v-5c0-.83.67-1.5 1.5-1.5z" />
    <path d="M3.5 14H5v1.5c0 .83-.67 1.5-1.5 1.5S2 16.33 2 15.5 2.67 14 3.5 14z" />
    <path d="M14 14.5c0-.83.67-1.5 1.5-1.5h5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5h-5c-.83 0-1.5-.67-1.5-1.5z" />
    <path d="M15.5 19H14v1.5c0 .83.67 1.5 1.5 1.5s1.5-.67 1.5-1.5-.67-1.5-1.5-1.5z" />
    <path d="M10 9.5C10 8.67 9.33 8 8.5 8h-5C2.67 8 2 8.67 2 9.5S2.67 11 3.5 11h5c.83 0 1.5-.67 1.5-1.5z" />
    <path d="M8.5 5H10V3.5C10 2.67 9.33 2 8.5 2S7 2.67 7 3.5 7.67 5 8.5 5z" />
  </svg>`;
}

function DiscordIcon() {
	return html`<svg width="16" height="16" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" stroke-width="1.5">
    <path d="M9 11.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z" />
    <path d="M15 11.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z" />
    <path d="M7.5 4.5C5.5 5.5 4 7 3.5 9c-.5 2-.5 4 0 6 .5 2 2 3.5 4 4.5l1-2" />
    <path d="M16.5 4.5c2 1 3.5 2.5 4 4.5.5 2 .5 4 0 6-.5 2-2 3.5-4 4.5l-1-2" />
    <path d="M8.5 4.5c1-.5 2.3-.5 3.5-.5s2.5 0 3.5.5" />
    <path d="M8.5 17.5c1 .5 2.3.5 3.5.5s2.5 0 3.5-.5" />
  </svg>`;
}

function ChannelIcon({ type }) {
	if (type === "slack") return html`<${SlackIcon} />`;
	if (type === "discord") return html`<${DiscordIcon} />`;
	return html`<${TelegramIcon} />`;
}

// ── Channel card ─────────────────────────────────────────────
function ChannelCard(props) {
	var ch = props.channel;
	var channelType = ch.type || "telegram";

	function onRemove() {
		requestConfirm(`Remove ${ch.name || ch.account_id}?`).then((yes) => {
			if (!yes) return;
			sendRpc("channels.remove", { account_id: ch.account_id, type: channelType }).then((r) => {
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

	var typeLabel = channelType.charAt(0).toUpperCase() + channelType.slice(1);

	return html`<div class="provider-card" style="padding:12px 14px;border-radius:8px;margin-bottom:8px;">
    <div style="display:flex;align-items:center;gap:10px;">
      <span style="display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;border-radius:6px;background:var(--surface2);">
        <${ChannelIcon} type=${channelType} />
      </span>
      <div style="display:flex;flex-direction:column;gap:2px;">
        <span class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id || typeLabel}</span>
        ${ch.details && html`<span class="text-xs text-[var(--muted)]">${ch.details}</span>`}
        ${sessionLine && html`<span class="text-xs text-[var(--muted)]">${sessionLine}</span>`}
      </div>
      <span class="provider-item-badge ${statusClass}">${ch.status || "unknown"}</span>
    </div>
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

// ── Channels tab ─────────────────────────────────────────────
function ChannelsTab() {
	if (channels.value.length === 0) {
		return html`<div style="text-align:center;padding:40px 0;">
      <div class="text-sm text-[var(--muted)]" style="margin-bottom:12px;">No channels connected.</div>
      <div class="text-xs text-[var(--muted)]">Click "Connect Channel" to add Telegram, Slack, or Discord.</div>
    </div>`;
	}
	return html`${channels.value.map((ch) => html`<${ChannelCard} key=${ch.account_id} channel=${ch} />`)}`;
}

// ── Senders tab ──────────────────────────────────────────────
function SendersTab() {
	useEffect(() => {
		if (channels.value.length > 0 && !sendersAccount.value) {
			sendersAccount.value = channels.value[0].account_id;
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
		sendRpc(rpc, {
			account_id: sendersAccount.value,
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
					(ch) => html`<option key=${ch.account_id} value=${ch.account_id}>${ch.name || ch.account_id}</option>`,
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
        ${senders.value.map((s) => {
					var identifier = s.username || s.peer_id;
					var lastSeenMs = s.last_seen ? s.last_seen * 1000 : 0;
					return html`<tr key=${s.peer_id}>
            <td class="senders-td">${s.sender_name || s.peer_id}</td>
            <td class="senders-td" style="color:var(--muted);">${s.username ? `@${s.username}` : "\u2014"}</td>
            <td class="senders-td">${s.message_count}</td>
            <td class="senders-td" style="color:var(--muted);font-size:12px;">${lastSeenMs ? html`<time data-epoch-ms="${lastSeenMs}">${new Date(lastSeenMs).toISOString()}</time>` : "\u2014"}</td>
            <td class="senders-td">
              ${
								s.otp_pending
									? html`<span class="provider-item-badge cursor-pointer select-none" style="background:var(--warning-bg, #fef3c7);color:var(--warning-text, #92400e);" onClick=${() => {
											navigator.clipboard.writeText(s.otp_pending.code).then(() => showToast("OTP code copied"));
										}}>OTP: <code class="text-xs">${s.otp_pending.code}</code></span>`
									: html`<span class="provider-item-badge ${s.allowed ? "configured" : "oauth"}">${s.allowed ? "Allowed" : "Denied"}</span>`
							}
            </td>
            <td class="senders-td">
              ${
								s.allowed
									? html`<button class="provider-btn provider-btn-sm provider-btn-danger" onClick=${() => onAction(identifier, "deny")}>Deny</button>`
									: html`<button class="provider-btn provider-btn-sm" onClick=${() => onAction(identifier, "approve")}>Approve</button>`
							}
            </td>
          </tr>`;
				})}
      </tbody>
    </table>`
		}
  </div>`;
}

// ── Connect channel dropdown ─────────────────────────────────
function ConnectChannelDropdown() {
	var showMenu = useSignal(false);

	function selectChannel(type) {
		showMenu.value = false;
		showAddModal.value = type;
	}

	return html`<div style="position:relative;">
    <button class="provider-btn"
      onClick=${() => {
				if (connected.value) showMenu.value = !showMenu.value;
			}}>
      + Connect Channel
    </button>
    ${
			showMenu.value &&
			html`<div class="dropdown-menu" style="position:absolute;top:100%;left:0;margin-top:4px;background:var(--surface);border:1px solid var(--border);border-radius:6px;box-shadow:0 4px 12px rgba(0,0,0,0.15);z-index:100;min-width:180px;">
        <button class="dropdown-item" style="display:flex;align-items:center;gap:8px;width:100%;padding:10px 14px;border:none;background:none;color:var(--text);cursor:pointer;text-align:left;font-size:14px;"
          onClick=${() => selectChannel("telegram")}
          onMouseOver=${(e) => {
						e.currentTarget.style.background = "var(--surface2)";
					}}
          onMouseOut=${(e) => {
						e.currentTarget.style.background = "none";
					}}>
          <${TelegramIcon} /> Telegram
        </button>
        <button class="dropdown-item" style="display:flex;align-items:center;gap:8px;width:100%;padding:10px 14px;border:none;background:none;color:var(--text);cursor:pointer;text-align:left;font-size:14px;"
          onClick=${() => selectChannel("slack")}
          onMouseOver=${(e) => {
						e.currentTarget.style.background = "var(--surface2)";
					}}
          onMouseOut=${(e) => {
						e.currentTarget.style.background = "none";
					}}>
          <${SlackIcon} /> Slack
        </button>
        <button class="dropdown-item" style="display:flex;align-items:center;gap:8px;width:100%;padding:10px 14px;border:none;background:none;color:var(--text);cursor:pointer;text-align:left;font-size:14px;"
          onClick=${() => selectChannel("discord")}
          onMouseOver=${(e) => {
						e.currentTarget.style.background = "var(--surface2)";
					}}
          onMouseOut=${(e) => {
						e.currentTarget.style.background = "none";
					}}>
          <${DiscordIcon} /> Discord
        </button>
      </div>`
		}
  </div>`;
}

// ── Add Telegram modal ───────────────────────────────────────
function AddTelegramModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = form.querySelector("[data-field=accountId]").value.trim();
		var token = form.querySelector("[data-field=token]").value.trim();
		var v = validateChannelFields(accountId, token);
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		var allowlist = form
			.querySelector("[data-field=allowlist]")
			.value.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		error.value = "";
		saving.value = true;
		var addConfig = {
			token: token,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: allowlist,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		addChannel("telegram", accountId, addConfig).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddModal.value = null;
				addModel.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect bot.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	var selectStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";
	var inputStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;";

	return html`<${Modal} show=${showAddModal.value === "telegram"} onClose=${() => {
		showAddModal.value = null;
	}} title="Connect Telegram Bot">
    <div class="channel-form">
      <div class="channel-card">
        <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
        <div class="text-xs text-[var(--muted)] channel-help">1. Open <a href="https://t.me/BotFather" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">@BotFather</a> in Telegram</div>
        <div class="text-xs text-[var(--muted)]">2. Send /newbot and follow the prompts to choose a name and username</div>
        <div class="text-xs text-[var(--muted)]">3. Copy the bot token (looks like 123456:ABC-DEF...) and paste it below</div>
        <div class="text-xs text-[var(--muted)] channel-help" style="margin-top:2px;">See the <a href="https://core.telegram.org/bots/tutorial" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">Telegram Bot Tutorial</a> for more details.</div>
      </div>
	      <label class="text-xs text-[var(--muted)]">Bot username</label>
	      <input data-field="accountId" type="text" placeholder="e.g. my_assistant_bot" style=${inputStyle} />
	      <label class="text-xs text-[var(--muted)]">Bot Token (from @BotFather)</label>
	      <input data-field="token" type="password" placeholder="123456:ABC-DEF..." style=${inputStyle}
	        autocomplete="new-password"
	        autocapitalize="none"
	        autocorrect="off"
	        spellcheck="false"
	        name="telegram_bot_token" />
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${selectStyle}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
      <select data-field="mentionMode" style=${selectStyle}>
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
      <label class="text-xs text-[var(--muted)]">DM Allowlist (one username per line)</label>
      <textarea data-field="allowlist" placeholder="user1\nuser2" rows="3"
        style="font-family:var(--font-body);resize:vertical;background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;" />
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn"
        onClick=${onSubmit} disabled=${saving.value}>
        ${saving.value ? "Connecting\u2026" : "Connect Bot"}
      </button>
    </div>
  </${Modal}>`;
}

// ── Add Slack modal ──────────────────────────────────────────
function AddSlackModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = form.querySelector("[data-field=accountId]").value.trim();
		var botToken = form.querySelector("[data-field=botToken]").value.trim();
		var appToken = form.querySelector("[data-field=appToken]").value.trim();
		if (!accountId) {
			error.value = "Workspace name is required.";
			return;
		}
		if (!botToken) {
			error.value = "Bot token is required.";
			return;
		}
		if (!appToken) {
			error.value = "App-level token is required for Socket Mode.";
			return;
		}
		var userAllowlist = form
			.querySelector("[data-field=userAllowlist]")
			.value.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		error.value = "";
		saving.value = true;
		var addConfig = {
			bot_token: botToken,
			app_token: appToken,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			channel_policy: form.querySelector("[data-field=channelPolicy]").value,
			activation_mode: form.querySelector("[data-field=activationMode]").value,
			user_allowlist: userAllowlist,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		sendRpc("channels.add", {
			type: "slack",
			account_id: accountId,
			config: addConfig,
		}).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddModal.value = null;
				addModel.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect Slack.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	var selectStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";
	var inputStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;";

	return html`<${Modal} show=${showAddModal.value === "slack"} onClose=${() => {
		showAddModal.value = null;
	}} title="Connect Slack Workspace">
    <div class="channel-form">
      <div class="channel-card">
        <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Slack app</span>
        <div class="text-xs text-[var(--muted)] channel-help">1. Go to <a href="https://api.slack.com/apps" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">api.slack.com/apps</a> and create a new app "from scratch"</div>
        <div class="text-xs text-[var(--muted)]">2. Enable <b>Socket Mode</b> and generate an App-Level Token with <code>connections:write</code> scope</div>
        <div class="text-xs text-[var(--muted)]">3. In <b>OAuth & Permissions</b>, add Bot Token Scopes: chat:write, im:write, channels:history, im:history, users:read, reactions:read</div>
        <div class="text-xs text-[var(--muted)]">4. In <b>Event Subscriptions</b>, subscribe to: message.im, app_mention, reaction_added</div>
        <div class="text-xs text-[var(--muted)]">5. Install to workspace and copy the Bot Token (xoxb-) and App Token (xapp-)</div>
        <div class="text-xs text-[var(--muted)] channel-help" style="margin-top:2px;">See the <a href="https://docs.openclaw.ai/channels/slack" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">OpenClaw Slack docs</a> for the full setup guide.</div>
      </div>
      <label class="text-xs text-[var(--muted)]">Workspace name (identifier)</label>
      <input data-field="accountId" type="text" placeholder="e.g. my-company" style=${inputStyle} />
      <label class="text-xs text-[var(--muted)]">Bot Token (xoxb-...)</label>
      <input data-field="botToken" type="password" placeholder="xoxb-..." style=${inputStyle} />
      <label class="text-xs text-[var(--muted)]">App-Level Token (xapp-...)</label>
      <input data-field="appToken" type="password" placeholder="xapp-..." style=${inputStyle} />
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${selectStyle}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Channel Policy</label>
      <select data-field="channelPolicy" style=${selectStyle}>
        <option value="open">Open (all channels)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Activation Mode</label>
      <select data-field="activationMode" style=${selectStyle}>
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="thread_only">Thread only</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
        onChange=${(v) => {
					addModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">User Allowlist (one Slack user ID per line)</label>
      <textarea data-field="userAllowlist" placeholder="U01234567\nU98765432" rows="3"
        style="font-family:var(--font-body);resize:vertical;background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;" />
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn"
        onClick=${onSubmit} disabled=${saving.value}>
        ${saving.value ? "Connecting\u2026" : "Connect Workspace"}
      </button>
    </div>
  </${Modal}>`;
}

// ── Add Discord modal ────────────────────────────────────────
function AddDiscordModal() {
	var error = useSignal("");
	var saving = useSignal(false);
	var addModel = useSignal("");

	function onSubmit(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		var accountId = form.querySelector("[data-field=accountId]").value.trim();
		var token = form.querySelector("[data-field=token]").value.trim();
		if (!accountId) {
			error.value = "Bot name is required.";
			return;
		}
		if (!token) {
			error.value = "Bot token is required.";
			return;
		}
		var userAllowlist = form
			.querySelector("[data-field=userAllowlist]")
			.value.trim()
			.split(/\n/)
			.map((s) => s.trim())
			.filter(Boolean);
		error.value = "";
		saving.value = true;
		var addConfig = {
			token: token,
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			guild_policy: form.querySelector("[data-field=guildPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			user_allowlist: userAllowlist,
		};
		if (addModel.value) {
			addConfig.model = addModel.value;
			var found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		sendRpc("channels.add", {
			type: "discord",
			account_id: accountId,
			config: addConfig,
		}).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showAddModal.value = null;
				addModel.value = "";
				loadChannels();
			} else {
				error.value = (res?.error && (res.error.message || res.error.detail)) || "Failed to connect Discord.";
			}
		});
	}

	var defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	var selectStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";
	var inputStyle =
		"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;";

	return html`<${Modal} show=${showAddModal.value === "discord"} onClose=${() => {
		showAddModal.value = null;
	}} title="Connect Discord Bot">
    <div class="channel-form">
      <div class="channel-card">
        <span class="text-xs font-medium text-[var(--text-strong)]">How to create a Discord bot</span>
        <div class="text-xs text-[var(--muted)] channel-help">1. Go to <a href="https://discord.com/developers/applications" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">Discord Developer Portal</a> and create a new application</div>
        <div class="text-xs text-[var(--muted)]">2. Go to <b>Bot</b> section, create a bot and copy the token</div>
        <div class="text-xs text-[var(--muted)]">3. Enable <b>MESSAGE CONTENT INTENT</b> and <b>SERVER MEMBERS INTENT</b> (both required)</div>
        <div class="text-xs text-[var(--muted)]">4. Go to <b>OAuth2 > URL Generator</b>, select <code>bot</code> + <code>applications.commands</code> scopes</div>
        <div class="text-xs text-[var(--muted)]">5. Select permissions: View Channels, Send Messages, Read Message History, Embed Links</div>
        <div class="text-xs text-[var(--muted)] channel-help" style="margin-top:2px;">See the <a href="https://docs.openclaw.ai/channels/discord" target="_blank" class="text-[var(--accent)]" style="text-decoration:underline;">OpenClaw Discord docs</a> for the full setup guide.</div>
      </div>
      <label class="text-xs text-[var(--muted)]">Bot name (identifier)</label>
      <input data-field="accountId" type="text" placeholder="e.g. my-assistant" style=${inputStyle} />
      <label class="text-xs text-[var(--muted)]">Bot Token</label>
      <input data-field="token" type="password" placeholder="Bot token from Developer Portal" style=${inputStyle} />
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${selectStyle}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Server (Guild) Policy</label>
      <select data-field="guildPolicy" style=${selectStyle}>
        <option value="open">Open (all servers)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Mention Mode</label>
      <select data-field="mentionMode" style=${selectStyle}>
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="none">Don't respond in servers</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${addModel.value}
        onChange=${(v) => {
					addModel.value = v;
				}}
        placeholder=${defaultPlaceholder} />
      <label class="text-xs text-[var(--muted)]">User Allowlist (one Discord user ID per line)</label>
      <textarea data-field="userAllowlist" placeholder="123456789012345678" rows="3"
        style="font-family:var(--font-body);resize:vertical;background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;" />
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn"
        onClick=${onSubmit} disabled=${saving.value}>
        ${saving.value ? "Connecting\u2026" : "Connect Bot"}
      </button>
    </div>
  </${Modal}>`;
}

// ── Edit channel modals (split by type to reduce complexity) ─
var EDIT_SELECT_STYLE =
	"font-family:var(--font-body);background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;cursor:pointer;";
var EDIT_TEXTAREA_STYLE =
	"font-family:var(--font-body);resize:vertical;background:var(--surface2);color:var(--text);border:1px solid var(--border);border-radius:4px;padding:8px 12px;font-size:.85rem;";

function parseAllowlist(form, fieldName) {
	return form
		.querySelector(`[data-field=${fieldName}]`)
		.value.trim()
		.split(/\n/)
		.map((s) => s.trim())
		.filter(Boolean);
}

function getModelPlaceholder() {
	return modelsSig.value.length > 0
		? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
		: "(server default)";
}

function saveChannelUpdate(ch, channelType, updateConfig, editModelValue, saving, error) {
	if (editModelValue) {
		updateConfig.model = editModelValue;
		var found = modelsSig.value.find((x) => x.id === editModelValue);
		if (found?.provider) updateConfig.model_provider = found.provider;
	}
	sendRpc("channels.update", {
		account_id: ch.account_id,
		type: channelType,
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

function EditTelegramModal({ ch }) {
	var cfg = ch.config || {};
	var error = useSignal("");
	var saving = useSignal(false);
	var editModel = useSignal(cfg.model || "");

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		error.value = "";
		saving.value = true;
		var updateConfig = {
			token: cfg.token || "",
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			allowlist: parseAllowlist(form, "allowlist"),
		};
		saveChannelUpdate(ch, "telegram", updateConfig, editModel.value, saving, error);
	}

	return html`<${Modal} show=${true} onClose=${() => {
		editingChannel.value = null;
	}} title="Edit Telegram Bot">
    <div class="channel-form">
      <div class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id}</div>
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${EDIT_SELECT_STYLE} value=${cfg.dm_policy || "open"}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Group Mention Mode</label>
      <select data-field="mentionMode" style=${EDIT_SELECT_STYLE} value=${cfg.mention_mode || "mention"}>
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="none">Don't respond in groups</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${editModel.value}
        onChange=${(v) => {
					editModel.value = v;
				}}
        placeholder=${getModelPlaceholder()} />
      <label class="text-xs text-[var(--muted)]">DM Allowlist (one username per line)</label>
      <textarea data-field="allowlist" rows="3" style=${EDIT_TEXTAREA_STYLE}>${(cfg.allowlist || []).join("\n")}</textarea>
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn" onClick=${onSave} disabled=${saving.value}>
        ${saving.value ? "Saving\u2026" : "Save Changes"}
      </button>
    </div>
  </${Modal}>`;
}

function EditSlackModal({ ch }) {
	var cfg = ch.config || {};
	var error = useSignal("");
	var saving = useSignal(false);
	var editModel = useSignal(cfg.model || "");

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		error.value = "";
		saving.value = true;
		var updateConfig = {
			bot_token: cfg.bot_token || "",
			app_token: cfg.app_token || "",
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			channel_policy: form.querySelector("[data-field=channelPolicy]").value,
			activation_mode: form.querySelector("[data-field=activationMode]").value,
			user_allowlist: parseAllowlist(form, "userAllowlist"),
		};
		saveChannelUpdate(ch, "slack", updateConfig, editModel.value, saving, error);
	}

	return html`<${Modal} show=${true} onClose=${() => {
		editingChannel.value = null;
	}} title="Edit Slack Workspace">
    <div class="channel-form">
      <div class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id}</div>
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${EDIT_SELECT_STYLE} value=${cfg.dm_policy || "open"}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Channel Policy</label>
      <select data-field="channelPolicy" style=${EDIT_SELECT_STYLE} value=${cfg.channel_policy || "open"}>
        <option value="open">Open (all channels)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Activation Mode</label>
      <select data-field="activationMode" style=${EDIT_SELECT_STYLE} value=${cfg.activation_mode || "mention"}>
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="thread_only">Thread only</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${editModel.value}
        onChange=${(v) => {
					editModel.value = v;
				}}
        placeholder=${getModelPlaceholder()} />
      <label class="text-xs text-[var(--muted)]">User Allowlist (one Slack user ID per line)</label>
      <textarea data-field="userAllowlist" rows="3" style=${EDIT_TEXTAREA_STYLE}>${(cfg.user_allowlist || []).join("\n")}</textarea>
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn" onClick=${onSave} disabled=${saving.value}>
        ${saving.value ? "Saving\u2026" : "Save Changes"}
      </button>
    </div>
  </${Modal}>`;
}

function EditDiscordModal({ ch }) {
	var cfg = ch.config || {};
	var error = useSignal("");
	var saving = useSignal(false);
	var editModel = useSignal(cfg.model || "");

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".channel-form");
		error.value = "";
		saving.value = true;
		var updateConfig = {
			token: cfg.token || "",
			dm_policy: form.querySelector("[data-field=dmPolicy]").value,
			guild_policy: form.querySelector("[data-field=guildPolicy]").value,
			mention_mode: form.querySelector("[data-field=mentionMode]").value,
			user_allowlist: parseAllowlist(form, "userAllowlist"),
		};
		saveChannelUpdate(ch, "discord", updateConfig, editModel.value, saving, error);
	}

	return html`<${Modal} show=${true} onClose=${() => {
		editingChannel.value = null;
	}} title="Edit Discord Bot">
    <div class="channel-form">
      <div class="text-sm text-[var(--text-strong)]">${ch.name || ch.account_id}</div>
      <label class="text-xs text-[var(--muted)]">DM Policy</label>
      <select data-field="dmPolicy" style=${EDIT_SELECT_STYLE} value=${cfg.dm_policy || "open"}>
        <option value="open">Open (anyone)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Server (Guild) Policy</label>
      <select data-field="guildPolicy" style=${EDIT_SELECT_STYLE} value=${cfg.guild_policy || "open"}>
        <option value="open">Open (all servers)</option>
        <option value="allowlist">Allowlist only</option>
        <option value="disabled">Disabled</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Mention Mode</label>
      <select data-field="mentionMode" style=${EDIT_SELECT_STYLE} value=${cfg.mention_mode || "mention"}>
        <option value="mention">Must @mention bot</option>
        <option value="always">Always respond</option>
        <option value="none">Don't respond in servers</option>
      </select>
      <label class="text-xs text-[var(--muted)]">Default Model</label>
      <${ModelSelect} models=${modelsSig.value} value=${editModel.value}
        onChange=${(v) => {
					editModel.value = v;
				}}
        placeholder=${getModelPlaceholder()} />
      <label class="text-xs text-[var(--muted)]">User Allowlist (one Discord user ID per line)</label>
      <textarea data-field="userAllowlist" rows="3" style=${EDIT_TEXTAREA_STYLE}>${(cfg.user_allowlist || []).join("\n")}</textarea>
      ${error.value && html`<div class="text-xs text-[var(--error)] channel-error" style="display:block;">${error.value}</div>`}
      <button class="provider-btn" onClick=${onSave} disabled=${saving.value}>
        ${saving.value ? "Saving\u2026" : "Save Changes"}
      </button>
    </div>
  </${Modal}>`;
}

function EditChannelModal() {
	var ch = editingChannel.value;
	if (!ch) return null;
	var channelType = ch.type || "telegram";
	if (channelType === "telegram") return html`<${EditTelegramModal} ch=${ch} />`;
	if (channelType === "slack") return html`<${EditSlackModal} ch=${ch} />`;
	if (channelType === "discord") return html`<${EditDiscordModal} ch=${ch} />`;
	return null;
}

// ── Section content components ───────────────────────────────
function ChannelsSection() {
	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<div class="flex items-center gap-3">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Channels</h2>
			<${ConnectChannelDropdown} />
		</div>
		<${ChannelsTab} />
	</div>`;
}

function SendersSection() {
	return html`<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
		<h2 class="text-lg font-medium text-[var(--text-strong)]">Senders</h2>
		<${SendersTab} />
	</div>`;
}

// ── Main page component ──────────────────────────────────────
function ChannelsPage() {
	useEffect(() => {
		// Use prefetched cache for instant render
		if (S.cachedChannels !== null) channels.value = S.cachedChannels;
		if (connected.value) loadChannels();

		var unsub = onEvent("channel", (p) => {
			if (p.kind === "inbound_message" && activeSection.value === "senders" && sendersAccount.value === p.account_id) {
				loadSenders();
			}
		});
		S.setChannelEventUnsub(unsub);

		return () => {
			if (unsub) unsub();
			S.setChannelEventUnsub(null);
		};
	}, [connected.value]);

	var section = activeSection.value;

	return html`<div class="settings-layout">
		<${ChannelsSidebar} />
		${section === "list" ? html`<${ChannelsSection} />` : null}
		${section === "senders" ? html`<${SendersSection} />` : null}
		<${AddTelegramModal} />
		<${AddSlackModal} />
		<${AddDiscordModal} />
		<${EditChannelModal} />
		<${ConfirmDialog} />
	</div>`;
}

registerPrefix(
	"/channels",
	(container, param) => {
		containerRef = container;
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		var isValidSection = param && sections.some((s) => s.id === param);
		var section = isValidSection ? param : "list";
		activeSection.value = section;
		if (!isValidSection) {
			history.replaceState(null, "", `/channels/${section}`);
		}
		showAddModal.value = null;
		editingChannel.value = null;
		sendersAccount.value = "";
		senders.value = [];
		render(html`<${ChannelsPage} />`, container);
	},
	() => {
		S.setRefreshChannelsPage(null);
		if (S.channelEventUnsub) {
			S.channelEventUnsub();
			S.setChannelEventUnsub(null);
		}
		if (containerRef) render(null, containerRef);
		containerRef = null;
		activeSection.value = "list";
	},
);

// ── Exported init/teardown for settings page embedding ───────
export function initChannels(container) {
	containerRef = container;
	container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
	activeSection.value = "list";
	showAddModal.value = null;
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
	if (containerRef) render(null, containerRef);
	containerRef = null;
	activeSection.value = "list";
}
