// ── Webhooks page (Preact + Signals) ────────────────────────

import { signal, useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import * as gon from "../gon";
import { parseAgentsListPayload, sendRpc } from "../helpers";
import { models as modelsSig } from "../stores/model-store";
import { ComboSelect, ConfirmDialog, Modal, ModelSelect, requestConfirm } from "../ui";

// ── Types ───────────────────────────────────────────────────

interface Webhook {
	id: string;
	publicId: string;
	name: string;
	description?: string;
	enabled: boolean;
	sourceProfile?: string;
	agentId?: string;
	model?: string;
	authMode?: string;
	sessionMode?: string;
	systemPromptSuffix?: string;
	deliveryCount?: number;
}

interface WebhookProfile {
	id: string;
	displayName: string;
	defaultAuthMode?: string;
}

interface Delivery {
	id: string;
	status: string;
	eventType?: string;
	receivedAt?: string;
	durationMs?: number;
	sessionKey?: string;
}

interface ComboOption {
	value: string;
	label: string;
}

// ── State ─────────────��────────────────────────────────────────────────

const webhooks = signal<Webhook[]>((gon.get("webhooks") as Webhook[]) || []);
const profiles = signal<WebhookProfile[]>((gon.get("webhook_profiles") as WebhookProfile[]) || []);
const showCreateModal = signal(false);
const editingWebhook = signal<Webhook | null>(null);
const viewingDeliveries = signal<string | null>(null); // webhook id or null
const deliveries = signal<Delivery[]>([]);
const publicBaseUrl = signal("");

let _container: HTMLElement | null = null;

function fetchPublicBaseUrl(): void {
	// Prefer ngrok public URL, then tailscale funnel, then window.location.origin.
	fetch("/api/ngrok/status")
		.then((r) => (r.ok ? r.json() : null))
		.then((data: { public_url?: string } | null) => {
			if (data?.public_url) {
				publicBaseUrl.value = data.public_url.replace(/\/$/, "");
				return;
			}
			return fetch("/api/tailscale/status")
				.then((r) => (r.ok ? r.json() : null))
				.then((ts: { mode?: string; url?: string } | null) => {
					if (ts?.mode === "funnel" && ts?.url) {
						publicBaseUrl.value = ts.url.replace(/\/$/, "");
					} else {
						publicBaseUrl.value = window.location.origin;
					}
				});
		})
		.catch(() => {
			publicBaseUrl.value = window.location.origin;
		});
}

export function initWebhooks(container: HTMLElement): void {
	_container = container;
	container.style.cssText = "padding:0;overflow:hidden;";
	webhooks.value = (gon.get("webhooks") as Webhook[]) || [];
	profiles.value = (gon.get("webhook_profiles") as WebhookProfile[]) || [];
	fetchPublicBaseUrl();
	render(<WebhooksPageComponent />, container);
}

export function teardownWebhooks(): void {
	if (_container) render(null, _container);
	_container = null;
}

function loadWebhooks(): void {
	sendRpc("webhooks.list", {}).then((res) => {
		if (res?.ok) webhooks.value = (res.payload as Webhook[]) || [];
	});
}

function loadDeliveries(webhookId: string): void {
	sendRpc("webhooks.deliveries", { webhookId, limit: 50, offset: 0 }).then((res) => {
		if (res?.ok) deliveries.value = (res.payload as Delivery[]) || [];
	});
}

// ── Main Page ─────────────��────────────────────────────────────────────

function WebhooksPageComponent(): VNode {
	return (
		<>
			<div className="flex-1 flex flex-col min-w-0 overflow-y-auto">
				{viewingDeliveries.value ? <DeliveriesPanel /> : <WebhooksListPanel />}
			</div>
			<WebhookModal />
			<ConfirmDialog />
		</>
	);
}

// ── Webhooks List ────────────���─────────────────────────────────────────

function WebhooksListPanel(): VNode {
	return (
		<div className="p-4 flex flex-col gap-4">
			<div className="flex items-center gap-3">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Webhooks</h2>
				<button
					className="provider-btn"
					onClick={() => {
						editingWebhook.value = null;
						showCreateModal.value = true;
					}}
				>
					+ Create webhook
				</button>
			</div>

			{webhooks.value.length === 0 ? (
				<div className="text-sm text-[var(--muted)] py-8 text-center">No webhooks configured.</div>
			) : (
				<div className="flex flex-col gap-2">
					{webhooks.value.map((wh) => (
						<WebhookCard key={wh.id} webhook={wh} />
					))}
				</div>
			)}

			<div className="text-xs text-[var(--muted)]">
				Use the "Copy test command" button on each webhook, or the{" "}
				<a
					href="https://hoppscotch.io/download"
					target="_blank"
					rel="noopener"
					className="text-[var(--accent)] underline"
				>
					Hoppscotch desktop app
				</a>{" "}
				to test.
			</div>
		</div>
	);
}

function CopyTestCommandButton({ webhook }: { webhook: Webhook }): VNode {
	const [copied, setCopied] = useState(false);
	const url = `${publicBaseUrl.value || window.location.origin}/api/webhooks/ingest/${webhook.publicId}`;
	const cmd = `curl -sk -X POST ${url} \\
  -H 'Content-Type: application/json' \\
  -d '{"event": "test", "timestamp": "${new Date().toISOString()}"}'`;

	return (
		<button
			className="provider-btn provider-btn-sm provider-btn-secondary"
			style={{ whiteSpace: "nowrap" }}
			title={copied ? "Copied!" : "Copy a curl command to test this webhook"}
			onClick={() => {
				navigator.clipboard.writeText(cmd).then(() => {
					setCopied(true);
					setTimeout(() => setCopied(false), 2000);
				});
			}}
		>
			{copied ? "Copied!" : "Copy test command"}
		</button>
	);
}

function WebhookCard({ webhook }: { webhook: Webhook }): VNode {
	const wh = webhook;
	const profileInfo = profiles.value.find((p) => p.id === wh.sourceProfile);
	const profileName = profileInfo?.displayName || wh.sourceProfile || "Generic";

	return (
		<div className="channel-card">
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-3">
					<div>
						<div className="text-sm font-medium text-[var(--text-strong)]">{wh.name}</div>
						<div className="text-xs text-[var(--muted)] mt-0.5">
							{profileName}
							{wh.agentId ? ` \u00B7 Agent: ${wh.agentId}` : ""}
						</div>
					</div>
				</div>

				<div className="flex items-center gap-2">
					{wh.deliveryCount != null && wh.deliveryCount > 0 ? (
						<span className="text-xs text-[var(--muted)]">{wh.deliveryCount} deliveries</span>
					) : (
						""
					)}

					<span
						className={`provider-item-badge ${wh.enabled ? "configured" : ""}`}
						style={wh.enabled ? undefined : { background: "var(--surface2)", color: "var(--muted)" }}
					>
						{wh.enabled ? "active" : "paused"}
					</span>

					<label className="cron-toggle" title={wh.enabled ? "Pause webhook" : "Enable webhook"}>
						<input
							type="checkbox"
							checked={wh.enabled}
							onChange={(e) => {
								sendRpc("webhooks.update", {
									id: wh.id,
									patch: { enabled: (e.target as HTMLInputElement).checked },
								}).then(() => loadWebhooks());
							}}
						/>
						<span className="cron-slider" />
					</label>

					<button
						className="provider-btn provider-btn-sm provider-btn-secondary"
						onClick={() => {
							viewingDeliveries.value = wh.id;
							loadDeliveries(wh.id);
						}}
					>
						Deliveries
					</button>

					<button
						className="provider-btn provider-btn-sm provider-btn-secondary"
						onClick={() => {
							editingWebhook.value = wh;
							showCreateModal.value = true;
						}}
					>
						Edit
					</button>

					<button
						className="provider-btn provider-btn-sm provider-btn-danger"
						onClick={async () => {
							const ok = await requestConfirm(
								`Delete webhook "${wh.name}"? This removes all delivery records. Chat sessions created by deliveries are preserved.`,
							);
							if (ok) {
								sendRpc("webhooks.delete", { id: wh.id }).then(() => loadWebhooks());
							}
						}}
					>
						Delete
					</button>
				</div>
			</div>

			<div className="flex items-center gap-2 mt-1">
				<div className="text-xs text-[var(--muted)] font-mono select-all flex-1 min-w-0 truncate">
					{publicBaseUrl.value || window.location.origin}/api/webhooks/ingest/{wh.publicId}
				</div>
				<CopyTestCommandButton webhook={wh} />
			</div>
		</div>
	);
}

// ── Deliveries Panel ───────────────────────────────────────────────────

function DeliveriesPanel(): VNode {
	const wh = webhooks.value.find((w) => w.id === viewingDeliveries.value);
	const name = wh?.name || "Webhook";

	return (
		<div className="p-4 flex flex-col gap-4">
			<div className="flex items-center gap-3">
				<button
					className="provider-btn provider-btn-secondary"
					onClick={() => {
						viewingDeliveries.value = null;
					}}
				>
					Back
				</button>
				<h2 className="text-lg font-medium text-[var(--text-strong)]">
					{name} {"\u2014"} Deliveries
				</h2>
			</div>

			{deliveries.value.length === 0 ? (
				<div className="text-sm text-[var(--muted)] py-8 text-center">No deliveries yet.</div>
			) : (
				<div className="flex flex-col gap-1">
					{deliveries.value.map((d) => (
						<DeliveryRow key={d.id} delivery={d} />
					))}
				</div>
			)}
		</div>
	);
}

function DeliveryRow({ delivery }: { delivery: Delivery }): VNode {
	const d = delivery;
	const statusColors: Record<string, string> = {
		completed: "var(--ok)",
		failed: "var(--error)",
		processing: "var(--warning)",
		filtered: "var(--muted)",
		deduplicated: "var(--muted)",
		rejected: "var(--error)",
	};
	const statusColor = statusColors[d.status];

	return (
		<div className="provider-card text-xs">
			<div className="flex items-center gap-3 flex-1 min-w-0">
				<span className="font-medium" style={statusColor ? { color: statusColor } : undefined}>
					{d.status}
				</span>
				<span className="text-[var(--muted)]">{d.eventType || "\u2014"}</span>
				<span className="text-[var(--muted)]">{d.receivedAt ? new Date(d.receivedAt).toLocaleString() : ""}</span>
				{d.durationMs != null ? <span className="text-[var(--muted)]">{d.durationMs}ms</span> : ""}
			</div>
			<div className="flex items-center gap-2">
				{d.sessionKey ? <span className="text-[var(--muted)] font-mono text-[10px]">{d.sessionKey}</span> : ""}
			</div>
		</div>
	);
}

// ── Create / Edit Modal ───────────────���────────────────────────────────

function WebhookModal(): VNode | null {
	const isEdit = !!editingWebhook.value;
	const saving = useSignal(false);
	const error = useSignal("");

	const nameRef = useRef<HTMLInputElement>(null);
	const descRef = useRef<HTMLInputElement>(null);
	const promptSuffixRef = useRef<HTMLTextAreaElement>(null);
	const authSecretRef = useRef<HTMLInputElement>(null);

	const selectedAgent = useSignal(isEdit ? editingWebhook.value?.agentId || "" : "");
	const selectedModel = useSignal(isEdit ? editingWebhook.value?.model || "" : "");
	const sourceProfile = useSignal(isEdit ? editingWebhook.value?.sourceProfile || "generic" : "generic");
	const authMode = useSignal(isEdit ? editingWebhook.value?.authMode || "static_header" : "static_header");
	const sessionMode = useSignal(isEdit ? editingWebhook.value?.sessionMode || "per_delivery" : "per_delivery");

	const gonAgents = parseAgentsListPayload(gon.get("agents") as Parameters<typeof parseAgentsListPayload>[0]);
	const agentOptions: ComboOption[] = (Array.isArray(gonAgents?.agents) ? gonAgents.agents : []).map((a) => ({
		value: a.id as string,
		label: (a.name as string) || (a.id as string),
	}));

	useEffect(() => {
		if (editingWebhook.value) {
			selectedAgent.value = editingWebhook.value.agentId || "";
			selectedModel.value = editingWebhook.value.model || "";
			sourceProfile.value = editingWebhook.value.sourceProfile || "generic";
			authMode.value = editingWebhook.value.authMode || "static_header";
			sessionMode.value = editingWebhook.value.sessionMode || "per_delivery";
		} else {
			selectedAgent.value = "";
			selectedModel.value = "";
			sourceProfile.value = "generic";
			authMode.value = "static_header";
			sessionMode.value = "per_delivery";
		}
	}, [editingWebhook.value]);

	function onSave(e: Event): void {
		e.preventDefault();
		saving.value = true;
		error.value = "";

		const name = nameRef.current?.value?.trim();
		if (!name) {
			error.value = "Name is required";
			saving.value = false;
			return;
		}

		const params: Record<string, unknown> = {
			name,
			description: descRef.current?.value?.trim() || null,
			agentId: selectedAgent.value || null,
			model: selectedModel.value || null,
			systemPromptSuffix: promptSuffixRef.current?.value?.trim() || null,
			authMode: authMode.value,
			sessionMode: sessionMode.value,
		};
		// source_profile is immutable after creation -- only include on create.
		if (!isEdit) {
			params.sourceProfile = sourceProfile.value;
		}

		// Build auth config based on mode. The key name must match what
		// the Rust verify function expects: "token" for bearer/gitlab,
		// "secret" for all HMAC-based modes, "header"+"value" for static.
		const secret = authSecretRef.current?.value?.trim();
		if (secret && authMode.value !== "none") {
			if (authMode.value === "static_header") {
				params.authConfig = { header: "X-Webhook-Secret", value: secret };
			} else if (authMode.value === "bearer" || authMode.value === "gitlab_token") {
				params.authConfig = { token: secret };
			} else {
				// All HMAC-based modes (github, stripe, linear, pagerduty, sentry)
				params.authConfig = { secret };
			}
		}

		const method = isEdit ? "webhooks.update" : "webhooks.create";
		const rpcParams = isEdit ? { id: editingWebhook.value?.id, patch: params } : params;

		sendRpc(method, rpcParams).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showCreateModal.value = false;
				editingWebhook.value = null;
				loadWebhooks();
			} else {
				error.value = res?.error?.message || "Failed to save";
			}
		});
	}

	const profileOptions: ComboOption[] = profiles.value.map((p) => ({
		value: p.id,
		label: p.displayName,
	}));

	const authOptions: ComboOption[] = [
		{ value: "none", label: "None (testing only)" },
		{ value: "static_header", label: "Static Header" },
		{ value: "bearer", label: "Bearer Token" },
		{ value: "github_hmac_sha256", label: "GitHub HMAC-SHA256" },
		{ value: "gitlab_token", label: "GitLab Token" },
		{ value: "stripe_webhook_signature", label: "Stripe Signature" },
		{ value: "linear_webhook_signature", label: "Linear Signature" },
		{ value: "pagerduty_v2_signature", label: "PagerDuty v2 Signature" },
		{ value: "sentry_webhook_signature", label: "Sentry Signature" },
	];

	const sessionOptions: ComboOption[] = [
		{ value: "per_delivery", label: "New session per delivery" },
		{ value: "per_entity", label: "Group by entity (PR, issue, etc.)" },
		{ value: "named_session", label: "Named session (accumulative)" },
	];

	const wh = editingWebhook.value;

	return (
		<Modal
			show={showCreateModal.value}
			onClose={() => {
				showCreateModal.value = false;
				editingWebhook.value = null;
			}}
			title={isEdit ? "Edit Webhook" : "Create Webhook"}
		>
			<form onSubmit={onSave} className="provider-key-form" style={{ maxWidth: "460px" }}>
				{error.value && <div className="text-xs text-[var(--error)] mb-2">{error.value}</div>}

				<label className="text-xs text-[var(--muted)]">Name</label>
				<input
					ref={nameRef}
					className="provider-key-input"
					placeholder="e.g. GitHub PR Review"
					value={wh?.name || ""}
				/>

				<label className="text-xs text-[var(--muted)]">Description</label>
				<input
					ref={descRef}
					className="provider-key-input"
					placeholder="Optional description"
					value={wh?.description || ""}
				/>

				<label className="text-xs text-[var(--muted)]">
					Source Profile{isEdit ? " (read-only after creation)" : ""}
				</label>
				<select
					className="provider-key-input"
					value={sourceProfile.value}
					disabled={isEdit}
					onChange={(e) => {
						sourceProfile.value = (e.target as HTMLSelectElement).value;
						const prof = profiles.value.find((p) => p.id === (e.target as HTMLSelectElement).value);
						if (prof?.defaultAuthMode) authMode.value = prof.defaultAuthMode;
					}}
				>
					{profileOptions.map((o) => (
						<option key={o.value} value={o.value}>
							{o.label}
						</option>
					))}
				</select>

				<label className="text-xs text-[var(--muted)]">Auth Mode</label>
				<select
					className="provider-key-input"
					value={authMode.value}
					onChange={(e) => {
						authMode.value = (e.target as HTMLSelectElement).value;
					}}
				>
					{authOptions.map((o) => (
						<option key={o.value} value={o.value}>
							{o.label}
						</option>
					))}
				</select>

				{authMode.value !== "none" && (
					<div>
						<label className="text-xs text-[var(--muted)]">Secret / Token</label>
						<input
							ref={authSecretRef}
							type="password"
							className="provider-key-input"
							placeholder="Webhook secret or token"
						/>
					</div>
				)}

				<label className="text-xs text-[var(--muted)]">Agent</label>
				<ComboSelect
					options={agentOptions}
					value={selectedAgent.value}
					onChange={(v: string) => {
						selectedAgent.value = v;
					}}
					placeholder="Default agent"
					searchPlaceholder={"Search agents\u2026"}
				/>

				<label className="text-xs text-[var(--muted)]">Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={selectedModel.value}
					onChange={(v: string) => {
						selectedModel.value = v;
					}}
					placeholder={
						modelsSig.value.length > 0
							? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
							: "(server default)"
					}
				/>

				<label className="text-xs text-[var(--muted)]">Session Mode</label>
				<select
					className="provider-key-input"
					value={sessionMode.value}
					onChange={(e) => {
						sessionMode.value = (e.target as HTMLSelectElement).value;
					}}
				>
					{sessionOptions.map((o) => (
						<option key={o.value} value={o.value}>
							{o.label}
						</option>
					))}
				</select>

				<label className="text-xs text-[var(--muted)]">System Prompt Suffix (optional)</label>
				<textarea
					ref={promptSuffixRef}
					className="provider-key-input"
					style={{
						minHeight: "80px",
						resize: "vertical",
						fontFamily: "var(--font-mono)",
						fontSize: "0.75rem",
					}}
					placeholder="Additional instructions for the agent when processing this webhook..."
				>
					{wh?.systemPromptSuffix || ""}
				</textarea>

				<div className="flex gap-2 justify-end mt-2">
					<button
						type="button"
						className="provider-btn provider-btn-secondary"
						onClick={() => {
							showCreateModal.value = false;
							editingWebhook.value = null;
						}}
					>
						Cancel
					</button>
					<button type="submit" className="provider-btn" disabled={saving.value}>
						{saving.value ? "Saving..." : isEdit ? "Save" : "Create"}
					</button>
				</div>
			</form>
		</Modal>
	);
}
