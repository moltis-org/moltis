// ── Monitoring page ────────────────────────────────────────────────
// Displays metrics in a dashboard format with time-series charts
// showing historical usage patterns. Uses WebSocket for live updates.

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import uPlot from "uplot";
import { onEvent } from "./events.js";
import { registerPrefix } from "./router.js";
import { routes } from "./routes.js";

var metricsData = signal(null);
var historyPoints = signal([]);
var loading = signal(true);
var error = signal(null);
var isLive = signal(false);
var unsubscribe = null;
var _monitoringContainer = null;
var monitoringPathBase = routes.monitoring;
var monitoringSyncPath = true;

// Time range options (in seconds)
var TIME_RANGES = {
	"5m": { label: "5 min", seconds: 5 * 60, maxPoints: 30 },
	"1h": { label: "1 hour", seconds: 60 * 60, maxPoints: 360 },
	"24h": { label: "24 hours", seconds: 24 * 60 * 60, maxPoints: 1440 },
	"7d": { label: "7 days", seconds: 7 * 24 * 60 * 60, maxPoints: 2016 },
};

async function fetchMetrics() {
	try {
		var resp = await fetch("/api/metrics");
		if (!resp.ok) {
			if (resp.status === 503) {
				error.value = "Metrics are not enabled. Enable them in moltis.toml with [metrics] enabled = true";
				loading.value = false;
			}
			// For transient errors (401, 5xx, etc.) stay in loading state —
			// the WebSocket subscription will deliver data once connected.
			return;
		}
		var data = await resp.json();
		metricsData.value = data;
		error.value = null;
		loading.value = false;
	} catch (_e) {
		// Network or parse errors are transient — stay in loading state
		// and let the WebSocket subscription deliver data.
	}
}

async function fetchHistory() {
	try {
		var resp = await fetch("/api/metrics/history");
		if (resp.ok) {
			var data = await resp.json();
			if (data.points) {
				historyPoints.value = data.points;
			}
		}
	} catch (e) {
		console.warn("Failed to fetch metrics history:", e);
	}
}

function subscribeToMetrics() {
	// Subscribe to live metrics updates via WebSocket
	unsubscribe = onEvent("metrics.update", (payload) => {
		isLive.value = true;
		if (payload.snapshot) {
			metricsData.value = payload.snapshot;
		}
		if (payload.point) {
			// Add new point to history, keeping max points based on longest time range
			var maxPoints = TIME_RANGES["7d"].maxPoints;
			var points = [...historyPoints.value, payload.point];
			if (points.length > maxPoints) {
				points = points.slice(points.length - maxPoints);
			}
			historyPoints.value = points;
		}
		loading.value = false;
		error.value = null;
	});
}

function formatNumber(n) {
	if (n === undefined || n === null) return "—";
	if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
	if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
	return n.toString();
}

function formatUptime(seconds) {
	if (!seconds) return "—";
	var days = Math.floor(seconds / 86400);
	var hours = Math.floor((seconds % 86400) / 3600);
	var mins = Math.floor((seconds % 3600) / 60);
	if (days > 0) return `${days}d ${hours}h`;
	if (hours > 0) return `${hours}h ${mins}m`;
	return `${mins}m`;
}

// Empty state component with icon
function EmptyState({ icon, title, description }) {
	return html`
		<div class="flex flex-col items-center justify-center py-20 px-8 bg-[var(--surface)] border border-[var(--border)] rounded-lg">
			<div class="w-20 h-20 mb-6 text-[var(--muted)] opacity-40">
				${icon}
			</div>
			<h3 class="text-lg font-medium text-[var(--text)] mb-3">${title}</h3>
			<p class="text-sm text-[var(--muted)] text-center max-w-md">${description}</p>
		</div>
	`;
}

// Chart icon for empty state
var chartIcon = html`<span class="icon icon-chart-bar w-full h-full"></span>`;

// Activity icon for empty metrics
var activityIcon = html`<span class="icon icon-activity w-full h-full"></span>`;

// Live indicator with green dot
function LiveIndicator({ live }) {
	if (!live) {
		return html`
			<div class="flex items-center gap-2 text-xs text-[var(--muted)]">
				<span class="inline-flex rounded-full h-2.5 w-2.5 bg-gray-500"></span>
				Connecting...
			</div>
		`;
	}
	return html`
		<div class="flex items-center gap-2 text-xs text-green-500">
			<span class="relative flex h-2.5 w-2.5">
				<span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span>
				<span class="relative inline-flex rounded-full h-2.5 w-2.5 bg-green-500"></span>
			</span>
			Live
		</div>
	`;
}

function MetricCard({ title, value, subtitle, trend }) {
	return html`
		<div class="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-6">
			<div class="text-xs text-[var(--muted)] uppercase tracking-wide mb-2">${title}</div>
			<div class="flex items-baseline gap-2">
				<div class="text-2xl font-semibold">${value}</div>
				${
					trend !== undefined &&
					html`
					<span class="text-xs ${trend >= 0 ? "text-green-500" : "text-red-500"}">
						${trend >= 0 ? "+" : ""}${trend}%
					</span>
				`
				}
			</div>
			${subtitle && html`<div class="text-xs text-[var(--muted)] mt-2">${subtitle}</div>`}
		</div>
	`;
}

// Chart color palette (CSS variables with fallbacks)
var chartColors = {
	primary: "#22c55e", // green
	secondary: "#3b82f6", // blue
	tertiary: "#f59e0b", // amber
	error: "#ef4444", // red
	muted: "#6b7280", // gray
};

// Get CSS variable or fallback
function getCssVar(name, fallback) {
	if (typeof document === "undefined") return fallback;
	var style = getComputedStyle(document.documentElement);
	return style.getPropertyValue(name).trim() || fallback;
}

function TimeSeriesChart({ title, data, series, height = 220 }) {
	var containerRef = useRef(null);
	var chartRef = useRef(null);

	useEffect(() => {
		if (!(containerRef.current && data) || data.length === 0 || !data[0] || data[0].length === 0) return;

		// uPlot options
		var opts = {
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
				{ label: "Time" },
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
		chartRef.current = new uPlot(opts, data, containerRef.current);

		// Handle resize
		var resizeObserver = new ResizeObserver(() => {
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
			chartRef.current.setData(data);
		}
	}, [data]);

	return html`
		<div class="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-6">
			<h3 class="text-sm font-medium mb-4">${title}</h3>
			<div ref=${containerRef} class="w-full"></div>
		</div>
	`;
}

function filterPointsByTimeRange(points, rangeKey) {
	if (!points || points.length === 0) return [];

	var range = TIME_RANGES[rangeKey];
	var now = Date.now();
	var cutoff = now - range.seconds * 1000;

	return points.filter((p) => p.timestamp >= cutoff);
}

function prepareChartData(points, fields) {
	if (!points || points.length === 0) {
		return null;
	}

	// uPlot expects data as array of arrays: [[timestamps], [series1], [series2], ...]
	var timestamps = points.map((p) => p.timestamp / 1000); // Convert to seconds
	var seriesData = fields.map((field) => points.map((p) => p[field] ?? 0));

	return [timestamps, ...seriesData];
}

// Get unique provider names from history points
function getProviders(points) {
	var providers = new Set();
	for (var p of points) {
		if (p.by_provider) {
			for (var name of Object.keys(p.by_provider)) {
				providers.add(name);
			}
		}
	}
	return Array.from(providers).sort();
}

// Prepare per-provider chart data for a specific metric (input_tokens, output_tokens, etc.)
function prepareProviderChartData(points, providers, metric) {
	if (!points || points.length === 0 || providers.length === 0) {
		return null;
	}

	var timestamps = points.map((p) => p.timestamp / 1000);
	var seriesData = providers.map((provider) =>
		points.map((p) => {
			var providerData = p.by_provider?.[provider];
			return providerData?.[metric] ?? 0;
		}),
	);

	return [timestamps, ...seriesData];
}

// Provider color palette (distinct colors for different providers)
var providerColors = [
	"#10b981", // emerald (primary)
	"#8b5cf6", // violet
	"#f59e0b", // amber
	"#ef4444", // red
	"#3b82f6", // blue
	"#ec4899", // pink
	"#14b8a6", // teal
	"#f97316", // orange
];

function MetricsGrid({ categories }) {
	if (!categories) return null;

	var { llm, http, tools, mcp, system } = categories;

	// Check if there's any meaningful data
	var hasData = system?.uptime_seconds > 0 || http?.total > 0 || llm?.completions_total > 0 || tools?.total > 0;

	if (!hasData) {
		return html`
			<${EmptyState}
				icon=${activityIcon}
				title="No activity yet"
				description="Metrics will appear here once you start using moltis. Try sending a message or running a tool to see data."
			/>
		`;
	}

	return html`
		<div class="space-y-10">
			<!-- System Overview -->
			<section>
				<h3 class="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">System</h3>
				<div class="grid grid-cols-2 md:grid-cols-4 gap-6">
					<${MetricCard} title="Uptime" value=${formatUptime(system?.uptime_seconds)} />
					<${MetricCard} title="Connected Clients" value=${formatNumber(system?.connected_clients)} />
					<${MetricCard} title="Active Sessions" value=${formatNumber(system?.active_sessions)} />
					<${MetricCard} title="HTTP Requests" value=${formatNumber(http?.total)} />
				</div>
			</section>

			<!-- LLM Metrics -->
			<section>
				<h3 class="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">LLM Usage</h3>
				<div class="grid grid-cols-2 md:grid-cols-4 gap-6">
					<${MetricCard}
						title="Completions"
						value=${formatNumber(llm?.completions_total)}
						subtitle=${llm?.errors > 0 ? `${llm.errors} errors` : undefined}
					/>
					<${MetricCard} title="Input Tokens" value=${formatNumber(llm?.input_tokens)} />
					<${MetricCard} title="Output Tokens" value=${formatNumber(llm?.output_tokens)} />
					<${MetricCard}
						title="Cache Tokens"
						value=${formatNumber((llm?.cache_read_tokens || 0) + (llm?.cache_write_tokens || 0))}
						subtitle=${llm?.cache_read_tokens ? `read: ${formatNumber(llm.cache_read_tokens)}` : undefined}
					/>
				</div>
			</section>

			<!-- Tools & MCP -->
			<section>
				<h3 class="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">Tools & MCP</h3>
				<div class="grid grid-cols-2 md:grid-cols-4 gap-6">
					<${MetricCard}
						title="Tool Executions"
						value=${formatNumber(tools?.total)}
						subtitle=${tools?.errors > 0 ? `${tools.errors} errors` : undefined}
					/>
					<${MetricCard} title="Tools Active" value=${formatNumber(tools?.active)} />
					<${MetricCard}
						title="MCP Tool Calls"
						value=${formatNumber(mcp?.total)}
						subtitle=${mcp?.errors > 0 ? `${mcp.errors} errors` : undefined}
					/>
					<${MetricCard} title="MCP Servers" value=${formatNumber(mcp?.active)} />
				</div>
			</section>
		</div>
	`;
}

function ChartsSection({ points, timeRange, onTimeRangeChange }) {
	var filteredPoints = filterPointsByTimeRange(points, timeRange);

	if (!filteredPoints || filteredPoints.length < 2) {
		return html`
			<div class="space-y-8">
				<${TimeRangeSelector} value=${timeRange} onChange=${onTimeRangeChange} />
				<${EmptyState}
					icon=${chartIcon}
					title="Collecting data..."
					description="Historical charts will appear here after a few data points are collected. This typically takes about 20-30 seconds."
				/>
			</div>
		`;
	}

	// Prepare chart data
	var tokenData = prepareChartData(filteredPoints, ["llm_input_tokens", "llm_output_tokens"]);
	var requestData = prepareChartData(filteredPoints, ["http_requests", "llm_completions"]);
	var connectionsData = prepareChartData(filteredPoints, ["ws_active", "active_sessions"]);
	var toolsData = prepareChartData(filteredPoints, ["tool_executions", "mcp_calls"]);

	// Prepare per-provider charts
	var providers = getProviders(filteredPoints);
	var providerInputData = prepareProviderChartData(filteredPoints, providers, "input_tokens");
	var providerOutputData = prepareProviderChartData(filteredPoints, providers, "output_tokens");
	var providerSeries = providers.map((name, i) => ({
		label: name,
		color: providerColors[i % providerColors.length],
	}));

	return html`
		<div class="space-y-8">
			<${TimeRangeSelector} value=${timeRange} onChange=${onTimeRangeChange} />
			<div class="grid grid-cols-1 xl:grid-cols-2 gap-8">
				${
					tokenData &&
					html`
					<${TimeSeriesChart}
						title="Token Usage (Total)"
						data=${tokenData}
						series=${[
							{ label: "Input Tokens", color: chartColors.primary },
							{ label: "Output Tokens", color: chartColors.secondary },
						]}
					/>
				`
				}
				${
					providerInputData &&
					providers.length > 0 &&
					html`
					<${TimeSeriesChart}
						title="Input Tokens by Provider"
						data=${providerInputData}
						series=${providerSeries}
					/>
				`
				}
				${
					providerOutputData &&
					providers.length > 0 &&
					html`
					<${TimeSeriesChart}
						title="Output Tokens by Provider"
						data=${providerOutputData}
						series=${providerSeries}
					/>
				`
				}
				${
					requestData &&
					html`
					<${TimeSeriesChart}
						title="Requests"
						data=${requestData}
						series=${[
							{ label: "HTTP Requests", color: chartColors.tertiary },
							{ label: "LLM Completions", color: chartColors.primary },
						]}
					/>
				`
				}
				${
					connectionsData &&
					html`
					<${TimeSeriesChart}
						title="Connections"
						data=${connectionsData}
						series=${[
							{ label: "WebSocket Active", color: chartColors.secondary },
							{ label: "Active Sessions", color: chartColors.tertiary },
						]}
					/>
				`
				}
				${
					toolsData &&
					html`
					<${TimeSeriesChart}
						title="Tool Activity"
						data=${toolsData}
						series=${[
							{ label: "Tool Executions", color: chartColors.primary },
							{ label: "MCP Calls", color: chartColors.secondary },
						]}
					/>
				`
				}
			</div>
		</div>
	`;
}

function TimeRangeSelector({ value, onChange }) {
	return html`
		<div class="flex items-center gap-1 bg-[var(--surface)] border border-[var(--border)] rounded-md p-1">
			${Object.entries(TIME_RANGES).map(
				([key, range]) => html`
				<button
					key=${key}
					class="px-3 py-1.5 text-xs rounded transition-colors ${value === key ? "bg-[var(--surface2)] text-[var(--text)] font-medium" : "text-[var(--muted)] hover:text-[var(--text)] hover:bg-[var(--surface2)]"}"
					onClick=${() => onChange(key)}
				>
					${range.label}
				</button>
			`,
			)}
		</div>
	`;
}

function ProviderTable({ byProvider }) {
	if (!byProvider || Object.keys(byProvider).length === 0) return null;

	return html`
		<section>
			<h3 class="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">By Provider</h3>
			<div class="bg-[var(--surface)] border border-[var(--border)] rounded-lg overflow-hidden">
				<table class="w-full text-sm">
					<thead>
						<tr class="border-b border-[var(--border)] bg-[var(--surface2)]">
							<th class="text-left px-6 py-4 font-medium">Provider</th>
							<th class="text-right px-6 py-4 font-medium">Completions</th>
							<th class="text-right px-6 py-4 font-medium">Input Tokens</th>
							<th class="text-right px-6 py-4 font-medium">Output Tokens</th>
							<th class="text-right px-6 py-4 font-medium">Errors</th>
						</tr>
					</thead>
					<tbody>
						${Object.entries(byProvider).map(
							([name, stats]) => html`
							<tr class="border-b border-[var(--border)] last:border-0">
								<td class="px-6 py-4">${name}</td>
								<td class="text-right px-6 py-4">${formatNumber(stats.completions)}</td>
								<td class="text-right px-6 py-4">${formatNumber(stats.input_tokens)}</td>
								<td class="text-right px-6 py-4">${formatNumber(stats.output_tokens)}</td>
								<td class="text-right px-6 py-4 ${stats.errors > 0 ? "text-[var(--error)]" : ""}">${formatNumber(stats.errors)}</td>
							</tr>
						`,
						)}
					</tbody>
				</table>
			</div>
		</section>
	`;
}

function PrometheusEndpoint() {
	var [copied, setCopied] = useState(false);
	var endpoint = `${window.location.origin}/metrics`;

	function copyEndpoint() {
		navigator.clipboard.writeText(endpoint).then(() => {
			setCopied(true);
			setTimeout(() => setCopied(false), 2000);
		});
	}

	return html`
		<section>
			<h3 class="text-sm font-medium text-[var(--muted)] uppercase tracking-wide mb-5">Prometheus Endpoint</h3>
			<div class="p-6 bg-[var(--surface)] border border-[var(--border)] rounded-lg">
				<p class="text-sm text-[var(--muted)] mb-5">
					Scrape this endpoint with Prometheus or import into Grafana for advanced visualization.
				</p>
				<div class="flex items-center gap-4">
					<code class="flex-1 px-4 py-3 bg-[var(--surface2)] rounded-md text-sm font-mono overflow-x-auto">${endpoint}</code>
					<button
						class="provider-btn provider-btn-secondary text-sm shrink-0"
						onClick=${copyEndpoint}
					>
						${copied ? "Copied!" : "Copy"}
					</button>
				</div>
			</div>
		</section>
	`;
}

function MonitoringPage({ initialTab }) {
	var [activeTab, setActiveTab] = useState(initialTab || "overview");
	var [timeRange, setTimeRange] = useState("1h");

	// Update URL when tab changes
	function handleTabChange(tab) {
		setActiveTab(tab);
		if (monitoringSyncPath) {
			var newPath = tab === "charts" ? `${monitoringPathBase}/charts` : monitoringPathBase;
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
		return html`
			<div class="flex items-center justify-center h-64 text-[var(--muted)]">
				<div class="text-center">
					<div class="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin mb-4"></div>
					<p>Loading metrics...</p>
				</div>
			</div>
		`;
	}

	if (error.value) {
		return html`
			<div class="p-10">
				<div class="max-w-3xl mx-auto space-y-10">
					<div class="p-6 bg-[var(--error-bg)] border border-[var(--error)] rounded-lg text-[var(--error)]">
						${error.value}
					</div>
					<${PrometheusEndpoint} />
				</div>
			</div>
		`;
	}

	return html`
		<div class="p-10 overflow-y-auto">
			<div class="max-w-7xl mx-auto">
				<div class="flex items-center justify-between mb-10">
					<div class="flex items-center gap-4">
						<h2 class="text-xl font-semibold">Monitoring</h2>
						<${LiveIndicator} live=${isLive.value} />
					</div>
					<div class="flex items-center gap-4">
						<div class="flex border border-[var(--border)] rounded-md overflow-hidden">
							<button
								class="px-5 py-2.5 text-sm transition-colors ${activeTab === "overview" ? "bg-[var(--surface2)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--text)]"}"
								onClick=${() => handleTabChange("overview")}
							>
								Overview
							</button>
							<button
								class="px-5 py-2.5 text-sm transition-colors ${activeTab === "charts" ? "bg-[var(--surface2)] text-[var(--text)]" : "text-[var(--muted)] hover:bg-[var(--surface)] hover:text-[var(--text)]"}"
								onClick=${() => handleTabChange("charts")}
							>
								Charts
							</button>
						</div>
					</div>
				</div>

				${
					activeTab === "overview" &&
					html`
					<div class="space-y-10">
						<${MetricsGrid} categories=${metricsData.value?.categories} />
						<${ProviderTable} byProvider=${metricsData.value?.categories?.llm?.by_provider} />
						<${PrometheusEndpoint} />
					</div>
				`
				}

				${
					activeTab === "charts" &&
					html`
					<${ChartsSection}
						points=${historyPoints.value}
						timeRange=${timeRange}
						onTimeRangeChange=${setTimeRange}
					/>
				`
				}
			</div>
		</div>
	`;
}

export function initMonitoring(container, param, options) {
	// param is "charts" for /monitoring/charts, null for /monitoring
	_monitoringContainer = container;
	monitoringPathBase = options?.pathBase || routes.monitoring;
	monitoringSyncPath = options?.syncPath !== false;
	var initialTab = param === "charts" ? "charts" : "overview";
	render(html`<${MonitoringPage} initialTab=${initialTab} />`, container);
}

export function teardownMonitoring() {
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
registerPrefix(routes.monitoring, initMonitoring, teardownMonitoring);
