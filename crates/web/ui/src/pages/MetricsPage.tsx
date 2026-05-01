// ── Monitoring page ────────────────────────────────────────────────
// Displays metrics in a dashboard format with time-series charts
// showing historical usage patterns. Uses WebSocket for live updates.

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import prettyBytes from "pretty-bytes";
import uPlot from "uplot";
import { TabBar } from "../components/forms";
import { onEvent } from "../events";
import { t } from "../i18n";
import { registerPrefix } from "../router";
import { routes } from "../routes";
import { copyToClipboard } from "../ui";

// ── Types ────────────────────────────────────────────────────────

interface MetricsCategories {
	llm?: {
		completions_total?: number;
		input_tokens?: number;
		output_tokens?: number;
		cache_read_tokens?: number;
		cache_write_tokens?: number;
		errors?: number;
		by_provider?: Record<string, ProviderStats>;
	};
	http?: { total?: number };
	tools?: { total?: number; errors?: number; active?: number };
	mcp?: { total?: number; errors?: number; active?: number };
	system?: { uptime_seconds?: number; connected_clients?: number; active_sessions?: number };
}

interface ProviderStats {
	completions?: number;
	input_tokens?: number;
	output_tokens?: number;
	errors?: number;
}

interface MetricsSnapshot {
	categories?: MetricsCategories;
}

interface HistoryPoint {
	timestamp: number;
	llm_input_tokens?: number;
	llm_output_tokens?: number;
	http_requests?: number;
	llm_completions?: number;
	ws_active?: number;
	active_sessions?: number;
	tool_executions?: number;
	mcp_calls?: number;
	process_memory_bytes?: number;
	local_llama_cpp_bytes?: number;
	by_provider?: Record<string, { input_tokens?: number; output_tokens?: number }>;
	[key: string]: unknown;
}

interface ChartSeries {
	label: string;
	color?: string;
	fill?: boolean;
}

interface TimeRange {
	label: () => string;
	seconds: number;
	maxPoints: number;
}

interface InitMonitoringOptions {
	pathBase?: string;
	syncPath?: boolean;
}

// ── Signals ──────────────────────────────────────────────────────

const metricsData = signal<MetricsSnapshot | null>(null);
const historyPoints = signal<HistoryPoint[]>([]);
const loading = signal(true);
const error = signal<string | null>(null);
const isLive = signal(false);
let unsubscribe: (() => void) | null = null;
let _monitoringContainer: HTMLElement | null = null;
let monitoringPathBase = routes.monitoring;
let monitoringSyncPath = true;

// Time range options (in seconds)
const TIME_RANGES: Record<string, TimeRange> = {
	"5m": { label: () => t("metrics:timeRange.fiveMin"), seconds: 5 * 60, maxPoints: 30 },
	"1h": { label: () => t("metrics:timeRange.oneHour"), seconds: 60 * 60, maxPoints: 360 },
	"24h": { label: () => t("metrics:timeRange.twentyFourHours"), seconds: 24 * 60 * 60, maxPoints: 1440 },
	"7d": { label: () => t("metrics:timeRange.sevenDays"), seconds: 7 * 24 * 60 * 60, maxPoints: 2016 },
};

async function fetchMetrics(): Promise<void> {
	try {
		const resp = await fetch("/api/metrics");
		if (!resp.ok) {
			if (resp.status === 503) {
				error.value = t("metrics:metricsDisabled");
				loading.value = false;
			}
			// For transient errors (401, 5xx, etc.) stay in loading state --
			// the WebSocket subscription will deliver data once connected.
			return;
		}
		const data = await resp.json();
		metricsData.value = data;
		error.value = null;
		loading.value = false;
	} catch (_e) {
		// Network or parse errors are transient -- stay in loading state
		// and let the WebSocket subscription deliver data.
	}
}

async function fetchHistory(): Promise<void> {
	try {
		const resp = await fetch("/api/metrics/history");
		if (resp.ok) {
			const data = await resp.json();
			if (data.points) {
				historyPoints.value = data.points;
			}
		}
	} catch (e) {
		console.warn("Failed to fetch metrics history:", e);
	}
}

function subscribeToMetrics(): void {
	// Subscribe to live metrics updates via WebSocket
	unsubscribe = onEvent("metrics.update", (payload: unknown) => {
		const p = payload as { snapshot?: MetricsSnapshot; point?: HistoryPoint };
		isLive.value = true;
		if (p.snapshot) {
			metricsData.value = p.snapshot;
		}
		if (p.point) {
			// Add new point to history, keeping max points based on longest time range
			const maxPoints = TIME_RANGES["7d"].maxPoints;
			let points = [...historyPoints.value, p.point];
			if (points.length > maxPoints) {
				points = points.slice(points.length - maxPoints);
			}
			historyPoints.value = points;
		}
		loading.value = false;
		error.value = null;
	});
}

function formatNumber(n: number | undefined | null): string {
	if (n === undefined || n === null) return "\u2014";
	if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
	if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
	return n.toString();
}

function formatMemoryBytes(bytes: number | undefined | null): string {
	if (bytes === undefined || bytes === null || bytes <= 0) return "\u2014";
	return prettyBytes(bytes, { maximumFractionDigits: 0, space: false });
}

function formatUptime(seconds: number | undefined | null): string {
	if (!seconds) return "\u2014";
	const days = Math.floor(seconds / 86400);
	const hours = Math.floor((seconds % 86400) / 3600);
	const mins = Math.floor((seconds % 3600) / 60);
	if (days > 0) return `${days}d ${hours}h`;
	if (hours > 0) return `${hours}h ${mins}m`;
	return `${mins}m`;
}

// Empty state component with icon
function EmptyState({ icon, title, description }: { icon: VNode; title: string; description: string }): VNode {
	return (
		<div className="flex flex-col items-center justify-center py-20 px-8 bg-[var(--surface)] border border-[var(--border)] rounded-lg">
			<div className="w-20 h-20 mb-6 text-[var(--muted)] opacity-40">{icon}</div>
			<h3 className="text-lg font-medium text-[var(--text)] mb-3">{title}</h3>
			<p className="text-sm text-[var(--muted)] text-center max-w-md">{description}</p>
		</div>
	);
}

// Chart icon for empty state
const chartIcon = <span className="icon icon-chart-bar w-full h-full" />;

// Activity icon for empty metrics
const activityIcon = <span className="icon icon-activity w-full h-full" />;

// Live indicator with green dot
function LiveIndicator({ live }: { live: boolean }): VNode {
	if (!live) {
		return (
			<div className="flex items-center gap-2 text-xs text-[var(--muted)]">
				<span className="inline-flex rounded-full h-2.5 w-2.5 bg-gray-500" />
				{t("common:status.connecting")}
			</div>
		);
	}
	return (
		<div className="flex items-center gap-2 text-xs text-green-500">
			<span className="relative flex h-2.5 w-2.5">
				<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75" />
				<span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-green-500" />
			</span>
			{t("metrics:live")}
		</div>
	);
}

function MetricCard({
	title,
	value,
	subtitle,
	trend,
}: {
	title: string;
	value: string;
	subtitle?: string;
	trend?: number;
}): VNode {
	return (
		<div className="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-6">
			<div className="text-xs text-[var(--muted)] uppercase tracking-wide mb-2">{title}</div>
			<div className="flex items-baseline gap-2">
				<div className="text-2xl font-semibold">{value}</div>
				{trend !== undefined && (
					<span className={`text-xs ${trend >= 0 ? "text-green-500" : "text-red-500"}`}>
						{trend >= 0 ? "+" : ""}
						{trend}%
					</span>
				)}
			</div>
			{subtitle && <div className="text-xs text-[var(--muted)] mt-2">{subtitle}</div>}
		</div>
	);
}

// Chart color palette (CSS variables with fallbacks)
const chartColors: Record<string, string> = {
	primary: "#22c55e", // green
	secondary: "#3b82f6", // blue
	tertiary: "#f59e0b", // amber
	error: "#ef4444", // red
	muted: "#6b7280", // gray
};

// Get CSS variable or fallback
function getCssVar(name: string, fallback: string): string {
	if (typeof document === "undefined") return fallback;
	const style = getComputedStyle(document.documentElement);
	return style.getPropertyValue(name).trim() || fallback;
}

function TimeSeriesChart({
	title,
	data,
	series,
	height = 220,
}: {
	title: string;
	data: (number | null)[][];
	series: ChartSeries[];
	height?: number;
}): VNode {
	const containerRef = useRef<HTMLDivElement>(null);
	const chartRef = useRef<uPlot | null>(null);

	useEffect(() => {
		if (!(containerRef.current && data) || data.length === 0 || !data[0] || data[0].length === 0) return;

		// uPlot options
		const opts: uPlot.Options = {
			width: containerRef.current.offsetWidth,
			height: height,
			padding: [12, 12, 0, 0],
			cursor: {
				show: true,
				drag: { x: false, y: false },
			},
			legend: {
				show: true,
				live: true,
			},
			scales: {
				x: { time: true },
			},
			axes: [
				{
					stroke: getCssVar("--muted", "#6b7280"),
					grid: { stroke: getCssVar("--border", "#333"), width: 1 },
					ticks: { stroke: getCssVar("--border", "#333"), width: 1 },
					font: "11px system-ui",
				},
				{
					stroke: getCssVar("--muted", "#6b7280"),
					grid: { stroke: getCssVar("--border", "#333"), width: 1 },
					ticks: { stroke: getCssVar("--border", "#333"), width: 1 },
					font: "11px system-ui",
					size: 50,
				},
			],
			series: [
				{ label: t("metrics:series.time") },
				...series.map((s, i) => ({
					label: s.label,
					stroke: s.color || Object.values(chartColors)[i % Object.values(chartColors).length],
					width: 2,
					fill: s.fill ? `${s.color}20` : undefined,
				})),
			],
		};

		// Destroy previous chart
		if (chartRef.current) {
			chartRef.current.destroy();
		}

		// Create new chart
		chartRef.current = new uPlot(opts, data as uPlot.AlignedData, containerRef.current);

		// Handle resize
		const resizeObserver = new ResizeObserver(() => {
			if (chartRef.current && containerRef.current) {
				chartRef.current.setSize({
					width: containerRef.current.offsetWidth,
					height: height,
				});
			}
		});
		resizeObserver.observe(containerRef.current);

		return () => {
			resizeObserver.disconnect();
			if (chartRef.current) {
				chartRef.current.destroy();
				chartRef.current = null;
			}
		};
	}, [data, series, height]);

	// Update data without destroying chart
	useEffect(() => {
		if (chartRef.current && data && data.length > 0 && data[0] && data[0].length > 0) {
			chartRef.current.setData(data as uPlot.AlignedData);
		}
	}, [data]);

	return (
		<div className="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-6">
			<h3 className="text-sm font-medium mb-4">{title}</h3>
			<div ref={containerRef} className="w-full" />
		</div>
	);
}

function filterPointsByTimeRange(points: HistoryPoint[], rangeKey: string): HistoryPoint[] {
	if (!points || points.length === 0) return [];

	const range = TIME_RANGES[rangeKey];
	const now = Date.now();
	const cutoff = now - range.seconds * 1000;

	return points.filter((p) => p.timestamp >= cutoff);
}

function prepareChartData(points: HistoryPoint[], fields: string[]): (number | null)[][] | null {
	if (!points || points.length === 0) {
		return null;
	}

	// uPlot expects data as array of arrays: [[timestamps], [series1], [series2], ...]
	const timestamps = points.map((p) => p.timestamp / 1000); // Convert to seconds
	const seriesData = fields.map((field) => points.map((p) => (p[field] as number) ?? 0));

	return [timestamps, ...seriesData];
}

function prepareMemoryChart(points: HistoryPoint[]): { data: (number | null)[][]; series: ChartSeries[] } | null {
	if (!points || points.length === 0) {
		return null;
	}

	const mib = 1024 * 1024;
	const timestamps = points.map((p) => p.timestamp / 1000);
	const processMemory = points.map((p) => (p.process_memory_bytes || 0) / mib);
	const hasLocalLlama = points.some((p) => ((p.local_llama_cpp_bytes as number) || 0) > 0);

	const data: (number | null)[][] = [timestamps, processMemory];
	const series: ChartSeries[] = [{ label: t("metrics:series.processMemory"), color: chartColors.error }];

	if (hasLocalLlama) {
		data.push(points.map((p) => ((p.local_llama_cpp_bytes as number) || 0) / mib));
		series.push({ label: t("metrics:series.localLlamaCpp"), color: chartColors.primary });
	}

	return { data, series };
}

// Get unique provider names from history points
function getProviders(points: HistoryPoint[]): string[] {
	const providers = new Set<string>();
	for (const p of points) {
		if (p.by_provider) {
			for (const name of Object.keys(p.by_provider)) {
				providers.add(name);
			}
		}
	}
	return Array.from(providers).sort();
}

// Prepare per-provider chart data for a specific metric (input_tokens, output_tokens, etc.)
function prepareProviderChartData(
	points: HistoryPoint[],
	providers: string[],
	metric: string,
): (number | null)[][] | null {
	if (!points || points.length === 0 || providers.length === 0) {
		return null;
	}

	const timestamps = points.map((p) => p.timestamp / 1000);
	const seriesData = providers.map((provider) =>
		points.map((p) => {
			const providerData = p.by_provider?.[provider];
			return (providerData as Record<string, number> | undefined)?.[metric] ?? 0;
		}),
	);

	return [timestamps, ...seriesData];
}

// Provider color palette (distinct colors for different providers)
const providerColors = [
	"#10b981", // emerald (primary)
	"#8b5cf6", // violet
	"#f59e0b", // amber
	"#ef4444", // red
	"#3b82f6", // blue
	"#ec4899", // pink
	"#14b8a6", // teal
	"#f97316", // orange
];

function MetricsGrid({
	categories,
	latestPoint,
}: {
	categories?: MetricsCategories;
	latestPoint?: HistoryPoint;
}): VNode | null {
	if (!categories) return null;

	const { llm, http, tools, mcp, system } = categories;
	const processMemory = latestPoint?.process_memory_bytes || 0;

	// Check if there's any meaningful data
	const hasData =
		(system?.uptime_seconds ?? 0) > 0 ||
		(http?.total ?? 0) > 0 ||
		(llm?.completions_total ?? 0) > 0 ||
		(tools?.total ?? 0) > 0 ||
		processMemory > 0;

	if (!hasData) {
		return (
			<EmptyState
				icon={activityIcon}
				title={t("metrics:noActivityTitle")}
				description={t("metrics:noActivityDescription")}
			/>
		);
	}

	return (
		<div className="space-y-10">
			{/* System Overview */}
			<section>
				<h3 className="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">
					{t("metrics:sections.system")}
				</h3>
				<div className="grid grid-cols-2 md:grid-cols-4 gap-6">
					<MetricCard title={t("metrics:cards.uptime")} value={formatUptime(system?.uptime_seconds)} />
					<MetricCard title={t("metrics:cards.connectedClients")} value={formatNumber(system?.connected_clients)} />
					<MetricCard title={t("metrics:cards.activeSessions")} value={formatNumber(system?.active_sessions)} />
					<MetricCard title={t("metrics:cards.httpRequests")} value={formatNumber(http?.total)} />
					<MetricCard title={t("metrics:cards.processMemory")} value={formatMemoryBytes(processMemory)} />
				</div>
			</section>

			{/* LLM Metrics */}
			<section>
				<h3 className="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">
					{t("metrics:sections.llmUsage")}
				</h3>
				<div className="grid grid-cols-2 md:grid-cols-4 gap-6">
					<MetricCard
						title={t("metrics:cards.completions")}
						value={formatNumber(llm?.completions_total)}
						subtitle={(llm?.errors ?? 0) > 0 ? t("metrics:errorsCount", { count: llm?.errors }) : undefined}
					/>
					<MetricCard title={t("metrics:cards.inputTokens")} value={formatNumber(llm?.input_tokens)} />
					<MetricCard title={t("metrics:cards.outputTokens")} value={formatNumber(llm?.output_tokens)} />
					<MetricCard
						title={t("metrics:cards.cacheTokens")}
						value={formatNumber((llm?.cache_read_tokens || 0) + (llm?.cache_write_tokens || 0))}
						subtitle={
							llm?.cache_read_tokens
								? t("metrics:cacheRead", { value: formatNumber(llm.cache_read_tokens) })
								: undefined
						}
					/>
				</div>
			</section>

			{/* Tools & MCP */}
			<section>
				<h3 className="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">
					{t("metrics:sections.toolsMcp")}
				</h3>
				<div className="grid grid-cols-2 md:grid-cols-4 gap-6">
					<MetricCard
						title={t("metrics:cards.toolExecutions")}
						value={formatNumber(tools?.total)}
						subtitle={(tools?.errors ?? 0) > 0 ? t("metrics:errorsCount", { count: tools?.errors }) : undefined}
					/>
					<MetricCard title={t("metrics:cards.toolsActive")} value={formatNumber(tools?.active)} />
					<MetricCard
						title={t("metrics:cards.mcpToolCalls")}
						value={formatNumber(mcp?.total)}
						subtitle={(mcp?.errors ?? 0) > 0 ? t("metrics:errorsCount", { count: mcp?.errors }) : undefined}
					/>
					<MetricCard title={t("metrics:cards.mcpServers")} value={formatNumber(mcp?.active)} />
				</div>
			</section>
		</div>
	);
}

function ChartsSection({
	points,
	timeRange,
	onTimeRangeChange,
}: {
	points: HistoryPoint[];
	timeRange: string;
	onTimeRangeChange: (key: string) => void;
}): VNode {
	const filteredPoints = filterPointsByTimeRange(points, timeRange);

	if (!filteredPoints || filteredPoints.length < 2) {
		return (
			<div className="space-y-8">
				<TimeRangeSelector value={timeRange} onChange={onTimeRangeChange} />
				<EmptyState
					icon={chartIcon}
					title={t("metrics:collectingTitle")}
					description={t("metrics:collectingDescription")}
				/>
			</div>
		);
	}

	// Prepare chart data
	const tokenData = prepareChartData(filteredPoints, ["llm_input_tokens", "llm_output_tokens"]);
	const requestData = prepareChartData(filteredPoints, ["http_requests", "llm_completions"]);
	const connectionsData = prepareChartData(filteredPoints, ["ws_active", "active_sessions"]);
	const toolsData = prepareChartData(filteredPoints, ["tool_executions", "mcp_calls"]);
	const memoryChart = prepareMemoryChart(filteredPoints);

	// Prepare per-provider charts
	const providers = getProviders(filteredPoints);
	const providerInputData = prepareProviderChartData(filteredPoints, providers, "input_tokens");
	const providerOutputData = prepareProviderChartData(filteredPoints, providers, "output_tokens");
	const providerSeries = providers.map((name, i) => ({
		label: name,
		color: providerColors[i % providerColors.length],
	}));

	return (
		<div className="space-y-8">
			<TimeRangeSelector value={timeRange} onChange={onTimeRangeChange} />
			<div className="grid grid-cols-1 xl:grid-cols-2 gap-8">
				{tokenData && (
					<TimeSeriesChart
						title={t("metrics:charts.tokenUsageTotal")}
						data={tokenData}
						series={[
							{ label: t("metrics:series.inputTokens"), color: chartColors.primary },
							{ label: t("metrics:series.outputTokens"), color: chartColors.secondary },
						]}
					/>
				)}
				{providerInputData && providers.length > 0 && (
					<TimeSeriesChart
						title={t("metrics:charts.inputTokensByProvider")}
						data={providerInputData}
						series={providerSeries}
					/>
				)}
				{providerOutputData && providers.length > 0 && (
					<TimeSeriesChart
						title={t("metrics:charts.outputTokensByProvider")}
						data={providerOutputData}
						series={providerSeries}
					/>
				)}
				{requestData && (
					<TimeSeriesChart
						title={t("metrics:charts.requests")}
						data={requestData}
						series={[
							{ label: t("metrics:series.httpRequests"), color: chartColors.tertiary },
							{ label: t("metrics:series.llmCompletions"), color: chartColors.primary },
						]}
					/>
				)}
				{connectionsData && (
					<TimeSeriesChart
						title={t("metrics:charts.connections")}
						data={connectionsData}
						series={[
							{ label: t("metrics:series.wsActive"), color: chartColors.secondary },
							{ label: t("metrics:series.activeSessions"), color: chartColors.tertiary },
						]}
					/>
				)}
				{memoryChart && (
					<TimeSeriesChart
						title={t("metrics:charts.memoryUsage")}
						data={memoryChart.data}
						series={memoryChart.series}
					/>
				)}
				{toolsData && (
					<TimeSeriesChart
						title={t("metrics:charts.toolActivity")}
						data={toolsData}
						series={[
							{ label: t("metrics:series.toolExecutions"), color: chartColors.primary },
							{ label: t("metrics:series.mcpCalls"), color: chartColors.secondary },
						]}
					/>
				)}
			</div>
		</div>
	);
}

function TimeRangeSelector({ value, onChange }: { value: string; onChange: (key: string) => void }): VNode {
	return (
		<div className="flex items-center gap-1 bg-[var(--surface)] border border-[var(--border)] rounded-md p-1">
			{Object.entries(TIME_RANGES).map(([key, range]) => (
				<button
					key={key}
					className={`px-3 py-1.5 text-xs rounded transition-colors ${value === key ? "bg-[var(--surface2)] text-[var(--text)] font-medium" : "text-[var(--muted)] hover:text-[var(--text)] hover:bg-[var(--surface2)]"}`}
					onClick={() => onChange(key)}
				>
					{range.label()}
				</button>
			))}
		</div>
	);
}

function ProviderTable({ byProvider }: { byProvider?: Record<string, ProviderStats> }): VNode | null {
	if (!byProvider || Object.keys(byProvider).length === 0) return null;

	return (
		<section>
			<h3 className="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">
				{t("metrics:sections.byProvider")}
			</h3>
			<div className="bg-[var(--surface)] border border-[var(--border)] rounded-lg overflow-hidden">
				<table className="w-full text-sm">
					<thead>
						<tr className="border-b border-[var(--border)] bg-[var(--surface2)]">
							<th className="text-left px-6 py-4 font-medium">{t("metrics:table.provider")}</th>
							<th className="text-right px-6 py-4 font-medium">{t("metrics:table.completions")}</th>
							<th className="text-right px-6 py-4 font-medium">{t("metrics:table.inputTokens")}</th>
							<th className="text-right px-6 py-4 font-medium">{t("metrics:table.outputTokens")}</th>
							<th className="text-right px-6 py-4 font-medium">{t("metrics:table.errors")}</th>
						</tr>
					</thead>
					<tbody>
						{Object.entries(byProvider).map(([name, stats]) => (
							<tr key={name} className="border-b border-[var(--border)] last:border-0">
								<td className="px-6 py-4">{name}</td>
								<td className="text-right px-6 py-4">{formatNumber(stats.completions)}</td>
								<td className="text-right px-6 py-4">{formatNumber(stats.input_tokens)}</td>
								<td className="text-right px-6 py-4">{formatNumber(stats.output_tokens)}</td>
								<td className={`text-right px-6 py-4 ${(stats.errors ?? 0) > 0 ? "text-[var(--error)]" : ""}`}>
									{formatNumber(stats.errors)}
								</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
		</section>
	);
}

function PrometheusEndpoint(): VNode {
	const [copied, setCopied] = useState(false);
	const endpoint = `${window.location.origin}/metrics`;

	function copyEndpoint(): void {
		copyToClipboard(endpoint, "", "").then((ok) => {
			if (!ok) return;
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		});
	}

	return (
		<section>
			<h3 className="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">
				{t("metrics:sections.prometheus")}
			</h3>
			<div className="p-6 bg-[var(--surface)] border border-[var(--border)] rounded-lg">
				<p className="text-sm text-[var(--muted)] mb-5">{t("metrics:prometheusDescription")}</p>
				<div className="flex items-center gap-4">
					<code className="flex-1 px-4 py-3 bg-[var(--surface2)] rounded-md text-sm font-mono overflow-x-auto">
						{endpoint}
					</code>
					<button className="provider-btn provider-btn-secondary text-sm shrink-0" onClick={copyEndpoint}>
						{copied ? t("common:actions.copied") : t("common:actions.copy")}
					</button>
				</div>
			</div>
		</section>
	);
}

// ── Insights tab ────────────────────────────────────────────────

interface InsightsData {
	days: number;
	completions: number;
	input_tokens: number;
	output_tokens: number;
	total_tokens: number;
	errors: number;
	tool_executions: number;
	tool_errors: number;
	by_provider: Record<string, { input_tokens: number; output_tokens: number; completions: number }>;
	data_points: number;
	span_hours: number;
}

const INSIGHTS_RANGES = [
	{ label: "7 days", days: 7 },
	{ label: "30 days", days: 30 },
	{ label: "90 days", days: 90 },
];

function InsightsTab(): VNode {
	const [data, setData] = useState<InsightsData | null>(null);
	const [insightsDays, setInsightsDays] = useState(30);
	const [insightsLoading, setInsightsLoading] = useState(true);

	useEffect(() => {
		setInsightsLoading(true);
		fetch(`/api/metrics/insights?days=${insightsDays}`)
			.then((resp) => {
				if (resp.ok) return resp.json();
				throw new Error(`HTTP ${resp.status}`);
			})
			.then((d: InsightsData) => {
				setData(d);
				setInsightsLoading(false);
			})
			.catch(() => {
				setData(null);
				setInsightsLoading(false);
			});
	}, [insightsDays]);

	if (insightsLoading) {
		return <div className="flex items-center justify-center h-32 text-[var(--muted)]">Loading insights...</div>;
	}

	if (!data || data.data_points === 0) {
		return (
			<div className="text-center text-[var(--muted)] py-16">
				<p className="text-lg mb-2">No usage data yet</p>
				<p className="text-sm">Metrics are collected while the gateway is running.</p>
			</div>
		);
	}

	const providers = Object.entries(data.by_provider).sort(
		(a, b) => b[1].input_tokens + b[1].output_tokens - (a[1].input_tokens + a[1].output_tokens),
	);

	const completionsPerHour = data.span_hours > 0 ? (data.completions / data.span_hours).toFixed(1) : "—";

	return (
		<div className="space-y-8">
			{/* Time range selector */}
			<div className="flex gap-2">
				{INSIGHTS_RANGES.map((r) => (
					<button
						key={r.days}
						type="button"
						className={`px-3 py-1.5 rounded text-sm transition-colors ${
							insightsDays === r.days
								? "bg-[var(--accent)] text-white"
								: "bg-[var(--bg-secondary)] text-[var(--fg)] hover:bg-[var(--bg-hover)]"
						}`}
						onClick={() => setInsightsDays(r.days)}
					>
						{r.label}
					</button>
				))}
			</div>

			{/* Summary cards */}
			<div className="grid grid-cols-2 md:grid-cols-4 gap-4">
				<InsightCard label="LLM Completions" value={formatNumber(data.completions)} />
				<InsightCard label="Total Tokens" value={formatNumber(data.total_tokens)} />
				<InsightCard label="Input Tokens" value={formatNumber(data.input_tokens)} />
				<InsightCard label="Output Tokens" value={formatNumber(data.output_tokens)} />
				<InsightCard label="Completions/hour" value={completionsPerHour} />
				<InsightCard label="LLM Errors" value={formatNumber(data.errors)} alert={data.errors > 0} />
				<InsightCard label="Tool Executions" value={formatNumber(data.tool_executions)} />
				<InsightCard label="Tool Errors" value={formatNumber(data.tool_errors)} alert={data.tool_errors > 0} />
			</div>

			{/* Provider breakdown */}
			{providers.length > 0 && (
				<div>
					<h3 className="text-sm font-semibold text-[var(--muted)] uppercase tracking-wider mb-4">Usage by Provider</h3>
					<div className="border border-[var(--border)] rounded-lg overflow-hidden">
						<table className="w-full text-sm">
							<thead>
								<tr className="bg-[var(--bg-secondary)] text-[var(--muted)]">
									<th className="text-left px-4 py-2 font-medium">Provider</th>
									<th className="text-right px-4 py-2 font-medium">Completions</th>
									<th className="text-right px-4 py-2 font-medium">Input Tokens</th>
									<th className="text-right px-4 py-2 font-medium">Output Tokens</th>
									<th className="text-right px-4 py-2 font-medium">Total</th>
								</tr>
							</thead>
							<tbody>
								{providers.map(([name, stats]) => (
									<tr key={name} className="border-t border-[var(--border)]">
										<td className="px-4 py-2 font-medium">{name}</td>
										<td className="text-right px-4 py-2 tabular-nums">{formatNumber(stats.completions)}</td>
										<td className="text-right px-4 py-2 tabular-nums">{formatNumber(stats.input_tokens)}</td>
										<td className="text-right px-4 py-2 tabular-nums">{formatNumber(stats.output_tokens)}</td>
										<td className="text-right px-4 py-2 tabular-nums font-medium">
											{formatNumber(stats.input_tokens + stats.output_tokens)}
										</td>
									</tr>
								))}
							</tbody>
						</table>
					</div>
				</div>
			)}

			{/* Footer */}
			<p className="text-xs text-[var(--muted)]">
				Based on {formatNumber(data.data_points)} data points over {data.span_hours.toFixed(1)} hours.
			</p>
		</div>
	);
}

function InsightCard({ label, value, alert }: { label: string; value: string; alert?: boolean }): VNode {
	return (
		<div
			className={`p-4 rounded-lg border ${
				alert ? "border-[var(--error)] bg-[var(--error-bg)]" : "border-[var(--border)] bg-[var(--bg-secondary)]"
			}`}
		>
			<div className="text-xs text-[var(--muted)] mb-1">{label}</div>
			<div className={`text-xl font-semibold tabular-nums ${alert ? "text-[var(--error)]" : ""}`}>{value}</div>
		</div>
	);
}

function MonitoringPage({ initialTab }: { initialTab: string }): VNode {
	const [activeTab, setActiveTab] = useState(initialTab || "overview");
	const [timeRange, setTimeRange] = useState("1h");

	// Update URL when tab changes
	function handleTabChange(tab: string): void {
		setActiveTab(tab);
		if (monitoringSyncPath) {
			const newPath = tab === "overview" ? monitoringPathBase : `${monitoringPathBase}/${tab}`;
			if (window.location.pathname !== newPath) {
				history.pushState(null, "", newPath);
			}
		}
	}

	useEffect(() => {
		// Fetch initial data
		fetchMetrics();
		fetchHistory();

		// Subscribe to live updates
		subscribeToMetrics();

		return () => {
			if (unsubscribe) {
				unsubscribe();
				unsubscribe = null;
			}
		};
	}, []);

	if (loading.value) {
		return (
			<div className="flex items-center justify-center h-64 text-[var(--muted)]">
				<div className="text-center">
					<div className="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin mb-4" />
					<p>{t("metrics:loadingMetrics")}</p>
				</div>
			</div>
		);
	}

	if (error.value) {
		return (
			<div className="p-10">
				<div className="max-w-3xl mx-auto space-y-10">
					<div className="p-6 bg-[var(--error-bg)] border border-[var(--error)] rounded-lg text-[var(--error)]">
						{error.value}
					</div>
					<PrometheusEndpoint />
				</div>
			</div>
		);
	}

	return (
		<div className="p-10 overflow-y-auto">
			<div className="max-w-7xl mx-auto">
				<div className="flex items-center justify-between mb-10">
					<div className="flex items-center gap-4">
						<h2 className="text-xl font-semibold">{t("metrics:title")}</h2>
						<LiveIndicator live={isLive.value} />
					</div>
					<TabBar
						tabs={[
							{ id: "overview", label: t("metrics:tabs.overview") },
							{ id: "charts", label: t("metrics:tabs.charts") },
							{ id: "insights", label: "Insights" },
						]}
						active={activeTab}
						onChange={handleTabChange}
					/>
				</div>

				{activeTab === "overview" && (
					<div className="space-y-10">
						<MetricsGrid
							categories={metricsData.value?.categories}
							latestPoint={historyPoints.value[historyPoints.value.length - 1]}
						/>
						<ProviderTable byProvider={metricsData.value?.categories?.llm?.by_provider} />
						<PrometheusEndpoint />
					</div>
				)}

				{activeTab === "charts" && (
					<ChartsSection points={historyPoints.value} timeRange={timeRange} onTimeRangeChange={setTimeRange} />
				)}

				{activeTab === "insights" && <InsightsTab />}
			</div>
		</div>
	);
}

export function initMonitoring(container: HTMLElement, param?: string | null, options?: InitMonitoringOptions): void {
	// param is "charts" for /monitoring/charts, null for /monitoring
	_monitoringContainer = container;
	monitoringPathBase = options?.pathBase || routes.monitoring;
	monitoringSyncPath = options?.syncPath !== false;
	const initialTab = param === "charts" ? "charts" : param === "insights" ? "insights" : "overview";
	render(<MonitoringPage initialTab={initialTab} />, container);
}

export function teardownMonitoring(): void {
	if (unsubscribe) {
		unsubscribe();
		unsubscribe = null;
	}
	metricsData.value = null;
	historyPoints.value = [];
	loading.value = true;
	error.value = null;
	isLive.value = false;
	monitoringPathBase = routes.monitoring;
	monitoringSyncPath = true;
	if (_monitoringContainer) render(null, _monitoringContainer);
	_monitoringContainer = null;
}

// Register as prefix route: /monitoring and /monitoring/charts
registerPrefix(routes.monitoring!, initMonitoring, teardownMonitoring);
