// ── Reusable settings section layout components ─────────────
//
// DRY wrappers for the heading + content + save/status pattern
// that repeats across every settings section.

import type { VNode } from "preact";
import { useCallback, useState } from "preact/hooks";

// ── Section heading ─────────────────────────────────────────

interface SectionHeadingProps {
	title: string;
	subtitle?: string;
	children?: VNode | VNode[];
}

export function SectionHeading({ title, subtitle, children }: SectionHeadingProps): VNode {
	return (
		<div className="flex items-center justify-between mb-4">
			<div>
				<h2 className="text-lg font-medium text-[var(--text-strong)]">{title}</h2>
				{subtitle && <p className="text-xs text-[var(--muted)] mt-0.5">{subtitle}</p>}
			</div>
			{children}
		</div>
	);
}

// ── Sub-heading ─────────────────────────────────────────────

interface SubHeadingProps {
	title: string;
	className?: string;
}

export function SubHeading({ title, className }: SubHeadingProps): VNode {
	return (
		<h3 className={className ?? "text-sm font-medium text-[var(--text-strong)]"} style={{ marginBottom: "8px" }}>
			{title}
		</h3>
	);
}

// ── Save button with flash feedback ─────────────────────────

interface SaveButtonProps {
	saving: boolean;
	saved?: boolean;
	disabled?: boolean;
	label?: string;
	savingLabel?: string;
	savedLabel?: string;
	onClick?: (e: Event) => void;
	type?: "button" | "submit" | "reset";
	danger?: boolean;
	className?: string;
}

export function SaveButton({
	saving,
	saved,
	disabled,
	label = "Save",
	savingLabel = "Saving…",
	savedLabel = "Saved ✓",
	onClick,
	type = "button",
	danger,
	className,
}: SaveButtonProps): VNode {
	const btnClass = [danger ? "provider-btn provider-btn-danger" : "provider-btn", className ?? ""]
		.filter(Boolean)
		.join(" ");

	return (
		<button type={type} className={btnClass} disabled={saving || disabled} onClick={onClick}>
			{saving ? savingLabel : saved ? savedLabel : label}
		</button>
	);
}

// ── useSaveState hook ───────────────────────────────────────
//
// Encapsulates the saving/saved/error state machine that every
// settings section duplicates.

interface SaveState {
	saving: boolean;
	saved: boolean;
	error: string | null;
	setSaving: (v: boolean) => void;
	flashSaved: () => void;
	setError: (msg: string | null) => void;
	reset: () => void;
}

export function useSaveState(flashDurationMs = 2000): SaveState {
	const [saving, setSaving] = useState(false);
	const [saved, setSaved] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const flashSaved = useCallback(() => {
		setSaved(true);
		setTimeout(() => setSaved(false), flashDurationMs);
	}, [flashDurationMs]);

	const reset = useCallback(() => {
		setSaving(false);
		setSaved(false);
		setError(null);
	}, []);

	return { saving, saved, error, setSaving, flashSaved, setError, reset };
}

// ── Status message ──────────────────────────────────────────

interface StatusMessageProps {
	error?: string | null;
	success?: string | null;
	className?: string;
}

export function StatusMessage({ error, success, className }: StatusMessageProps): VNode | null {
	if (!(error || success)) return null;
	const color = error ? "var(--error)" : "var(--accent)";
	const text = error ?? success;
	return (
		<div className={className ?? "text-xs mt-2"} style={{ color }}>
			{text}
		</div>
	);
}

// ── Settings card / panel ───────────────────────────────────

interface SettingsCardProps {
	children: VNode | VNode[];
	className?: string;
}

export function SettingsCard({ children, className }: SettingsCardProps): VNode {
	return (
		<div className={className ?? "bg-[var(--surface)] border border-[var(--border)] rounded-lg p-4 mb-4"}>
			{children}
		</div>
	);
}
