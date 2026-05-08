// ── Add Slack modal ──────────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { addChannel, parseChannelConfigPatch } from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal } from "../../../ui";
import { type ChannelConfig, ConnectionModeHint, loadChannels, showAddSlack } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput, SharedChannelFields } from "../ChannelFields";

export function AddSlackModal(): VNode {
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
		addChannel(ChannelType.Slack, accountId, addConfig).then((res: unknown) => {
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
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect Slack.";
			}
		});
	}

	return (
		<Modal
			show={showAddSlack.value}
			onClose={() => {
				showAddSlack.value = false;
			}}
			title="Connect Slack"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up a Slack bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							1. Go to{" "}
							<a
								href="https://api.slack.com/apps"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								api.slack.com/apps
							</a>{" "}
							and create a new app
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. Under OAuth & Permissions, add bot scopes: <code className="text-[var(--accent)]">chat:write</code>,{" "}
							<code className="text-[var(--accent)]">channels:history</code>,{" "}
							<code className="text-[var(--accent)]">im:history</code>,{" "}
							<code className="text-[var(--accent)]">app_mentions:read</code>
						</div>
						<div className="text-xs text-[var(--muted)]">
							3. Install the app to your workspace and copy the Bot User OAuth Token
						</div>
						<div className="text-xs text-[var(--muted)]">
							4. For Socket Mode: enable Socket Mode and generate an App-Level Token with{" "}
							<code className="text-[var(--accent)]">connections:write</code> scope
						</div>
						<div className="text-xs text-[var(--muted)]">
							5. For Events API: set the Request URL to your server's webhook endpoint
						</div>
					</div>
				</div>
				<ConnectionModeHint type={ChannelType.Slack} />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-slack-bot"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Bot Token (xoxb-...)</label>
				<input
					data-field="botToken"
					type="password"
					placeholder="xoxb-..."
					className="channel-input"
					value={botTokenDraft.value}
					onInput={(e) => {
						botTokenDraft.value = targetValue(e);
					}}
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
					onChange={(e) => {
						connectionMode.value = targetValue(e);
					}}
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
							onInput={(e) => {
								appTokenDraft.value = targetValue(e);
							}}
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
							onInput={(e) => {
								signingSecretDraft.value = targetValue(e);
							}}
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
					onChange={(items) => {
						channelAllowlistItems.value = items;
					}}
				/>
				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(value) => {
						advancedConfigPatch.value = value;
					}}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Slack"}
				</button>
			</div>
		</Modal>
	);
}
