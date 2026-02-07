// ── Crons page (Preact + HTM + Signals) ──────────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import * as gon from "./gon.js";
import { refresh as refreshGon } from "./gon.js";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { navigate, registerPrefix } from "./router.js";
import { models as modelsSig } from "./signals.js";
import * as S from "./state.js";
import { ComboSelect, ConfirmDialog, Modal, ModelSelect, requestConfirm } from "./ui.js";

var initialCrons = gon.get("crons") || [];
var cronJobs = signal(initialCrons);
var cronStatus = signal(gon.get("cron_status"));
if (initialCrons.length) {
	updateNavCount("crons", initialCrons.filter((j) => j.enabled).length);
}
var runsHistory = signal(null); // { jobId, jobName, runs }
var showModal = signal(false);
var editingJob = signal(null);
var activeSection = signal("jobs");

// ── Heartbeat state ──────────────────────────────────────────
var heartbeatStatus = signal(null);
var heartbeatRuns = signal(gon.get("heartbeat_runs") || []);
var heartbeatSaving = signal(false);
var heartbeatRunning = signal(false);
var heartbeatConfig = signal(gon.get("heartbeat_config") || {});
var sandboxImages = signal([]);
var heartbeatModel = signal(gon.get("heartbeat_config")?.model || "");
var heartbeatSandboxImage = signal(gon.get("heartbeat_config")?.sandbox_image || "");

function loadSandboxImages() {
	fetch("/api/images/cached")
		.then((r) => r.json())
		.then((data) => {
			sandboxImages.value = data?.images || [];
		})
		.catch(() => {
			// Ignore fetch errors — images list is optional.
		});
}

function loadHeartbeatStatus() {
	sendRpc("heartbeat.status", {}).then((res) => {
		if (res?.ok) heartbeatStatus.value = res.payload;
	});
}

function loadHeartbeatRuns() {
	heartbeatRuns.value = null;
	sendRpc("heartbeat.runs", { limit: 10 }).then((res) => {
		heartbeatRuns.value = res?.ok ? res.payload || [] : [];
	});
}

function loadStatus() {
	sendRpc("cron.status", {}).then((res) => {
		if (res?.ok) cronStatus.value = res.payload;
	});
}

function loadJobs() {
	sendRpc("cron.list", {}).then((res) => {
		if (res?.ok) {
			cronJobs.value = res.payload || [];
			updateNavCount("crons", cronJobs.value.filter((j) => j.enabled).length);
		}
	});
}

function formatSchedule(sched) {
	if (sched.kind === "at") return `At ${new Date(sched.atMs).toLocaleString()}`;
	if (sched.kind === "every") {
		var ms = sched.everyMs;
		if (ms >= 3600000) return `Every ${ms / 3600000}h`;
		if (ms >= 60000) return `Every ${ms / 60000}m`;
		return `Every ${ms / 1000}s`;
	}
	if (sched.kind === "cron") return sched.expr + (sched.tz ? ` (${sched.tz})` : "");
	return JSON.stringify(sched);
}

// ── Sidebar navigation ──────────────────────────────────────

var sections = [
	{
		id: "jobs",
		label: "Cron Jobs",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z"/></svg>`,
	},
	{
		id: "heartbeat",
		label: "Heartbeat",
		icon: html`<svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" width="16" height="16"><path stroke-linecap="round" stroke-linejoin="round" d="M21 8.25c0-2.485-2.099-4.5-4.688-4.5-1.935 0-3.597 1.126-4.312 2.733-.715-1.607-2.377-2.733-4.313-2.733C5.1 3.75 3 5.765 3 8.25c0 7.22 9 12 9 12s9-4.78 9-12Z"/></svg>`,
	},
];

var sectionIds = sections.map((s) => s.id);

function CronsSidebar() {
	return html`<div class="settings-sidebar">
		<div class="settings-sidebar-nav">
			${sections.map(
				(s) => html`
				<button
					key=${s.id}
					class="settings-nav-item ${activeSection.value === s.id ? "active" : ""}"
					onClick=${() => {
						navigate(`/crons/${s.id}`);
					}}
				>
					${s.icon}
					${s.label}
				</button>
			`,
			)}
		</div>
	</div>`;
}

// ── Heartbeat Card ───────────────────────────────────────────

function formatTokens(n) {
	if (n == null) return null;
	if (n >= 1000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}K`;
	return String(n);
}

function TokenBadge({ run }) {
	if (run.inputTokens == null && run.outputTokens == null) return null;
	var parts = [];
	if (run.inputTokens != null) parts.push(`${formatTokens(run.inputTokens)} in`);
	if (run.outputTokens != null) parts.push(`${formatTokens(run.outputTokens)} out`);
	return html`<span class="text-xs text-[var(--muted)] font-mono">${parts.join(" / ")}</span>`;
}

function HeartbeatRunsList({ runs }) {
	if (runs === null) return html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`;
	if (runs.length === 0) return html`<div class="text-xs text-[var(--muted)]">No runs yet.</div>`;
	return html`<div class="flex flex-col">
    ${runs.map(
			(
				run,
			) => html`<div key=${run.startedAtMs} class="flex items-center gap-3 py-2 border-b border-[var(--border)]" style="min-height:36px;">
        <span class="status-dot ${run.status === "ok" ? "connected" : ""}"></span>
        <span class="cron-badge ${run.status}">${run.status}</span>
        <span class="text-xs text-[var(--muted)] font-mono">${run.durationMs}ms</span>
        <${TokenBadge} run=${run} />
        ${run.error && html`<span class="text-xs text-[var(--error)] truncate">${run.error}</span>`}
        <span class="flex-1"></span>
        <span class="text-xs text-[var(--muted)]"><time data-epoch-ms="${run.startedAtMs}">${new Date(run.startedAtMs).toISOString()}</time></span>
      </div>`,
		)}
  </div>`;
}

function HeartbeatJobStatus({ job }) {
	if (!job) return null;
	var statusDotClass = job.enabled ? "connected" : "";
	return html`<div class="info-bar" style="margin-top:16px;margin-bottom:16px;">
    <span class="info-field">
      <span class="status-dot ${statusDotClass}"></span>
      <span class="info-label">${job.enabled ? "Enabled" : "Disabled"}</span>
    </span>
    ${
			job.state?.lastStatus &&
			html`<span class="info-field">
      <span class="info-label">Last:</span>
      <span class="cron-badge ${job.state.lastStatus}">${job.state.lastStatus}</span>
    </span>`
		}
    ${
			job.state?.nextRunAtMs &&
			html`<span class="info-field">
      <span class="info-label">Next:</span>
      <span class="info-value"><time data-epoch-ms="${job.state.nextRunAtMs}">${new Date(job.state.nextRunAtMs).toLocaleString()}</time></span>
    </span>`
		}
  </div>`;
}

function heartbeatModelPlaceholder() {
	return modelsSig.value.length > 0
		? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
		: "(server default)";
}

function collectHeartbeatForm(form) {
	return {
		enabled: form.querySelector("[data-hb=enabled]").checked,
		every: form.querySelector("[data-hb=every]").value.trim() || "30m",
		model: heartbeatModel.value || null,
		prompt: form.querySelector("[data-hb=prompt]").value.trim() || null,
		ack_max_chars: parseInt(form.querySelector("[data-hb=ackMax]").value, 10) || 300,
		active_hours: {
			start: form.querySelector("[data-hb=ahStart]").value.trim() || "08:00",
			end: form.querySelector("[data-hb=ahEnd]").value.trim() || "24:00",
			timezone: form.querySelector("[data-hb=ahTz]").value.trim() || "local",
		},
		sandbox_enabled: form.querySelector("[data-hb=sandboxEnabled]").checked,
		sandbox_image: heartbeatSandboxImage.value || null,
	};
}

var systemTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone;

function HeartbeatSection() {
	var cfg = heartbeatConfig.value;
	var saving = heartbeatSaving.value;
	var promptSource = heartbeatStatus.value?.promptSource || "default";
	// Get heartbeat job from cronJobs (loaded from gon) for immediate availability on page load.
	// Falls back to heartbeatStatus for updates after RPC calls.
	var job = cronJobs.value.find((j) => j.name === "__heartbeat__") || heartbeatStatus.value?.job;

	function onSave(e) {
		e.preventDefault();
		var updated = collectHeartbeatForm(e.target.closest(".heartbeat-form"));
		heartbeatSaving.value = true;
		sendRpc("heartbeat.update", updated).then((res) => {
			heartbeatSaving.value = false;
			if (res?.ok) {
				heartbeatConfig.value = updated;
				heartbeatModel.value = updated.model || "";
				heartbeatSandboxImage.value = updated.sandbox_image || "";
				refreshGon();
				loadHeartbeatStatus();
				loadJobs();
				loadStatus();
			}
		});
	}

	function onRunNow() {
		heartbeatRunning.value = true;
		sendRpc("heartbeat.run", {}).then(() => {
			heartbeatRunning.value = false;
			loadHeartbeatStatus();
			loadHeartbeatRuns();
			loadJobs();
			loadStatus();
		});
	}

	function onToggleEnabled(e) {
		var newEnabled = e.target.checked;
		var updated = { ...cfg, enabled: newEnabled };
		sendRpc("heartbeat.update", updated).then((res) => {
			if (res?.ok) {
				heartbeatConfig.value = updated;
				refreshGon();
				loadHeartbeatStatus();
				loadJobs();
				loadStatus();
			}
		});
	}

	var running = heartbeatRunning.value;
	var promptSourceText =
		promptSource === "config"
			? "config custom prompt"
			: promptSource === "heartbeat_md"
				? "HEARTBEAT.md"
				: "built-in default";

	return html`<div class="heartbeat-form" style="max-width:600px;">
    <!-- Header -->
    <div class="flex items-center justify-between mb-2">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Heartbeat</h2>
        <label class="cron-toggle">
          <input data-hb="enabled" type="checkbox" checked=${cfg.enabled !== false} onChange=${onToggleEnabled} />
          <span class="cron-slider"></span>
        </label>
        <span class="text-xs text-[var(--muted)]">Enable</span>
      </div>
      <button class="provider-btn provider-btn-secondary" onClick=${onRunNow} disabled=${running}>
        ${running ? "Running\u2026" : "Run Now"}
      </button>
    </div>
    <p class="text-sm text-[var(--muted)] mb-4">Periodic AI check-in that monitors your environment and reports status.</p>

    <${HeartbeatJobStatus} job=${job} />

    <!-- Schedule -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Schedule</h3>
      <div class="grid gap-4" style="grid-template-columns:1fr 1fr;">
        <div>
          <label class="block text-xs text-[var(--muted)] mb-1">Interval</label>
          <input data-hb="every" class="provider-key-input" placeholder="30m" value=${cfg.every || "30m"} />
        </div>
        <div>
          <label class="block text-xs text-[var(--muted)] mb-1">Model</label>
          <${ModelSelect} models=${modelsSig.value} value=${heartbeatModel.value}
            onChange=${(v) => {
							heartbeatModel.value = v;
						}}
            placeholder=${heartbeatModelPlaceholder()} />
        </div>
      </div>
    </div>

    <!-- Prompt -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Prompt</h3>
      <label class="block text-xs text-[var(--muted)] mb-1">Custom Prompt (optional)</label>
      <textarea data-hb="prompt" class="provider-key-input textarea-sm" placeholder="Leave blank to use default heartbeat prompt">${cfg.prompt || ""}</textarea>
      <p class="text-xs text-[var(--muted)] mt-2">Leave this empty to use <code>HEARTBEAT.md</code> in your workspace root (falls back to built-in default if missing).</p>
      <p class="text-xs text-[var(--muted)] mt-1">Effective prompt source: <span class="text-[var(--text)]">${promptSourceText}</span></p>
      <div class="grid gap-4 mt-3" style="grid-template-columns:1fr;">
        <div>
          <label class="block text-xs text-[var(--muted)] mb-1">Max Response Characters</label>
          <input data-hb="ackMax" class="provider-key-input" type="number" min="50" value=${cfg.ack_max_chars || 300} />
        </div>
      </div>
    </div>

    <!-- Active Hours -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Active Hours</h3>
      <p class="text-xs text-[var(--muted)] mb-3">Only run heartbeat during these hours.</p>
      <div class="grid gap-4" style="grid-template-columns:1fr 1fr;">
        <div>
          <label class="block text-xs text-[var(--muted)] mb-1">Start</label>
          <input data-hb="ahStart" type="time" class="provider-key-input" value=${cfg.active_hours?.start || "08:00"} />
        </div>
        <div>
          <label class="block text-xs text-[var(--muted)] mb-1">End</label>
          <input data-hb="ahEnd" type="time" class="provider-key-input" value=${cfg.active_hours?.end === "24:00" ? "23:59" : cfg.active_hours?.end || "23:59"} />
        </div>
      </div>
      <div class="mt-3">
        <label class="block text-xs text-[var(--muted)] mb-1">Timezone</label>
        <select data-hb="ahTz" class="provider-key-input">
          <option value="local" selected=${!cfg.active_hours?.timezone || cfg.active_hours?.timezone === "local"}>Local (${systemTimezone})</option>
          <option value="UTC" selected=${cfg.active_hours?.timezone === "UTC"}>UTC</option>
          <option value="America/New_York" selected=${cfg.active_hours?.timezone === "America/New_York"}>America/New_York (EST/EDT)</option>
          <option value="America/Chicago" selected=${cfg.active_hours?.timezone === "America/Chicago"}>America/Chicago (CST/CDT)</option>
          <option value="America/Denver" selected=${cfg.active_hours?.timezone === "America/Denver"}>America/Denver (MST/MDT)</option>
          <option value="America/Los_Angeles" selected=${cfg.active_hours?.timezone === "America/Los_Angeles"}>America/Los_Angeles (PST/PDT)</option>
          <option value="Europe/London" selected=${cfg.active_hours?.timezone === "Europe/London"}>Europe/London (GMT/BST)</option>
          <option value="Europe/Paris" selected=${cfg.active_hours?.timezone === "Europe/Paris"}>Europe/Paris (CET/CEST)</option>
          <option value="Europe/Berlin" selected=${cfg.active_hours?.timezone === "Europe/Berlin"}>Europe/Berlin (CET/CEST)</option>
          <option value="Asia/Tokyo" selected=${cfg.active_hours?.timezone === "Asia/Tokyo"}>Asia/Tokyo (JST)</option>
          <option value="Asia/Shanghai" selected=${cfg.active_hours?.timezone === "Asia/Shanghai"}>Asia/Shanghai (CST)</option>
          <option value="Asia/Singapore" selected=${cfg.active_hours?.timezone === "Asia/Singapore"}>Asia/Singapore (SGT)</option>
          <option value="Australia/Sydney" selected=${cfg.active_hours?.timezone === "Australia/Sydney"}>Australia/Sydney (AEST/AEDT)</option>
        </select>
      </div>
    </div>

    <!-- Sandbox -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Sandbox</h3>
      <p class="text-xs text-[var(--muted)] mb-3">Run heartbeat commands in an isolated container.</p>
      <div class="flex items-center gap-3 mb-3">
        <label class="cron-toggle">
          <input data-hb="sandboxEnabled" type="checkbox" checked=${cfg.sandbox_enabled !== false} />
          <span class="cron-slider"></span>
        </label>
        <span class="text-sm text-[var(--text)]">Enable sandbox</span>
      </div>
      <div>
        <label class="block text-xs text-[var(--muted)] mb-1">Sandbox Image</label>
        <${ComboSelect}
          options=${sandboxImages.value.map((img) => ({ value: img.tag, label: img.tag }))}
          value=${heartbeatSandboxImage.value}
          onChange=${(v) => {
						heartbeatSandboxImage.value = v;
					}}
          placeholder="Default image"
          searchPlaceholder="Search images\u2026"
        />
      </div>
    </div>

    <!-- Recent Runs -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <h3 class="text-sm font-medium text-[var(--text-strong)] mb-3">Recent Runs</h3>
      <${HeartbeatRunsList} runs=${heartbeatRuns.value} />
    </div>

    <!-- Save -->
    <div style="margin-top:24px;border-top:1px solid var(--border);padding-top:16px;">
      <button class="provider-btn" onClick=${onSave} disabled=${saving}>
        ${saving ? "Saving\u2026" : "Save"}
      </button>
    </div>
  </div>`;
}

// ── Cron Jobs (existing) ─────────────────────────────────────

function StatusBar() {
	var s = cronStatus.value;
	if (!s) return html`<div class="cron-status-bar">Loading\u2026</div>`;
	var parts = [
		s.running ? "Running" : "Stopped",
		`${s.jobCount} job${s.jobCount !== 1 ? "s" : ""}`,
		`${s.enabledCount} enabled`,
	];
	if (s.nextRunAtMs) {
		parts.push(`next: ${new Date(s.nextRunAtMs).toLocaleString()}`);
	}
	return html`<div class="cron-status-bar">${parts.join(" \u2022 ")}</div>`;
}

function CronJobRow(props) {
	var job = props.job;

	function onToggle(e) {
		sendRpc("cron.update", {
			id: job.id,
			patch: { enabled: e.target.checked },
		}).then(() => {
			loadJobs();
			loadStatus();
		});
	}

	function onRun() {
		sendRpc("cron.run", { id: job.id, force: true }).then(() => {
			loadJobs();
			loadStatus();
		});
	}

	function onDelete() {
		requestConfirm(`Delete job '${job.name}'?`).then((yes) => {
			if (!yes) return;
			sendRpc("cron.remove", { id: job.id }).then(() => {
				loadJobs();
				loadStatus();
			});
		});
	}

	function onHistory() {
		runsHistory.value = { jobId: job.id, jobName: job.name, runs: null };
		sendRpc("cron.runs", { id: job.id }).then((res) => {
			if (res?.ok)
				runsHistory.value = {
					jobId: job.id,
					jobName: job.name,
					runs: res.payload || [],
				};
		});
	}

	return html`<tr>
    <td>${job.name}</td>
    <td class="cron-mono">${formatSchedule(job.schedule)}</td>
    <td class="cron-mono">${job.state?.nextRunAtMs ? html`<time data-epoch-ms="${job.state.nextRunAtMs}">${new Date(job.state.nextRunAtMs).toISOString()}</time>` : "\u2014"}</td>
    <td>${job.state?.lastStatus ? html`<span class="cron-badge ${job.state.lastStatus}">${job.state.lastStatus}</span>` : "\u2014"}</td>
    <td class="cron-actions">
      <button class="cron-action-btn" onClick=${() => {
				editingJob.value = job;
				showModal.value = true;
			}}>Edit</button>
      <button class="cron-action-btn" onClick=${onRun}>Run</button>
      <button class="cron-action-btn" onClick=${onHistory}>History</button>
      <button class="cron-action-btn cron-action-danger" onClick=${onDelete}>Delete</button>
    </td>
    <td>
      <label class="cron-toggle">
        <input type="checkbox" checked=${job.enabled} onChange=${onToggle} />
        <span class="cron-slider" />
      </label>
    </td>
  </tr>`;
}

function CronJobTable() {
	// Filter out system jobs (e.g. heartbeat).
	var jobs = cronJobs.value.filter((j) => !j.system);
	if (jobs.length === 0) {
		return html`<div class="text-sm text-[var(--muted)]">No cron jobs configured.</div>`;
	}
	return html`<table class="cron-table">
    <thead>
      <tr>
        <th>Name</th><th>Schedule</th>
        <th>Next Run</th><th>Last Status</th><th>Actions</th><th>Enabled</th>
      </tr>
    </thead>
    <tbody>
      ${jobs.map((job) => html`<${CronJobRow} key=${job.id} job=${job} />`)}
    </tbody>
  </table>`;
}

function RunHistoryPanel() {
	var h = runsHistory.value;
	if (!h) return null;

	return html`<div class="mb-md">
    <div class="flex items-center justify-between mb-md">
      <span class="text-sm font-medium text-[var(--text-strong)]">Run History: ${h.jobName}</span>
      <button class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none hover:text-[var(--text)]"
        onClick=${() => {
					runsHistory.value = null;
				}}>\u2715 Close</button>
    </div>
    ${h.runs === null && html`<div class="text-sm text-[var(--muted)]">Loading\u2026</div>`}
    ${h.runs !== null && h.runs.length === 0 && html`<div class="text-xs text-[var(--muted)]">No runs yet.</div>`}
    ${h.runs?.map(
			(run) => html`<div class="cron-run-item" key=${run.startedAtMs}>
        <span class="text-xs text-[var(--muted)]"><time data-epoch-ms="${run.startedAtMs}">${new Date(run.startedAtMs).toISOString()}</time></span>
        <span class="cron-badge ${run.status}">${run.status}</span>
        <span class="text-xs text-[var(--muted)]">${run.durationMs}ms</span>
        <${TokenBadge} run=${run} />
        ${run.error && html`<span class="text-xs text-[var(--error)]">${run.error}</span>`}
      </div>`,
		)}
  </div>`;
}

function parseScheduleFromForm(form, kind) {
	if (kind === "at") {
		var ts = new Date(form.querySelector("[data-field=at]").value).getTime();
		if (Number.isNaN(ts)) return { error: "at" };
		return { schedule: { kind: "at", atMs: ts } };
	}
	if (kind === "every") {
		var secs = parseInt(form.querySelector("[data-field=every]").value, 10);
		if (Number.isNaN(secs) || secs <= 0) return { error: "every" };
		return { schedule: { kind: "every", everyMs: secs * 1000 } };
	}
	var expr = form.querySelector("[data-field=cron]").value.trim();
	if (!expr) return { error: "cron" };
	var schedule = { kind: "cron", expr: expr };
	var tz = form.querySelector("[data-field=tz]").value.trim();
	if (tz) schedule.tz = tz;
	return { schedule: schedule };
}

function schedDefault(kind, job) {
	if (!job) return "";
	if (kind === "at" && job.schedule.kind === "at" && job.schedule.atMs) {
		return new Date(job.schedule.atMs).toISOString().slice(0, 16);
	}
	if (kind === "every" && job.schedule.kind === "every" && job.schedule.everyMs) {
		return Math.round(job.schedule.everyMs / 1000);
	}
	return "";
}

function CronModal() {
	var isEdit = !!editingJob.value;
	var job = editingJob.value;
	var saving = signal(false);
	var schedKind = signal(isEdit ? job.schedule.kind : "cron");
	var errorField = signal(null);

	function onSave(e) {
		e.preventDefault();
		var form = e.target.closest(".provider-key-form");
		var name = form.querySelector("[data-field=name]").value.trim();
		if (!name) {
			errorField.value = "name";
			return;
		}
		var parsed = parseScheduleFromForm(form, schedKind.value);
		if (parsed.error) {
			errorField.value = parsed.error;
			return;
		}
		var msgText = form.querySelector("[data-field=message]").value.trim();
		if (!msgText) {
			errorField.value = "message";
			return;
		}
		var payloadKind = form.querySelector("[data-field=payloadKind]").value;
		var payload =
			payloadKind === "systemEvent"
				? { kind: "systemEvent", text: msgText }
				: { kind: "agentTurn", message: msgText, deliver: false };
		var fields = {
			name: name,
			schedule: parsed.schedule,
			payload: payload,
			sessionTarget: form.querySelector("[data-field=target]").value,
			deleteAfterRun: form.querySelector("[data-field=deleteAfter]").checked,
			enabled: form.querySelector("[data-field=enabled]").checked,
		};

		saving.value = true;
		var rpcMethod = isEdit ? "cron.update" : "cron.add";
		var rpcParams = isEdit ? { id: job.id, patch: fields } : fields;
		sendRpc(rpcMethod, rpcParams).then((res) => {
			saving.value = false;
			if (res?.ok) {
				showModal.value = false;
				editingJob.value = null;
				loadJobs();
				loadStatus();
			}
		});
	}

	function schedParams() {
		if (schedKind.value === "at") {
			return html`<input data-field="at" class="provider-key-input" type="datetime-local"
        value=${schedDefault("at", job)} />`;
		}
		if (schedKind.value === "every") {
			return html`<input data-field="every" class="provider-key-input" type="number" placeholder="Interval in seconds" min="1"
        value=${schedDefault("every", job)} />`;
		}
		return html`
      <input data-field="cron" class="provider-key-input" placeholder="*/5 * * * *"
        value=${isEdit && job.schedule.kind === "cron" ? job.schedule.expr || "" : ""} />
      <input data-field="tz" class="provider-key-input" placeholder="Timezone (optional, e.g. Europe/Paris)"
        value=${isEdit && job.schedule.kind === "cron" ? job.schedule.tz || "" : ""} />
    `;
	}

	return html`<${Modal} show=${showModal.value} onClose=${() => {
		showModal.value = false;
		editingJob.value = null;
	}} title=${isEdit ? "Edit Job" : "Add Job"}>
    <div class="provider-key-form">
      <label class="text-xs text-[var(--muted)]">Name</label>
      <input data-field="name" class="provider-key-input ${errorField.value === "name" ? "field-error" : ""}"
        placeholder="Job name" value=${isEdit ? job.name : ""} />

      <label class="text-xs text-[var(--muted)]">Schedule Type</label>
      <select data-field="schedKind" class="provider-key-input" value=${schedKind.value}
        onChange=${(e) => {
					schedKind.value = e.target.value;
				}}>
        <option value="at">At (one-shot)</option>
        <option value="every">Every (interval)</option>
        <option value="cron">Cron (expression)</option>
      </select>

      ${schedParams()}

      <label class="text-xs text-[var(--muted)]">Payload Type</label>
      <select data-field="payloadKind" class="provider-key-input"
        value=${isEdit ? job.payload.kind : "systemEvent"}>
        <option value="systemEvent">System Event</option>
        <option value="agentTurn">Agent Turn</option>
      </select>

      <label class="text-xs text-[var(--muted)]">Message</label>
      <textarea data-field="message" class="provider-key-input textarea-sm ${errorField.value === "message" ? "field-error" : ""}"
        placeholder="Message text">${isEdit ? job.payload.text || job.payload.message || "" : ""}</textarea>

      <label class="text-xs text-[var(--muted)]">Session Target</label>
      <select data-field="target" class="provider-key-input"
        value=${isEdit ? job.sessionTarget || "isolated" : "isolated"}>
        <option value="isolated">Isolated</option>
        <option value="main">Main</option>
      </select>

      <label class="text-xs text-[var(--muted)] flex items-center gap-2">
        <input data-field="deleteAfter" type="checkbox" checked=${isEdit ? job.deleteAfterRun : false} />
        Delete after run
      </label>
      <label class="text-xs text-[var(--muted)] flex items-center gap-2">
        <input data-field="enabled" type="checkbox" checked=${isEdit ? job.enabled : true} />
        Enabled
      </label>

      <div class="btn-row-mt">
        <button class="provider-btn provider-btn-secondary" onClick=${() => {
					showModal.value = false;
					editingJob.value = null;
				}}>Cancel</button>
        <button class="provider-btn" onClick=${onSave} disabled=${saving.value}>
          ${saving.value ? "Saving\u2026" : isEdit ? "Update" : "Create"}
        </button>
      </div>
    </div>
  </${Modal}>`;
}

// ── Section content panels ──────────────────────────────────

function HeartbeatPanel() {
	useEffect(() => {
		loadHeartbeatStatus();
		loadSandboxImages();
		loadHeartbeatRuns();
	}, []);

	return html`<div class="p-6">
    <${HeartbeatSection} />
  </div>`;
}

function CronJobsPanel() {
	useEffect(() => {
		loadStatus();
		loadJobs();
	}, []);

	return html`<div class="p-4 flex flex-col gap-4">
    <div class="flex items-center gap-3">
      <h2 class="text-lg font-medium text-[var(--text-strong)]">Cron Jobs</h2>
      <button class="provider-btn"
        onClick=${() => {
					editingJob.value = null;
					showModal.value = true;
				}}>+ Add Job</button>
    </div>
    <${StatusBar} />
    <${CronJobTable} />
    <${RunHistoryPanel} />
  </div>`;
}

// ── Main page ───────────────────────────────────────────────

function CronsPage() {
	return html`
    <div class="settings-layout">
      <${CronsSidebar} />
      <div class="flex-1 overflow-y-auto">
        ${activeSection.value === "jobs" && html`<${CronJobsPanel} />`}
        ${activeSection.value === "heartbeat" && html`<${HeartbeatPanel} />`}
      </div>
    </div>
    <${CronModal} />
    <${ConfirmDialog} />
  `;
}

registerPrefix(
	"/crons",
	function initCrons(container, param) {
		container.style.cssText = "flex-direction:row;padding:0;overflow:hidden;";
		cronJobs.value = gon.get("crons") || [];
		cronStatus.value = gon.get("cron_status");
		heartbeatConfig.value = gon.get("heartbeat_config") || {};
		runsHistory.value = null;
		showModal.value = false;
		editingJob.value = null;
		heartbeatStatus.value = null;
		heartbeatRuns.value = gon.get("heartbeat_runs") || [];
		sandboxImages.value = [];
		heartbeatModel.value = gon.get("heartbeat_config")?.model || "";
		heartbeatSandboxImage.value = gon.get("heartbeat_config")?.sandbox_image || "";

		var section = param && sectionIds.includes(param) ? param : "jobs";
		if (param && !sectionIds.includes(param)) {
			history.replaceState(null, "", "/crons/jobs");
		}
		activeSection.value = section;

		// Eagerly load heartbeat data so it's ready when the panel mounts.
		loadHeartbeatRuns();
		loadHeartbeatStatus();

		render(html`<${CronsPage} />`, container);
	},
	function teardownCrons() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
