// PWA utilities - service worker registration and install prompt handling

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
}

// Skip waiting and activate new service worker
export function activateUpdate(): void {
	if (swRegistration?.waiting) {
		swRegistration.waiting.postMessage({ type: "SKIP_WAITING" });
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

// Initialize PWA features
export function initPWA(): void {
	syncStandaloneClass();
	const hadControllerBeforeInit = Boolean(navigator.serviceWorker?.controller);

	// Register service worker
	registerServiceWorker();

	// Handle notification clicks (navigate to URL)
	setupNotificationHandler((url: string) => {
		if (url && url !== window.location.pathname) {
			window.location.href = url;
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
		window.location.reload();
	});
}
