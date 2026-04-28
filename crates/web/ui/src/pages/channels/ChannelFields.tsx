// ── Shared channel form components ────────────────────────────

import type { Signal } from "@preact/signals";
import { useSignal } from "@preact/signals";
import type { VNode } from "preact";

import { models as modelsSig } from "../../stores/model-store";
import { targetValue } from "../../typed-events";
import { ModelSelect } from "../../ui";
import type { ChannelConfig } from "../ChannelsPage";

// ── Advanced config patch field ──────────────────────────────

interface AdvancedConfigPatchFieldProps {
	value: string;
	onInput: (value: string) => void;
	currentConfig?: ChannelConfig | null;
}

export function AdvancedConfigPatchField({
	value,
	onInput,
	currentConfig = null,
}: AdvancedConfigPatchFieldProps): VNode {
	return (
		<details className="channel-card">
			<summary className="cursor-pointer text-xs font-medium text-[var(--text-strong)]">Advanced Config JSON</summary>
			<div className="mt-2 flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]">
					Optional JSON object merged on top of the form before save. Use this for channel-specific settings that do not
					have dedicated fields yet.
				</div>
				{currentConfig && (
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]">Current stored config (read-only)</label>
						<textarea
							className="channel-input min-h-[160px] font-mono text-xs"
							readOnly
							value={prettyConfigJson(currentConfig)}
						/>
					</div>
				)}
				<div className="flex flex-col gap-1">
					<label className="text-xs text-[var(--muted)]">Advanced config JSON patch (optional)</label>
					<textarea
						data-field="advancedConfigPatch"
						className="channel-input min-h-[140px] font-mono text-xs"
						value={value}
						onInput={(e) => {
							onInput(targetValue(e));
						}}
						placeholder='{"reply_to_message": true}'
					/>
				</div>
			</div>
		</details>
	);
}

function prettyConfigJson(value: unknown): string {
	try {
		return JSON.stringify(value || {}, null, 2);
	} catch (_error) {
		return "{}";
	}
}

// ── Tag-style allowlist input ────────────────────────────────

interface AllowlistInputProps {
	value: string[];
	onChange: (items: string[]) => void;
	preserveAt?: boolean;
	placeholder?: string;
}

export function AllowlistInput({ value, onChange, preserveAt, placeholder }: AllowlistInputProps): VNode {
	const input = useSignal("");

	function addTag(raw: string): void {
		const tag = preserveAt ? raw.trim() : raw.trim().replace(/^@/, "");
		if (tag && !value.includes(tag)) onChange([...value, tag]);
		input.value = "";
	}

	function removeTag(tag: string): void {
		onChange(value.filter((t) => t !== tag));
	}

	function onKeyDown(e: KeyboardEvent): void {
		if ((e.key === "Enter" || e.key === ",") && !e.isComposing) {
			e.preventDefault();
			if (input.value.trim()) addTag(input.value);
		} else if (e.key === "Backspace" && !input.value && value.length > 0) {
			onChange(value.slice(0, -1));
		}
	}

	return (
		<div
			className="flex flex-wrap items-center gap-1.5 rounded border border-[var(--border)] bg-[var(--surface2)] px-2 py-1.5"
			style={{ minHeight: "38px", cursor: "text" }}
			onClick={(e) => (e.currentTarget as HTMLElement).querySelector("input")?.focus()}
		>
			{value.map((tag) => (
				<span
					key={tag}
					className="inline-flex items-center gap-1 rounded-full bg-[var(--accent)]/10 px-2 py-0.5 text-xs text-[var(--accent)]"
				>
					{tag}
					<button
						type="button"
						className="inline-flex items-center text-[var(--muted)] hover:text-[var(--accent)]"
						style={{
							lineHeight: 1,
							fontSize: "14px",
							padding: 0,
							background: "none",
							border: "none",
							cursor: "pointer",
						}}
						onClick={(e) => {
							e.stopPropagation();
							removeTag(tag);
						}}
					>
						{"\u00d7"}
					</button>
				</span>
			))}
			<input
				type="text"
				value={input.value}
				onInput={(e) => {
					input.value = targetValue(e);
				}}
				onKeyDown={onKeyDown}
				placeholder={value.length === 0 ? placeholder || "Type a username and press Enter" : ""}
				className="flex-1 bg-transparent text-[var(--text)] text-sm outline-none border-none"
				style={{ minWidth: "80px", padding: "2px 0", fontFamily: "var(--font-body)" }}
			/>
		</div>
	);
}

// ── Shared form fields ───────────────────────────────────────

interface SharedChannelFieldsProps {
	addModel: Signal<string>;
	allowlistItems: Signal<string[]>;
}

export function SharedChannelFields({ addModel, allowlistItems }: SharedChannelFieldsProps): VNode {
	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<>
			<label className="text-xs text-[var(--muted)]">DM Policy</label>
			<select data-field="dmPolicy" className="channel-select">
				<option value="allowlist">Allowlist only</option>
				<option value="open">Open (anyone)</option>
				<option value="disabled">Disabled</option>
			</select>
			<label className="text-xs text-[var(--muted)]">Group Mention Mode</label>
			<select data-field="mentionMode" className="channel-select">
				<option value="mention">Must @mention bot</option>
				<option value="always">Always respond</option>
				<option value="none">Don't respond in groups</option>
			</select>
			<label className="text-xs text-[var(--muted)]">Default Model</label>
			<ModelSelect
				models={modelsSig.value}
				value={addModel.value}
				onChange={(v: string) => {
					addModel.value = v;
				}}
				placeholder={defaultPlaceholder}
			/>
			<label className="text-xs text-[var(--muted)]">DM Allowlist</label>
			<AllowlistInput
				value={allowlistItems.value}
				onChange={(v) => {
					allowlistItems.value = v;
				}}
			/>
		</>
	);
}
