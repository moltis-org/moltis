// PWA utilities - service worker registration and install prompt handling

import { initPresenceReporting } from "./push";
import { navigate } from "./router";

/** Extended Navigator interface for iOS standalone detection. */
interface NavigatorStandalone extends Navigator {
	standalone?: boolean;
}

/** The beforeinstallprompt event fired by Chrome/Edge. */
interface BeforeInstallPromptEvent extends Event {
	prompt(): Promise<void>;
	userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

let deferredInstallPrompt: BeforeInstallPromptEvent | null = null;
const INSTALLED_DISPLAY_MODES = ["standalone", "window-controls-overlay", "fullscreen", "minimal-ui"] as const;

// Check if running in standalone mode (installed PWA)
export function isStandalone(): boolean {
	return (
		INSTALLED_DISPLAY_MODES.some((mode) => window.matchMedia(`(display-mode: ${mode})`).matches) ||
		(navigator as NavigatorStandalone).standalone === true ||
		document.referrer.includes("android-app://")
	);
}

// Check if iOS device
export function isIOS(): boolean {
	return /iPhone|iPad|iPod/.test(navigator.userAgent);
}

// Check if Android device
export function isAndroid(): boolean {
	return /Android/.test(navigator.userAgent);
}

export function syncStandaloneClass(): void {
	document.documentElement.classList.toggle("pwa-standalone", isStandalone());
}

// Register service worker
export async function registerServiceWorker(): Promise<ServiceWorkerRegistration | null> {
	if (!("serviceWorker" in navigator)) {
		console.log("Service workers not supported");
		return null;
	}

	try {
		const registration = await navigator.serviceWorker.register("/sw.js", {
			scope: "/",
		});
		console.log("Service worker registered:", registration.scope);
		return registration;
	} catch (error) {
		console.error("Service worker registration failed:", error);
		return null;
	}
}

// Listen for beforeinstallprompt event (Android Chrome)
export function setupInstallPrompt(callback?: (e: BeforeInstallPromptEvent) => void): void {
	window.addEventListener("beforeinstallprompt", ((e: Event) => {
		e.preventDefault();
		deferredInstallPrompt = e as BeforeInstallPromptEvent;
		if (callback) callback(e as BeforeInstallPromptEvent);
	}) as EventListener);

	// Also listen for successful install
	window.addEventListener("appinstalled", () => {
		deferredInstallPrompt = null;
		console.log("PWA installed");
	});
}

// Trigger the install prompt (Android Chrome)
export async function promptInstall(): Promise<{ outcome: string }> {
	if (!deferredInstallPrompt) {
		return { outcome: "not-available" };
	}

	deferredInstallPrompt.prompt();
	const result = await deferredInstallPrompt.userChoice;
	deferredInstallPrompt = null;
	return result;
}

// Check if install prompt is available
export function canPromptInstall(): boolean {
	return deferredInstallPrompt !== null;
}

// Listen for notification clicks from service worker
export function setupNotificationHandler(callback?: (url: string) => void): void {
	navigator.serviceWorker?.addEventListener("message", (event: MessageEvent) => {
		if (!(event.data && event.data.type === "notification-click" && callback)) return;
		try {
			callback(event.data.url);
		} catch (error) {
			console.error("Failed to route notification click:", error);
			return;
		}
		event.ports[0]?.postMessage({ handled: true });
	});
}

// Request notification permission
export async function requestNotificationPermission(): Promise<NotificationPermission> {
	if (!("Notification" in window)) {
		return "denied";
	}

	if (Notification.permission === "granted") {
		return "granted";
	}

	if (Notification.permission === "denied") {
		return "denied";
	}

	return await Notification.requestPermission();
}

// Get current notification permission
export function getNotificationPermission(): NotificationPermission {
	if (!("Notification" in window)) {
		return "denied";
	}
	return Notification.permission;
}

/**
 * Tell the service worker whether this app is running installed.
 *
 * The worker badges the app icon when a push arrives with no page open, but it
 * cannot see display-mode itself and a badge is meaningless in a browser tab.
 * Reporting it here — and having the worker persist it — is what lets a closed
 * app still show a count.
 */
function reportInstalledState(installed = isStandalone()): void {
	// A regular browser tab cannot prove the PWA was uninstalled. Never let one
	// clear state previously reported by an installed window.
	if (!installed) return;
	navigator.serviceWorker?.ready
		.then((registration) => {
			registration.active?.postMessage({ type: "PWA_INSTALLED", installed: true });
		})
		.catch(() => {
			// No worker yet; the next page load reports again.
		});
}

/**
 * Set or clear the installed-app badge from the page.
 *
 * The worker handles the app-closed case; this keeps a running app in sync
 * immediately rather than waiting on the platform call.
 */
function setAppBadge(count: number): void {
	const nav = navigator as Navigator & {
		setAppBadge?: (count?: number) => Promise<void>;
		clearAppBadge?: () => Promise<void>;
	};
	const update = count > 0 ? nav.setAppBadge?.(count) : nav.clearAppBadge?.();
	update?.catch(() => {
		// Badging is unsupported on most desktop browsers.
	});
}

/** Apply badge counts pushed by the service worker. */
function setupBadgeHandler(): void {
	navigator.serviceWorker?.addEventListener("message", (event: MessageEvent) => {
		if (event.data?.type !== "badge-count") return;
		const count = typeof event.data.count === "number" ? event.data.count : 0;
		setAppBadge(count);
	});
}

function syncAppBadge(): void {
	navigator.serviceWorker?.ready
		.then((registration) => registration.active?.postMessage({ type: "SYNC_BADGE" }))
		.catch(() => {
			// The next visibility change retries once a worker is available.
		});
}

// Initialize PWA features
export function initPWA(): void {
	syncStandaloneClass();

	// Register service worker
	registerServiceWorker();

	// Report foreground presence so the server can skip push for the device
	// that is already watching the session.
	initPresenceReporting();

	// The service worker mirrors badge updates here — see setAppBadge().
	setupBadgeHandler();
	syncAppBadge();

	// Let the worker badge the icon while the app is closed.
	reportInstalledState();
	window.addEventListener("appinstalled", () => reportInstalledState(true));
	const onDisplayModeChange = (): void => {
		syncStandaloneClass();
		reportInstalledState();
	};
	for (const mode of INSTALLED_DISPLAY_MODES) {
		const media = window.matchMedia(`(display-mode: ${mode})`);
		if (typeof media.addEventListener === "function") {
			media.addEventListener("change", onDisplayModeChange);
		} else if (typeof media.addListener === "function") {
			media.addListener(onDisplayModeChange);
		}
	}

	// Handle notification clicks — route in-place so the SPA keeps its state
	// and open WebSocket instead of doing a full document reload.
	setupNotificationHandler((url: string) => {
		if (url && url !== window.location.pathname) {
			navigate(url);
		}
	});

	document.addEventListener("visibilitychange", () => {
		if (document.visibilityState === "visible") {
			syncAppBadge();
		}
	});
}
