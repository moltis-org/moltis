// ── Shared Preact UI components ───────────────────────────────

import type { Signal } from "@preact/signals";
import { signal } from "@preact/signals";
import type { ComponentChildren, VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { CommandPalette } from "./components/CommandPalette";
import { t } from "./i18n";

// ── Toast notifications ──────────────────────────────────────

interface Toast {
	id: number;
	message: string;
	type: string;
}

export const toasts: Signal<Toast[]> = signal([]);
let toastId = 0;

export function showToast(message: string, type: string = "info"): void {
	const id = ++toastId;
	toasts.value = toasts.value.concat([{ id: id, message: message, type: type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((toast) => toast.id !== id);
	}, 4000);
}

export function Toasts(): VNode {
	return (
		<div class="skills-toast-container">
			{toasts.value.map((toast) => {
				const bg = toast.type === "error" ? "var(--error, #e55)" : "var(--accent)";
				return (
					<div
						key={toast.id}
						style={{
							pointerEvents: "auto",
							maxWidth: "420px",
							padding: "10px 16px",
							borderRadius: "6px",
							fontSize: ".8rem",
							fontWeight: 500,
							color: "#fff",
							background: bg,
							boxShadow: "0 4px 12px rgba(0,0,0,.15)",
						}}
					>
						{toast.message}
					</div>
				);
			})}
		</div>
	);
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

	return (
		<div
			class="modal-overlay"
			onClick={onBackdrop}
			style="display:flex;position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:100;align-items:center;justify-content:center;"
		>
			<div
				class="modal-box"
				style="background:var(--surface);border-radius:var(--radius);padding:20px;max-width:500px;width:90%;max-height:85vh;overflow-y:auto;box-shadow:0 8px 32px rgba(0,0,0,.25);border:1px solid var(--border);"
			>
				<div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:14px;">
					<h3 style="margin:0;font-size:.95rem;font-weight:600;color:var(--text-strong)">{title}</h3>
					<button
						onClick={onClose}
						style="background:none;border:none;color:var(--muted);font-size:1.1rem;cursor:pointer;padding:2px 6px"
					>
						{"\u2715"}
					</button>
				</div>
				{props.children}
			</div>
		</div>
	);
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
		if (s) s.resolve(true);
		confirmState.value = null;
	}
	function no(): void {
		if (s) s.resolve(false);
		confirmState.value = null;
	}

	const label = s.opts.confirmLabel || t("common:actions.confirm");
	const danger = s.opts.danger;
	const btnClass = danger ? "provider-btn provider-btn-danger" : "provider-btn";

	return (
		<Modal show={true} onClose={no} title={t("common:actions.confirm")}>
			<p style="font-size:.85rem;color:var(--text);margin:0 0 16px;">{s.message}</p>
			<div style="display:flex;gap:8px;justify-content:flex-end;">
				<button onClick={no} class="provider-btn provider-btn-secondary">
					{t("common:actions.cancel")}
				</button>
				<button onClick={yes} class={btnClass}>
					{label}
				</button>
			</div>
		</Modal>
	);
}

// ── Confirm dialog (signal-driven) ──────────────────────────

interface VanillaConfirmState {
	message: string;
	resolve: (value: boolean) => void;
}

const vanillaConfirmState: Signal<VanillaConfirmState | null> = signal(null);

/**
 * Signal-driven confirm dialog rendered via Preact Modal.
 * Returns a Promise<boolean> — true if confirmed, false if cancelled.
 */
export function confirmDialog(message: string): Promise<boolean> {
	return new Promise((resolve) => {
		vanillaConfirmState.value = { message, resolve };
	});
}

export function VanillaConfirmDialog(): VNode | null {
	const s = vanillaConfirmState.value;
	if (!s) return null;
	function close(v: boolean): void {
		s?.resolve(v);
		vanillaConfirmState.value = null;
	}
	return (
		<div
			class="provider-modal-backdrop"
			onClick={(e: Event) => {
				if (e.target === e.currentTarget) close(false);
			}}
		>
			<div class="provider-modal" style="width:360px">
				<div class="provider-modal-body" style="gap:16px">
					<p style="font-size:.85rem;color:var(--text);margin:0">{s.message}</p>
					<div style="display:flex;gap:8px;justify-content:flex-end">
						<button type="button" onClick={() => close(false)} class="provider-btn provider-btn-secondary">
							{t("common:actions.cancel")}
						</button>
						<button type="button" onClick={() => close(true)} class="provider-btn provider-btn-danger">
							{t("common:actions.delete")}
						</button>
					</div>
				</div>
			</div>
		</div>
	);
}

// ── Share visibility dialog (signal-driven) ─────────────────

interface ShareVisibilityState {
	resolve: (value: string | null) => void;
}

const shareVisibilityState: Signal<ShareVisibilityState | null> = signal(null);

/**
 * Signal-driven share visibility picker rendered via Preact Modal.
 * Returns "public", "private", or null when cancelled.
 */
export function shareVisibilityDialog(): Promise<string | null> {
	return new Promise((resolve) => {
		shareVisibilityState.value = { resolve };
	});
}

export function ShareVisibilityDialog(): VNode | null {
	const s = shareVisibilityState.value;
	if (!s) return null;
	function close(value: string | null): void {
		s?.resolve(value);
		shareVisibilityState.value = null;
	}
	return (
		<div
			class="provider-modal-backdrop"
			onClick={(e: Event) => {
				if (e.target === e.currentTarget) close(null);
			}}
		>
			<div class="provider-modal" style="width:460px">
				<div class="provider-modal-header">
					<div class="provider-item-name">{t("chat:share.title")}</div>
					<button type="button" class="provider-btn provider-btn-secondary provider-btn-sm" onClick={() => close(null)}>
						{t("common:actions.cancel")}
					</button>
				</div>
				<div class="provider-modal-body" style="gap:10px">
					<p style="font-size:.8rem;color:var(--muted);margin:0">{t("chat:share.hint")}</p>
					<p style="font-size:.8rem;color:var(--text);margin:0;padding:8px 10px;border:1px solid color-mix(in srgb,var(--warn) 55%,var(--border) 45%);background:color-mix(in srgb,var(--warn) 12%,var(--surface2) 88%);border-radius:var(--radius-sm);line-height:1.45">
						{t("chat:share.redactionWarning")}
					</p>
					<button type="button" class="provider-item" data-share-visibility="public" onClick={() => close("public")}>
						<div class="provider-item-name">{t("chat:share.publicLink")}</div>
						<span class="provider-item-badge configured">{t("chat:share.publicBadge")}</span>
					</button>
					<button type="button" class="provider-item" data-share-visibility="private" onClick={() => close("private")}>
						<div class="provider-item-name">{t("chat:share.privateLink")}</div>
						<span class="provider-item-badge api-key">{t("chat:share.privateBadge")}</span>
					</button>
				</div>
			</div>
		</div>
	);
}

// ── Share link dialog (signal-driven) ───────────────────────

interface ShareLinkState {
	url: string;
	visibility: string;
	resolve: (value: string | null) => void;
}

const shareLinkState: Signal<ShareLinkState | null> = signal(null);

/**
 * Signal-driven share-link dialog rendered via Preact Modal.
 * Returns "copied" when copy succeeded, otherwise null on close/dismiss.
 */
export function shareLinkDialog(url: string, visibility: string): Promise<string | null> {
	return new Promise((resolve) => {
		shareLinkState.value = { url, visibility, resolve };
	});
}

export function ShareLinkDialog(): VNode | null {
	const s = shareLinkState.value;
	const inputRef = useRef<HTMLInputElement>(null);
	if (!s) return null;

	function close(value: string | null): void {
		s?.resolve(value);
		shareLinkState.value = null;
	}

	async function copyLink(): Promise<void> {
		try {
			if (navigator.clipboard?.writeText) {
				await navigator.clipboard.writeText(s?.url ?? "");
				showToast(t("chat:share.linkCopied"), "success");
				close("copied");
				return;
			}
		} catch (_err) {
			// Clipboard permissions can fail. Fall through to manual copy fallback.
		}
		const el = inputRef.current;
		if (el) {
			el.focus();
			el.select();
		}
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

	const hintText = s.visibility === "private" ? t("chat:share.privateHint") : t("chat:share.publicHint");

	return (
		<div
			class="provider-modal-backdrop"
			data-share-link-modal="true"
			onClick={(e: Event) => {
				if (e.target === e.currentTarget) close(null);
			}}
		>
			<div class="provider-modal" style="width:560px">
				<div class="provider-modal-header">
					<div class="provider-item-name">{t("chat:share.linkReady")}</div>
					<button
						type="button"
						class="provider-btn provider-btn-secondary"
						data-share-link-close="true"
						onClick={() => close(null)}
					>
						{t("common:actions.close")}
					</button>
				</div>
				<div class="provider-modal-body" style="gap:10px">
					<p style="font-size:.8rem;color:var(--muted);margin:0">{hintText}</p>
					<input
						ref={inputRef}
						class="provider-key-input"
						readOnly
						value={s.url}
						data-share-link-input="true"
						onFocus={() => inputRef.current?.select()}
						onClick={() => inputRef.current?.select()}
					/>
					<div style="display:flex;gap:8px;justify-content:flex-end;flex-wrap:wrap">
						<button
							type="button"
							class="provider-btn provider-btn-secondary"
							data-share-link-open="true"
							onClick={() => window.open(s?.url, "_blank", "noopener,noreferrer")}
						>
							{t("common:actions.openLink")}
						</button>
						<button type="button" class="provider-btn" data-share-link-copy="true" onClick={() => void copyLink()}>
							{t("common:actions.copyLink")}
						</button>
					</div>
				</div>
			</div>
		</div>
	);
}

// ── Global dialogs container ────────────────────────────────

/**
 * Renders all signal-driven dialogs. Mount once at the app root level.
 */
export function GlobalDialogs(): VNode {
	return (
		<>
			<VanillaConfirmDialog />
			<ShareVisibilityDialog />
			<ShareLinkDialog />
			<CommandPalette />
		</>
	);
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

	return (
		<div class="model-combo" ref={ref} style="width:100%;">
			<button type="button" class="model-combo-btn" style="width:100%;" onClick={() => setOpen(!open)}>
				<span class="model-item-label">{label}</span>
				<span class="icon icon-sm icon-chevron-down model-combo-chevron" />
			</button>
			{open && (
				<div class="model-dropdown" style="width:100%;" onKeyDown={onKeyDown}>
					<input
						class="model-search-input"
						ref={searchRef}
						placeholder={"Search models\u2026"}
						value={query}
						onInput={(e: Event) => setQuery((e.target as HTMLInputElement).value)}
					/>
					<div class="model-dropdown-list" ref={listRef}>
						<div class={`model-dropdown-item ${value ? "" : "selected"}`} onClick={() => pick(null)}>
							<span class="model-item-label">{placeholder || "(none)"}</span>
						</div>
						{filtered.map((m, i) => (
							<div
								key={m.id}
								class={`model-dropdown-item ${m.id === value ? "selected" : ""} ${i === kbIndex ? "kb-active" : ""}`}
								onClick={() => pick(m)}
							>
								<span class="model-item-label">{m.displayName || m.id}</span>
								{m.provider && <span class="model-item-provider">{m.provider}</span>}
							</div>
						))}
						{filtered.length === 0 && <div class="model-dropdown-empty">{t("common:labels.noMatches")}</div>}
					</div>
				</div>
			)}
		</div>
	);
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

	return (
		<div class="model-combo" ref={ref} style={fillStyle}>
			<button
				type="button"
				class="model-combo-btn"
				style={fillStyle}
				onClick={() => {
					if (!disabled) setOpen(!open);
				}}
				disabled={disabled}
			>
				<span class="model-item-label">{label}</span>
				<span class="icon icon-sm icon-chevron-down model-combo-chevron" />
			</button>
			{open && (
				<div
					class={`model-dropdown ${alignRight ? "align-right" : ""}`}
					ref={dropdownRef}
					tabIndex={-1}
					style={dropdownStyle}
					onKeyDown={onKeyDown}
				>
					{searchable && (
						<input
							class="model-search-input"
							ref={searchRef}
							placeholder={searchPlaceholder || "Search\u2026"}
							value={query}
							onInput={(e: Event) => setQuery((e.target as HTMLInputElement).value)}
						/>
					)}
					<div class="model-dropdown-list">
						{allowEmpty && (
							<div class={`model-dropdown-item ${value ? "" : "selected"}`} onClick={() => pick(null)}>
								<span class="model-item-label">{placeholder || "(none)"}</span>
							</div>
						)}
						{filtered.map((o, i) => (
							<div
								key={o.value}
								class={`model-dropdown-item ${o.value === value ? "selected" : ""} ${i === kbIndex ? "kb-active" : ""}`}
								onClick={() => pick(o)}
							>
								<span class="model-item-label">{o.label}</span>
							</div>
						))}
						{filtered.length === 0 && <div class="model-dropdown-empty">{t("common:labels.noMatches")}</div>}
					</div>
				</div>
			)}
		</div>
	);
}
