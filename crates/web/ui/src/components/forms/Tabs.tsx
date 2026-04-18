// ── Reusable tab bar component ───────────────────────────────
//
// Horizontal tab bar with active state management. Used across
// Channels, Metrics, Nodes, Terminal, and RunDetail pages.

import type { VNode } from "preact";

interface Tab {
	id: string;
	label: string;
	badge?: string | number;
}

interface TabBarProps {
	tabs: Tab[];
	active: string;
	onChange: (id: string) => void;
	className?: string;
}

export function TabBar({ tabs, active, onChange, className }: TabBarProps): VNode {
	return (
		<div className={className ?? "flex border-b border-[var(--border)] text-xs"} role="tablist">
			{tabs.map((tab) => {
				const isActive = tab.id === active;
				const tabClass = [
					"py-2 px-3 cursor-pointer bg-transparent border-b-2 transition-colors text-sm",
					isActive
						? "border-[var(--accent)] text-[var(--text)] font-medium"
						: "border-transparent text-[var(--muted)] hover:text-[var(--text)]",
				].join(" ");

				return (
					<button
						key={tab.id}
						type="button"
						role="tab"
						aria-selected={isActive}
						className={tabClass}
						onClick={() => onChange(tab.id)}
					>
						{tab.label}
						{tab.badge != null && (
							<span className="ml-1.5 text-xs px-1.5 py-0.5 rounded-full bg-[var(--surface2)] text-[var(--muted)]">
								{tab.badge}
							</span>
						)}
					</button>
				);
			})}
		</div>
	);
}
