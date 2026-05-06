// ── Add Discord modal ────────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { addChannel, parseChannelConfigPatch, validateChannelFields } from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal } from "../../../ui";
import { type ChannelConfig, ConnectionModeHint, loadChannels, showAddDiscord } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput, SharedChannelFields } from "../ChannelFields";

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

export function AddDiscordModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const channelNamePatterns = useSignal<string[]>([]);
	const categoryAllowlist = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const tokenDraft = useSignal("");
	const advancedConfigPatch = useSignal("");

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const credential = tokenDraft.value.trim();
		const v = validateChannelFields(ChannelType.Discord, accountId, credential);
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
		if (channelNamePatterns.value.length > 0) addConfig.channel_name_patterns = channelNamePatterns.value;
		if (categoryAllowlist.value.length > 0) addConfig.category_allowlist = categoryAllowlist.value;
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel(ChannelType.Discord, accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddDiscord.value = false;
				addModel.value = "";
				allowlistItems.value = [];
				channelNamePatterns.value = [];
				categoryAllowlist.value = [];
				accountDraft.value = "";
				tokenDraft.value = "";
				advancedConfigPatch.value = "";
				loadChannels();
			} else {
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect channel.";
			}
		});
	}

	const inviteUrl = discordInviteUrl(tokenDraft.value);

	return (
		<Modal
			show={showAddDiscord.value}
			onClose={() => {
				showAddDiscord.value = false;
			}}
			title="Connect Discord"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up a Discord bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							1. Go to the{" "}
							<a
								href="https://discord.com/developers/applications"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								Discord Developer Portal
							</a>
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. Create a new Application &rarr; Bot tab &rarr; copy the bot token
						</div>
						<div className="text-xs text-[var(--muted)]">
							3. Enable "Message Content Intent" under Privileged Gateway Intents
						</div>
						<div className="text-xs text-[var(--muted)]">
							4. Paste the token below &mdash; an invite link will be generated automatically
						</div>
						<div className="text-xs text-[var(--muted)]">
							5. You can also DM the bot directly without adding it to a server
						</div>
					</div>
				</div>
				<ConnectionModeHint type={ChannelType.Discord} />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-discord-bot"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Bot Token</label>
				<input
					data-field="credential"
					type="password"
					placeholder="Discord bot token"
					className="channel-input"
					value={tokenDraft.value}
					onInput={(e) => {
						tokenDraft.value = targetValue(e);
					}}
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="discord_bot_token"
				/>
				{inviteUrl && (
					<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2.5 flex flex-col gap-1">
						<span className="text-xs font-medium text-[var(--text-strong)]">Invite bot to a server</span>
						<span className="text-xs text-[var(--muted)]">
							Open this link to add the bot (Send Messages, Attach Files, Read Message History):
						</span>
						<a href={inviteUrl} target="_blank" className="text-xs text-[var(--accent)] underline break-all">
							{inviteUrl}
						</a>
					</div>
				)}
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<label className="text-xs text-[var(--muted)]">Channel Name Patterns (optional)</label>
				<AllowlistInput
					value={channelNamePatterns.value}
					onChange={(v) => {
						channelNamePatterns.value = v;
					}}
					placeholder="e.g. ticket-* (glob patterns, Enter to add)"
				/>
				<div className="text-xs text-[var(--muted)] -mt-1">
					When set, the bot only responds in guild channels whose name matches a pattern. Matched channels do not
					require @mention. Supports * wildcards.
				</div>
				<label className="text-xs text-[var(--muted)]">Category IDs (optional)</label>
				<AllowlistInput
					value={categoryAllowlist.value}
					onChange={(v) => {
						categoryAllowlist.value = v;
					}}
					placeholder="Discord category ID (Enter to add)"
				/>
				<div className="text-xs text-[var(--muted)] -mt-1">
					Only respond in channels under these Discord categories. Combined with name patterns via OR.
				</div>
				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(value) => {
						advancedConfigPatch.value = value;
					}}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Discord"}
				</button>
			</div>
		</Modal>
	);
}
