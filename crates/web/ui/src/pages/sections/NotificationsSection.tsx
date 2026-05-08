// ── Notifications section ─────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import { onEvent } from "../../events";
import * as push from "../../push";
import { isStandalone } from "../../pwa";
import { rerender } from "./_shared";

interface PushSubscription {
	endpoint: string;
	device?: string;
	ip?: string;
	created_at?: string;
}

interface PushServerStatus {
	subscription_count?: number;
	subscriptions?: PushSubscription[];
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Notifications section handles multiple states and conditions
export function NotificationsSection(): VNode {
	const [supported, setSupported] = useState(false);
	const [permission, setPermission] = useState("default");
	const [subscribed, setSubscribed] = useState(false);
	const [isLoading, setIsLoading] = useState(true);
	const [toggling, setToggling] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [serverStatus, setServerStatus] = useState<PushServerStatus | null>(null);

	async function checkStatus(): Promise<void> {
		setIsLoading(true);
		rerender();

		const pushSupported = push.isPushSupported();
		setSupported(pushSupported);

		if (pushSupported) {
			setPermission(push.getPermissionState());
			await push.initPushState();
			setSubscribed(push.isSubscribed());

			const status = await push.getPushStatus();
			setServerStatus(status as PushServerStatus);
		}

		setIsLoading(false);
		rerender();
	}

	async function refreshStatus(): Promise<void> {
		const status = await push.getPushStatus();
		setServerStatus(status as PushServerStatus);
		rerender();
	}

	async function onRemoveSubscription(endpoint: string): Promise<void> {
		const result = await push.removeSubscription(endpoint);
		if (!result.success) {
			setError(result.error || "Failed to remove subscription");
			rerender();
		}
	}

	useEffect(() => {
		checkStatus();
		const off = onEvent("push.subscriptions", () => {
			refreshStatus();
		});
		return off;
	}, []);

	async function onToggle(): Promise<void> {
		setError(null);
		setToggling(true);
		rerender();

		const result = subscribed ? await push.unsubscribeFromPush() : await push.subscribeToPush();

		if (result.success) {
			setSubscribed(!subscribed);
			if (!subscribed) setPermission("granted");
		} else {
			setError(result.error || (subscribed ? "Failed to unsubscribe" : "Failed to subscribe"));
		}

		setToggling(false);
		rerender();
	}

	if (isLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div className="text-xs text-[var(--muted)]">Loading{"\u2026"}</div>
			</div>
		);
	}

	if (!supported) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0 }}>
						Push notifications are not supported in this browser.
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						Try using Safari, Chrome, or Firefox on a device that supports web push.
					</p>
				</div>
			</div>
		);
	}

	if (serverStatus === null) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="Notifications" />
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0 }}>
						Push notifications are not configured on the server.
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						The server was built without the{" "}
						<code style={{ fontFamily: "var(--font-mono)", fontSize: ".75rem" }}>push-notifications</code> feature.
					</p>
				</div>
			</div>
		);
	}

	const standalone = isStandalone();
	const needsInstall = !standalone && /Safari/.test(navigator.userAgent) && !/Chrome/.test(navigator.userAgent);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="Notifications" />
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Receive push notifications when the agent completes a task or needs your attention.
			</p>

			<div style={{ maxWidth: "600px" }}>
				<div className="provider-item" style={{ marginBottom: 0 }}>
					<div style={{ flex: 1, minWidth: 0 }}>
						<div className="provider-item-name" style={{ fontSize: ".9rem" }}>
							Push Notifications
						</div>
						<div style={{ fontSize: ".75rem", color: "var(--muted)", marginTop: "2px" }}>
							{needsInstall
								? "Add this app to your Dock to enable notifications."
								: subscribed
									? "You will receive notifications on this device."
									: permission === "denied"
										? "Notifications are blocked. Enable them in browser settings."
										: "Enable to receive notifications on this device."}
						</div>
					</div>
					<button
						className={`provider-btn ${subscribed ? "provider-btn-danger" : ""}`}
						onClick={onToggle}
						disabled={toggling || permission === "denied" || needsInstall}
					>
						{toggling ? "\u2026" : subscribed ? "Disable" : "Enable"}
					</button>
				</div>
				<StatusMessage error={error} className="text-xs mt-2" />
			</div>

			{needsInstall ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--border)",
						background: "var(--surface)",
					}}
				>
					<p className="text-sm text-[var(--text)]" style={{ margin: 0, fontWeight: 500 }}>
						Installation required
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						On Safari, push notifications are only available for installed apps. Add moltis to your Dock using{" "}
						<strong>File {"\u2192"} Add to Dock</strong> (or Share {"\u2192"} Add to Dock on iOS), then open it from
						there.
					</p>
				</div>
			) : null}

			{permission === "denied" && !needsInstall ? (
				<div
					style={{
						maxWidth: "600px",
						padding: "12px 16px",
						borderRadius: "6px",
						border: "1px solid var(--error)",
						background: "color-mix(in srgb, var(--error) 5%, transparent)",
					}}
				>
					<p className="text-sm" style={{ color: "var(--error)", margin: 0, fontWeight: 500 }}>
						Notifications are blocked
					</p>
					<p className="text-xs text-[var(--muted)]" style={{ margin: "8px 0 0" }}>
						You previously blocked notifications for this site. To enable them, you'll need to update your browser's
						site settings and allow notifications for this origin.
					</p>
				</div>
			) : null}

			<div style={{ maxWidth: "600px", borderTop: "1px solid var(--border)", paddingTop: "16px", marginTop: "8px" }}>
				<SubHeading title={`Subscribed Devices (${serverStatus?.subscription_count || 0})`} />
				{(serverStatus?.subscriptions?.length || 0) > 0 ? (
					<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
						{serverStatus?.subscriptions?.map((sub) => (
							<div className="provider-item" style={{ marginBottom: 0 }} key={sub.endpoint}>
								<div style={{ flex: 1, minWidth: 0 }}>
									<div className="provider-item-name" style={{ fontSize: ".85rem" }}>
										{sub.device}
									</div>
									<div
										style={{
											fontSize: ".7rem",
											color: "var(--muted)",
											marginTop: "2px",
											display: "flex",
											gap: "12px",
											flexWrap: "wrap",
										}}
									>
										{sub.ip ? <span style={{ fontFamily: "var(--font-mono)" }}>{sub.ip}</span> : null}
										<time dateTime={sub.created_at}>{new Date(sub.created_at || "").toLocaleDateString()}</time>
									</div>
								</div>
								<button className="provider-btn provider-btn-danger" onClick={() => onRemoveSubscription(sub.endpoint)}>
									Remove
								</button>
							</div>
						))}
					</div>
				) : (
					<div className="text-xs text-[var(--muted)]" style={{ padding: "4px 0" }}>
						No devices subscribed yet.
					</div>
				)}
			</div>
		</div>
	);
}
