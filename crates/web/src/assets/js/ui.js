// ── Shared Preact UI components ───────────────────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { useEffect, useRef, useState } from "preact/hooks";

// ── Toast notifications ──────────────────────────────────────
export var toasts = signal([]);
var toastId = 0;

export function showToast(message, type) {
	var id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}

export function Toasts() {
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

// ── Modal wrapper ────────────────────────────────────────────
export function Modal(props) {
	var show = props.show;
	var onClose = props.onClose;
	var title = props.title;

	function onBackdrop(e) {
		if (e.target === e.currentTarget && onClose) onClose();
	}

	useEffect(() => {
		if (!show) return;
		function onKey(e) {
			if (e.key === "Escape" && onClose) onClose();
		}
		document.addEventListener("keydown", onKey);
		return () => document.removeEventListener("keydown", onKey);
	}, [show, onClose]);

	if (!show) return null;

	return html`<div class="modal-overlay" onClick=${onBackdrop} style="display:flex;position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:100;align-items:center;justify-content:center;">
    <div class="modal-box" style="background:var(--surface);border-radius:var(--radius);padding:20px;max-width:500px;width:90%;max-height:85vh;overflow-y:auto;box-shadow:0 8px 32px rgba(0,0,0,.25);border:1px solid var(--border);">
      <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:14px;">
        <h3 style="margin:0;font-size:.95rem;font-weight:600;color:var(--text-strong)">${title}</h3>
        <button onClick=${onClose} style="background:none;border:none;color:var(--muted);font-size:1.1rem;cursor:pointer;padding:2px 6px">\u2715</button>
      </div>
      ${props.children}
    </div>
  </div>`;
}

// ── Confirm dialog ───────────────────────────────────────────
var confirmState = signal(null);

export function requestConfirm(message, opts) {
	return new Promise((resolve) => {
		confirmState.value = { message: message, resolve: resolve, opts: opts || {} };
	});
}

export function ConfirmDialog() {
	var s = confirmState.value;
	if (!s) return null;

	function yes() {
		s.resolve(true);
		confirmState.value = null;
	}
	function no() {
		s.resolve(false);
		confirmState.value = null;
	}

	var label = s.opts.confirmLabel || "Confirm";
	var danger = s.opts.danger;
	var btnClass = danger ? "provider-btn provider-btn-danger" : "provider-btn";

	return html`<${Modal} show=${true} onClose=${no} title="Confirm">
    <p style="font-size:.85rem;color:var(--text);margin:0 0 16px;">${s.message}</p>
    <div style="display:flex;gap:8px;justify-content:flex-end;">
      <button onClick=${no} class="provider-btn provider-btn-secondary">Cancel</button>
      <button onClick=${yes} class=${btnClass}>${label}</button>
    </div>
  </${Modal}>`;
}

/**
 * Vanilla-JS confirm dialog (no Preact needed).
 * Returns a Promise<boolean> — true if confirmed, false if cancelled.
 * Safe: all content set via textContent, no user input in markup.
 */
export function confirmDialog(message) {
	return new Promise((resolve) => {
		var backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";

		var box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "360px";

		var body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "16px";

		var msg = document.createElement("p");
		msg.style.cssText = "font-size:.85rem;color:var(--text);margin:0";
		msg.textContent = message;

		var btnRow = document.createElement("div");
		btnRow.style.cssText = "display:flex;gap:8px;justify-content:flex-end";

		var cancelBtn = document.createElement("button");
		cancelBtn.className = "provider-btn provider-btn-secondary";
		cancelBtn.textContent = "Cancel";

		var deleteBtn = document.createElement("button");
		deleteBtn.className = "provider-btn provider-btn-danger";
		deleteBtn.textContent = "Delete";

		function close(val) {
			backdrop.remove();
			resolve(val);
		}
		cancelBtn.addEventListener("click", () => close(false));
		deleteBtn.addEventListener("click", () => close(true));
		backdrop.addEventListener("click", (e) => {
			if (e.target === backdrop) close(false);
		});

		btnRow.appendChild(cancelBtn);
		btnRow.appendChild(deleteBtn);
		body.appendChild(msg);
		body.appendChild(btnRow);
		box.appendChild(body);
		backdrop.appendChild(box);
		document.body.appendChild(backdrop);
		deleteBtn.focus();
	});
}

/**
 * Vanilla-JS share visibility picker using the standard provider modal style.
 * Returns "public", "private", or null when cancelled.
 */
export function shareVisibilityDialog() {
	return new Promise((resolve) => {
		var backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";

		var box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "460px";

		var header = document.createElement("div");
		header.className = "provider-modal-header";

		var title = document.createElement("div");
		title.className = "provider-item-name";
		title.textContent = "Share session snapshot";

		var cancelTopBtn = document.createElement("button");
		cancelTopBtn.className = "provider-btn provider-btn-secondary provider-btn-sm";
		cancelTopBtn.textContent = "Cancel";

		var body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "10px";

		var hint = document.createElement("p");
		hint.style.cssText = "font-size:.8rem;color:var(--muted);margin:0";
		hint.textContent = "A snapshot is frozen at this point, later chat messages stay private.";

		var warning = document.createElement("p");
		warning.style.cssText =
			"font-size:.8rem;color:var(--text);margin:0;padding:8px 10px;border:1px solid color-mix(in srgb,var(--warn) 55%,var(--border) 45%);background:color-mix(in srgb,var(--warn) 12%,var(--surface2) 88%);border-radius:var(--radius-sm);line-height:1.45";
		warning.textContent =
			"We do best-effort redaction for API keys and tokens in shared tool output, but always review before sharing.";

		var publicBtn = document.createElement("button");
		publicBtn.className = "provider-item";
		publicBtn.type = "button";
		publicBtn.setAttribute("data-share-visibility", "public");
		var publicName = document.createElement("div");
		publicName.className = "provider-item-name";
		publicName.textContent = "Public link";
		var publicBadge = document.createElement("span");
		publicBadge.className = "provider-item-badge configured";
		publicBadge.textContent = "Open";
		publicBtn.appendChild(publicName);
		publicBtn.appendChild(publicBadge);

		var privateBtn = document.createElement("button");
		privateBtn.className = "provider-item";
		privateBtn.type = "button";
		privateBtn.setAttribute("data-share-visibility", "private");
		var privateName = document.createElement("div");
		privateName.className = "provider-item-name";
		privateName.textContent = "Private link";
		var privateBadge = document.createElement("span");
		privateBadge.className = "provider-item-badge api-key";
		privateBadge.textContent = "Key required";
		privateBtn.appendChild(privateName);
		privateBtn.appendChild(privateBadge);

		function close(value) {
			document.removeEventListener("keydown", onKeydown);
			backdrop.remove();
			resolve(value);
		}

		function onKeydown(e) {
			if (e.key === "Escape") close(null);
		}

		publicBtn.addEventListener("click", () => close("public"));
		privateBtn.addEventListener("click", () => close("private"));
		cancelTopBtn.addEventListener("click", () => close(null));
		backdrop.addEventListener("click", (e) => {
			if (e.target === backdrop) close(null);
		});
		document.addEventListener("keydown", onKeydown);

		body.appendChild(hint);
		body.appendChild(warning);
		body.appendChild(publicBtn);
		body.appendChild(privateBtn);
		header.appendChild(title);
		header.appendChild(cancelTopBtn);
		box.appendChild(header);
		box.appendChild(body);
		backdrop.appendChild(box);
		document.body.appendChild(backdrop);

		publicBtn.focus();
	});
}

/**
 * Styled share-link dialog used when auto-copy is unavailable.
 * Returns "copied" when copy succeeded, otherwise null on close/dismiss.
 */
export function shareLinkDialog(url, visibility) {
	return new Promise((resolve) => {
		var backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";
		backdrop.setAttribute("data-share-link-modal", "true");

		var box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "560px";

		var header = document.createElement("div");
		header.className = "provider-modal-header";

		var title = document.createElement("div");
		title.className = "provider-item-name";
		title.textContent = "Share link ready";

		var closeTopBtn = document.createElement("button");
		closeTopBtn.className = "provider-btn provider-btn-secondary";
		closeTopBtn.textContent = "Close";
		closeTopBtn.setAttribute("data-share-link-close", "true");

		var body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "10px";

		var hint = document.createElement("p");
		hint.style.cssText = "font-size:.8rem;color:var(--muted);margin:0";
		hint.textContent =
			visibility === "private"
				? "This is a private share link with an embedded key. Send it only to trusted people."
				: "This snapshot is frozen at this point in time.";

		var input = document.createElement("input");
		input.className = "provider-key-input";
		input.readOnly = true;
		input.value = url;
		input.setAttribute("data-share-link-input", "true");
		input.addEventListener("focus", () => input.select());
		input.addEventListener("click", () => input.select());

		var btnRow = document.createElement("div");
		btnRow.style.cssText = "display:flex;gap:8px;justify-content:flex-end;flex-wrap:wrap";

		var openBtn = document.createElement("button");
		openBtn.className = "provider-btn provider-btn-secondary";
		openBtn.textContent = "Open link";
		openBtn.setAttribute("data-share-link-open", "true");

		var copyBtn = document.createElement("button");
		copyBtn.className = "provider-btn";
		copyBtn.textContent = "Copy link";
		copyBtn.setAttribute("data-share-link-copy", "true");

		function close(value) {
			document.removeEventListener("keydown", onKeydown);
			backdrop.remove();
			resolve(value);
		}

		function onKeydown(e) {
			if (e.key === "Escape") close(null);
		}

		async function copyLink() {
			try {
				if (navigator.clipboard?.writeText) {
					await navigator.clipboard.writeText(url);
					showToast("Share link copied", "success");
					close("copied");
					return;
				}
			} catch (_err) {
				// Clipboard permissions can fail. Fall through to manual copy fallback.
			}
			input.focus();
			input.select();
			var copied = false;
			try {
				copied = document.execCommand("copy");
			} catch (_err) {
				copied = false;
			}
			if (copied) {
				showToast("Share link copied", "success");
				close("copied");
				return;
			}
			showToast("Copy failed. Copy the link manually.", "error");
		}

		copyBtn.addEventListener("click", () => {
			void copyLink();
		});
		openBtn.addEventListener("click", () => {
			window.open(url, "_blank", "noopener,noreferrer");
		});
		closeTopBtn.addEventListener("click", () => close(null));
		backdrop.addEventListener("click", (e) => {
			if (e.target === backdrop) close(null);
		});
		document.addEventListener("keydown", onKeydown);

		btnRow.appendChild(openBtn);
		btnRow.appendChild(copyBtn);
		header.appendChild(title);
		header.appendChild(closeTopBtn);
		body.appendChild(hint);
		body.appendChild(input);
		body.appendChild(btnRow);
		box.appendChild(header);
		box.appendChild(body);
		backdrop.appendChild(box);
		document.body.appendChild(backdrop);
		copyBtn.focus();
	});
}

// ── Model select dropdown (Preact, reuses .model-combo CSS) ──
export function ModelSelect({ models, value, onChange, placeholder }) {
	var [open, setOpen] = useState(false);
	var [query, setQuery] = useState("");
	var [kbIndex, setKbIndex] = useState(-1);
	var ref = useRef(null);
	var searchRef = useRef(null);
	var listRef = useRef(null);

	var selected = models.find((m) => m.id === value);
	var label = selected ? selected.displayName || selected.id : placeholder || "(none)";

	var filtered = models.filter((m) => {
		if (!query) return true;
		var q = query.toLowerCase();
		return (
			(m.displayName || "").toLowerCase().includes(q) ||
			m.id.toLowerCase().includes(q) ||
			(m.provider || "").toLowerCase().includes(q)
		);
	});

	useEffect(() => {
		if (!open) return;
		function onClick(e) {
			if (ref.current && !ref.current.contains(e.target)) setOpen(false);
		}
		document.addEventListener("mousedown", onClick);
		return () => document.removeEventListener("mousedown", onClick);
	}, [open]);

	useEffect(() => {
		if (open && searchRef.current) searchRef.current.focus();
	}, [open]);

	useEffect(() => {
		setKbIndex(-1);
	}, [query]);

	function onKeyDown(e) {
		if (e.key === "Escape") {
			setOpen(false);
		} else if (e.key === "ArrowDown") {
			e.preventDefault();
			setKbIndex((i) => Math.min(i + 1, filtered.length - 1));
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			setKbIndex((i) => Math.max(i - 1, 0));
		} else if (e.key === "Enter") {
			e.preventDefault();
			var idx = kbIndex >= 0 ? kbIndex : 0;
			if (filtered[idx]) pick(filtered[idx]);
		}
	}

	function pick(m) {
		onChange(m ? m.id : "");
		setOpen(false);
		setQuery("");
	}

	return html`<div class="model-combo" ref=${ref} style="width:100%;">
    <button type="button" class="model-combo-btn" style="width:100%;" onClick=${() => setOpen(!open)}>
      <span class="model-item-label">${label}</span>
      <span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>
    </button>
    ${
			open &&
			html`<div class="model-dropdown" style="width:100%;" onKeyDown=${onKeyDown}>
      <input class="model-search-input" ref=${searchRef} placeholder="Search models\u2026"
        value=${query} onInput=${(e) => setQuery(e.target.value)} />
      <div class="model-dropdown-list" ref=${listRef}>
        <div class="model-dropdown-item ${value ? "" : "selected"}"
          onClick=${() => pick(null)}>
          <span class="model-item-label">${placeholder || "(none)"}</span>
        </div>
        ${filtered.map(
					(m, i) => html`<div key=${m.id}
            class="model-dropdown-item ${m.id === value ? "selected" : ""} ${i === kbIndex ? "kb-active" : ""}"
            onClick=${() => pick(m)}>
            <span class="model-item-label">${m.displayName || m.id}</span>
            ${m.provider && html`<span class="model-item-provider">${m.provider}</span>`}
          </div>`,
				)}
        ${filtered.length === 0 && html`<div class="model-dropdown-empty">No matches</div>`}
      </div>
    </div>`
		}
  </div>`;
}

/**
 * Generic combo select for simple value/label options.
 * @param {Array<{value: string, label: string}>} options
 * @param {string} value - current selected value
 * @param {function} onChange - callback with selected value
 * @param {string} placeholder - placeholder when nothing selected
 * @param {string} searchPlaceholder - placeholder for search input
 * @param {boolean} searchable - whether to show the search input
 */
export function ComboSelect({ options, value, onChange, placeholder, searchPlaceholder, searchable = true }) {
	var [open, setOpen] = useState(false);
	var [query, setQuery] = useState("");
	var [kbIndex, setKbIndex] = useState(-1);
	var ref = useRef(null);
	var searchRef = useRef(null);
	var dropdownRef = useRef(null);

	var selected = options.find((o) => o.value === value);
	var label = selected ? selected.label : placeholder || "(none)";

	var filtered = options.filter((o) => {
		if (!(searchable && query)) return true;
		var q = query.toLowerCase();
		return o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q);
	});

	useEffect(() => {
		if (!open) return;
		function onClick(e) {
			if (ref.current && !ref.current.contains(e.target)) setOpen(false);
		}
		document.addEventListener("mousedown", onClick);
		return () => document.removeEventListener("mousedown", onClick);
	}, [open]);

	useEffect(() => {
		if (!open) return;
		if (searchable && searchRef.current) searchRef.current.focus();
		else if (!searchable && dropdownRef.current) dropdownRef.current.focus();
	}, [open, searchable]);

	useEffect(() => {
		setKbIndex(-1);
	}, [query]);

	function onKeyDown(e) {
		if (e.key === "Escape") {
			setOpen(false);
		} else if (e.key === "ArrowDown") {
			e.preventDefault();
			setKbIndex((i) => Math.min(i + 1, filtered.length - 1));
		} else if (e.key === "ArrowUp") {
			e.preventDefault();
			setKbIndex((i) => Math.max(i - 1, 0));
		} else if (e.key === "Enter") {
			e.preventDefault();
			var idx = kbIndex >= 0 ? kbIndex : 0;
			if (filtered[idx]) pick(filtered[idx]);
		}
	}

	function pick(o) {
		onChange(o ? o.value : "");
		setOpen(false);
		setQuery("");
	}

	return html`<div class="model-combo" ref=${ref} style="width:100%;">
    <button type="button" class="model-combo-btn" style="width:100%;" onClick=${() => setOpen(!open)}>
      <span class="model-item-label">${label}</span>
      <span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>
    </button>
    ${
			open &&
			html`<div class="model-dropdown" ref=${dropdownRef} tabIndex="-1" style="width:100%;" onKeyDown=${onKeyDown}>
      ${
				searchable &&
				html`<input class="model-search-input" ref=${searchRef} placeholder=${searchPlaceholder || "Search\u2026"}
        value=${query} onInput=${(e) => setQuery(e.target.value)} />`
			}
      <div class="model-dropdown-list">
        <div class="model-dropdown-item ${value ? "" : "selected"}"
          onClick=${() => pick(null)}>
          <span class="model-item-label">${placeholder || "(none)"}</span>
        </div>
        ${filtered.map(
					(o, i) => html`<div key=${o.value}
            class="model-dropdown-item ${o.value === value ? "selected" : ""} ${i === kbIndex ? "kb-active" : ""}"
            onClick=${() => pick(o)}>
            <span class="model-item-label">${o.label}</span>
          </div>`,
				)}
        ${filtered.length === 0 && html`<div class="model-dropdown-empty">No matches</div>`}
      </div>
    </div>`
		}
  </div>`;
}
