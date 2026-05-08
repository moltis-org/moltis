// ── Add Telegram modal ───────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { addChannel, parseChannelConfigPatch, validateChannelFields } from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal } from "../../../ui";
import { type ChannelConfig, ConnectionModeHint, loadChannels, showAddTelegram } from "../../ChannelsPage";
import { AdvancedConfigPatchField, SharedChannelFields } from "../ChannelFields";

export function AddTelegramModal(): VNode {
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
		const v = validateChannelFields(ChannelType.Telegram, accountId, credential);
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
		addChannel(ChannelType.Telegram, accountId, addConfig).then((res: unknown) => {
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
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal
			show={showAddTelegram.value}
			onClose={() => {
				showAddTelegram.value = false;
			}}
			title="Connect Telegram"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to create a Telegram bot</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							1. Open{" "}
							<a
								href="https://t.me/BotFather"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								@BotFather
							</a>{" "}
							in Telegram
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. Send /newbot and follow the prompts to choose a name and username
						</div>
						<div className="text-xs text-[var(--muted)]">3. Copy the bot token and paste it below</div>
					</div>
				</div>
				<ConnectionModeHint type={ChannelType.Telegram} />
				<label className="text-xs text-[var(--muted)]">Bot username</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my_assistant_bot"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
					}}
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
						<a
							href={`https://t.me/${encodeURIComponent(accountDraft.value.trim())}`}
							target="_blank"
							rel="noopener noreferrer"
							className="text-[var(--accent)] underline"
						>
							t.me/{accountDraft.value.trim()}
						</a>
					</div>
				)}
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(value) => {
						advancedConfigPatch.value = value;
					}}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Telegram"}
				</button>
			</div>
		</Modal>
	);
}
