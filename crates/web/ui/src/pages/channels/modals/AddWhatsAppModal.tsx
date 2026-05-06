// ── Add WhatsApp modal ───────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";

import { addChannel, parseChannelConfigPatch } from "../../../channel-utils";
import { sendRpc } from "../../../helpers";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, ModelSelect, showToast } from "../../../ui";
import {
	type Channel,
	type ChannelConfig,
	ConnectionModeHint,
	loadChannels,
	showAddWhatsApp,
	waPairingAccountId,
	waPairingError,
	waQrData,
	waQrSvg,
} from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput } from "../ChannelFields";

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
		return (
			<div className="flex items-center justify-center p-8 text-[var(--muted)] text-sm">Waiting for QR code...</div>
		);

	return (
		<div className="flex flex-col items-center gap-3 p-4">
			<div
				className="rounded-lg bg-white p-3"
				style={{ width: "200px", height: "200px", display: "flex", alignItems: "center", justifyContent: "center" }}
			>
				{svgUrl ? (
					<img
						src={svgUrl}
						alt="WhatsApp pairing QR code"
						style={{ width: "100%", height: "100%", display: "block" }}
					/>
				) : (
					<div className="text-center text-xs text-gray-600">
						<div
							style={{
								fontFamily: "monospace",
								fontSize: "9px",
								wordBreak: "break-all",
								maxHeight: "180px",
								overflow: "hidden",
							}}
						>
							{data.substring(0, 200)}
						</div>
					</div>
				)}
			</div>
			<div className="text-xs text-[var(--muted)] text-center">
				Scan this QR code in your terminal output,
				<br />
				or open WhatsApp &gt; Settings &gt; Linked Devices &gt; Link a Device.
			</div>
		</div>
	);
}

export function AddWhatsAppModal(): VNode {
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
		addChannel(ChannelType.WhatsApp, accountId, addConfig).then((res: unknown) => {
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
						const ch = (st.payload?.channels || []).find(
							(c) => c.type === ChannelType.WhatsApp && c.account_id === accountId,
						);
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
				error.value = r?.error?.message || r?.error?.detail || "Failed to start pairing.";
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
						<div className="text-xs text-[var(--muted)]">
							QR code refreshes automatically. Keep this window open.
							<br />
							Only new messages will be processed &mdash; past conversations are not synced.
						</div>
					</div>
				) : (
					<>
						<div className="channel-card">
							<div>
								<span className="text-xs font-medium text-[var(--text-strong)]">Link your WhatsApp</span>
								<div className="text-xs text-[var(--muted)] channel-help">
									1. Choose an account ID below (any name you like)
								</div>
								<div className="text-xs text-[var(--muted)]">2. Click "Start Pairing" to generate a QR code</div>
								<div className="text-xs text-[var(--muted)]">
									3. Open WhatsApp on your phone &gt; Settings &gt; Linked Devices &gt; Link a Device
								</div>
								<div className="text-xs text-[var(--muted)]">4. Scan the QR code to connect</div>
							</div>
						</div>
						<ConnectionModeHint type={ChannelType.WhatsApp} />
						<label className="text-xs text-[var(--muted)]">Account ID</label>
						<input
							data-field="accountId"
							type="text"
							placeholder="main"
							className="channel-input"
							value={accountDraft.value}
							onInput={(e) => {
								accountDraft.value = targetValue(e);
							}}
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
						<AdvancedConfigPatchField
							value={advancedConfigPatch.value}
							onInput={(value) => {
								advancedConfigPatch.value = value;
							}}
						/>
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
