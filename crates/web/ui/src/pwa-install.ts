// PWA Install Banner - handles "Add to Homescreen" prompts

import { t } from "./i18n";
import { canPromptInstall, isAndroid, isIOS, isStandalone, promptInstall, setupInstallPrompt } from "./pwa";

const DISMISS_KEY = "pwa-install-dismissed";
const DISMISS_DAYS = 7;

// Check if user dismissed the banner recently
function isDismissed(): boolean {
	const dismissed = localStorage.getItem(DISMISS_KEY);
	if (!dismissed) return false;
	const ts = parseInt(dismissed, 10);
	const days = (Date.now() - ts) / (1000 * 60 * 60 * 24);
	return days < DISMISS_DAYS;
}

// Mark banner as dismissed
function dismiss(): void {
	localStorage.setItem(DISMISS_KEY, Date.now().toString());
	hideBanner();
}

// Get the banner element
function getBanner(): HTMLElement | null {
	return document.getElementById("installBanner");
}

// Show the install banner
function showBanner(): void {
	const banner = getBanner();
	if (banner) {
		banner.classList.remove("hidden");
		banner.classList.add("flex");
	}
}

// Hide the install banner
function hideBanner(): void {
	const banner = getBanner();
	if (banner) {
		banner.classList.add("hidden");
		banner.classList.remove("flex");
	}
}

// Check if running in Safari on iOS
function isIOSSafari(): boolean {
	const ua = navigator.userAgent;
	return isIOS() && /Safari/.test(ua) && !/CriOS|FxiOS|OPiOS|EdgiOS/.test(ua);
}

// Create share icon element
function createShareIcon(): HTMLSpanElement {
	const el = document.createElement("span");
	el.className = "icon icon-share inline-block text-[var(--accent)]";
	return el;
}

// Create menu icon element
function createMenuIcon(): HTMLSpanElement {
	const el = document.createElement("span");
	el.className = "icon icon-menu-dots inline-block text-[var(--accent)]";
	return el;
}

// Render iOS-specific instructions
function renderIOSInstructions(container: HTMLElement): void {
	while (container.firstChild) container.removeChild(container.firstChild);

	const title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = t("pwa:install.title");
	container.appendChild(title);

	const steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	const step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode(t("pwa:ios.step1")));
	const strong1 = document.createElement("strong");
	strong1.textContent = t("pwa:ios.step1Button");
	step1.appendChild(strong1);
	step1.appendChild(document.createTextNode(t("pwa:ios.step1After")));
	step1.appendChild(createShareIcon());
	steps.appendChild(step1);

	const step2 = document.createElement("li");
	step2.textContent = t("pwa:ios.step2");
	steps.appendChild(step2);

	container.appendChild(steps);

	if (!isIOSSafari()) {
		const note = document.createElement("p");
		note.className = "text-xs text-[var(--muted)] mt-2";
		note.textContent = t("pwa:ios.safariTip");
		container.appendChild(note);
	}
}

// Render Android-specific instructions (for non-Chrome browsers)
function renderAndroidInstructions(container: HTMLElement): void {
	while (container.firstChild) container.removeChild(container.firstChild);

	const title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = t("pwa:install.title");
	container.appendChild(title);

	const steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	const step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode(t("pwa:android.step1")));
	step1.appendChild(createMenuIcon());
	steps.appendChild(step1);

	const step2 = document.createElement("li");
	step2.textContent = t("pwa:android.step2");
	steps.appendChild(step2);

	container.appendChild(steps);
}

// Render native install prompt (Android Chrome)
function renderNativePrompt(container: HTMLElement): void {
	while (container.firstChild) container.removeChild(container.firstChild);

	const title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)]";
	title.textContent = t("pwa:install.quickAccessTitle");
	container.appendChild(title);

	const desc = document.createElement("p");
	desc.className = "text-xs text-[var(--muted)] mt-1";
	desc.textContent = t("pwa:install.quickAccessDesc");
	container.appendChild(desc);
}

// Handle install button click
async function handleInstall(): Promise<void> {
	const result = await promptInstall();
	if (result.outcome === "accepted") {
		hideBanner();
	}
}

// Initialize the install banner
export function initInstallBanner(): void {
	// Don't show if already installed or dismissed
	if (isStandalone() || isDismissed()) {
		return;
	}

	const banner = getBanner();
	if (!banner) return;

	const instructions = banner.querySelector("[data-instructions]") as HTMLElement | null;
	const installBtn = banner.querySelector("[data-install-btn]") as HTMLElement | null;
	const dismissBtn = banner.querySelector("[data-dismiss-btn]") as HTMLElement | null;

	if (!instructions) return;

	// Set up dismiss button
	if (dismissBtn) {
		dismissBtn.addEventListener("click", dismiss);
	}

	// Platform-specific setup
	if (isIOS()) {
		renderIOSInstructions(instructions);
		if (installBtn) installBtn.style.display = "none";
		showBanner();
	} else if (isAndroid()) {
		// Try to use native prompt first
		setupInstallPrompt(() => {
			renderNativePrompt(instructions);
			if (installBtn) {
				installBtn.style.display = "";
				installBtn.addEventListener("click", handleInstall);
			}
			showBanner();
		});

		// If no native prompt after a delay, show manual instructions
		setTimeout(() => {
			if (!(canPromptInstall() || isStandalone())) {
				renderAndroidInstructions(instructions);
				if (installBtn) installBtn.style.display = "none";
				showBanner();
			}
		}, 3000);
	}
}
