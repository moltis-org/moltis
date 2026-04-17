// ── Shared Preact UI components ───────────────────────────────

import type { Signal } from "@preact/signals";
import { signal } from "@preact/signals";
import { html } from "htm/preact";
import type { ComponentChildren, VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { t } from "./i18n";

// ── Toast notifications ──────────────────────────────────────

interface Toast {
	id: number;
	message: string;
	type: string;
}

export const toasts: Signal<Toast[]> = signal([]);
let toastId = 0;

export function showToast(message: string, type: string): void {
	const id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((toast) => toast.id !== id);
	}, 4000);
}

export function Toasts(): VNode {
	return html`<div class="skills-toast-container">
    ${toasts.value.map((toast) => {
			const bg = toast.type === "error" ? "var(--error, #e55)" : "var(--accent)";
			return html`<div key=${toast.id} style=${{
				pointerEvents: "auto",
				maxWidth: "420px",
				padding: "10px 16px",
				borderRadius: "6px",
				fontSize: ".8rem",
				fontWeight: 500,
				color: "#fff",
				background: bg,
				boxShadow: "0 4px 12px rgba(0,0,0,.15)",
			}}>${toast.message}</div>`;
		})}
  </div>`;
}

// ── Modal wrapper ────────────────────────────────────────────

interface ModalProps {
	show: boolean;
	onClose?: () => void;
	title?: string;
	children?: ComponentChildren;
}

export function Modal(props: ModalProps): VNode | null {
	const show = props.show;
	const onClose = props.onClose;
	const title = props.title;

	function onBackdrop(e: Event): void {
		if (e.target === e.currentTarget && onClose) onClose();
	}

	useEffect(() => {
		if (!show) return;
		function onKey(e: KeyboardEvent): void {
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

interface ConfirmState {
	message: string;
	resolve: (value: boolean) => void;
	opts: { confirmLabel?: string; danger?: boolean };
}

const confirmState: Signal<ConfirmState | null> = signal(null);

export function requestConfirm(message: string, opts?: { confirmLabel?: string; danger?: boolean }): Promise<boolean> {
	return new Promise((resolve) => {
		confirmState.value = { message: message, resolve: resolve, opts: opts || {} };
	});
}

export function ConfirmDialog(): VNode | null {
	const s = confirmState.value;
	if (!s) return null;

	function yes(): void {
		s.resolve(true);
		confirmState.value = null;
	}
	function no(): void {
		s.resolve(false);
		confirmState.value = null;
	}

	const label = s.opts.confirmLabel || t("common:actions.confirm");
	const danger = s.opts.danger;
	const btnClass = danger ? "provider-btn provider-btn-danger" : "provider-btn";

	return html`<${Modal} show=${true} onClose=${no} title=${t("common:actions.confirm")}>
    <p style="font-size:.85rem;color:var(--text);margin:0 0 16px;">${s.message}</p>
    <div style="display:flex;gap:8px;justify-content:flex-end;">
      <button onClick=${no} class="provider-btn provider-btn-secondary">${t("common:actions.cancel")}</button>
      <button onClick=${yes} class=${btnClass}>${label}</button>
    </div>
  </${Modal}>`;
}

/**
 * Vanilla-JS confirm dialog (no Preact needed).
 * Returns a Promise<boolean> -- true if confirmed, false if cancelled.
 * Safe: all content set via textContent, no user input in markup.
 */
export function confirmDialog(message: string): Promise<boolean> {
	return new Promise((resolve) => {
		const backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";

		const box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "360px";

		const body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "16px";

		const msg = document.createElement("p");
		msg.style.cssText = "font-size:.85rem;color:var(--text);margin:0";
		msg.textContent = message;

		const btnRow = document.createElement("div");
		btnRow.style.cssText = "display:flex;gap:8px;justify-content:flex-end";

		const cancelBtn = document.createElement("button");
		cancelBtn.className = "provider-btn provider-btn-secondary";
		cancelBtn.textContent = t("common:actions.cancel");

		const deleteBtn = document.createElement("button");
		deleteBtn.className = "provider-btn provider-btn-danger";
		deleteBtn.textContent = t("common:actions.delete");

		function close(val: boolean): void {
			backdrop.remove();
			resolve(val);
		}
		cancelBtn.addEventListener("click", () => close(false));
		deleteBtn.addEventListener("click", () => close(true));
		backdrop.addEventListener("click", (e: Event) => {
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
export function shareVisibilityDialog(): Promise<string | null> {
	return new Promise((resolve) => {
		const backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";

		const box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "460px";

		const header = document.createElement("div");
		header.className = "provider-modal-header";

		const title = document.createElement("div");
		title.className = "provider-item-name";
		title.textContent = t("chat:share.title");

		const cancelTopBtn = document.createElement("button");
		cancelTopBtn.className = "provider-btn provider-btn-secondary provider-btn-sm";
		cancelTopBtn.textContent = t("common:actions.cancel");

		const body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "10px";

		const hint = document.createElement("p");
		hint.style.cssText = "font-size:.8rem;color:var(--muted);margin:0";
		hint.textContent = t("chat:share.hint");

		const warning = document.createElement("p");
		warning.style.cssText =
			"font-size:.8rem;color:var(--text);margin:0;padding:8px 10px;border:1px solid color-mix(in srgb,var(--warn) 55%,var(--border) 45%);background:color-mix(in srgb,var(--warn) 12%,var(--surface2) 88%);border-radius:var(--radius-sm);line-height:1.45";
		warning.textContent = t("chat:share.redactionWarning");

		const publicBtn = document.createElement("button");
		publicBtn.className = "provider-item";
		publicBtn.type = "button";
		publicBtn.setAttribute("data-share-visibility", "public");
		const publicName = document.createElement("div");
		publicName.className = "provider-item-name";
		publicName.textContent = t("chat:share.publicLink");
		const publicBadge = document.createElement("span");
		publicBadge.className = "provider-item-badge configured";
		publicBadge.textContent = t("chat:share.publicBadge");
		publicBtn.appendChild(publicName);
		publicBtn.appendChild(publicBadge);

		const privateBtn = document.createElement("button");
		privateBtn.className = "provider-item";
		privateBtn.type = "button";
		privateBtn.setAttribute("data-share-visibility", "private");
		const privateName = document.createElement("div");
		privateName.className = "provider-item-name";
		privateName.textContent = t("chat:share.privateLink");
		const privateBadge = document.createElement("span");
		privateBadge.className = "provider-item-badge api-key";
		privateBadge.textContent = t("chat:share.privateBadge");
		privateBtn.appendChild(privateName);
		privateBtn.appendChild(privateBadge);

		function close(value: string | null): void {
			document.removeEventListener("keydown", onKeydown);
			backdrop.remove();
			resolve(value);
		}

		function onKeydown(e: KeyboardEvent): void {
			if (e.key === "Escape") close(null);
		}

		publicBtn.addEventListener("click", () => close("public"));
		privateBtn.addEventListener("click", () => close("private"));
		cancelTopBtn.addEventListener("click", () => close(null));
		backdrop.addEventListener("click", (e: Event) => {
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
export function shareLinkDialog(url: string, visibility: string): Promise<string | null> {
	return new Promise((resolve) => {
		const backdrop = document.createElement("div");
		backdrop.className = "provider-modal-backdrop";
		backdrop.setAttribute("data-share-link-modal", "true");

		const box = document.createElement("div");
		box.className = "provider-modal";
		box.style.width = "560px";

		const header = document.createElement("div");
		header.className = "provider-modal-header";

		const title = document.createElement("div");
		title.className = "provider-item-name";
		title.textContent = t("chat:share.linkReady");

		const closeTopBtn = document.createElement("button");
		closeTopBtn.className = "provider-btn provider-btn-secondary";
		closeTopBtn.textContent = t("common:actions.close");
		closeTopBtn.setAttribute("data-share-link-close", "true");

		const body = document.createElement("div");
		body.className = "provider-modal-body";
		body.style.gap = "10px";

		const hint = document.createElement("p");
		hint.style.cssText = "font-size:.8rem;color:var(--muted);margin:0";
		hint.textContent = visibility === "private" ? t("chat:share.privateHint") : t("chat:share.publicHint");

		const input = document.createElement("input");
		input.className = "provider-key-input";
		input.readOnly = true;
		input.value = url;
		input.setAttribute("data-share-link-input", "true");
		input.addEventListener("focus", () => input.select());
		input.addEventListener("click", () => input.select());

		const btnRow = document.createElement("div");
		btnRow.style.cssText = "display:flex;gap:8px;justify-content:flex-end;flex-wrap:wrap";

		const openBtn = document.createElement("button");
		openBtn.className = "provider-btn provider-btn-secondary";
		openBtn.textContent = t("common:actions.openLink");
		openBtn.setAttribute("data-share-link-open", "true");

		const copyBtn = document.createElement("button");
		copyBtn.className = "provider-btn";
		copyBtn.textContent = t("common:actions.copyLink");
		copyBtn.setAttribute("data-share-link-copy", "true");

		function close(value: string | null): void {
			document.removeEventListener("keydown", onKeydown);
			backdrop.remove();
			resolve(value);
		}

		function onKeydown(e: KeyboardEvent): void {
			if (e.key === "Escape") close(null);
		}

		async function copyLink(): Promise<void> {
			try {
				if (navigator.clipboard?.writeText) {
					await navigator.clipboard.writeText(url);
					showToast(t("chat:share.linkCopied"), "success");
					close("copied");
					return;
				}
			} catch (_err) {
				// Clipboard permissions can fail. Fall through to manual copy fallback.
			}
			input.focus();
			input.select();
			let copied = false;
			try {
				copied = document.execCommand("copy");
			} catch (_err) {
				copied = false;
			}
			if (copied) {
				showToast(t("chat:share.linkCopied"), "success");
				close("copied");
				return;
			}
			showToast(t("errors:copyFailed"), "error");
		}

		copyBtn.addEventListener("click", () => {
			void copyLink();
		});
		openBtn.addEventListener("click", () => {
			window.open(url, "_blank", "noopener,noreferrer");
		});
		closeTopBtn.addEventListener("click", () => close(null));
		backdrop.addEventListener("click", (e: Event) => {
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

interface ModelSelectModel {
	id: string;
	displayName?: string;
	provider?: string;
}

interface ModelSelectProps {
	models: ModelSelectModel[];
	value: string;
	onChange: (id: string) => void;
	placeholder?: string;
}

export function ModelSelect({ models, value, onChange, placeholder }: ModelSelectProps): VNode {
	const [open, setOpen] = useState(false);
	const [query, setQuery] = useState("");
	const [kbIndex, setKbIndex] = useState(-1);
	const ref = useRef<HTMLDivElement>(null);
	const searchRef = useRef<HTMLInputElement>(null);
	const listRef = useRef<HTMLDivElement>(null);

	const selected = models.find((m) => m.id === value);
	const label = selected ? selected.displayName || selected.id : placeholder || "(none)";

	const filtered = models.filter((m) => {
		if (!query) return true;
		const q = query.toLowerCase();
		return (
			(m.displayName || "").toLowerCase().includes(q) ||
			m.id.toLowerCase().includes(q) ||
			(m.provider || "").toLowerCase().includes(q)
		);
	});

	useEffect(() => {
		if (!open) return;
		function onClick(e: MouseEvent): void {
			if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
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

	function onKeyDown(e: KeyboardEvent): void {
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
			const idx = kbIndex >= 0 ? kbIndex : 0;
			if (filtered[idx]) pick(filtered[idx]);
		}
	}

	function pick(m: ModelSelectModel | null): void {
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
        value=${query} onInput=${(e: Event) => setQuery((e.target as HTMLInputElement).value)} />
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
        ${filtered.length === 0 && html`<div class="model-dropdown-empty">${t("common:labels.noMatches")}</div>`}
      </div>
    </div>`
		}
  </div>`;
}

/**
 * Generic combo select for simple value/label options.
 */

interface ComboOption {
	value: string;
	label: string;
}

interface ComboSelectProps {
	options: ComboOption[];
	value: string;
	onChange: (value: string) => void;
	placeholder?: string;
	searchPlaceholder?: string;
	searchable?: boolean;
	fullWidth?: boolean;
	allowEmpty?: boolean;
	disabled?: boolean;
}

export function ComboSelect({
	options,
	value,
	onChange,
	placeholder,
	searchPlaceholder,
	searchable = true,
	fullWidth = true,
	allowEmpty = true,
	disabled = false,
}: ComboSelectProps): VNode {
	const [open, setOpen] = useState(false);
	const [query, setQuery] = useState("");
	const [kbIndex, setKbIndex] = useState(-1);
	const [alignRight, setAlignRight] = useState(false);
	const ref = useRef<HTMLDivElement>(null);
	const searchRef = useRef<HTMLInputElement>(null);
	const dropdownRef = useRef<HTMLDivElement>(null);
	const fillStyle = fullWidth ? "width:100%;" : undefined;
	const dropdownStyle = fullWidth
		? "width:100%;"
		: searchable
			? undefined
			: "min-width:100%;width:max-content;max-width:min(360px,calc(100vw - 16px));";

	const selected = options.find((o) => o.value === value);
	const label = selected ? selected.label : placeholder || "(none)";

	const filtered = options.filter((o) => {
		if (!(searchable && query)) return true;
		const q = query.toLowerCase();
		return o.label.toLowerCase().includes(q) || o.value.toLowerCase().includes(q);
	});

	useEffect(() => {
		if (!open) return;
		function onClick(e: MouseEvent): void {
			if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
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
		if (!open) return;
		function updateAlignment(): void {
			if (!ref.current) return;
			const comboRect = ref.current.getBoundingClientRect();
			const dropdownWidth = dropdownRef.current?.offsetWidth || (fullWidth ? comboRect.width : 280);
			const viewportWidth = window.innerWidth || document.documentElement.clientWidth || 0;
			const rightEdge = comboRect.left + dropdownWidth;
			const shouldAlignRight = rightEdge > viewportWidth - 8 && comboRect.right - dropdownWidth >= 8;
			setAlignRight(shouldAlignRight);
		}
		requestAnimationFrame(updateAlignment);
		window.addEventListener("resize", updateAlignment);
		return () => window.removeEventListener("resize", updateAlignment);
	}, [open, fullWidth]);

	useEffect(() => {
		setKbIndex(-1);
	}, [query]);

	useEffect(() => {
		if (disabled) setOpen(false);
	}, [disabled]);

	function onKeyDown(e: KeyboardEvent): void {
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
			const idx = kbIndex >= 0 ? kbIndex : 0;
			if (filtered[idx]) pick(filtered[idx]);
		}
	}

	function pick(o: ComboOption | null): void {
		onChange(o ? o.value : "");
		setOpen(false);
		setQuery("");
	}

	return html`<div class="model-combo" ref=${ref} style=${fillStyle}>
    <button
      type="button"
      class="model-combo-btn"
      style=${fillStyle}
      onClick=${() => {
				if (!disabled) setOpen(!open);
			}}
      disabled=${disabled}
    >
      <span class="model-item-label">${label}</span>
      <span class="icon icon-sm icon-chevron-down model-combo-chevron"></span>
    </button>
    ${
			open &&
			html`<div
      class="model-dropdown ${alignRight ? "align-right" : ""}"
      ref=${dropdownRef}
      tabIndex="-1"
      style=${dropdownStyle}
      onKeyDown=${onKeyDown}
    >
      ${
				searchable &&
				html`<input class="model-search-input" ref=${searchRef} placeholder=${searchPlaceholder || "Search\u2026"}
        value=${query} onInput=${(e: Event) => setQuery((e.target as HTMLInputElement).value)} />`
			}
      <div class="model-dropdown-list">
        ${
					allowEmpty &&
					html`<div class="model-dropdown-item ${value ? "" : "selected"}"
          onClick=${() => pick(null)}>
          <span class="model-item-label">${placeholder || "(none)"}</span>
        </div>`
				}
        ${filtered.map(
					(o, i) => html`<div key=${o.value}
            class="model-dropdown-item ${o.value === value ? "selected" : ""} ${i === kbIndex ? "kb-active" : ""}"
            onClick=${() => pick(o)}>
            <span class="model-item-label">${o.label}</span>
          </div>`,
				)}
        ${filtered.length === 0 && html`<div class="model-dropdown-empty">${t("common:labels.noMatches")}</div>`}
      </div>
    </div>`
		}
  </div>`;
}
