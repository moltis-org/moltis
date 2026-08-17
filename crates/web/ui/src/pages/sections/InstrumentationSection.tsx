// ── Instrumentation section ─────────────────────────────────
//
// One page for every tracing backend, because they are all fed from a single
// instrumentation pass in the agent runtime. The page is deliberately explicit
// about the fact that Langfuse and the APM backends receive *different* data:
// that asymmetry is the design, not an accident, and an operator who does not
// know about it will either send prompts somewhere they should not, or wonder
// why their Datadog spans look empty.

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { Badge, Loading, SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import { sendRpc } from "../../helpers";
import { connected } from "../../signals";
import { rerender } from "./_shared";

type ContentMode = "full" | "metadata_only" | "none";

interface SkippedBackend {
	name: string;
	reason: string;
}

interface LangfuseConfig {
	enabled: boolean;
	host: string;
	public_key: string;
	secret_key_set: boolean;
	capture_input: boolean;
	capture_output: boolean;
	capture_tool_io: boolean;
}

interface OtlpConfig {
	enabled: boolean;
	endpoint: string;
	content: ContentMode;
	emit_user_id: boolean;
}

interface DatadogConfig {
	enabled: boolean;
	endpoint: string;
	service: string;
	api_key_set: boolean;
	content: ContentMode;
}

interface InstrumentationConfig {
	enabled: boolean;
	environment: string;
	sample_rate: number;
	queue_capacity: number;
	langfuse: LangfuseConfig;
	otlp: OtlpConfig;
	datadog: DatadogConfig;
}

interface InstrumentationStatus {
	active: boolean;
	backends: string[];
	skipped: SkippedBackend[];
	delivery: DeliveryStats[];
	config: InstrumentationConfig;
}

interface DeliveryStats {
	name: string;
	accepted: number;
	dropped_queue_full: number;
	dropped_failed: number;
	delivered: number;
	retries: number;
	last_success_at: string | null;
	last_error: string | null;
	last_error_at: string | null;
}

interface TestResult {
	ok: boolean;
	error?: string;
}

const CONTENT_LABELS: Record<ContentMode, string> = {
	full: "Full conversation",
	metadata_only: "Metadata only",
	none: "Nothing",
};

// ── Read-only field rows ────────────────────────────────────

interface RowProps {
	label: string;
	value: string;
	hint?: string;
}

function Row({ label, value, hint }: RowProps): VNode {
	return (
		<div className="flex items-baseline justify-between gap-4 py-1 border-b border-[var(--border)] last:border-0">
			<span className="text-xs text-[var(--muted)] shrink-0">{label}</span>
			<span className="text-xs text-right break-all">
				{value}
				{hint ? <span className="block text-[var(--muted)]">{hint}</span> : null}
			</span>
		</div>
	);
}

function deliveryLabel(name: string): string {
	if (name === "langfuse") return "Langfuse traces";
	if (name === "langfuse-scores") return "Langfuse scores";
	if (name === "otlp") return "OpenTelemetry";
	if (name === "datadog") return "Datadog";
	return name;
}

function formatDeliveryTime(value?: string | null): string {
	if (!value) return "never";
	const parsed = new Date(value);
	return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

interface DeliveryRowsProps {
	delivery: DeliveryStats[];
}

function DeliveryRows({ delivery }: DeliveryRowsProps): VNode {
	if (delivery.length === 0) {
		return <Row label="Delivery" value="no exporters running" />;
	}

	return (
		<>
			{delivery.map((stats) => {
				const failures = `${stats.dropped_queue_full} queue drops, ${stats.dropped_failed} failed, ${stats.retries} retries`;
				const latest = stats.last_error
					? `Last success: ${formatDeliveryTime(stats.last_success_at)}. Last error: ${stats.last_error} (${formatDeliveryTime(stats.last_error_at)}).`
					: `Last success: ${formatDeliveryTime(stats.last_success_at)}.`;
				return (
					<Row
						key={stats.name}
						label={deliveryLabel(stats.name)}
						value={`${stats.accepted} enqueued, ${stats.delivered} delivered`}
						hint={`${failures}. ${latest}`}
					/>
				);
			})}
		</>
	);
}

interface BackendCardProps {
	title: string;
	purpose: string;
	enabled: boolean;
	running: boolean;
	skippedReason?: string;
	children: VNode | VNode[];
}

function BackendCard({ title, purpose, enabled, running, skippedReason, children }: BackendCardProps): VNode {
	return (
		<div className="rounded border border-[var(--border)] p-3 flex flex-col gap-2">
			<div className="flex items-center justify-between gap-2">
				<span className="font-medium text-sm">{title}</span>
				{running ? (
					<Badge label="Running" variant="running" />
				) : enabled ? (
					<Badge label="Not running" variant="error" />
				) : (
					<Badge label="Off" variant="muted" />
				)}
			</div>
			<p className="text-xs text-[var(--muted)]">{purpose}</p>
			{skippedReason ? (
				<p className="text-xs text-[var(--danger)]">Enabled in config but did not start: {skippedReason}</p>
			) : null}
			{children}
		</div>
	);
}

// ── Section ─────────────────────────────────────────────────

interface BackendsProps {
	config: InstrumentationConfig;
	status: InstrumentationStatus;
	testing: boolean;
	onTestLangfuse: () => void;
}

/** The three backend cards, extracted so the section body stays readable. */
function Backends({ config, status, testing, onTestLangfuse }: BackendsProps): VNode {
	const skippedFor = (name: string): string | undefined => status.skipped.find((s) => s.name === name)?.reason;
	const isRunning = (name: string): boolean => status.backends.includes(name);

	const captures =
		[
			config.langfuse.capture_input ? "input" : null,
			config.langfuse.capture_output ? "output" : null,
			config.langfuse.capture_tool_io ? "tool I/O" : null,
		]
			.filter(Boolean)
			.join(", ") || "structure only";

	return (
		<>
			<BackendCard
				title="Langfuse"
				purpose="LLM observability: prompts, completions, sessions, inferred cost and reaction feedback."
				enabled={config.langfuse.enabled}
				running={isRunning("langfuse")}
				skippedReason={skippedFor("langfuse")}
			>
				<Row label="Host" value={config.langfuse.host || "not set"} />
				<Row label="Public key" value={config.langfuse.public_key || "not set"} />
				<Row label="Secret key" value={config.langfuse.secret_key_set ? "configured" : "not set"} />
				<Row label="Captures" value={captures} />
				<button
					type="button"
					className="provider-btn self-start"
					disabled={testing || !config.langfuse.enabled}
					onClick={onTestLangfuse}
				>
					{testing ? "Testing\u2026" : "Test connection"}
				</button>
			</BackendCard>

			<BackendCard
				title="OpenTelemetry (Grafana, Honeycomb, Collector)"
				purpose="Operational traces over OTLP. Pair with the Prometheus /metrics endpoint for dashboards."
				enabled={config.otlp.enabled}
				running={isRunning("otlp")}
				skippedReason={skippedFor("otlp")}
			>
				<Row label="Endpoint" value={config.otlp.endpoint || "not set"} />
				<Row label="Sends" value={CONTENT_LABELS[config.otlp.content]} />
				<Row
					label="End-user id"
					value={config.otlp.emit_user_id ? "included" : "omitted"}
					hint={config.otlp.emit_user_id ? undefined : "high-cardinality in an APM index"}
				/>
			</BackendCard>

			<BackendCard
				title="Datadog"
				purpose="Datadog APM through the Agent's OTLP intake. Tags are omitted: Datadog bills on tag cardinality."
				enabled={config.datadog.enabled}
				running={isRunning("datadog")}
				skippedReason={skippedFor("datadog")}
			>
				<Row label="Endpoint" value={config.datadog.endpoint || "not set"} />
				<Row label="Service" value={config.datadog.service} />
				<Row
					label="API key"
					value={config.datadog.api_key_set ? "configured" : "not set"}
					hint={config.datadog.api_key_set ? undefined : "not needed when using a local Agent"}
				/>
				<Row label="Sends" value={CONTENT_LABELS[config.datadog.content]} />
			</BackendCard>
		</>
	);
}

/** Explains why backends receive different data. */
function ProfileExplainer(): VNode {
	return (
		<div className="rounded border border-[var(--border)] p-3 flex flex-col gap-1">
			<SubHeading title="What each backend receives" />
			<p className="text-xs text-[var(--muted)]">
				Langfuse gets completed observations with the full conversation, token usage and session context. It infers cost
				from its current model pricing. OTLP and Datadog get operational shape only: latency, errors, model and token
				counts, with payload sizes instead of payloads. Prompt bodies in an APM mean unbounded span size, cardinality
				pressure, per-byte ingest billing, and conversation content in a system nobody scoped for it.
			</p>
		</div>
	);
}

export function InstrumentationSection(): VNode {
	const [status, setStatus] = useState<InstrumentationStatus | null>(null);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [success, setSuccess] = useState<string | null>(null);
	const [testing, setTesting] = useState(false);

	useEffect(() => {
		if (!connected.value) return;
		setLoading(true);
		setError(null);
		sendRpc<InstrumentationStatus>("instrumentation.status", {})
			.then((res) => {
				if (res.payload) setStatus(res.payload);
				else setError(res.error?.message ?? "Failed to load instrumentation status");
			})
			.catch((e: Error) => setError(e.message))
			.finally(() => {
				setLoading(false);
				rerender();
			});
	}, [connected.value]);

	function onTestLangfuse(): void {
		setTesting(true);
		setError(null);
		setSuccess(null);
		rerender();
		sendRpc<TestResult>("instrumentation.test", { backend: "langfuse" })
			.then((res) => {
				if (res.payload?.ok) setSuccess("Langfuse reachable and credentials accepted.");
				else setError(res.payload?.error ?? res.error?.message ?? "Connection test failed");
			})
			.catch((e: Error) => setError(e.message))
			.finally(() => {
				setTesting(false);
				rerender();
			});
	}

	if (loading) return <Loading />;
	if (!status) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Instrumentation" />
				<StatusMessage error={error} success={success} />
			</div>
		);
	}

	const { config } = status;

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Instrumentation" />

			<p className="text-xs text-[var(--muted)]">
				Exports what each agent run did (LLM calls, tool calls, retrievals) to an external backend. Configured in{" "}
				<code>moltis.toml</code> under <code>[instrumentation]</code>, or in Settings then Configuration, and applied on
				restart. This page is read-only: it shows what is running and lets you verify connectivity.
			</p>

			{config.enabled ? null : (
				<p className="text-xs text-[var(--muted)]">
					Instrumentation is off. It stays off until you enable it explicitly, because turning it on sends data about
					your conversations to a third party.
				</p>
			)}

			<StatusMessage error={error} success={success} />
			<ProfileExplainer />

			<div className="rounded border border-[var(--border)] p-3 flex flex-col gap-1">
				<SubHeading title="Status" />
				<Row label="Active" value={status.active ? "Yes" : "No"} />
				<Row label="Backends" value={status.backends.length > 0 ? status.backends.join(", ") : "none"} />
				<Row label="Environment" value={config.environment} />
				<Row label="Sample rate" value={`${Math.round(config.sample_rate * 100)}% of turns`} />
				<Row label="Queue capacity" value={String(config.queue_capacity)} />
				<DeliveryRows delivery={status.delivery} />
			</div>

			<Backends config={config} status={status} testing={testing} onTestLangfuse={onTestLangfuse} />
		</div>
	);
}
