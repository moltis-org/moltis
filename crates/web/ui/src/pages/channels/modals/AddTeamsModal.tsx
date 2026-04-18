// ── Add Microsoft Teams modal ────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect } from "preact/hooks";

import {
	addChannel,
	buildTeamsEndpoint,
	defaultTeamsBaseUrl,
	generateWebhookSecretHex,
	parseChannelConfigPatch,
	validateChannelFields,
} from "../../../channel-utils";
import { models as modelsSig } from "../../../stores/model-store";
import { targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, showToast } from "../../../ui";
import {
	type ChannelConfig,
	ConnectionModeHint,
	loadChannels,
	showAddTeams,
	type TailscaleStatus,
} from "../../ChannelsPage";
import { AdvancedConfigPatchField, SharedChannelFields } from "../ChannelFields";

export function AddTeamsModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const allowlistItems = useSignal<string[]>([]);
	const accountDraft = useSignal("");
	const webhookSecret = useSignal("");
	const baseUrlDraft = useSignal(defaultTeamsBaseUrl());
	const bootstrapEndpoint = useSignal("");
	const tsStatus = useSignal<TailscaleStatus | null>(null);
	const tsLoading = useSignal(true);
	const enablingFunnel = useSignal(false);
	const advancedConfigPatch = useSignal("");

	// Fetch Tailscale status on mount.
	useEffect(() => {
		fetch("/api/tailscale/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((data: TailscaleStatus | null) => {
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

	function onEnableFunnel(): void {
		enablingFunnel.value = true;
		error.value = "";
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode: "funnel" }),
		})
			.then((r) => r.json())
			.then((data: TailscaleStatus & { ok?: boolean; error?: string }) => {
				enablingFunnel.value = false;
				if (data?.ok !== false && data?.url) {
					baseUrlDraft.value = data.url.replace(/\/$/, "");
					tsStatus.value = data;
					refreshBootstrapEndpoint();
				} else {
					error.value = data?.error || "Failed to enable Tailscale Funnel.";
				}
			})
			.catch((e: Error) => {
				enablingFunnel.value = false;
				error.value = `Tailscale error: ${e.message}`;
			});
	}

	function refreshBootstrapEndpoint(): void {
		if (!bootstrapEndpoint.value) return;
		bootstrapEndpoint.value = buildTeamsEndpoint(baseUrlDraft.value, accountDraft.value, webhookSecret.value);
	}

	function onBootstrapTeams(): void {
		const accountId = accountDraft.value.trim();
		if (!accountId) {
			error.value = "Enter App ID / Account ID first.";
			return;
		}
		let secret = webhookSecret.value.trim();
		if (!secret) {
			secret = generateWebhookSecretHex();
			webhookSecret.value = secret;
		}
		const endpoint = buildTeamsEndpoint(baseUrlDraft.value, accountId, secret);
		if (!endpoint) {
			error.value = "Enter a valid public base URL (example: https://bot.example.com).";
			return;
		}
		bootstrapEndpoint.value = endpoint;
		error.value = "";
		showToast("Teams endpoint generated");
	}

	function copyBootstrapEndpoint(): void {
		if (!bootstrapEndpoint.value) return;
		if (typeof navigator === "undefined" || !navigator.clipboard?.writeText) {
			showToast("Clipboard is unavailable");
			return;
		}
		navigator.clipboard.writeText(bootstrapEndpoint.value).then(() => {
			showToast("Messaging endpoint copied");
		});
	}

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const accountId = accountDraft.value.trim();
		const credential = (form.querySelector("[data-field=credential]") as HTMLInputElement).value.trim();
		const v = validateChannelFields(ChannelType.MsTeams, accountId, credential);
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
			app_id: accountId,
			app_password: credential,
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			allowlist: allowlistItems.value,
		};
		if (webhookSecret.value.trim()) addConfig.webhook_secret = webhookSecret.value.trim();
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel(ChannelType.MsTeams, accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
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
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect channel.";
			}
		});
	}

	return (
		<Modal
			show={showAddTeams.value}
			onClose={() => {
				showAddTeams.value = false;
			}}
			title="Connect Microsoft Teams"
		>
			<div className="channel-form">
				{!(tsLoading.value || (tsStatus.value?.mode === "funnel" && tsStatus.value?.url)) && (
					<div className="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs flex flex-col gap-2">
						<span className="font-medium text-[var(--text-strong)]">Public URL required</span>
						<span className="text-[var(--muted)]">
							Teams sends messages to your server via webhook. Your Moltis instance must be reachable over HTTPS.
						</span>
						{tsStatus.value?.installed && tsStatus.value?.tailscale_up ? (
							<div className="flex flex-col gap-2">
								<span className="text-[var(--muted)]">
									Tailscale is connected. Enable <strong>Funnel</strong> to make it publicly reachable:
								</span>
								<button
									type="button"
									className="provider-btn provider-btn-sm"
									onClick={onEnableFunnel}
									disabled={enablingFunnel.value}
								>
									{enablingFunnel.value ? "Enabling\u2026" : "Enable Tailscale Funnel"}
								</button>
							</div>
						) : (
							<span className="text-[var(--muted)]">
								Enable <strong>Tailscale Funnel</strong> in Settings, or use{" "}
								<a href="https://ngrok.com/" target="_blank" className="text-[var(--accent)] underline" rel="noopener">
									ngrok
								</a>{" "}
								/{" "}
								<a
									href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/"
									target="_blank"
									className="text-[var(--accent)] underline"
									rel="noopener"
								>
									Cloudflare Tunnel
								</a>
								.
							</span>
						)}
					</div>
				)}
				{tsStatus.value?.mode === "funnel" && tsStatus.value?.url && (
					<div className="rounded-md border border-green-500/30 bg-green-500/5 p-3 text-xs flex items-center gap-2">
						<span className="text-green-600">{"\u2713"}</span>
						<span className="text-[var(--muted)]">
							Tailscale Funnel active &mdash; publicly reachable at <strong>{tsStatus.value.url}</strong>
						</span>
					</div>
				)}
				<div className="channel-card">
					<div className="flex flex-col gap-1">
						<span className="text-xs font-medium text-[var(--text-strong)]">How to create a Teams bot</span>
						<span className="text-xs font-medium text-[var(--text-strong)] opacity-70" style={{ fontSize: "10px" }}>
							Option A: Teams Developer Portal (easiest)
						</span>
						<div className="text-xs text-[var(--muted)]">
							1. Open{" "}
							<a
								href="https://dev.teams.microsoft.com/bots"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								Teams Developer Portal &rarr; Bot Management
							</a>
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. Click <strong>+ New Bot</strong>, give it a name, copy the <strong>Bot ID</strong> (App ID)
						</div>
						<div className="text-xs text-[var(--muted)]">
							3. Under <strong>Client secrets</strong>, add a secret and copy the value (App Password)
						</div>
						<span
							className="text-xs font-medium text-[var(--text-strong)] opacity-70"
							style={{ fontSize: "10px", marginTop: "4px" }}
						>
							Option B: Azure Portal
						</span>
						<div className="text-xs text-[var(--muted)]">
							1.{" "}
							<a
								href="https://portal.azure.com/#create/Microsoft.AzureBot"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								Create an Azure Bot
							</a>
							, then find App ID in Configuration
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. Click <strong>Manage Password</strong> &rarr; <strong>New client secret</strong> for the App Password
						</div>
						<div className="text-xs text-[var(--muted)]" style={{ marginTop: "4px" }}>
							Then generate the endpoint below and paste it as the <strong>Messaging endpoint</strong> in your bot
							settings.{" "}
							<a
								href="https://docs.moltis.org/teams.html"
								target="_blank"
								className="text-[var(--accent)] underline"
								rel="noopener"
							>
								Full guide &rarr;
							</a>
						</div>
					</div>
				</div>
				<ConnectionModeHint type={ChannelType.MsTeams} />
				<label className="text-xs text-[var(--muted)]">App ID (Bot ID from Azure)</label>
				<input
					data-field="accountId"
					type="text"
					placeholder="e.g. 12345678-abcd-efgh-ijkl-000000000000"
					value={accountDraft.value}
					onInput={(e) => {
						accountDraft.value = targetValue(e);
						refreshBootstrapEndpoint();
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">App Password (client secret from Azure)</label>
				<input
					data-field="credential"
					type="password"
					placeholder="Client secret value"
					className="channel-input"
					autoComplete="new-password"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
					name="teams_app_password"
				/>
				<div>
					<label className="text-xs text-[var(--muted)]">
						Webhook Secret <span className="opacity-60">(optional &mdash; auto-generated if blank)</span>
					</label>
					<input
						type="text"
						placeholder="Leave blank to auto-generate"
						className="channel-input"
						value={webhookSecret.value}
						onInput={(e) => {
							webhookSecret.value = targetValue(e);
							refreshBootstrapEndpoint();
						}}
					/>
					<label className="text-xs text-[var(--muted)] mt-2">
						Public Base URL <span className="opacity-60">(your server's HTTPS address)</span>
					</label>
					<input
						type="text"
						placeholder="https://bot.example.com"
						className="channel-input"
						value={baseUrlDraft.value}
						onInput={(e) => {
							baseUrlDraft.value = targetValue(e);
							refreshBootstrapEndpoint();
						}}
					/>
					<div className="flex gap-2 mt-2">
						<button
							type="button"
							className="provider-btn provider-btn-sm provider-btn-secondary"
							onClick={onBootstrapTeams}
						>
							Bootstrap Teams
						</button>
						{bootstrapEndpoint.value && (
							<button
								type="button"
								className="provider-btn provider-btn-sm provider-btn-secondary"
								onClick={copyBootstrapEndpoint}
							>
								Copy Endpoint
							</button>
						)}
					</div>
					{bootstrapEndpoint.value && (
						<div className="mt-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] p-2">
							<div className="text-xs text-[var(--muted)] mb-1">
								Messaging endpoint &mdash; paste this into your bot's configuration:
							</div>
							<code className="text-xs block break-all select-all">{bootstrapEndpoint.value}</code>
						</div>
					)}
					<div className="text-[10px] text-[var(--muted)] mt-1 opacity-70">
						Teams requires HTTPS. For local dev, use{" "}
						<a href="https://ngrok.com/" target="_blank" className="text-[var(--accent)] underline" rel="noopener">
							ngrok
						</a>{" "}
						or{" "}
						<a
							href="https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/"
							target="_blank"
							className="text-[var(--accent)] underline"
							rel="noopener"
						>
							Cloudflare Tunnel
						</a>
						.
					</div>
				</div>
				<SharedChannelFields addModel={addModel} allowlistItems={allowlistItems} />
				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(value) => {
						advancedConfigPatch.value = value;
					}}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value}>
					{saving.value ? "Connecting\u2026" : "Connect Microsoft Teams"}
				</button>
			</div>
		</Modal>
	);
}
