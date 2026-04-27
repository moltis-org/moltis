// ── Edit channel modal ───────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect } from "preact/hooks";

import {
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
} from "../../../channel-utils";
import { sendRpc } from "../../../helpers";
import { models as modelsSig } from "../../../stores/model-store";
import { targetChecked, targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, ModelSelect } from "../../../ui";
import { type ChannelConfig, channelLabel, channelType, editingChannel, loadChannels } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput } from "../ChannelFields";

export function EditChannelModal(): VNode | null {
	const ch = editingChannel.value;
	const error = useSignal("");
	const saving = useSignal(false);
	const editModel = useSignal("");
	const editAgent = useSignal("");
	const agentsList = useSignal<Array<{ id: string; name: string; emoji?: string }>>([]);
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
	const editSignalAccount = useSignal("");
	const editSignalHttpUrl = useSignal("http://127.0.0.1:8080");
	const editChannelNamePatterns = useSignal<string[]>([]);
	const editCategoryAllowlist = useSignal<string[]>([]);
	const editAdvancedConfigPatch = useSignal("");

	useEffect(() => {
		editModel.value = (ch?.config?.model as string) || "";
		editAgent.value = (ch?.config?.agent_id as string) || "";
		allowlistItems.value = (ch?.config?.allowlist ||
			ch?.config?.user_allowlist ||
			ch?.config?.allowed_pubkeys ||
			[]) as string[];
		roomAllowlistItems.value = (ch?.config?.room_allowlist || ch?.config?.group_allowlist || []) as string[];
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
		editSignalAccount.value = (ch?.config?.account as string) || "";
		editSignalHttpUrl.value = (ch?.config?.http_url as string) || "http://127.0.0.1:8080";
		editChannelNamePatterns.value = (ch?.config?.channel_name_patterns || []) as string[];
		editCategoryAllowlist.value = (ch?.config?.category_allowlist || []) as string[];
		editAdvancedConfigPatch.value = "";
	}, [ch]);

	useEffect(() => {
		sendRpc("agents.list", {}).then((res) => {
			if (res?.ok) {
				const payload = res.payload as { agents?: Array<{ id: string; name: string; emoji?: string }> };
				agentsList.value = payload?.agents || [];
			}
		});
	}, []);

	if (!ch) return null;

	const cfg = ch.config || {};
	const chType = channelType(ch.type);
	const isTeams = chType === ChannelType.MsTeams;
	const isDiscord = chType === ChannelType.Discord;
	const isWhatsApp = chType === ChannelType.WhatsApp;
	const isTelegram = chType === ChannelType.Telegram;
	const isMatrix = chType === ChannelType.Matrix;
	const isNostr = chType === ChannelType.Nostr;
	const isSignal = chType === ChannelType.Signal;

	function addModelToConfig(config: ChannelConfig): void {
		if (!editModel.value) return;
		config.model = editModel.value;
		const found = modelsSig.value.find((x) => x.id === editModel.value);
		if (found?.provider) config.model_provider = found.provider;
	}

	function addAgentToConfig(config: ChannelConfig): void {
		config.agent_id = editAgent.value || null;
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
			config.relays = relaysVal
				.split(",")
				.map((r) => r.trim())
				.filter(Boolean);
		} else if (isSignal) {
			config.account = editSignalAccount.value.trim();
			config.http_url = editSignalHttpUrl.value.trim() || "http://127.0.0.1:8080";
		} else if (isMatrix) {
			config.homeserver =
				(form.querySelector("[data-field=homeserver]") as HTMLInputElement)?.value || cfg.homeserver || "";
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
		const dmFallback = isWhatsApp ? "open" : "allowlist";
		updateConfig.dm_policy = (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement)?.value || dmFallback;
		updateConfig.allowlist = allowlistItems.value;
		if (isMatrix) {
			updateConfig.user_allowlist = allowlistItems.value;
			updateConfig.room_policy =
				(form.querySelector("[data-field=roomPolicy]") as HTMLSelectElement)?.value || cfg.room_policy || "allowlist";
			updateConfig.auto_join =
				(form.querySelector("[data-field=autoJoin]") as HTMLSelectElement)?.value || cfg.auto_join || "always";
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
		if (isSignal) {
			updateConfig.group_policy =
				(form.querySelector("[data-field=groupPolicy]") as HTMLSelectElement)?.value || cfg.group_policy || "disabled";
			updateConfig.group_allowlist = roomAllowlistItems.value;
			updateConfig.otp_self_approval = cfg.otp_self_approval !== false;
			updateConfig.otp_cooldown_secs = cfg.otp_cooldown_secs ?? 300;
			updateConfig.ignore_stories = cfg.ignore_stories !== false;
			updateConfig.text_chunk_limit = (cfg.text_chunk_limit as number) || 4000;
			if (cfg.account_uuid) updateConfig.account_uuid = cfg.account_uuid as string;
		}
		if (!(isWhatsApp || isNostr)) {
			updateConfig.mention_mode =
				(form.querySelector("[data-field=mentionMode]") as HTMLSelectElement)?.value || "mention";
		}
		if (isDiscord) {
			updateConfig.channel_name_patterns = editChannelNamePatterns.value;
			updateConfig.category_allowlist = editCategoryAllowlist.value;
		}
		addChannelCredentials(updateConfig, form);
		addModelToConfig(updateConfig);
		addAgentToConfig(updateConfig);
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
				error.value =
					(res?.error as { message?: string; detail?: string })?.message ||
					(res?.error as { detail?: string })?.detail ||
					"Failed to update channel.";
			}
		});
	}

	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<Modal
			show={true}
			onClose={() => {
				editingChannel.value = null;
			}}
			title={`Edit ${channelLabel(ch.type)} Channel`}
		>
			<div className="channel-form">
				<div className="text-sm text-[var(--text-strong)]">{ch.name || ch.account_id}</div>
				{isTelegram && ch.account_id && (
					<a
						href={`https://t.me/${ch.account_id}`}
						target="_blank"
						className="text-xs text-[var(--accent)] underline"
						rel="noopener"
					>
						t.me/{ch.account_id}
					</a>
				)}
				{isTeams && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">App Password (optional: leave blank to keep existing)</label>
						<input
							type="password"
							className="channel-input w-full"
							value={editCredential.value}
							onInput={(e) => {
								editCredential.value = targetValue(e);
							}}
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
								onInput={(e) => {
									editWebhookSecret.value = targetValue(e);
								}}
							/>
						</div>
						<div className="flex gap-3">
							<div className="flex-1">
								<label className="text-xs text-[var(--muted)]">Streaming</label>
								<select
									className="channel-select"
									value={editStreamMode.value}
									onChange={(e) => {
										editStreamMode.value = targetValue(e);
									}}
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
									onChange={(e) => {
										editReplyStyle.value = targetValue(e);
									}}
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
									onInput={(e) => {
										editBotName.value = targetValue(e);
									}}
									placeholder="Moltis"
								/>
							</div>
							<label className="flex items-center gap-2 text-xs text-[var(--muted)] pb-2 cursor-pointer">
								<input
									type="checkbox"
									checked={editWelcomeCard.value}
									onChange={(e) => {
										editWelcomeCard.value = targetChecked(e);
									}}
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
							onInput={(e) => {
								editCredential.value = targetValue(e);
							}}
						/>
					</div>
				)}
				{isDiscord && (
					<>
						<label className="text-xs text-[var(--muted)]">Channel Name Patterns (optional)</label>
						<AllowlistInput
							value={editChannelNamePatterns.value}
							onChange={(v) => {
								editChannelNamePatterns.value = v;
							}}
							placeholder="e.g. ticket-* (glob patterns, Enter to add)"
						/>
						<div className="text-xs text-[var(--muted)] -mt-1">
							When set, the bot only responds in guild channels whose name matches a pattern. Matched channels do not
							require @mention. Supports * wildcards.
						</div>
						<label className="text-xs text-[var(--muted)]">Category IDs (optional)</label>
						<AllowlistInput
							value={editCategoryAllowlist.value}
							onChange={(v) => {
								editCategoryAllowlist.value = v;
							}}
							placeholder="Discord category ID (Enter to add)"
						/>
						<div className="text-xs text-[var(--muted)] -mt-1">
							Only respond in channels under these Discord categories. Combined with name patterns via OR.
						</div>
					</>
				)}
				{isNostr && (
					<>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Secret Key (optional: leave blank to keep existing)</label>
							<input
								type="password"
								className="channel-input w-full"
								value={editCredential.value}
								onInput={(e) => {
									editCredential.value = targetValue(e);
								}}
								autoComplete="new-password"
							/>
						</div>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Relays (comma-separated)</label>
							<input
								data-field="relays"
								type="text"
								className="channel-input w-full"
								defaultValue={((cfg.relays as string[]) || []).join(", ")}
							/>
						</div>
					</>
				)}
				{isSignal && (
					<>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">Signal Account</label>
							<input
								type="text"
								className="channel-input w-full"
								value={editSignalAccount.value}
								onInput={(e) => {
									editSignalAccount.value = targetValue(e);
								}}
								placeholder="+15551234567"
							/>
						</div>
						<div className="flex flex-col gap-1">
							<label className="text-xs text-[var(--muted)]">signal-cli Daemon URL</label>
							<input
								type="url"
								className="channel-input w-full"
								value={editSignalHttpUrl.value}
								onInput={(e) => {
									editSignalHttpUrl.value = targetValue(e);
								}}
								placeholder="http://127.0.0.1:8080"
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
							onChange={(e) => {
								editMatrixAuthMode.value = normalizeMatrixAuthMode(targetValue(e));
							}}
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
									onChange={(e) => {
										editMatrixOwnershipMode.value = targetChecked(e) ? "moltis_owned" : "user_managed";
									}}
								/>
								<span className="flex flex-col gap-1">
									<span className="text-xs font-medium text-[var(--text-strong)]">
										Let Moltis own this Matrix account
									</span>
									<span className="text-xs text-[var(--muted)]">
										{matrixOwnershipModeGuidance(editMatrixAuthMode.value, editMatrixOwnershipMode.value)}
									</span>
								</span>
							</label>
						) : (
							<div className="text-xs text-[var(--muted)]">
								{matrixOwnershipModeGuidance(editMatrixAuthMode.value, "user_managed")}
							</div>
						)}
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Homeserver URL</label>
						<input
							data-field="homeserver"
							type="text"
							className="channel-input w-full"
							defaultValue={(cfg.homeserver as string) || ""}
						/>
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">
							Matrix User ID{editMatrixAuthMode.value === "password" ? " (required)" : " (optional)"}
						</label>
						<input
							data-field="userId"
							type="text"
							className="channel-input w-full"
							defaultValue={(cfg.user_id as string) || ""}
						/>
					</div>
				)}
				{isMatrix && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">
							{matrixCredentialLabel(editMatrixAuthMode.value)} (optional: leave blank to keep existing)
						</label>
						<input
							type="password"
							className="channel-input w-full"
							value={editCredential.value}
							onInput={(e) => {
								editCredential.value = targetValue(e);
							}}
							placeholder={matrixCredentialPlaceholder(editMatrixAuthMode.value)}
						/>
						<div className="text-xs text-[var(--muted)]">
							{editMatrixAuthMode.value === "password" ? (
								"Password auth is required for encrypted Matrix chats because Moltis needs its own Matrix device keys."
							) : (
								<>
									Access token mode does <span className="font-medium">not</span> support encrypted Matrix chats because
									Moltis cannot import the existing device's private encryption keys.
								</>
							)}{" "}
							<a href={MATRIX_DOCS_URL} target="_blank" rel="noreferrer" className="text-[var(--accent)] underline">
								Matrix setup docs
							</a>
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
							onInput={(e) => {
								editMatrixDeviceDisplayName.value = targetValue(e);
							}}
						/>
					</div>
				)}
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select
					data-field="dmPolicy"
					className="channel-select"
					value={(cfg.dm_policy as string) || (isWhatsApp ? "open" : "allowlist")}
				>
					{isWhatsApp && <option value="open">Open (anyone)</option>}
					<option value="allowlist">Allowlist only</option>
					{!isWhatsApp && <option value="open">Open (anyone)</option>}
					<option value="disabled">Disabled</option>
				</select>
				{!isWhatsApp && (
					<>
						<label className="text-xs text-[var(--muted)]">Group Mention Mode</label>
						<select
							data-field="mentionMode"
							className="channel-select"
							value={(cfg.mention_mode as string) || "mention"}
						>
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
							onChange={(e) => {
								editMatrixOtpSelfApproval.value = targetValue(e) !== "off";
							}}
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
							onInput={(e) => {
								editMatrixOtpCooldown.value = targetValue(e);
							}}
						/>
						<div className="text-xs text-[var(--muted)]">
							With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.
						</div>
						<label className="text-xs text-[var(--muted)]">Room Policy</label>
						<select
							data-field="roomPolicy"
							className="channel-select"
							value={(cfg.room_policy as string) || "allowlist"}
						>
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
				{isSignal && (
					<>
						<label className="text-xs text-[var(--muted)]">Group Policy</label>
						<select
							data-field="groupPolicy"
							className="channel-select"
							value={(cfg.group_policy as string) || "disabled"}
						>
							<option value="disabled">Disabled</option>
							<option value="allowlist">Allowlist only</option>
							<option value="open">Open (any group)</option>
						</select>
					</>
				)}
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={editModel.value}
					onChange={(v: string) => {
						editModel.value = v;
					}}
					placeholder={defaultPlaceholder}
				/>
				<label className="text-xs text-[var(--muted)]">Agent</label>
				<select
					className="channel-select"
					value={editAgent.value}
					onChange={(e: Event) => {
						editAgent.value = targetValue(e);
					}}
				>
					<option value="">(default agent)</option>
					{agentsList.value.map((a) => (
						<option key={a.id} value={a.id}>
							{a.emoji ? `${a.emoji} ` : ""}
							{a.name}
						</option>
					))}
				</select>
				<label className="text-xs text-[var(--muted)]">DM Allowlist</label>
				<AllowlistInput
					value={allowlistItems.value}
					preserveAt={isMatrix}
					onChange={(v) => {
						allowlistItems.value = v;
					}}
				/>
				{isMatrix && (
					<>
						<label className="text-xs text-[var(--muted)]">Room Allowlist</label>
						<AllowlistInput
							value={roomAllowlistItems.value}
							preserveAt={true}
							onChange={(v) => {
								roomAllowlistItems.value = v;
							}}
						/>
					</>
				)}
				{isSignal && (
					<>
						<label className="text-xs text-[var(--muted)]">Group Allowlist</label>
						<AllowlistInput
							value={roomAllowlistItems.value}
							onChange={(v) => {
								roomAllowlistItems.value = v;
							}}
						/>
					</>
				)}
				<AdvancedConfigPatchField
					value={editAdvancedConfigPatch.value}
					onInput={(value) => {
						editAdvancedConfigPatch.value = value;
					}}
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
