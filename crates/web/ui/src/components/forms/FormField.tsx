// ── Reusable form field components ───────────────────────────
//
// DRY wrappers for the label + input + help text pattern that
// repeats across every settings section and modal.

import type { VNode } from "preact";
import { targetChecked, targetValue } from "../../typed-events";

// ── Text input ──────────────────────────────────────────────

interface TextFieldProps {
	label: string;
	value: string;
	onInput: (value: string) => void;
	id?: string;
	type?: string;
	placeholder?: string;
	help?: string;
	disabled?: boolean;
	required?: boolean;
	className?: string;
	inputClassName?: string;
	monospace?: boolean;
	autoComplete?: string;
	children?: VNode | VNode[];
}

export function TextField({
	label,
	value,
	onInput,
	id,
	type = "text",
	placeholder,
	help,
	disabled,
	required,
	className,
	inputClassName,
	monospace,
	autoComplete,
	children,
}: TextFieldProps): VNode {
	const fieldId = id ?? `field-${label.toLowerCase().replace(/\s+/g, "-")}`;
	const inputCls = [
		"w-full text-sm bg-[var(--bg)] border border-[var(--border)] rounded px-3 py-2 text-[var(--text)] focus:outline-none focus:border-[var(--border-strong)]",
		monospace ? "font-mono text-xs" : "",
		inputClassName ?? "",
	]
		.filter(Boolean)
		.join(" ");

	return (
		<div className={className ?? "mb-3"}>
			<label htmlFor={fieldId} className="block text-xs text-[var(--muted)] mb-1">
				{label}
				{required && <span className="text-[var(--error)] ml-0.5">*</span>}
			</label>
			<input
				id={fieldId}
				type={type}
				value={value}
				onInput={(e) => onInput(targetValue(e))}
				placeholder={placeholder}
				disabled={disabled}
				autoComplete={autoComplete}
				className={inputCls}
			/>
			{help && <div className="text-xs text-[var(--muted)] mt-1">{help}</div>}
			{children}
		</div>
	);
}

// ── Textarea ────────────────────────────────────────────────

interface TextAreaFieldProps {
	label: string;
	value: string;
	onInput: (value: string) => void;
	id?: string;
	placeholder?: string;
	help?: string;
	disabled?: boolean;
	rows?: number;
	className?: string;
	monospace?: boolean;
}

export function TextAreaField({
	label,
	value,
	onInput,
	id,
	placeholder,
	help,
	disabled,
	rows = 3,
	className,
	monospace,
}: TextAreaFieldProps): VNode {
	const fieldId = id ?? `field-${label.toLowerCase().replace(/\s+/g, "-")}`;
	const textCls = [
		"w-full text-sm bg-[var(--bg)] border border-[var(--border)] rounded px-3 py-2 text-[var(--text)] focus:outline-none focus:border-[var(--border-strong)] resize-y",
		monospace ? "font-mono text-xs" : "",
	]
		.filter(Boolean)
		.join(" ");

	return (
		<div className={className ?? "mb-3"}>
			<label htmlFor={fieldId} className="block text-xs text-[var(--muted)] mb-1">
				{label}
			</label>
			<textarea
				id={fieldId}
				value={value}
				onInput={(e) => onInput(targetValue(e))}
				placeholder={placeholder}
				disabled={disabled}
				rows={rows}
				className={textCls}
			/>
			{help && <div className="text-xs text-[var(--muted)] mt-1">{help}</div>}
		</div>
	);
}

// ── Select ──────────────────────────────────────────────────

interface SelectFieldProps {
	label: string;
	value: string;
	onChange: (value: string) => void;
	options: Array<{ value: string; label: string }>;
	id?: string;
	help?: string;
	disabled?: boolean;
	className?: string;
}

export function SelectField({
	label,
	value,
	onChange,
	options,
	id,
	help,
	disabled,
	className,
}: SelectFieldProps): VNode {
	const fieldId = id ?? `field-${label.toLowerCase().replace(/\s+/g, "-")}`;

	return (
		<div className={className ?? "mb-3"}>
			<label htmlFor={fieldId} className="block text-xs text-[var(--muted)] mb-1">
				{label}
			</label>
			<select
				id={fieldId}
				value={value}
				onChange={(e) => onChange(targetValue(e))}
				disabled={disabled}
				className="w-full text-sm bg-[var(--bg)] border border-[var(--border)] rounded px-3 py-2 text-[var(--text)] focus:outline-none focus:border-[var(--border-strong)]"
			>
				{options.map((opt) => (
					<option key={opt.value} value={opt.value}>
						{opt.label}
					</option>
				))}
			</select>
			{help && <div className="text-xs text-[var(--muted)] mt-1">{help}</div>}
		</div>
	);
}

// ── Checkbox ────────────────────────────────────────────────

interface CheckboxFieldProps {
	label: string;
	checked: boolean;
	onChange: (checked: boolean) => void;
	id?: string;
	help?: string;
	disabled?: boolean;
	className?: string;
}

export function CheckboxField({ label, checked, onChange, id, help, disabled, className }: CheckboxFieldProps): VNode {
	const fieldId = id ?? `field-${label.toLowerCase().replace(/\s+/g, "-")}`;

	return (
		<label
			htmlFor={fieldId}
			className={className ?? "flex items-center gap-2 text-sm text-[var(--text)] cursor-pointer mb-2"}
		>
			<input
				id={fieldId}
				type="checkbox"
				checked={checked}
				onChange={(e) => onChange(targetChecked(e))}
				disabled={disabled}
				className="cursor-pointer"
			/>
			<span>{label}</span>
			{help && <span className="text-xs text-[var(--muted)]">({help})</span>}
		</label>
	);
}
