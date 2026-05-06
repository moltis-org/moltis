// ── Add Nostr modal ──────────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { addChannel, parseChannelConfigPatch } from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, ModelSelect } from "../../../ui";
import { type ChannelConfig, ConnectionModeHint, loadChannels, showAddNostr } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput } from "../ChannelFields";

export function AddNostrModal(): VNode {
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
		addChannel(ChannelType.Nostr, accountId, addConfig).then((res: unknown) => {
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
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal
			show={showAddNostr.value}
			onClose={() => {
				showAddNostr.value = false;
			}}
			title="Connect Nostr"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">How to set up Nostr DMs</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							1. Generate or use an existing Nostr secret key (nsec1... or hex)
						</div>
						<div className="text-xs text-[var(--muted)]">2. Configure relay URLs (defaults are provided)</div>
						<div className="text-xs text-[var(--muted)]">
							3. Add allowed public keys (npub1... or hex) to the allowlist
						</div>
						<div className="text-xs text-[var(--muted)]">
							4. Send a DM to the bot's public key from any Nostr client
						</div>
					</div>
				</div>
				<ConnectionModeHint type={ChannelType.Nostr} />
				<label className="text-xs text-[var(--muted)]">Account ID</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. my-nostr-bot"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">Secret Key</label>
				<input
					data-field="credential"
					type="password"
					placeholder="nsec1... or 64-char hex"
					className="channel-input"
					value={secretKeyDraft.value}
					onInput={(e) => {
						secretKeyDraft.value = targetValue(e);
					}}
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
					onInput={(e) => {
						relaysDraft.value = targetValue(e);
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select data-field="dmPolicy" className="channel-select">
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={addModel.value}
					onChange={(v: string) => {
						addModel.value = v;
					}}
				/>
				<label className="text-xs text-[var(--muted)]">Allowed Public Keys</label>
				<AllowlistInput
					value={allowlistItems.value}
					onChange={(v) => {
						allowlistItems.value = v;
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
					{saving.value ? "Connecting\u2026" : "Connect Nostr"}
				</button>
			</div>
		</Modal>
	);
}
