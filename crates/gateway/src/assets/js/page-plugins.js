// ── Plugins page (Preact + HTM + Signals) ───
// Shows plugin-format repos (Claude Code, Codex, etc.) — distinct from native SKILL.md skills.
// eslint-disable-next-line -- body_html is server-rendered trusted content

import { computed, signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import * as S from "./state.js";

// ── Signals ─────────────────────────────────────────────────
var repos = signal([]);
var enabledPlugins = signal([]);
var loading = signal(false);
var toasts = signal([]);
var toastId = 0;

var prefetchPromise = null;
function ensurePrefetch() {
	if (!prefetchPromise) {
		prefetchPromise = fetch("/api/plugins")
			.then((r) => r.json())
			.then((data) => {
				if (data.skills) enabledPlugins.value = data.skills;
				if (data.repos) repos.value = data.repos;
				return data;
			})
			.catch(() => null);
	}
	return prefetchPromise;
}

function requestConfirm(message, opts) {
	opts = opts || {};
	return Promise.resolve(window.confirm(message));
}

// ── Helpers ─────────────────────────────────────────────────
function showToast(message, type) {
	var id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

function fetchAll() {
	loading.value = true;
	fetch("/api/plugins")
		.then((r) => r.json())
		.then((data) => {
			if (data.skills) enabledPlugins.value = data.skills;
			if (data.repos) repos.value = data.repos;
			loading.value = false;
			var pluginCount = (data.repos || []).filter((r) => r.format && r.format !== "skill").length;
			updateNavCount("plugins", pluginCount);
		})
		.catch(() => {
			loading.value = false;
		});
}

function doInstall(source) {
	if (!(source && S.connected)) {
		if (!S.connected) showToast("Not connected to gateway.", "error");
		return Promise.resolve();
	}
	return sendRpc("plugins.install", { source: source }).then((res) => {
		if (res?.ok) {
			var p = res.payload || {};
			var count = (p.installed || []).length;
			showToast(`Installed ${source} (${count} skill${count !== 1 ? "s" : ""})`, "success");
			fetchAll();
		} else {
			showToast(`Failed: ${res?.error || "unknown error"}`, "error");
		}
	});
}

function searchSkills(source, query) {
	return fetch(`/api/plugins/search?source=${encodeURIComponent(source)}&q=${encodeURIComponent(query)}`)
		.then((r) => r.json())
		.then((data) => data.skills || []);
}

// ── Components ──────────────────────────────────────────────

function Toasts() {
	return html`<div class="skills-toast-container">
    ${toasts.value.map((t) => {
			var bg = t.type === "error" ? "var(--error, #e55)" : "var(--accent)";
			return html`<div key=${t.id} style=${{
				pointerEvents: "auto",
				maxWidth: "420px",
				padding: "10px 16px",
				borderRadius: "6px",
				fontSize: ".8rem",
				fontWeight: 500,
				color: "#fff",
				background: bg,
				boxShadow: "0 4px 12px rgba(0,0,0,.15)",
			}}>${t.message}</div>`;
		})}
  </div>`;
}

function InstallBox() {
	var inputRef = useRef(null);
	var installing = useSignal(false);
	function onInstall() {
		var val = inputRef.current?.value.trim();
		if (!val) return;
		installing.value = true;
		doInstall(val).then(() => {
			installing.value = false;
			if (inputRef.current) inputRef.current.value = "";
		});
	}
	function onKey(e) {
		if (e.key === "Enter") onInstall();
	}
	return html`<div class="skills-install-box">
    <input ref=${inputRef} type="text" placeholder="owner/repo or full URL (e.g. anthropics/claude-plugins-official)" class="skills-install-input" onKeyDown=${onKey} />
    <button class="skills-install-btn" onClick=${onInstall} disabled=${installing.value}>
      ${installing.value ? "Installing\u2026" : "Install"}
    </button>
  </div>`;
}

var featuredPlugins = [
	{
		repo: "anthropics/claude-plugins-official",
		desc: "Official Anthropic plugin directory (code-review, pr-review-toolkit, commit-commands, LSP servers, and more)",
	},
];

function FeaturedCard(props) {
	var f = props.plugin;
	var installing = useSignal(false);
	var href = /^https?:\/\//.test(f.repo) ? f.repo : `https://github.com/${f.repo}`;
	function onInstall() {
		installing.value = true;
		doInstall(f.repo).then(() => {
			installing.value = false;
		});
	}
	return html`<div class="skills-featured-card">
    <div>
      <a href=${href} target="_blank" rel="noopener noreferrer"
         style="font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong);text-decoration:none">${f.repo}</a>
      <div style="font-size:.75rem;color:var(--muted)">${f.desc}</div>
    </div>
    <button onClick=${onInstall} disabled=${installing.value}
      style="background:var(--surface2);border:1px solid var(--border);color:var(--text);border-radius:var(--radius-sm);font-size:.72rem;padding:4px 10px;cursor:pointer;white-space:nowrap">
      ${installing.value ? "Installing\u2026" : "Install"}
    </button>
  </div>`;
}

function FeaturedSection() {
	if (featuredPlugins.length === 0) return null;
	return html`<div class="skills-section">
    <h3 class="skills-section-title">Featured Repositories</h3>
    <div class="skills-featured-grid">
      ${featuredPlugins.map((f) => html`<${FeaturedCard} key=${f.repo} plugin=${f} />`)}
    </div>
  </div>`;
}

// ── Plugin metadata row ──────────────────────────────────────
function PluginMetadata(props) {
	var d = props.detail;
	if (!(d.author || d.homepage || d.source_url || d.commit_sha || d.commit_age_days != null)) return null;
	return html`<div style="display:flex;align-items:center;gap:12px;margin-bottom:8px;font-size:.75rem;color:var(--muted);flex-wrap:wrap">
    ${d.author && html`<span>Author: ${d.author}</span>`}
    ${d.commit_sha && d.commit_url && html`<a href=${d.commit_url} target="_blank" rel="noopener noreferrer" style="color:var(--accent);text-decoration:none;font-size:.75rem">Commit: ${d.commit_sha.slice(0, 12)}</a>`}
    ${d.commit_sha && !d.commit_url && html`<span>Commit: ${d.commit_sha.slice(0, 12)}</span>`}
    ${d.commit_age_days != null && html`<span>Commit age: ${d.commit_age_days} day${d.commit_age_days === 1 ? "" : "s"}</span>`}
    ${d.homepage && html`<a href=${d.homepage} target="_blank" rel="noopener noreferrer" style="color:var(--accent);text-decoration:none;font-size:.75rem">${d.homepage.replace(/^https?:\/\//, "")}</a>`}
    ${d.source_url && html`<a href=${d.source_url} target="_blank" rel="noopener noreferrer" style="color:var(--accent);text-decoration:none;font-size:.75rem">View source</a>`}
  </div>`;
}

// ── Skill detail panel ───────────────────────────────────────
function SkillDetail(props) {
	var d = props.detail;
	var onClose = props.onClose;

	// Safe: body_html is server-rendered trusted HTML from the Rust gateway markdown renderer
	var bodyRef = useRef(null);
	var toggling = useSignal(false);
	useEffect(() => {
		if (bodyRef.current && d?.body_html) {
			bodyRef.current.textContent = "";
			var tpl = document.createElement("template");
			tpl.innerHTML = d.body_html; // eslint-disable-line no-unsanitized/property -- server-rendered
			bodyRef.current.appendChild(tpl.content);
			bodyRef.current.querySelectorAll("a").forEach((a) => {
				a.setAttribute("target", "_blank");
				a.setAttribute("rel", "noopener");
			});
		}
	}, [d?.body_html]);

	if (!d) return null;
	var needsTrust = d.trusted === false;

	function onToggle() {
		if (!S.connected) return;
		if (!d.enabled && needsTrust) {
			requestConfirm(`Trust plugin "${d.name}" from ${props.repoSource}?`, {
				confirmLabel: "Trust & Enable",
			}).then((yes) => {
				if (!yes) return;
				toggling.value = true;
				sendRpc("plugins.skill.trust", { source: props.repoSource, skill: d.name }).then((trustRes) => {
					if (!trustRes?.ok) {
						toggling.value = false;
						showToast(`Trust failed: ${trustRes?.error || "unknown"}`, "error");
						return;
					}
					sendRpc("plugins.skill.enable", { source: props.repoSource, skill: d.name }).then((r) => {
						toggling.value = false;
						if (r?.ok) fetchAll();
						else showToast(`Enable failed: ${r?.error || "unknown"}`, "error");
					});
				});
			});
			return;
		}

		toggling.value = true;
		var method = d.enabled ? "plugins.skill.disable" : "plugins.skill.enable";
		sendRpc(method, { source: props.repoSource, skill: d.name }).then((r) => {
			toggling.value = false;
			if (r?.ok) fetchAll();
			else showToast(`Failed: ${r?.error || "unknown"}`, "error");
		});
	}

	return html`<div class="skills-detail-panel" style="display:block;margin-top:8px">
    <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:8px">
      <div style="display:flex;align-items:center;gap:8px">
        <span style="font-family:var(--font-mono);font-size:.9rem;font-weight:600;color:var(--text-strong)">${d.display_name || d.name}</span>
        ${d.display_name && html`<span style="font-family:var(--font-mono);font-size:.72rem;color:var(--muted)">${d.name}</span>`}
        ${d.license && d.license_url && html`<a href=${d.license_url} target="_blank" rel="noopener noreferrer" style="font-size:.65rem;padding:1px 6px;border-radius:9999px;background:var(--surface2);color:var(--muted);text-decoration:none">${d.license}</a>`}
        ${d.license && !d.license_url && html`<span style="font-size:.65rem;padding:1px 6px;border-radius:9999px;background:var(--surface2);color:var(--muted)">${d.license}</span>`}
        ${d.trusted === false && html`<span style="font-size:.65rem;padding:1px 6px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff">untrusted</span>`}
      </div>
      <div style="display:flex;align-items:center;gap:6px">
        <button onClick=${onToggle} disabled=${toggling.value} style=${{
					background: d.enabled ? "none" : "var(--accent)",
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-sm)",
					fontSize: ".72rem",
					padding: "3px 10px",
					cursor: toggling.value ? "wait" : "pointer",
					color: d.enabled ? "var(--muted)" : "#fff",
					fontWeight: 500,
					opacity: toggling.value ? 0.6 : 1,
				}}>${toggling.value ? "Loading\u2026" : d.enabled ? "Disable" : "Enable"}</button>
        <button onClick=${onClose} style="background:none;border:none;color:var(--muted);font-size:.9rem;cursor:pointer;padding:2px 4px">\u2715</button>
      </div>
    </div>
    <${PluginMetadata} detail=${d} />
    ${d.commit_age_days != null && d.commit_age_days <= 14 && html`<div style="margin:0 0 10px;padding:10px 12px;border:1px solid var(--warning, #c77d00);background:color-mix(in srgb, var(--warning, #c77d00) 14%, transparent);border-radius:var(--radius-sm);font-size:.8rem;color:var(--text)"><strong style="color:var(--warning, #c77d00)">Recent commit warning:</strong> This plugin was updated ${d.commit_age_days} day${d.commit_age_days === 1 ? "" : "s"} ago. Review commit diff before trusting or enabling.</div>`}
    ${d.description && html`<p style="margin:0 0 8px;font-size:.82rem;color:var(--text)">${d.description}</p>`}
    ${d.allowed_tools && d.allowed_tools.length > 0 && html`<div style="margin-bottom:8px;font-size:.75rem;color:var(--muted)">Allowed tools: ${d.allowed_tools.join(", ")}</div>`}
    ${d.body_html && html`<div ref=${bodyRef} class="skill-body-md" style="border-top:1px solid var(--border);padding-top:8px;margin-top:8px;max-height:400px;overflow-y:auto;font-size:.8rem;color:var(--text);line-height:1.5" />`}
    ${!d.body_html && d.body && html`<div style="border-top:1px solid var(--border);padding-top:8px;margin-top:8px"><pre style="white-space:pre-wrap;word-break:break-word;font-size:.78rem;color:var(--text);font-family:var(--font-mono);margin:0;max-height:400px;overflow-y:auto">${d.body}</pre></div>`}
  </div>`;
}

// ── Skill row inside a repo card ─────────────────────────────
function SkillRow(props) {
	var skill = props.skill;
	var repoSource = props.repoSource;
	var toggling = useSignal(false);
	var needsTrust = skill.trusted === false;

	function onToggle() {
		if (!S.connected) return;
		if (!skill.enabled && needsTrust) {
			requestConfirm(`Trust plugin "${skill.name}" from ${repoSource}?`, {
				confirmLabel: "Trust & Enable",
			}).then((yes) => {
				if (!yes) return;
				toggling.value = true;
				sendRpc("plugins.skill.trust", { source: repoSource, skill: skill.name }).then((trustRes) => {
					if (!trustRes?.ok) {
						toggling.value = false;
						showToast(`Trust failed: ${trustRes?.error || "unknown"}`, "error");
						return;
					}
					sendRpc("plugins.skill.enable", { source: repoSource, skill: skill.name }).then((r) => {
						toggling.value = false;
						if (r?.ok) {
							fetchAll();
							if (props.onToggled) props.onToggled();
						} else {
							showToast(`Enable failed: ${r?.error || "unknown"}`, "error");
						}
					});
				});
			});
			return;
		}

		toggling.value = true;
		var method = skill.enabled ? "plugins.skill.disable" : "plugins.skill.enable";
		sendRpc(method, { source: repoSource, skill: skill.name }).then((r) => {
			toggling.value = false;
			if (r?.ok) {
				fetchAll();
				if (props.onToggled) props.onToggled();
			} else {
				showToast(`Failed: ${r?.error || "unknown"}`, "error");
			}
		});
	}

	return html`<div style="display:flex;align-items:center;justify-content:space-between;padding:6px 0;border-bottom:1px solid var(--border)"
    onMouseEnter=${(e) => {
			e.currentTarget.style.background = "var(--bg-hover)";
		}}
    onMouseLeave=${(e) => {
			e.currentTarget.style.background = "";
		}}>
    <div style="display:flex;align-items:center;gap:8px;min-width:0;flex:1;overflow:hidden;cursor:pointer" onClick=${() => props.onSelect?.(skill)}>
      <span style="font-family:var(--font-mono);font-size:.8rem;font-weight:500;color:var(--text-strong);white-space:nowrap">${skill.display_name || skill.name}</span>
      ${skill.display_name && html`<span style="font-size:.68rem;color:var(--muted);font-family:var(--font-mono);white-space:nowrap">${skill.name}</span>`}
      ${skill.description && html`<span style="color:var(--muted);font-size:.72rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${skill.description}</span>`}
    </div>
    <div style="display:flex;align-items:center;gap:6px;flex-shrink:0;margin-left:8px">
      ${skill.trusted === false && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">untrusted</span>`}
      ${skill.eligible === false && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--error, #e55);color:#fff;font-weight:500">blocked</span>`}
      <button onClick=${onToggle} disabled=${toggling.value} style=${{
				background: skill.enabled ? "none" : "var(--accent)",
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-sm)",
				fontSize: ".7rem",
				padding: "2px 8px",
				cursor: toggling.value ? "wait" : "pointer",
				color: skill.enabled ? "var(--muted)" : "#fff",
				fontWeight: 500,
				opacity: toggling.value ? 0.6 : 1,
			}}>${toggling.value ? "\u2026" : skill.enabled ? "Disable" : "Enable"}</button>
    </div>
  </div>`;
}

// ── Repo card: expand shows all skills, search filters them ──
function RepoCard(props) {
	var repo = props.repo;
	var expanded = useSignal(false);
	var allSkills = useSignal(null); // null = not loaded, [] = loaded
	var skillsLoading = useSignal(false);
	var searchQuery = useSignal("");
	var activeDetail = useSignal(null);
	var detailLoading = useSignal(false);
	var removingRepo = useSignal(false);
	var isOrphan = repo.orphaned === true || String(repo.source || "").startsWith("orphan:");
	var sourceLabel = isOrphan ? repo.repo_name : repo.source;

	var href = isOrphan ? null : /^https?:\/\//.test(repo.source) ? repo.source : `https://github.com/${repo.source}`;
	var formatLabel =
		repo.format === "claude_code" ? "Claude Code" : repo.format === "codex" ? "Codex" : repo.format || "Plugin";

	function toggleExpand() {
		expanded.value = !expanded.value;
		// Load all skills on first expand
		if (expanded.value && allSkills.value === null && !isOrphan) {
			skillsLoading.value = true;
			searchSkills(repo.source, "").then((results) => {
				allSkills.value = results;
				skillsLoading.value = false;
			});
		}
	}

	// Filtered skills based on search query
	var filteredSkills = computed(() => {
		var skills = allSkills.value;
		if (!skills) return [];
		var q = searchQuery.value.toLowerCase().trim();
		if (!q || isOrphan) return skills;
		return skills.filter((s) => {
			var name = (s.name || "").toLowerCase();
			var display = (s.display_name || "").toLowerCase();
			var desc = (s.description || "").toLowerCase();
			return name.includes(q) || display.includes(q) || desc.includes(q);
		});
	});

	function loadDetail(skill) {
		detailLoading.value = true;
		sendRpc("plugins.skill.detail", {
			source: repo.source,
			skill: skill.name,
		}).then((res) => {
			detailLoading.value = false;
			if (res?.ok) activeDetail.value = res.payload || {};
			else showToast(`Failed to load: ${res?.error || "unknown"}`, "error");
		});
	}

	function removeRepo(e) {
		e.stopPropagation();
		if (!S.connected || removingRepo.value) return;
		removingRepo.value = true;
		sendRpc("plugins.repos.remove", { source: repo.source }).then((res) => {
			removingRepo.value = false;
			if (res?.ok) fetchAll();
			else showToast(`Failed: ${res?.error || "unknown"}`, "error");
		});
	}

	return html`<div class="skills-repo-card">
    <div class="skills-repo-header" onClick=${toggleExpand}>
      <div style="display:flex;align-items:center;gap:8px">
        <span style=${{ fontSize: ".65rem", color: "var(--muted)", transition: "transform .15s", transform: expanded.value ? "rotate(90deg)" : "" }}>\u25B6</span>
				${
					href
						? html`<a href=${href} target="_blank" rel="noopener noreferrer" onClick=${(e) => {
								e.stopPropagation();
							}}
           style="font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong);text-decoration:none">${sourceLabel}</a>`
						: html`<span style="font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong)">${sourceLabel}</span>`
				}
        <span style="font-size:.62rem;padding:1px 6px;border-radius:9999px;background:var(--surface2);color:var(--muted);font-weight:500">${formatLabel}</span>
        <span style="font-size:.72rem;color:var(--muted)">${repo.enabled_count}/${repo.skill_count} enabled</span>
				${isOrphan && html`<span style="font-size:.64rem;padding:1px 6px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">orphaned on disk</span>`}
      </div>
      <button onClick=${removeRepo} disabled=${removingRepo.value}
        class="provider-btn provider-btn-sm provider-btn-danger">${removingRepo.value ? "Removing..." : "Remove"}</button>
    </div>
    ${
			expanded.value &&
			html`<div class="skills-repo-detail" style="display:block">
      ${isOrphan && html`<div style="color:var(--warning, #c77d00);font-size:.78rem;padding:8px 0">This repository exists on disk but is missing manifest metadata. Remove it or reinstall to restore normal plugin metadata.</div>`}
      ${
				allSkills.value &&
				allSkills.value.length > 8 &&
				html`
        <input type="text" placeholder=${`Filter ${allSkills.value.length} plugins\u2026`} value=${searchQuery.value}
          onInput=${(e) => {
						searchQuery.value = e.target.value;
						activeDetail.value = null;
					}}
          style="width:100%;padding:6px 10px;margin-bottom:8px;border:1px solid var(--border);border-radius:var(--radius-sm);background:var(--surface);color:var(--text);font-size:.8rem;font-family:var(--font-mono);box-sizing:border-box" />
      `
			}
      ${skillsLoading.value && html`<div style="color:var(--muted);font-size:.8rem;padding:8px 0">Loading plugins\u2026</div>`}
      ${allSkills.value && allSkills.value.length === 0 && html`<div style="color:var(--muted);font-size:.8rem;padding:8px 0">No plugins found in this repository.</div>`}
      ${
				filteredSkills.value.length > 0 &&
				html`<div style="max-height:360px;overflow-y:auto">
        ${filteredSkills.value.map(
					(skill) => html`<${SkillRow}
          key=${skill.name}
          skill=${skill}
          repoSource=${repo.source}
          onSelect=${(s) => {
						loadDetail(s);
					}}
          onToggled=${() => {
						searchSkills(repo.source, "").then((results) => {
							allSkills.value = results;
						});
					}}
        />`,
				)}
      </div>`
			}
      ${
				searchQuery.value &&
				filteredSkills.value.length === 0 &&
				allSkills.value &&
				allSkills.value.length > 0 &&
				html`
        <div style="color:var(--muted);font-size:.78rem;padding:8px 0">No plugins matching "${searchQuery.value}".</div>
      `
			}
      ${detailLoading.value && html`<div style="color:var(--muted);font-size:.8rem;padding:4px 0">Loading detail\u2026</div>`}
      ${
				activeDetail.value &&
				html`<${SkillDetail}
        detail=${activeDetail.value}
        repoSource=${repo.source}
        onClose=${() => {
					activeDetail.value = null;
				}}
      />`
			}
    </div>`
		}
  </div>`;
}

function ReposSection() {
	var r = repos.value;
	return html`<div class="skills-section">
    <h3 class="skills-section-title">Installed Repositories</h3>
    <div class="skills-section">
      ${(!r || r.length === 0) && html`<div style="padding:12px;color:var(--muted);font-size:.82rem">No plugin repositories installed. Install one from the featured list or enter a repo above.</div>`}
      ${r.map((repo) => html`<${RepoCard} key=${repo.source} repo=${repo} />`)}
    </div>
  </div>`;
}

function EnabledPluginsTable() {
	var s = enabledPlugins.value;
	var activeDetail = useSignal(null);
	var detailLoading = useSignal(false);
	if (!s || s.length === 0) return null;

	function onDisable(skill) {
		var source = skill.source;
		if (!(source && S.connected)) return;
		sendRpc("plugins.skill.disable", { source: source, skill: skill.name }).then((res) => {
			if (res?.ok) {
				activeDetail.value = null;
				fetchAll();
			}
		});
	}

	function loadDetail(skill) {
		// Toggle: close if clicking the same skill
		if (activeDetail.value && activeDetail.value.name === skill.name) {
			activeDetail.value = null;
			return;
		}
		var source = skill.source;
		if (!source) return;
		detailLoading.value = true;
		sendRpc("plugins.skill.detail", { source: source, skill: skill.name }).then((res) => {
			detailLoading.value = false;
			if (res?.ok) {
				activeDetail.value = res.payload || {};
			} else {
				showToast(`Failed to load: ${res?.error || "unknown"}`, "error");
			}
		});
	}

	return html`<div class="skills-section">
    <h3 class="skills-section-title">Enabled Plugins</h3>
    <div class="skills-table-wrap">
      <table style="width:100%;border-collapse:collapse;font-size:.82rem">
        <thead>
          <tr style="border-bottom:1px solid var(--border);background:var(--surface)">
            <th style="text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em">Name</th>
            <th style="text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em">Source</th>
            <th style="text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em"></th>
          </tr>
        </thead>
        <tbody>
          ${s.map(
						(skill) => html`<tr key=${skill.name} class="cursor-pointer" style="border-bottom:1px solid var(--border)"
              onClick=${() => {
								loadDetail(skill);
							}}
              onMouseEnter=${(e) => {
								e.currentTarget.style.background = "var(--bg-hover)";
							}}
              onMouseLeave=${(e) => {
								e.currentTarget.style.background = "";
							}}>
              <td style="padding:8px 12px;font-weight:500;color:var(--accent);font-family:var(--font-mono)">${skill.name}</td>
              <td style="padding:8px 12px;color:var(--muted);font-size:.75rem">${skill.source}</td>
              <td style="padding:8px 12px;text-align:right">
                <button class="provider-btn provider-btn-sm provider-btn-secondary" onClick=${(e) => {
									e.stopPropagation();
									onDisable(skill);
								}}>Disable</button>
              </td>
            </tr>`,
					)}
        </tbody>
      </table>
    </div>
    ${detailLoading.value && html`<div class="text-sm text-[var(--muted)] p-3">Loading\u2026</div>`}
    ${
			activeDetail.value &&
			html`<${SkillDetail}
      detail=${activeDetail.value}
      repoSource=${activeDetail.value.source}
      onClose=${() => {
				activeDetail.value = null;
			}}
    />`
		}
  </div>`;
}

function PluginsPage() {
	useEffect(() => {
		ensurePrefetch().then(() => {
			fetchAll();
		});
	}, []);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Plugins</h2>
        <button class="logs-btn" onClick=${fetchAll}>Refresh</button>
      </div>
      <p class="text-sm text-[var(--muted)]">Multi-format plugin repositories (Claude Code, Codex, etc.) normalized into the skills system.</p>
      <div class="skills-warn max-w-[600px]">
        <div class="skills-warn-title">\u26a0\ufe0f Third-party plugins run code on your machine</div>
        <div>Plugins from external repositories execute with <strong>your full system privileges</strong>. The featured Anthropic repository is maintained by a known source, but third-party repositories you add yourself may contain malicious code.</div>
        <div style="margin-top:4px"><strong>Triple-check the source code</strong> of any third-party repository before installing it. Review every plugin's instructions before enabling \u2014 these are the commands the agent will execute on your behalf.</div>
      </div>
      <${InstallBox} />
      <${FeaturedSection} />
      <${ReposSection} />
      ${loading.value && repos.value.length === 0 && html`<div style="padding:24px;text-align:center;color:var(--muted);font-size:.85rem">Loading plugins\u2026</div>`}
      <${EnabledPluginsTable} />
    </div>
    <${Toasts} />
  `;
}

// ── Router integration ───────────────────────────────────────
registerPage(
	"/plugins",
	function initPlugins(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		render(html`<${PluginsPage} />`, container);
	},
	function teardownPlugins() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
