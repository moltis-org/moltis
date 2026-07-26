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
let swRegistration: ServiceWorkerRegistration | null = null;

// Check if running in standalone mode (installed PWA)
export function isStandalone(): boolean {
	return (
		window.matchMedia("(display-mode: standalone)").matches ||
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
		swRegistration = await navigator.serviceWorker.register("/sw.js", {
			scope: "/",
		});
		console.log("Service worker registered:", swRegistration.scope);

		// A worker that installed while the page was closed is already waiting.
		if (swRegistration.waiting && navigator.serviceWorker.controller) {
			dispatchUpdateAvailable();
		}

		// Handle updates
		swRegistration.addEventListener("updatefound", () => {
			const newWorker = swRegistration?.installing;
			if (newWorker) {
				newWorker.addEventListener("statechange", () => {
					if (newWorker.state === "installed" && navigator.serviceWorker.controller) {
						// New content is available, notify user
						dispatchUpdateAvailable();
					}
				});
			}
		});

		return swRegistration;
	} catch (error) {
		console.error("Service worker registration failed:", error);
		return null;
	}
}

// Dispatch custom event when update is available
function dispatchUpdateAvailable(): void {
	window.dispatchEvent(new CustomEvent("sw-update-available"));
	scheduleUpdateActivation();
}

// Skip waiting and activate new service worker
export function activateUpdate(): void {
	if (swRegistration?.waiting) {
		swRegistration.waiting.postMessage({ type: "SKIP_WAITING" });
	}
}

/**
 * Activate a waiting worker the next time the app is out of view.
 *
 * The service worker deliberately does not call skipWaiting() at install time,
 * so something has to hand control over eventually or updates would never land.
 * Doing it while the tab is hidden means the swap — and the reload that follows
 * it — never happens under someone who is mid-conversation.
 */
function scheduleUpdateActivation(): void {
	if (document.visibilityState === "hidden") {
		activateUpdate();
		return;
	}

	const onHidden = (): void => {
		if (document.visibilityState !== "hidden") return;
		document.removeEventListener("visibilitychange", onHidden);
		activateUpdate();
	};
	document.addEventListener("visibilitychange", onHidden);
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
		if (event.data && event.data.type === "notification-click" && callback) callback(event.data.url);
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
 * Set or clear the installed-app badge.
 *
 * Badging lives here rather than in the service worker: calling it from a
 * worker crashes the renderer in some Chromium builds, and a crash is not
 * catchable. In a page it is safe and merely a no-op where unsupported.
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

// Clear the installed-app badge once the user is looking at the app.
function clearAppBadge(): void {
	setAppBadge(0);
}

/** Apply badge counts pushed by the service worker. */
function setupBadgeHandler(): void {
	navigator.serviceWorker?.addEventListener("message", (event: MessageEvent) => {
		if (event.data?.type !== "badge-count") return;
		const count = typeof event.data.count === "number" ? event.data.count : 0;
		setAppBadge(count);
	});
}

// Initialize PWA features
export function initPWA(): void {
	syncStandaloneClass();
	const hadControllerBeforeInit = Boolean(navigator.serviceWorker?.controller);

	// Register service worker
	registerServiceWorker();

	// Report foreground presence so the server can skip push for the device
	// that is already watching the session.
	initPresenceReporting();

	// The service worker delegates badge updates here — see setAppBadge().
	setupBadgeHandler();

	// Handle notification clicks — route in-place so the SPA keeps its state
	// and open WebSocket instead of doing a full document reload.
	setupNotificationHandler((url: string) => {
		clearAppBadge();
		if (url && url !== window.location.pathname) {
			navigate(url);
		}
	});

	if (document.visibilityState === "visible") {
		clearAppBadge();
	}
	document.addEventListener("visibilitychange", () => {
		if (document.visibilityState === "visible") {
			clearAppBadge();
		}
	});

	// Listen for controller change (new SW activated)
	navigator.serviceWorker?.addEventListener("controllerchange", () => {
		// First service worker install should not force a reload.
		if (!hadControllerBeforeInit) {
			return;
		}
		// Avoid forced reload churn on onboarding; the app boot path will
		// fetch fresh assets on the next navigation to the main UI.
		if (window.location.pathname === "/onboarding") {
			return;
		}
		reloadWhenHidden();
	});
}

/**
 * Reload to pick up the new worker's assets, but only while the app is out of
 * view — a reload under an open conversation loses whatever is in the composer.
 */
function reloadWhenHidden(): void {
	if (document.visibilityState === "hidden") {
		window.location.reload();
		return;
	}

	const onHidden = (): void => {
		if (document.visibilityState !== "hidden") return;
		document.removeEventListener("visibilitychange", onHidden);
		window.location.reload();
	};
	document.addEventListener("visibilitychange", onHidden);
}
