// @ts-nocheck
import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import * as gon from "./gon.js";
import { parseAgentsListPayload, sendRpc } from "./helpers.js";
import { models as modelsSig } from "./stores/model-store.js";
import { ComboSelect, ConfirmDialog, Modal, ModelSelect, requestConfirm } from "./ui.js";

// ── State ──────────────────────────────────────────────────────────────

var webhooks = signal(gon.get("webhooks") || []);
var profiles = signal(gon.get("webhook_profiles") || []);
var showCreateModal = signal(false);
var editingWebhook = signal(null);
var viewingDeliveries = signal(null); // webhook id or null
var deliveries = signal([]);
var publicBaseUrl = signal("");

var _container = null;

function fetchPublicBaseUrl() {
	// Prefer ngrok public URL, then tailscale funnel, then window.location.origin.
	fetch("/api/ngrok/status")
		.then((r) => (r.ok ? r.json() : null))
		.then((data) => {
			if (data?.public_url) {
				publicBaseUrl.value = data.public_url.replace(/\/$/, "");
				return;
			}
			return fetch("/api/tailscale/status")
				.then((r) => (r.ok ? r.json() : null))
				.then((ts) => {
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

export function initWebhooks(container) {
	_container = container;
	container.style.cssText = "padding:0;overflow:hidden;";
	webhooks.value = gon.get("webhooks") || [];
	profiles.value = gon.get("webhook_profiles") || [];
	fetchPublicBaseUrl();
	render(html`<${WebhooksPage} />`, container);
}

export function teardownWebhooks() {
	if (_container) render(null, _container);
	_container = null;
}

function loadWebhooks() {
	sendRpc("webhooks.list", {}).then((res) => {
		if (res?.ok) webhooks.value = res.payload || [];
	});
}

function loadDeliveries(webhookId) {
	sendRpc("webhooks.deliveries", { webhookId, limit: 50, offset: 0 }).then((res) => {
		if (res?.ok) deliveries.value = res.payload || [];
	});
}

// ── Main Page ──────────────────────────────────────────────────────────

function WebhooksPage() {
	return html`
    <div class="flex-1 flex flex-col min-w-0 overflow-y-auto">
      ${viewingDeliveries.value ? html`<${DeliveriesPanel} />` : html`<${WebhooksListPanel} />`}
    </div>
    <${WebhookModal} />
    <${ConfirmDialog} />
  `;
}

// ── Webhooks List ──────────────────────────────────────────────────────

function WebhooksListPanel() {
	return html`
    <div class="p-4 flex flex-col gap-4">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Webhooks</h2>
        <button
          class="provider-btn"
          onClick=${() => {
						editingWebhook.value = null;
						showCreateModal.value = true;
					}}
        >
          + Create webhook
        </button>
      </div>

      ${
				webhooks.value.length === 0
					? html`<div class="text-sm text-[var(--muted)] py-8 text-center">
            No webhooks configured.
          </div>`
					: html`<div class="flex flex-col gap-2">
            ${webhooks.value.map((wh) => html`<${WebhookCard} key=${wh.id} webhook=${wh} />`)}
          </div>`
			}

      <div class="text-xs text-[var(--muted)]">
        Use the "Copy test command" button on each webhook, or the${" "}
        <a href="https://hoppscotch.io/download" target="_blank" rel="noopener"
          class="text-[var(--accent)] underline">Hoppscotch desktop app</a>${" "}to test.
      </div>
    </div>
  `;
}

function CopyTestCommandButton({ webhook }) {
	var [copied, setCopied] = useState(false);
	var url = `${publicBaseUrl.value || window.location.origin}/api/webhooks/ingest/${webhook.publicId}`;
	var cmd = `curl -sk -X POST ${url} \\
  -H 'Content-Type: application/json' \\
  -d '{"event": "test", "timestamp": "${new Date().toISOString()}"}'`;

	return html`<button
    class="provider-btn provider-btn-sm provider-btn-secondary"
    style="white-space:nowrap;"
    title=${copied ? "Copied!" : "Copy a curl command to test this webhook"}
    onClick=${() => {
			navigator.clipboard.writeText(cmd).then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), 2000);
			});
		}}
  >
    ${copied ? "Copied!" : "Copy test command"}
  </button>`;
}

function WebhookCard({ webhook }) {
	var wh = webhook;
	var profileInfo = profiles.value.find((p) => p.id === wh.sourceProfile);
	var profileName = profileInfo?.displayName || wh.sourceProfile || "Generic";

	return html`
    <div class="channel-card">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div>
            <div class="text-sm font-medium text-[var(--text-strong)]">
              ${wh.name}
            </div>
            <div class="text-xs text-[var(--muted)] mt-0.5">
              ${profileName}
              ${wh.agentId ? html` · Agent: ${wh.agentId}` : ""}
            </div>
          </div>
        </div>

        <div class="flex items-center gap-2">
          ${
						wh.deliveryCount > 0
							? html`<span class="text-xs text-[var(--muted)]">
                ${wh.deliveryCount} deliveries
              </span>`
							: ""
					}

          <span
            class="provider-item-badge ${wh.enabled ? "configured" : ""}"
            style=${wh.enabled ? "" : "background:var(--surface2);color:var(--muted)"}
          >
            ${wh.enabled ? "active" : "paused"}
          </span>

          <label class="cron-toggle" title=${wh.enabled ? "Pause webhook" : "Enable webhook"}>
            <input
              type="checkbox"
              checked=${wh.enabled}
              onChange=${(e) => {
								sendRpc("webhooks.update", {
									id: wh.id,
									patch: { enabled: e.target.checked },
								}).then(() => loadWebhooks());
							}}
            />
            <span class="cron-slider"></span>
          </label>

          <button
            class="provider-btn provider-btn-sm provider-btn-secondary"
            onClick=${() => {
							viewingDeliveries.value = wh.id;
							loadDeliveries(wh.id);
						}}
          >
            Deliveries
          </button>

          <button
            class="provider-btn provider-btn-sm provider-btn-secondary"
            onClick=${() => {
							editingWebhook.value = wh;
							showCreateModal.value = true;
						}}
          >
            Edit
          </button>

          <button
            class="provider-btn provider-btn-sm provider-btn-danger"
            onClick=${async () => {
							var ok = await requestConfirm(
								html`Delete webhook <strong>${wh.name}</strong>? This removes all delivery records. Chat sessions created by deliveries are preserved.`,
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

      <div class="flex items-center gap-2 mt-1">
        <div class="text-xs text-[var(--muted)] font-mono select-all flex-1 min-w-0 truncate">
          ${publicBaseUrl.value || window.location.origin}/api/webhooks/ingest/${wh.publicId}
        </div>
        <${CopyTestCommandButton} webhook=${wh} />
      </div>
    </div>
  `;
}

// ── Deliveries Panel ───────────────────────────────────────────────────

function DeliveriesPanel() {
	var wh = webhooks.value.find((w) => w.id === viewingDeliveries.value);
	var name = wh?.name || "Webhook";

	return html`
    <div class="p-4 flex flex-col gap-4">
      <div class="flex items-center gap-3">
        <button
          class="provider-btn provider-btn-secondary"
          onClick=${() => {
						viewingDeliveries.value = null;
					}}
        >
          Back
        </button>
        <h2 class="text-lg font-medium text-[var(--text-strong)]">
          ${name} — Deliveries
        </h2>
      </div>

      ${
				deliveries.value.length === 0
					? html`<div class="text-sm text-[var(--muted)] py-8 text-center">
            No deliveries yet.
          </div>`
					: html`<div class="flex flex-col gap-1">
            ${deliveries.value.map((d) => html`<${DeliveryRow} key=${d.id} delivery=${d} />`)}
          </div>`
			}
    </div>
  `;
}

function DeliveryRow({ delivery }) {
	var d = delivery;
	var statusColors = {
		completed: "color:var(--ok)",
		failed: "color:var(--error)",
		processing: "color:var(--warning)",
		filtered: "color:var(--muted)",
		deduplicated: "color:var(--muted)",
		rejected: "color:var(--error)",
	};
	var statusStyle = statusColors[d.status] || "";

	return html`
    <div class="provider-card text-xs">
      <div class="flex items-center gap-3 flex-1 min-w-0">
        <span
          class="font-medium"
          style=${statusStyle}
        >
          ${d.status}
        </span>
        <span class="text-[var(--muted)]">${d.eventType || "—"}</span>
        <span class="text-[var(--muted)]">
          ${d.receivedAt ? new Date(d.receivedAt).toLocaleString() : ""}
        </span>
        ${d.durationMs != null ? html`<span class="text-[var(--muted)]">${d.durationMs}ms</span>` : ""}
      </div>
      <div class="flex items-center gap-2">
        ${
					d.sessionKey
						? html`<span class="text-[var(--muted)] font-mono text-[10px]">
              ${d.sessionKey}
            </span>`
						: ""
				}
      </div>
    </div>
  `;
}

// ── Create / Edit Modal ────────────────────────────────────────────────

function WebhookModal() {
	var isEdit = !!editingWebhook.value;
	var saving = useSignal(false);
	var error = useSignal("");

	var nameRef = useRef(null);
	var descRef = useRef(null);
	var promptSuffixRef = useRef(null);
	var authSecretRef = useRef(null);

	var selectedAgent = useSignal(isEdit ? editingWebhook.value?.agentId || "" : "");
	var selectedModel = useSignal(isEdit ? editingWebhook.value?.model || "" : "");
	var sourceProfile = useSignal(isEdit ? editingWebhook.value?.sourceProfile || "generic" : "generic");
	var authMode = useSignal(isEdit ? editingWebhook.value?.authMode || "static_header" : "static_header");
	var sessionMode = useSignal(isEdit ? editingWebhook.value?.sessionMode || "per_delivery" : "per_delivery");

	var gonAgents = parseAgentsListPayload(gon.get("agents"));
	var agentOptions = (Array.isArray(gonAgents?.agents) ? gonAgents.agents : []).map((a) => ({
		value: a.id,
		label: a.name || a.id,
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

	function onSave(e) {
		e.preventDefault();
		saving.value = true;
		error.value = "";

		var name = nameRef.current?.value?.trim();
		if (!name) {
			error.value = "Name is required";
			saving.value = false;
			return;
		}

		var params = {
			name,
			description: descRef.current?.value?.trim() || null,
			agentId: selectedAgent.value || null,
			model: selectedModel.value || null,
			systemPromptSuffix: promptSuffixRef.current?.value?.trim() || null,
			authMode: authMode.value,
			sessionMode: sessionMode.value,
		};
		// source_profile is immutable after creation — only include on create.
		if (!isEdit) {
			params.sourceProfile = sourceProfile.value;
		}

		// Build auth config based on mode. The key name must match what
		// the Rust verify function expects: "token" for bearer/gitlab,
		// "secret" for all HMAC-based modes, "header"+"value" for static.
		var secret = authSecretRef.current?.value?.trim();
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

		var method = isEdit ? "webhooks.update" : "webhooks.create";
		var rpcParams = isEdit ? { id: editingWebhook.value.id, patch: params } : params;

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

	var profileOptions = profiles.value.map((p) => ({
		value: p.id,
		label: p.displayName,
	}));

	var authOptions = [
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

	var sessionOptions = [
		{ value: "per_delivery", label: "New session per delivery" },
		{ value: "per_entity", label: "Group by entity (PR, issue, etc.)" },
		{ value: "named_session", label: "Named session (accumulative)" },
	];

	var wh = editingWebhook.value;

	return html`<${Modal}
    show=${showCreateModal.value}
    onClose=${() => {
			showCreateModal.value = false;
			editingWebhook.value = null;
		}}
    title=${isEdit ? "Edit Webhook" : "Create Webhook"}
  >
    <form onSubmit=${onSave} class="provider-key-form" style="max-width:460px;">
      ${
				error.value &&
				html`<div
        class="text-xs text-[var(--error)] mb-2"
      >
        ${error.value}
      </div>`
			}

      <label class="text-xs text-[var(--muted)]">Name</label>
      <input
        ref=${nameRef}
        class="provider-key-input"
        placeholder="e.g. GitHub PR Review"
        value=${wh?.name || ""}
      />

      <label class="text-xs text-[var(--muted)]">Description</label>
      <input
        ref=${descRef}
        class="provider-key-input"
        placeholder="Optional description"
        value=${wh?.description || ""}
      />

      <label class="text-xs text-[var(--muted)]">Source Profile${isEdit ? " (read-only after creation)" : ""}</label>
      <select
        class="provider-key-input"
        value=${sourceProfile.value}
        disabled=${isEdit}
        onChange=${(e) => {
					sourceProfile.value = e.target.value;
					var prof = profiles.value.find((p) => p.id === e.target.value);
					if (prof?.defaultAuthMode) authMode.value = prof.defaultAuthMode;
				}}
      >
        ${profileOptions.map((o) => html`<option value=${o.value}>${o.label}</option>`)}
      </select>

      <label class="text-xs text-[var(--muted)]">Auth Mode</label>
      <select
        class="provider-key-input"
        value=${authMode.value}
        onChange=${(e) => (authMode.value = e.target.value)}
      >
        ${authOptions.map((o) => html`<option value=${o.value}>${o.label}</option>`)}
      </select>

      ${
				authMode.value !== "none" &&
				html`<div>
        <label class="text-xs text-[var(--muted)]">Secret / Token</label>
        <input
          ref=${authSecretRef}
          type="password"
          class="provider-key-input"
          placeholder="Webhook secret or token"
        />
      </div>`
			}

      <label class="text-xs text-[var(--muted)]">Agent</label>
      <${ComboSelect}
        options=${agentOptions}
        value=${selectedAgent.value}
        onChange=${(v) => {
					selectedAgent.value = v;
				}}
        placeholder="Default agent"
        searchPlaceholder="Search agents…"
      />

      <label class="text-xs text-[var(--muted)]">Model</label>
      <${ModelSelect}
        models=${modelsSig.value}
        value=${selectedModel.value}
        onChange=${(v) => {
					selectedModel.value = v;
				}}
        placeholder=${
					modelsSig.value.length > 0
						? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
						: "(server default)"
				}
      />

      <label class="text-xs text-[var(--muted)]">Session Mode</label>
      <select
        class="provider-key-input"
        value=${sessionMode.value}
        onChange=${(e) => (sessionMode.value = e.target.value)}
      >
        ${sessionOptions.map((o) => html`<option value=${o.value}>${o.label}</option>`)}
      </select>

      <label class="text-xs text-[var(--muted)]">System Prompt Suffix (optional)</label>
      <textarea
        ref=${promptSuffixRef}
        class="provider-key-input"
        style="min-height:80px;resize:vertical;font-family:var(--font-mono);font-size:0.75rem;"
        placeholder="Additional instructions for the agent when processing this webhook..."
      >
${wh?.systemPromptSuffix || ""}</textarea
      >

      <div class="flex gap-2 justify-end mt-2">
        <button
          type="button"
          class="provider-btn provider-btn-secondary"
          onClick=${() => {
						showCreateModal.value = false;
						editingWebhook.value = null;
					}}
        >
          Cancel
        </button>
        <button type="submit" class="provider-btn" disabled=${saving.value}>
          ${saving.value ? "Saving..." : isEdit ? "Save" : "Create"}
        </button>
      </div>
    </form>
  </${Modal}>`;
}
