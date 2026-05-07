// ── Phone section (Settings > Phone) ─────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	fetchPhoneProviders,
	type PhoneProviderData,
	type PhoneProviders,
	removePhoneKey,
	savePhoneKey,
	savePhoneSettings,
	togglePhoneProvider,
} from "../../phone-utils";
import { connected } from "../../signals";
import { targetChecked, targetValue } from "../../typed-events";
import { Modal } from "../../ui";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";

const showConfigModal = signal(false);
const selectedProvider = signal<PhoneProviderData | null>(null);

export function PhoneSection(): VNode {
	const [providers, setProviders] = useState<PhoneProviderData[]>([]);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState<string | null>(null);
	const [msg, setMsg] = useState<string | null>(null);
	const [err, setErr] = useState<string | null>(null);

	function fetchStatus(silent?: boolean): void {
		if (!silent) {
			setLoading(true);
			rerender();
		}
		fetchPhoneProviders()
			.then((r: unknown) => {
				const res = r as RpcResponse;
				if (res?.ok) {
					const data = res.payload as PhoneProviders;
					setProviders(data?.providers || []);
				}
				if (!silent) setLoading(false);
				rerender();
			})
			.catch(() => {
				if (!silent) setLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		if (connected.value) fetchStatus();
	}, [connected.value]);

	function onToggle(provider: PhoneProviderData, enabled: boolean): void {
		setErr(null);
		setMsg(null);
		setSaving(provider.id);
		rerender();
		togglePhoneProvider(provider.id, enabled)
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSaving(null);
				if (res?.ok) {
					setMsg(`${provider.name} ${enabled ? "enabled" : "disabled"}.`);
					setTimeout(() => {
						setMsg(null);
						rerender();
					}, 2000);
					fetchStatus(true);
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to toggle provider");
				}
				rerender();
			})
			.catch((e: Error) => {
				setSaving(null);
				setErr(e.message);
				rerender();
			});
	}

	function onConfigure(provider: PhoneProviderData): void {
		selectedProvider.value = provider;
		showConfigModal.value = true;
	}

	function onRemoveKey(provider: PhoneProviderData): void {
		setSaving(provider.id);
		removePhoneKey(provider.id)
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSaving(null);
				if (res?.ok) {
					setMsg(`${provider.name} credentials removed.`);
					setTimeout(() => {
						setMsg(null);
						rerender();
					}, 2000);
					fetchStatus(true);
				} else {
					setErr((res?.error as { message?: string })?.message || "Failed to remove key");
				}
				rerender();
			})
			.catch((e: Error) => {
				setSaving(null);
				setErr(e.message);
				rerender();
			});
	}

	if (loading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-3 overflow-y-auto">
				<h2 className="text-base font-medium text-[var(--text-strong)]">Phone</h2>
				<div className="text-xs text-[var(--muted)]">Loading providers...</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-3 overflow-y-auto">
			<h2 className="text-base font-medium text-[var(--text-strong)]">Phone</h2>
			<p className="text-xs text-[var(--muted)] max-w-form leading-relaxed">
				Configure telephony providers for making and receiving phone calls. Enable a provider, add your credentials, and
				agents can initiate calls via the <code>voice_call</code> tool.
			</p>

			{msg && <div className="text-xs text-green-600">{msg}</div>}
			{err && <div className="text-xs text-red-500">{err}</div>}

			<div className="flex flex-col gap-2 max-w-form">
				{providers.map((p) => (
					<PhoneProviderCard
						key={p.id}
						provider={p}
						saving={saving === p.id}
						onToggle={(enabled) => onToggle(p, enabled)}
						onConfigure={() => onConfigure(p)}
						onRemoveKey={() => onRemoveKey(p)}
					/>
				))}
				{providers.length === 0 && <div className="text-xs text-[var(--muted)]">No telephony providers available.</div>}
			</div>

			<PhoneConfigModal
				onSaved={() => {
					showConfigModal.value = false;
					fetchStatus(true);
				}}
			/>
		</div>
	);
}

// ── Provider card ──────────────────────────────────────────

interface PhoneProviderCardProps {
	provider: PhoneProviderData;
	saving: boolean;
	onToggle: (enabled: boolean) => void;
	onConfigure: () => void;
	onRemoveKey: () => void;
}

function PhoneProviderCard({ provider, saving, onToggle, onConfigure, onRemoveKey }: PhoneProviderCardProps): VNode {
	const configured = provider.available;
	const enabled = provider.enabled;

	return (
		<div
			className={`rounded-lg border p-3 flex flex-col gap-2 ${enabled ? "border-[var(--accent)]" : "border-[var(--border)]"}`}
		>
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-2">
					<span className="icon icon-phone" />
					<span className="text-sm font-medium text-[var(--text-strong)]">{provider.name}</span>
					{provider.category && (
						<span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--surface2)] text-[var(--muted)]">
							{provider.category}
						</span>
					)}
				</div>
				<div className="flex items-center gap-2">
					{configured && (
						<label className="flex items-center gap-1.5 cursor-pointer">
							<input type="checkbox" checked={enabled} disabled={saving} onChange={(e) => onToggle(targetChecked(e))} />
							<span className="text-xs text-[var(--muted)]">{enabled ? "Enabled" : "Disabled"}</span>
						</label>
					)}
				</div>
			</div>

			{provider.description && <div className="text-xs text-[var(--muted)]">{provider.description}</div>}

			{provider.settings?.from_number && (
				<div className="text-xs text-[var(--muted)]">
					Number: <span className="font-mono text-[var(--text-strong)]">{provider.settings.from_number}</span>
				</div>
			)}

			<div className="flex items-center gap-2 mt-1">
				<button type="button" className="provider-btn provider-btn-secondary text-xs px-2 py-1" onClick={onConfigure}>
					{configured ? "Configure" : "Set up credentials"}
				</button>
				{configured && (
					<button
						type="button"
						className="provider-btn provider-btn-danger text-xs px-2 py-1"
						disabled={saving}
						onClick={onRemoveKey}
					>
						Remove
					</button>
				)}
			</div>
		</div>
	);
}

// ── Config modal ──────────────────────────────────────────

interface PhoneConfigModalProps {
	onSaved: () => void;
}

function PhoneConfigModal({ onSaved }: PhoneConfigModalProps): VNode {
	const provider = selectedProvider.value;
	const credentialFields = phoneCredentialFields(provider);
	const [primaryCredential, setPrimaryCredential] = useState("");
	const [secondaryCredential, setSecondaryCredential] = useState("");
	const [fromNumber, setFromNumber] = useState("");
	const [webhookUrl, setWebhookUrl] = useState("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (provider) {
			setPrimaryCredential("");
			setSecondaryCredential("");
			setFromNumber(provider.settings?.from_number || "");
			setWebhookUrl(provider.settings?.webhook_url || "");
			setError(null);
		}
	}, [provider?.id, showConfigModal.value]);

	function onSubmit(event: Event): void {
		event.preventDefault();
		if (!provider) return;

		// If credentials are provided, save them. Otherwise just save settings.
		const hasCredentials = primaryCredential.trim() && secondaryCredential.trim();

		if (fromNumber.trim() && !fromNumber.trim().startsWith("+")) {
			setError("Phone number must be in E.164 format (start with +).");
			return;
		}

		setError(null);
		setSaving(true);

		const opts = {
			from_number: fromNumber.trim(),
			webhook_url: webhookUrl.trim(),
		};

		const promise = hasCredentials
			? savePhoneKey(provider.id, primaryCredential.trim(), secondaryCredential.trim(), opts)
			: savePhoneSettings(provider.id, opts);

		promise
			.then((r: unknown) => {
				setSaving(false);
				const res = r as RpcResponse;
				if (res?.ok) {
					onSaved();
				} else {
					setError((res?.error as { message?: string })?.message || "Failed to save.");
				}
			})
			.catch((e: Error) => {
				setSaving(false);
				setError(e.message);
			});
	}

	return (
		<Modal
			show={showConfigModal.value}
			onClose={() => {
				showConfigModal.value = false;
			}}
			title={provider ? `Configure ${provider.name}` : "Configure Provider"}
		>
			<form className="flex flex-col gap-3" onSubmit={onSubmit}>
				{provider?.hint && <div className="text-xs text-[var(--muted)]">{provider.hint}</div>}

				{provider?.keyUrl && (
					<div className="text-xs text-[var(--muted)]">
						Get credentials from{" "}
						<a
							href={provider.keyUrl}
							target="_blank"
							rel="noopener noreferrer"
							className="underline text-[var(--accent)]"
						>
							{provider.keyUrlLabel || provider.keyUrl}
						</a>
					</div>
				)}

				<label className="text-xs text-[var(--muted)]" htmlFor="phone-primary-credential">
					{credentialFields.primaryLabel}
				</label>
				<input
					id="phone-primary-credential"
					type={credentialFields.primaryIsSecret ? "password" : "text"}
					className="channel-input"
					placeholder={credentialFields.primaryPlaceholder}
					value={primaryCredential}
					onInput={(e) => setPrimaryCredential(targetValue(e))}
				/>

				<label className="text-xs text-[var(--muted)]" htmlFor="phone-secondary-credential">
					{credentialFields.secondaryLabel}
				</label>
				<input
					id="phone-secondary-credential"
					type={credentialFields.secondaryIsSecret ? "password" : "text"}
					className="channel-input"
					placeholder={credentialFields.secondaryPlaceholder}
					value={secondaryCredential}
					onInput={(e) => setSecondaryCredential(targetValue(e))}
				/>

				<label className="text-xs text-[var(--muted)]" htmlFor="phone-from-number">
					Phone Number (E.164)
				</label>
				<input
					id="phone-from-number"
					type="text"
					className="channel-input"
					placeholder="+15551234567"
					value={fromNumber}
					onInput={(e) => setFromNumber(targetValue(e))}
				/>

				<label className="text-xs text-[var(--muted)]" htmlFor="phone-webhook-url">
					Webhook URL (optional)
				</label>
				<input
					id="phone-webhook-url"
					type="text"
					className="channel-input"
					placeholder="https://your-domain.com"
					value={webhookUrl}
					onInput={(e) => setWebhookUrl(targetValue(e))}
				/>
				<span className="text-[10px] text-[var(--muted)]">
					Public HTTPS URL for provider callbacks. Required for inbound calls. Auto-detected from Tailscale/ngrok if
					configured.
				</span>

				{error && <div className="text-xs text-red-500">{error}</div>}

				<div className="flex gap-2 mt-1">
					<button type="submit" className="provider-btn text-xs" disabled={saving}>
						{saving ? "Saving..." : "Save"}
					</button>
					<button
						type="button"
						className="provider-btn provider-btn-secondary text-xs"
						onClick={() => {
							showConfigModal.value = false;
						}}
					>
						Cancel
					</button>
				</div>
			</form>
		</Modal>
	);
}

interface PhoneCredentialFields {
	primaryLabel: string;
	primaryPlaceholder: string;
	primaryIsSecret: boolean;
	secondaryLabel: string;
	secondaryPlaceholder: string;
	secondaryIsSecret: boolean;
}

function phoneCredentialFields(provider: PhoneProviderData | null): PhoneCredentialFields {
	switch (provider?.id) {
		case "telnyx":
			return {
				primaryLabel: "API Key",
				primaryPlaceholder: provider.keyPlaceholder || "KEY_...",
				primaryIsSecret: true,
				secondaryLabel: "Connection ID",
				secondaryPlaceholder: "Call Control connection ID",
				secondaryIsSecret: false,
			};
		case "plivo":
			return {
				primaryLabel: "Auth ID",
				primaryPlaceholder: provider.keyPlaceholder || "MA...",
				primaryIsSecret: false,
				secondaryLabel: "Auth Token",
				secondaryPlaceholder: "Auth token",
				secondaryIsSecret: true,
			};
		default:
			return {
				primaryLabel: "Account SID",
				primaryPlaceholder: provider?.keyPlaceholder || "AC...",
				primaryIsSecret: false,
				secondaryLabel: "Auth Token",
				secondaryPlaceholder: "Auth token",
				secondaryIsSecret: true,
			};
	}
}
