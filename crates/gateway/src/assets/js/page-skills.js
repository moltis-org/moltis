// ── Skills page (Preact + HTM + Signals proof of concept) ───
// eslint-disable-next-line -- body_html is server-rendered trusted content

import { computed, signal, useSignal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import { updateNavCount } from "./nav-counts.js";
import { registerPage } from "./router.js";
import * as S from "./state.js";
import { ConfirmDialog, requestConfirm } from "./ui.js";

// ── Signals (reactive state) ─────────────────────────────────
var repos = signal([]); // lightweight summaries: { source, skill_count, enabled_count }
var enabledSkills = signal([]); // only enabled skills (from skills.list)
var loading = signal(false);
var toasts = signal([]);
var toastId = 0;

// Lazy prefetch: starts on first navigation to /skills, not at module load
var prefetchPromise = null;
function ensurePrefetch() {
	if (!prefetchPromise) {
		prefetchPromise = fetch("/api/skills")
			.then((r) => r.json())
			.then((data) => {
				if (data.skills) enabledSkills.value = data.skills;
				if (data.repos) repos.value = data.repos;
				return data;
			})
			.catch(() => null);
	}
	return prefetchPromise;
}

// Map enabled skill name → repo source (derived from enabled skills data)
var skillRepoMap = computed(() => {
	var map = {};
	enabledSkills.value.forEach((s) => {
		if (s.source) map[s.name] = s.source;
	});
	return map;
});

// ── Helpers ──────────────────────────────────────────────────
function showToast(message, type) {
	var id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

function shortSha(sha) {
	if (!(sha && typeof sha === "string")) return "";
	return sha.slice(0, 12);
}

function fetchAll() {
	loading.value = true;
	fetch("/api/skills")
		.then((r) => r.json())
		.then((data) => {
			if (data.skills) enabledSkills.value = data.skills;
			if (data.repos) repos.value = data.repos;
			loading.value = false;
			updateNavCount("skills", (data.skills || []).length);
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
	return sendRpc("skills.install", { source: source }).then((res) => {
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

// Debounced server-side search for skills within a repo
function searchSkills(source, query) {
	return fetch(`/api/skills/search?source=${encodeURIComponent(source)}&q=${encodeURIComponent(query)}`)
		.then((r) => r.json())
		.then((data) => data.skills || []);
}

// ── Components ───────────────────────────────────────────────

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

function SecurityWarning() {
	var dismissed = useSignal(!!localStorage.getItem("moltis-skills-warning-dismissed"));
	if (dismissed.value) return null;
	var threats = [
		"Execute arbitrary shell commands on your machine (install malware, cryptominers, backdoors)",
		"Read and exfiltrate sensitive data \u2014 SSH keys, API tokens, browser cookies, credentials, env variables",
		"Modify or delete files across your filesystem, including other projects",
		"Send your data to remote servers via curl/wget without your knowledge",
	];
	function dismiss() {
		localStorage.setItem("moltis-skills-warning-dismissed", "1");
		dismissed.value = true;
	}

	function emergencyDisableAll() {
		requestConfirm("Disable all third-party skills and plugins now?", {
			confirmLabel: "Disable All",
			danger: true,
		}).then((yes) => {
			if (!yes) return;
			sendRpc("skills.emergency_disable", {}).then((res) => {
				if (!res?.ok) {
					showToast(`Emergency disable failed: ${res?.error || "unknown"}`, "error");
					return;
				}
				var p = res.payload || {};
				showToast(`Disabled ${p.skills_disabled || 0} skills and ${p.plugins_disabled || 0} plugins`, "success");
				fetchAll();
			});
		});
	}

	return html`<div class="skills-warn">
    <div class="skills-warn-title">\u26a0\ufe0f Skills run code on your machine \u2014 treat every skill as untrusted</div>
    <div>Skills are community-authored instructions that the AI agent follows <strong>with your full system privileges</strong>. Popularity or download count does not mean a skill is safe. A malicious skill can instruct the agent to:</div>
    <ul style="margin:6px 0 6px 18px;padding:0">
      ${threats.map((t) => html`<li>${t}</li>`)}
    </ul>
    <div style="margin-top:4px"><strong>Triple-check the source code</strong> of every skill before enabling it. Read the full SKILL.md and any scripts it references \u2014 these are the exact instructions the agent will execute on your behalf. Do not trust a skill just because it is popular, highly downloaded, or appears on a leaderboard.</div>
    <div style="margin-top:6px;color:var(--success, #4a4)">With sandbox mode enabled (Docker, Apple Container, or cgroup), command execution is isolated and the damage a malicious skill can do is significantly limited.</div>
    <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-top:8px">
      <button onClick=${dismiss} style="background:none;border:1px solid var(--border);border-radius:var(--radius-sm);font-size:.72rem;padding:3px 10px;cursor:pointer;color:var(--muted)">Dismiss</button>
      <button class="provider-btn provider-btn-danger provider-btn-sm" onClick=${emergencyDisableAll}>Disable all third-party skills</button>
    </div>
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
    <input ref=${inputRef} type="text" placeholder="owner/repo or full URL (e.g. anthropics/skills)" class="skills-install-input" onKeyDown=${onKey} />
    <button class="provider-btn" onClick=${onInstall} disabled=${installing.value}>
      ${installing.value ? "Installing\u2026" : "Install"}
    </button>
  </div>`;
}

var featuredSkills = [
	{ repo: "openclaw/skills", desc: "Community skills from ClawdHub" },
	{ repo: "anthropics/skills", desc: "Official Anthropic agent skills" },
	{ repo: "vercel-labs/agent-skills", desc: "Vercel agent skills collection" },
	{ repo: "vercel-labs/skills", desc: "Vercel skills toolkit" },
];

function FeaturedCard(props) {
	var f = props.skill;
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
	return html`<div class="skills-section">
    <h3 class="skills-section-title">Featured Repositories</h3>
    <div class="skills-featured-grid">
      ${featuredSkills.map((f) => html`<${FeaturedCard} key=${f.repo} skill=${f} />`)}
    </div>
  </div>`;
}

// ── Skill detail sub-components ──────────────────────────────

function eligibilityBadge(d) {
	var hasReqs = d.requires && (d.requires.bins?.length || d.requires.any_bins?.length);
	if (d.eligible === false)
		return html`<span style="font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--error, #e55);color:#fff;font-weight:500">blocked</span>`;
	if (hasReqs)
		return html`<span style="font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--success, #4a4);color:#fff;font-weight:500">eligible</span>`;
	return html`<span style="font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--surface2);color:var(--muted);font-weight:500">no deps declared</span>`;
}

function trustBadge(d) {
	if (d.trusted === false)
		return html`<span style="font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">untrusted</span>`;
	return null;
}

function SkillMetadata(props) {
	var d = props.detail;
	if (!(d.author || d.version || d.homepage || d.source_url || d.commit_sha)) return null;
	return html`<div style="display:flex;align-items:center;gap:12px;margin-bottom:8px;font-size:.75rem;color:var(--muted);flex-wrap:wrap">
    ${d.author && html`<span>Author: ${d.author}</span>`}
    ${d.version && html`<span>v${d.version}</span>`}
    ${d.commit_sha && html`<span>Commit: <code>${shortSha(d.commit_sha)}</code></span>`}
    ${d.homepage && html`<a href=${d.homepage} target="_blank" rel="noopener noreferrer" style="color:var(--accent);text-decoration:none;font-size:.75rem">${d.homepage.replace(/^https?:\/\//, "")}</a>`}
    ${d.source_url && html`<a href=${d.source_url} target="_blank" rel="noopener noreferrer" style="color:var(--accent);text-decoration:none;font-size:.75rem">View source</a>`}
  </div>`;
}

function MissingDepsSection(props) {
	var d = props.detail;
	if (!(d.eligible === false && d.missing_bins && d.missing_bins.length > 0)) return null;
	return html`<div style="margin-bottom:8px;font-size:.78rem">
    <span style="color:var(--error, #e55);font-weight:500">Missing: ${d.missing_bins.map((b) => `bin:${b}`).join(", ")}</span>
    ${(d.install_options || []).map(
			(opt, idx) =>
				html`<button onClick=${() => {
					var preview = opt?.label || `Install via ${opt?.kind || "package manager"}`;
					var confirmed = window.confirm(
						`Install dependency for ${d.name}?\n\n${preview}\n\nOnly continue if you trust this skill and its source.`,
					);
					if (!confirmed) return;
					sendRpc("skills.install_dep", { skill: d.name, index: idx, confirm: true }).then((r) => {
						if (r?.ok) {
							showToast(`Installed dependency for ${d.name}`, "success");
							props.onReload?.();
						} else showToast(`Install failed: ${r?.error || "unknown"}`, "error");
					});
				}} style="margin-left:6px;background:var(--accent);color:#fff;border:none;border-radius:var(--radius-sm);font-size:.7rem;padding:2px 8px;cursor:pointer">${opt.label || `Install via ${opt.kind}`}</button>`,
		)}
  </div>`;
}

// ── Skill detail panel ───────────────────────────────────────
function SkillDetail(props) {
	var d = props.detail;
	var onClose = props.onClose;

	var panelRef = useRef(null);
	var didScroll = useRef(false);

	var bodyRef = useRef(null);
	useEffect(() => {
		if (bodyRef.current && d?.body_html) {
			// Safe: body_html is server-rendered trusted HTML from the Rust gateway (SKILL.md → pulldown-cmark)
			bodyRef.current.textContent = "";
			var tpl = document.createElement("template");
			tpl.innerHTML = d.body_html; // eslint-disable-line no-unsanitized/property
			bodyRef.current.appendChild(tpl.content);
			bodyRef.current.querySelectorAll("a").forEach((a) => {
				a.setAttribute("target", "_blank");
				a.setAttribute("rel", "noopener");
			});
		}
		// Scroll only on first render (panel just opened), not when switching skills.
		if (panelRef.current && !didScroll.current) {
			didScroll.current = true;
			var el = panelRef.current;
			var scrollParent = el.parentElement;
			while (
				scrollParent &&
				getComputedStyle(scrollParent).overflowY !== "auto" &&
				getComputedStyle(scrollParent).overflowY !== "scroll"
			) {
				scrollParent = scrollParent.parentElement;
			}
			if (scrollParent) {
				var panelTop =
					el.getBoundingClientRect().top - scrollParent.getBoundingClientRect().top + scrollParent.scrollTop;
				scrollParent.scrollTo({ top: panelTop, behavior: "smooth" });
			}
		}
	}, [d?.body_html]);

	if (!d) return null;

	var isDisc = d.source === "personal" || d.source === "project";
	var needsTrust = !isDisc && d.trusted === false;

	function doToggle() {
		var method = d.enabled ? "skills.skill.disable" : "skills.skill.enable";
		sendRpc(method, { source: props.repoSource, skill: d.name }).then((r) => {
			if (r?.ok) {
				if (isDisc) onClose();
				fetchAll();
				props.onReload?.();
			} else {
				showToast(`Failed: ${r?.error || "unknown error"}`, "error");
			}
		});
	}

	function onToggle() {
		if (!S.connected) return;
		if (!d.enabled && needsTrust) {
			requestConfirm(`Trust skill "${d.name}" from ${props.repoSource}?`, {
				confirmLabel: "Trust & Enable",
			}).then((yes) => {
				if (!yes) return;
				sendRpc("skills.skill.trust", { source: props.repoSource, skill: d.name }).then((res) => {
					if (!res?.ok) {
						showToast(`Trust failed: ${res?.error || "unknown error"}`, "error");
						return;
					}
					doToggle();
				});
			});
			return;
		}
		if (isDisc && d.enabled) {
			requestConfirm(`Delete skill "${d.name}"? This removes the SKILL.md file.`, {
				confirmLabel: "Delete",
				danger: true,
			}).then((yes) => {
				if (yes) doToggle();
			});
			return;
		}
		doToggle();
	}

	return html`<div ref=${panelRef} class="skills-detail-panel" style="display:block">
    <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:8px">
      <div style="display:flex;align-items:center;gap:8px">
        <span style="font-family:var(--font-mono);font-size:.9rem;font-weight:600;color:var(--text-strong)">${d.display_name || d.name}</span>
        ${d.display_name && html`<span style="font-family:var(--font-mono);font-size:.72rem;color:var(--muted)">${d.name}</span>`}
        ${d.license && html`<span style="font-size:.65rem;padding:1px 6px;border-radius:9999px;background:var(--surface2);color:var(--muted)">${d.license}</span>`}
        ${eligibilityBadge(d)}
        ${trustBadge(d)}
      </div>
      <div style="display:flex;align-items:center;gap:6px">
        <button onClick=${onToggle} class=${isDisc && d.enabled ? "provider-btn provider-btn-sm provider-btn-danger" : ""} style=${
					isDisc && d.enabled
						? {}
						: {
								background: d.enabled ? "none" : "var(--accent)",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								fontSize: ".72rem",
								padding: "3px 10px",
								cursor: "pointer",
								color: d.enabled ? "var(--muted)" : "#fff",
								fontWeight: 500,
							}
				}>${isDisc && d.enabled ? "Delete" : d.enabled ? "Disable" : "Enable"}</button>
        <button onClick=${onClose} style="background:none;border:none;color:var(--muted);font-size:.9rem;cursor:pointer;padding:2px 4px">\u2715</button>
      </div>
    </div>
    <${SkillMetadata} detail=${d} />
    ${d.drifted && html`<div style="margin:0 0 8px;font-size:.75rem;color:var(--warning, #c77d00)">Source changed since last trust; review updates before enabling again.</div>`}
    ${d.description && html`<p style="margin:0 0 8px;font-size:.82rem;color:var(--text)">${d.description}</p>`}
    <${MissingDepsSection} detail=${d} onReload=${props.onReload} />
    ${d.compatibility && html`<div style="margin-bottom:8px;font-size:.75rem;color:var(--muted);font-style:italic">${d.compatibility}</div>`}
    ${d.allowed_tools && d.allowed_tools.length > 0 && html`<div style="margin-bottom:8px;font-size:.75rem;color:var(--muted)">Allowed tools: ${d.allowed_tools.join(", ")}</div>`}
    ${d.body_html && html`<div ref=${bodyRef} class="skill-body-md" style="border-top:1px solid var(--border);padding-top:8px;margin-top:8px;max-height:400px;overflow-y:auto;font-size:.8rem;color:var(--text);line-height:1.5" />`}
    ${!d.body_html && d.body && html`<div style="border-top:1px solid var(--border);padding-top:8px;margin-top:8px"><pre style="white-space:pre-wrap;word-break:break-word;font-size:.78rem;color:var(--text);font-family:var(--font-mono);margin:0;max-height:400px;overflow-y:auto">${d.body}</pre></div>`}
  </div>`;
}

// ── Repo card with server-side search ────────────────────────
function RepoCard(props) {
	var repo = props.repo;
	var expanded = useSignal(false);
	var searchQuery = useSignal("");
	var searchResults = useSignal([]);
	var searching = useSignal(false);
	var activeDetail = useSignal(null);
	var detailLoading = useSignal(false);
	var searchTimer = useRef(null);

	var href = /^https?:\/\//.test(repo.source) ? repo.source : `https://github.com/${repo.source}`;

	function toggleExpand() {
		expanded.value = !expanded.value;
	}

	function onSearchInput(e) {
		var q = e.target.value;
		searchQuery.value = q;
		activeDetail.value = null;
		if (searchTimer.current) clearTimeout(searchTimer.current);
		if (!q.trim()) {
			searchResults.value = [];
			return;
		}
		searching.value = true;
		searchTimer.current = setTimeout(() => {
			searchSkills(repo.source, q.trim()).then((results) => {
				searchResults.value = results;
				searching.value = false;
			});
		}, 200);
	}

	function loadDetail(skill) {
		searchQuery.value = skill.name;
		searchResults.value = [];
		detailLoading.value = true;
		sendRpc("skills.skill.detail", {
			source: repo.source,
			skill: skill.name,
		}).then((res) => {
			detailLoading.value = false;
			if (res?.ok) {
				activeDetail.value = res.payload || {};
			} else {
				showToast(`Failed to load: ${res?.error || "unknown"}`, "error");
			}
		});
	}

	function removeRepo(e) {
		e.stopPropagation();
		if (!S.connected) return;
		sendRpc("skills.repos.remove", { source: repo.source }).then((res) => {
			if (res?.ok) fetchAll();
		});
	}

	return html`<div class="skills-repo-card">
    <div class="skills-repo-header" onClick=${toggleExpand}>
      <div style="display:flex;align-items:center;gap:8px">
        <span style=${{ fontSize: ".65rem", color: "var(--muted)", transition: "transform .15s", transform: expanded.value ? "rotate(90deg)" : "" }}>\u25B6</span>
        <a href=${href} target="_blank" rel="noopener noreferrer" onClick=${(e) => {
					e.stopPropagation();
				}}
           style="font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong);text-decoration:none">${repo.source}</a>
        <span style="font-size:.72rem;color:var(--muted)">${repo.enabled_count}/${repo.skill_count} enabled</span>
				${repo.commit_sha && html`<span style="font-size:.68rem;color:var(--muted)">sha ${shortSha(repo.commit_sha)}</span>`}
				${repo.drifted && html`<span style="font-size:.64rem;padding:1px 6px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">source changed</span>`}
      </div>
      <button class="provider-btn provider-btn-sm provider-btn-danger" onClick=${removeRepo}>Remove</button>
    </div>
    ${
			expanded.value &&
			html`<div class="skills-repo-detail" style="display:block">
      <div style="position:relative;margin-bottom:8px">
        <input type="text" placeholder=${`Search skills in ${repo.source}\u2026`} value=${searchQuery.value}
          onInput=${onSearchInput}
          style="width:100%;padding:6px 10px;border:1px solid var(--border);border-radius:var(--radius-sm);background:var(--surface);color:var(--text);font-size:.8rem;font-family:var(--font-mono);box-sizing:border-box" />
        ${
					searchQuery.value &&
					searchResults.value.length > 0 &&
					!activeDetail.value &&
					html`
          <div class="skills-ac-dropdown" style="display:block">
            ${searchResults.value.map(
							(skill) => html`<div key=${skill.name} class="skills-ac-item" onClick=${() => {
								loadDetail(skill);
							}}>
                <div style="display:flex;align-items:center;gap:6px;min-width:0">
                  <span style="font-family:var(--font-mono);font-weight:500;color:var(--text-strong);white-space:nowrap">${skill.display_name || skill.name}</span>
                  ${skill.display_name && html`<span style="color:var(--muted);font-size:.68rem;font-family:var(--font-mono);white-space:nowrap">${skill.name}</span>`}
                  ${skill.description && html`<span style="color:var(--muted);font-size:.72rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">${skill.description}</span>`}
                </div>
                <div style="display:flex;align-items:center;gap:4px;flex-shrink:0;margin-left:8px">
                  ${skill.enabled && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--accent);color:#fff;font-weight:500">enabled</span>`}
                  ${skill.trusted === false && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">untrusted</span>`}
                  ${skill.drifted && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--warning, #c77d00);color:#fff;font-weight:500">source changed</span>`}
                  ${skill.eligible === false && html`<span style="font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--error, #e55);color:#fff;font-weight:500">blocked</span>`}
                </div>
              </div>`,
						)}
          </div>
        `
				}
        ${
					searchQuery.value &&
					searchResults.value.length === 0 &&
					!activeDetail.value &&
					!searching.value &&
					html`
          <div class="skills-ac-dropdown" style="display:block">
            <div style="padding:8px 10px;color:var(--muted);font-size:.78rem">No matching skills.</div>
          </div>
        `
				}
      </div>
      ${detailLoading.value && html`<div style="color:var(--muted);font-size:.8rem">Loading\u2026</div>`}
      ${
				activeDetail.value &&
				html`<${SkillDetail}
        detail=${activeDetail.value}
        repoSource=${repo.source}
        onClose=${() => {
					activeDetail.value = null;
					searchQuery.value = "";
				}}
        onReload=${() => {
					loadDetail({ name: activeDetail.value.name });
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
      ${(!r || r.length === 0) && html`<div style="padding:12px;color:var(--muted);font-size:.82rem">No repositories installed.</div>`}
      ${r.map((repo) => html`<${RepoCard} key=${repo.source} repo=${repo} />`)}
    </div>
  </div>`;
}

function SourceBadge(props) {
	var src = props.source || "";
	// Discovered skills have source types like "personal", "project".
	// Registry skills have repo sources like "owner/repo".
	var isType = !src.includes("/");
	var label = isType ? src.charAt(0).toUpperCase() + src.slice(1) : src;
	var cls = isType ? "recommended-badge" : "tier-badge";
	return html`<span class=${cls}>${label}</span>`;
}

function EnabledSkillsTable() {
	var s = enabledSkills.value;
	var map = skillRepoMap.value;
	var activeDetail = useSignal(null);
	var detailLoading = useSignal(false);
	if (!s || s.length === 0) return null;

	function isDiscovered(skill) {
		var src = skill.source || "";
		return src === "personal" || src === "project";
	}

	function doDisable(skill) {
		var source = map[skill.name] || skill.source;
		sendRpc("skills.skill.disable", { source: source, skill: skill.name }).then((res) => {
			if (res?.ok) {
				activeDetail.value = null;
				showToast(isDiscovered(skill) ? `Deleted ${skill.name}` : `Disabled ${skill.name}`, "success");
				fetchAll();
			} else {
				showToast(`Failed: ${res?.error?.message || res?.error || "unknown error"}`, "error");
			}
		});
	}

	function onDisable(skill) {
		var source = map[skill.name] || skill.source;
		if (!source) {
			showToast("Cannot disable: unknown source for skill.", "error");
			return;
		}
		if (isDiscovered(skill)) {
			requestConfirm(`Delete skill "${skill.name}"? This removes the SKILL.md file.`, {
				confirmLabel: "Delete",
				danger: true,
			}).then((yes) => {
				if (yes) doDisable(skill);
			});
			return;
		}
		doDisable(skill);
	}

	function loadDetail(skill) {
		// Toggle: close if clicking the same skill
		if (activeDetail.value && activeDetail.value.name === skill.name) {
			activeDetail.value = null;
			return;
		}
		var source = map[skill.name] || skill.source;
		if (!source) return;
		detailLoading.value = true;
		sendRpc("skills.skill.detail", { source: source, skill: skill.name }).then((res) => {
			detailLoading.value = false;
			if (res?.ok) {
				activeDetail.value = res.payload || {};
			} else {
				showToast(`Failed to load: ${res?.error || "unknown"}`, "error");
			}
		});
	}

	return html`<div class="skills-section">
    <h3 class="skills-section-title">Enabled Skills</h3>
    <div class="skills-table-wrap">
      <table style="width:100%;border-collapse:collapse;font-size:.82rem">
        <thead>
          <tr style="border-bottom:1px solid var(--border);background:var(--surface)">
            <th style="text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em">Name</th>
            <th style="text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em">Description</th>
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
              <td style="padding:8px 12px;color:var(--text)">${skill.description || "\u2014"}</td>
              <td style="padding:8px 12px"><${SourceBadge} source=${skill.source} /></td>
              <td style="padding:8px 12px;text-align:right">
                <button class=${isDiscovered(skill) ? "provider-btn provider-btn-sm provider-btn-danger" : "provider-btn provider-btn-sm provider-btn-secondary"} onClick=${(
									e,
								) => {
									e.stopPropagation();
									onDisable(skill);
								}}>${isDiscovered(skill) ? "Delete" : "Disable"}</button>
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
      onReload=${() => {
				loadDetail({ name: activeDetail.value.name, source: activeDetail.value.source });
			}}
    />`
		}
  </div>`;
}

function SkillsPage() {
	useEffect(() => {
		ensurePrefetch().then(() => {
			fetchAll();
		});
	}, []);

	return html`
    <div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
      <div class="flex items-center gap-3">
        <h2 class="text-lg font-medium text-[var(--text-strong)]">Skills</h2>
        <button class="logs-btn" onClick=${fetchAll}>Refresh</button>
      </div>
      <p class="text-sm text-[var(--muted)]">SKILL.md-based skills discovered from project, personal, and installed paths.</p>
      <${SecurityWarning} />
      <${InstallBox} />
      <${FeaturedSection} />
      <${ReposSection} />
      ${loading.value && enabledSkills.value.length === 0 && repos.value.length === 0 && html`<div style="padding:24px;text-align:center;color:var(--muted);font-size:.85rem">Loading skills\u2026</div>`}
      <${EnabledSkillsTable} />
    </div>
    <${Toasts} />
    <${ConfirmDialog} />
  `;
}

// ── Router integration ───────────────────────────────────────
registerPage(
	"/skills",
	function initSkills(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		render(html`<${SkillsPage} />`, container);
	},
	function teardownSkills() {
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);
