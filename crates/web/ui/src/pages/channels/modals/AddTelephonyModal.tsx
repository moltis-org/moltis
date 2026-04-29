// -- Add Telephony modal --

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { addChannel, parseChannelConfigPatch } from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, ModelSelect } from "../../../ui";
import { type ChannelConfig, loadChannels, showAddTelephony } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput } from "../ChannelFields";

export function AddTelephonyModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("default");
	const accountSidDraft = useSignal("");
	const authTokenDraft = useSignal("");
	const fromNumberDraft = useSignal("");
	const webhookUrlDraft = useSignal("");
	const inboundPolicy = useSignal("disabled");
	const advancedConfigPatch = useSignal("");

	function reset(): void {
		addModel.value = "";
		allowlistItems.value = [];
		accountDraft.value = "default";
		accountSidDraft.value = "";
		authTokenDraft.value = "";
		fromNumberDraft.value = "";
		webhookUrlDraft.value = "";
		inboundPolicy.value = "disabled";
		advancedConfigPatch.value = "";
		error.value = "";
	}

	function onSubmit(e: Event): void {
		e.preventDefault();
		const accountId = accountDraft.value.trim() || "default";
		const accountSid = accountSidDraft.value.trim();
		const authToken = authTokenDraft.value.trim();
		const fromNumber = fromNumberDraft.value.trim();

		if (!accountSid) {
			error.value = "Twilio Account SID is required.";
			return;
		}
		if (!authToken) {
			error.value = "Twilio Auth Token is required.";
			return;
		}
		if (!fromNumber) {
			error.value = "Phone number (From) is required in E.164 format (e.g. +15551234567).";
			return;
		}
		if (!fromNumber.startsWith("+")) {
			error.value = "Phone number must be in E.164 format (start with +).";
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
			account_sid: accountSid,
			auth_token: authToken,
			from_number: fromNumber,
			inbound_policy: inboundPolicy.value,
			allowlist: allowlistItems.value,
		};
		if (webhookUrlDraft.value.trim()) {
			(addConfig as Record<string, unknown>).webhook_url = webhookUrlDraft.value.trim();
		}
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);

		addChannel(ChannelType.Telephony, accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddTelephony.value = false;
				reset();
				loadChannels();
			} else {
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect telephony.";
			}
		});
	}

	return (
		<Modal
			show={showAddTelephony.value}
			onClose={() => {
				showAddTelephony.value = false;
			}}
			title="Connect Phone Calls"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">Twilio Phone Calls</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							Make and receive phone calls via Twilio. You need a{" "}
							<a
								href="https://www.twilio.com/console"
								target="_blank"
								rel="noopener noreferrer"
								className="underline text-[var(--text-strong)]"
							>
								Twilio account
							</a>{" "}
							with a phone number.
						</div>
					</div>
				</div>

				<label className="text-xs text-[var(--muted)]">Account Name</label>
				<input
					type="text"
					className="channel-input"
					placeholder="default"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
					}}
				/>

				<label className="text-xs text-[var(--muted)]">Twilio Account SID</label>
				<input
					type="text"
					className="channel-input"
					placeholder="AC..."
					value={accountSidDraft.value}
					onInput={(e) => {
						accountSidDraft.value = targetValue(e);
					}}
				/>

				<label className="text-xs text-[var(--muted)]">Twilio Auth Token</label>
				<input
					type="password"
					className="channel-input"
					placeholder="Auth token"
					value={authTokenDraft.value}
					onInput={(e) => {
						authTokenDraft.value = targetValue(e);
					}}
				/>

				<label className="text-xs text-[var(--muted)]">Phone Number (E.164)</label>
				<input
					type="text"
					className="channel-input"
					placeholder="+15551234567"
					value={fromNumberDraft.value}
					onInput={(e) => {
						fromNumberDraft.value = targetValue(e);
					}}
				/>

				<label className="text-xs text-[var(--muted)]">Webhook URL (optional)</label>
				<input
					type="text"
					className="channel-input"
					placeholder="https://your-domain.com"
					value={webhookUrlDraft.value}
					onInput={(e) => {
						webhookUrlDraft.value = targetValue(e);
					}}
				/>
				<span className="text-[10px] text-[var(--muted)]">
					Public HTTPS URL for Twilio callbacks. Required for inbound calls.
				</span>

				<label className="text-xs text-[var(--muted)]">Inbound Call Policy</label>
				<select
					className="channel-select"
					value={inboundPolicy.value}
					onChange={(e) => {
						inboundPolicy.value = targetValue(e);
					}}
				>
					<option value="disabled">Disabled (outbound only)</option>
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone can call)</option>
				</select>

				{inboundPolicy.value === "allowlist" && (
					<>
						<label className="text-xs text-[var(--muted)]">Allowed Callers</label>
						<AllowlistInput
							value={allowlistItems.value}
							onChange={(v) => {
								allowlistItems.value = v;
							}}
							placeholder="+15559876543"
						/>
					</>
				)}

				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={addModel.value}
					onChange={(v) => {
						addModel.value = v;
					}}
				/>

				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(v) => {
						advancedConfigPatch.value = v;
					}}
				/>

				{error.value && <div className="text-xs text-red-500 mt-1">{error.value}</div>}
				<button type="submit" className="provider-btn mt-2" disabled={saving.value} onClick={onSubmit}>
					{saving.value ? "Connecting..." : "Connect Phone Calls"}
				</button>
			</div>
		</Modal>
	);
}
