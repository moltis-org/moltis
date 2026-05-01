// ── Reusable list item components ────────────────────────────
//
// DRY wrappers for the provider-item / CRUD row pattern that
// repeats across settings sections: a bordered row with name,
// badges, metadata, and action buttons.

import type { VNode } from "preact";

// ── List item row ───────────────────────────────────────────

interface ListItemProps {
	/** Primary display name */
	name: string | VNode;
	/** Optional secondary metadata line */
	meta?: string | VNode;
	/** Badge elements shown after the name */
	badges?: VNode[];
	/** Action buttons on the right side */
	actions?: VNode | VNode[];
	/** Extra className on the container */
	className?: string;
	/** Unique key (pass-through) */
	children?: VNode | VNode[];
}

export function ListItem({ name, meta, badges, actions, className, children }: ListItemProps): VNode {
	return (
		<div className={`provider-item ${className ?? ""}`.trim()} style={{ marginBottom: 0 }}>
			<div style={{ flex: 1, minWidth: 0 }}>
				<div className="provider-item-name" style={{ fontSize: ".85rem" }}>
					{name}
					{badges?.map((badge, i) => (
						<span key={i} style={{ marginLeft: "6px" }}>
							{badge}
						</span>
					))}
				</div>
				{meta && <div style={{ fontSize: ".7rem", color: "var(--muted)", marginTop: "2px" }}>{meta}</div>}
				{children}
			</div>
			{actions && <div style={{ display: "flex", gap: "4px" }}>{actions}</div>}
		</div>
	);
}

// ── Badge ───────────────────────────────────────────────────

interface BadgeProps {
	label: string;
	variant?: "configured" | "muted" | "warning" | "running" | "error";
}

export function Badge({ label, variant = "muted" }: BadgeProps): VNode {
	return <span className={`provider-item-badge ${variant}`}>{label}</span>;
}

// ── Empty state ─────────────────────────────────────────────

interface EmptyStateProps {
	message: string;
	className?: string;
}

export function EmptyState({ message, className }: EmptyStateProps): VNode {
	return (
		<div className={className ?? "text-xs text-[var(--muted)]"} style={{ padding: "12px 0" }}>
			{message}
		</div>
	);
}

// ── Loading indicator ───────────────────────────────────────

interface LoadingProps {
	message?: string;
	className?: string;
}

export function Loading({ message = "Loading\u2026", className }: LoadingProps): VNode {
	return <div className={className ?? "text-xs text-[var(--muted)]"}>{message}</div>;
}

// ── Copy button ─────────────────────────────────────────────

import { useCallback, useState } from "preact/hooks";
import { copyToClipboard } from "../../ui";

interface CopyButtonProps {
	/** Text to copy to clipboard */
	text: string;
	/** Button label (default: "Copy") */
	label?: string;
	/** Label shown after copying (default: "Copied!") */
	copiedLabel?: string;
	/** Extra className */
	className?: string;
	/** Button size variant */
	small?: boolean;
}

export function CopyButton({
	text,
	label = "Copy",
	copiedLabel = "Copied!",
	className,
	small,
}: CopyButtonProps): VNode {
	const [copied, setCopied] = useState(false);

	const onClick = useCallback(() => {
		copyToClipboard(text, "", "").then((ok) => {
			if (!ok) return;
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		});
	}, [text]);

	const btnCls = [small ? "provider-btn provider-btn-sm" : "provider-btn", className ?? ""].filter(Boolean).join(" ");

	return (
		<button type="button" className={btnCls} onClick={onClick}>
			{copied ? copiedLabel : label}
		</button>
	);
}

// ── Danger zone ─────────────────────────────────────────────

interface DangerZoneProps {
	title?: string;
	children: VNode | VNode[];
}

export function DangerZone({ title = "Danger Zone", children }: DangerZoneProps): VNode {
	return (
		<div
			style={{
				maxWidth: "600px",
				marginTop: "24px",
				borderTop: "1px solid var(--border)",
				paddingTop: "16px",
			}}
		>
			<h3 className="text-sm font-medium" style={{ color: "var(--error)", marginBottom: "8px" }}>
				{title}
			</h3>
			{children}
		</div>
	);
}

// ── Code display ────────────────────────────────────────────

interface CodeDisplayProps {
	children: string;
	className?: string;
}

export function CodeDisplay({ children, className }: CodeDisplayProps): VNode {
	return <code className={className ?? "font-mono text-xs block my-1 p-1.5 bg-[var(--bg)] rounded"}>{children}</code>;
}
