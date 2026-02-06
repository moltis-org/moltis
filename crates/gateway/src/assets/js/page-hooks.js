// ── Hooks page ─────────────────────────────────────────────

import { signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import * as S from "./state.js";

// ── Signals ─────────────────────────────────────────────────
var hooks = signal([]);
var loading = signal(false);
var toasts = signal([]);
var toastId = 0;

// ── Helpers ─────────────────────────────────────────────────
function showToast(message, type) {
	var id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

async function refreshHooks() {
	loading.value = true;
	try {
		var res = await fetch("/api/hooks");
		if (res.ok) {
			var data = await res.json();
			hooks.value = data?.hooks || [];
		}
	} catch {
		var rpc = await sendRpc("hooks.list", {});
		if (rpc.ok) hooks.value = rpc.payload?.hooks || [];
	}
	loading.value = false;
	updateNavCount("hooks", hooks.value.length);
}

// ── Components ──────────────────────────────────────────────

function Toasts() {
	if (toasts.value.length === 0) return null;
	return html`
    <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
      ${toasts.value.map(
				(t) => html`
          <div key=${t.id} class="px-4 py-2.5 rounded-[var(--radius)] text-sm shadow-lg border ${t.type === "error" ? "bg-[var(--danger-bg)] border-[var(--danger)] text-[var(--danger)]" : "bg-[var(--surface2)] border-[var(--border)] text-[var(--text)]"}">
            ${t.message}
          </div>`,
			)}
    </div>`;
}

function StatusBadge({ hook }) {
	if (!hook.eligible) {
		return html`<span class="tier-badge">Ineligible</span>`;
	}
	if (!hook.enabled) {
		return html`<span class="tier-badge">Disabled</span>`;
	}
	return html`<span class="recommended-badge">Active</span>`;
}

function SourceBadge({ source }) {
	var label =
		source === "project" ? "Project" : source === "user" ? "User" : source === "builtin" ? "Built-in" : source;
	return html`<span class="tier-badge">${label}</span>`;
}

// Safe: body_html is server-rendered trusted HTML produced by the Rust gateway
// (pulldown-cmark), NOT user-supplied browser input. Same pattern as page-skills.js
// and page-plugins.js which also render server-produced HTML.
function MarkdownPreview({ html: serverHtml }) {
	var divRef = useRef(null);
	useEffect(() => {
		if (divRef.current) divRef.current.innerHTML = serverHtml || ""; // eslint-disable-line no-unsanitized/property
	}, [serverHtml]);
	return html`<div
		ref=${divRef}
		class="skill-body-md text-sm bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius-sm)] p-3 overflow-y-auto"
		style="min-height:120px;max-height:400px"
	/>`;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Card component with expand/collapse, stats, and editor
function HookCard({ hook }) {
	var expanded = useSignal(false);
	var editContent = useSignal(hook.body);
	var saving = useSignal(false);
	var dirty = useSignal(false);
	var tab = useSignal("preview");
	var textareaRef = useRef(null);

	// Reset content when hook body changes (e.g. after reload).
	useEffect(() => {
		editContent.value = hook.body;
		dirty.value = false;
	}, [hook.body]);

	function handleToggle() {
		expanded.value = !expanded.value;
	}

	async function handleEnableDisable() {
		var method = hook.enabled ? "hooks.disable" : "hooks.enable";
		var res = await sendRpc(method, { name: hook.name });
		if (res?.ok) {
			showToast(`Hook "${hook.name}" ${hook.enabled ? "disabled" : "enabled"}`, "success");
		} else {
			showToast(`Failed: ${res?.error?.message || "unknown error"}`, "error");
		}
	}

	async function handleSave() {
		saving.value = true;
		var res = await sendRpc("hooks.save", {
			name: hook.name,
			content: editContent.value,
		});
		saving.value = false;
		if (res?.ok) {
			dirty.value = false;
			showToast(`Saved "${hook.name}"`, "success");
		} else {
			showToast(`Failed to save: ${res?.error?.message || "unknown error"}`, "error");
		}
	}

	function handleInput(e) {
		editContent.value = e.target.value;
		dirty.value = e.target.value !== hook.body;
	}

	var missingInfo = [];
	if (hook.missing_os) missingInfo.push("OS not supported");
	if (hook.missing_bins.length > 0) missingInfo.push(`Missing: ${hook.missing_bins.join(", ")}`);
	if (hook.missing_env.length > 0) missingInfo.push(`Env: ${hook.missing_env.join(", ")}`);

	return html`
    <div class="bg-[var(--surface)] border border-[var(--border)] rounded-[var(--radius)] overflow-hidden">
      <div class="flex items-center gap-3 px-4 py-3 cursor-pointer hover:bg-[var(--bg-hover)]" onClick=${handleToggle}>
        ${hook.emoji ? html`<span class="text-base">${hook.emoji}</span>` : null}
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium text-[var(--text-strong)]">${hook.name}</span>
            <${StatusBadge} hook=${hook} />
            <${SourceBadge} source=${hook.source} />
          </div>
          ${hook.description ? html`<div class="text-xs text-[var(--muted)] mt-0.5 truncate">${hook.description}</div>` : null}
        </div>
        <div class="flex items-center gap-2 text-xs text-[var(--muted)] shrink-0">
          ${
						hook.enabled && hook.call_count > 0
							? html`
            <span title="Calls">${hook.call_count} calls</span>
            ${hook.failure_count > 0 ? html`<span class="text-[var(--danger)]">${hook.failure_count} failed</span>` : null}
            ${hook.avg_latency_ms > 0 ? html`<span>${hook.avg_latency_ms}ms avg</span>` : null}
          `
							: null
					}
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"
               class="transition-transform ${expanded.value ? "rotate-180" : ""}">
            <path stroke-linecap="round" stroke-linejoin="round" d="m19.5 8.25-7.5 7.5-7.5-7.5" />
          </svg>
        </div>
      </div>

      ${
				expanded.value
					? html`
        <div class="border-t border-[var(--border)] px-4 py-3 flex flex-col gap-3">
          <div class="flex flex-wrap gap-2 text-xs">
            <span class="text-[var(--muted)]">Events:</span>
            ${hook.events.map((ev) => html`<span key=${ev} class="tier-badge">${ev}</span>`)}
          </div>
          ${
						hook.command
							? html`
            <div class="flex items-center gap-2 text-xs">
              <span class="text-[var(--muted)]">Command:</span>
              <code class="font-mono text-[var(--text)]">${hook.command}</code>
            </div>
          `
							: null
					}
          <div class="flex items-center gap-2 text-xs text-[var(--muted)]">
            <span>Priority: ${hook.priority}</span>
            <span>Timeout: ${hook.timeout}s</span>
            <span class="truncate cursor-pointer hover:text-[var(--text)] transition-colors" title="Click to copy path" onClick=${(
							e,
						) => {
							e.stopPropagation();
							navigator.clipboard.writeText(hook.source_path).then(() => showToast("Path copied", "success"));
						}}>${hook.source_path}</span>
          </div>
          ${
						missingInfo.length > 0
							? html`
            <div class="text-xs text-[var(--warn)] bg-[rgba(234,179,8,0.08)] border border-[var(--warn)] rounded-[var(--radius-sm)] px-3 py-2">
              ${missingInfo.join(" \u2022 ")}
            </div>
          `
							: null
					}

          <div class="flex flex-col gap-1">
            <div class="flex items-center gap-0 border-b border-[var(--border)]">
              <button
                class="px-3 py-1.5 text-xs font-medium border-b-2 transition-colors ${tab.value === "preview" ? "border-[var(--accent)] text-[var(--text-strong)]" : "border-transparent text-[var(--muted)] hover:text-[var(--text)]"}"
                onClick=${() => {
									tab.value = "preview";
								}}>Preview</button>
              <button
                class="px-3 py-1.5 text-xs font-medium border-b-2 transition-colors ${tab.value === "source" ? "border-[var(--accent)] text-[var(--text-strong)]" : "border-transparent text-[var(--muted)] hover:text-[var(--text)]"}"
                onClick=${() => {
									tab.value = "source";
								}}>Source</button>
            </div>
            ${
							tab.value === "source"
								? html`
              <textarea
                ref=${textareaRef}
                class="w-full font-mono text-xs bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius-sm)] p-3 text-[var(--text)] focus:outline-none focus:border-[var(--border-strong)] resize-y"
                rows="16"
                spellcheck="false"
                value=${editContent.value}
                onInput=${handleInput}
              />
            `
								: html`
              <${MarkdownPreview} html=${hook.body_html} />
            `
						}
          </div>

          <div class="flex items-center gap-2">
            <button class=${`provider-btn provider-btn-sm ${hook.enabled ? "provider-btn-secondary" : ""}`}
                    onClick=${handleEnableDisable}>
              ${hook.enabled ? "Disable" : "Enable"}
            </button>
            ${
							dirty.value
								? html`
              <button class="provider-btn provider-btn-sm" onClick=${handleSave} disabled=${saving.value}>
                ${saving.value ? "Saving\u2026" : "Save"}
              </button>
            `
								: null
						}
          </div>
        </div>
      `
					: null
			}
    </div>
  `;
}

function HooksPage() {
	useEffect(() => {
		refreshHooks();
		var off = onEvent("hooks.status", (payload) => {
			if (payload?.hooks) {
				hooks.value = payload.hooks;
				updateNavCount("hooks", payload.hooks.length);
			} else {
				refreshHooks();
			}
		});
		return off;
	}, []);

	async function handleReload() {
		loading.value = true;
		var res = await sendRpc("hooks.reload", {});
		if (res?.ok) {
			showToast("Hooks reloaded", "success");
		} else {
			showToast(`Reload failed: ${res?.error?.message || "unknown error"}`, "error");
		}
		loading.value = false;
	}

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Hooks</h2>
        <button class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${handleReload} disabled=${loading.value}>
          ${loading.value ? "Reloading\u2026" : "Reload"}
        </button>
      </div>

      <div class="max-w-[600px] bg-[var(--surface2)] border border-[var(--border)] rounded-[var(--radius)] px-5 py-4 leading-relaxed">
        <p class="text-sm text-[var(--text)] mb-2.5">
          <strong class="text-[var(--text-strong)]">Hooks</strong> run shell commands in response to lifecycle events (tool calls, messages, sessions, etc.). They live in <code class="font-mono text-xs">.moltis/hooks/</code> directories.
        </p>
        <div class="flex items-center gap-2 my-3 px-3.5 py-2.5 bg-[var(--surface)] rounded-[var(--radius-sm)] font-mono text-xs text-[var(--text-strong)]">
          <span class="opacity-50">Event</span>
          <span class="text-[var(--accent)]">\u2192</span>
          <span>Hook Script</span>
          <span class="text-[var(--accent)]">\u2192</span>
          <span>Continue / Modify / Block</span>
        </div>
        <p class="text-xs text-[var(--muted)]">
          Each hook is a directory containing a <code class="font-mono">HOOK.md</code> file with TOML frontmatter (events, command, requirements) and optional documentation. Edit the content below and click <strong>Save</strong> to update.
        </p>
      </div>

      ${
				hooks.value.length === 0 && !loading.value
					? html`
        <div class="max-w-[600px] text-sm text-[var(--muted)] px-1">
          No hooks discovered. Create a <code class="font-mono text-xs">HOOK.md</code> file in <code class="font-mono text-xs">.moltis/hooks/my-hook/</code> or <code class="font-mono text-xs">~/.moltis/hooks/my-hook/</code> to get started.
        </div>
      `
					: null
			}

      <div class="max-w-[700px] flex flex-col gap-3">
        ${hooks.value.map((h) => html`<${HookCard} key=${h.name} hook=${h} />`)}
      </div>

      ${loading.value && hooks.value.length === 0 ? html`<div class="p-6 text-center text-[var(--muted)] text-sm">Loading hooks\u2026</div>` : null}
    </div>
    <${Toasts} />
  `;
}

// ── Router integration ──────────────────────────────────────
registerPage(
	"/hooks",
	function initHooks(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		render(html`<${HooksPage} />`, container);
	},
	function teardownHooks() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
