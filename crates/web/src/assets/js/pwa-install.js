// PWA Install Banner - handles "Add to Homescreen" prompts

import { t } from "./i18n.js";
import { canPromptInstall, isAndroid, isIOS, isStandalone, promptInstall, setupInstallPrompt } from "./pwa.js";

var DISMISS_KEY = "pwa-install-dismissed";
var DISMISS_DAYS = 7;

// Check if user dismissed the banner recently
function isDismissed() {
	var dismissed = localStorage.getItem(DISMISS_KEY);
	if (!dismissed) return false;
	var ts = parseInt(dismissed, 10);
	var days = (Date.now() - ts) / (1000 * 60 * 60 * 24);
	return days < DISMISS_DAYS;
}

// Mark banner as dismissed
function dismiss() {
	localStorage.setItem(DISMISS_KEY, Date.now().toString());
	hideBanner();
}

// Get the banner element
function getBanner() {
	return document.getElementById("installBanner");
}

// Show the install banner
function showBanner() {
	var banner = getBanner();
	if (banner) {
		banner.classList.remove("hidden");
		banner.classList.add("flex");
	}
}

// Hide the install banner
function hideBanner() {
	var banner = getBanner();
	if (banner) {
		banner.classList.add("hidden");
		banner.classList.remove("flex");
	}
}

// Check if running in Safari on iOS
function isIOSSafari() {
	var ua = navigator.userAgent;
	return isIOS() && /Safari/.test(ua) && !/CriOS|FxiOS|OPiOS|EdgiOS/.test(ua);
}

// Create share icon element
function createShareIcon() {
	var el = document.createElement("span");
	el.className = "icon icon-share inline-block text-[var(--accent)]";
	return el;
}

// Create menu icon element
function createMenuIcon() {
	var el = document.createElement("span");
	el.className = "icon icon-menu-dots inline-block text-[var(--accent)]";
	return el;
}

// Render iOS-specific instructions
function renderIOSInstructions(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = t("pwa:install.title");
	container.appendChild(title);

	var steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	var step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode(t("pwa:ios.step1")));
	var strong1 = document.createElement("strong");
	strong1.textContent = t("pwa:ios.step1Button");
	step1.appendChild(strong1);
	step1.appendChild(document.createTextNode(t("pwa:ios.step1After")));
	step1.appendChild(createShareIcon());
	steps.appendChild(step1);

	var step2 = document.createElement("li");
	step2.textContent = t("pwa:ios.step2");
	steps.appendChild(step2);

	container.appendChild(steps);

	if (!isIOSSafari()) {
		var note = document.createElement("p");
		note.className = "text-xs text-[var(--muted)] mt-2";
		note.textContent = t("pwa:ios.safariTip");
		container.appendChild(note);
	}
}

// Render Android-specific instructions (for non-Chrome browsers)
function renderAndroidInstructions(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = t("pwa:install.title");
	container.appendChild(title);

	var steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	var step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode(t("pwa:android.step1")));
	step1.appendChild(createMenuIcon());
	steps.appendChild(step1);

	var step2 = document.createElement("li");
	step2.textContent = t("pwa:android.step2");
	steps.appendChild(step2);

	container.appendChild(steps);
}

// Render native install prompt (Android Chrome)
function renderNativePrompt(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)]";
	title.textContent = t("pwa:install.quickAccessTitle");
	container.appendChild(title);

	var desc = document.createElement("p");
	desc.className = "text-xs text-[var(--muted)] mt-1";
	desc.textContent = t("pwa:install.quickAccessDesc");
	container.appendChild(desc);
}

// Handle install button click
async function handleInstall() {
	var result = await promptInstall();
	if (result.outcome === "accepted") {
		hideBanner();
	}
}

// Initialize the install banner
export function initInstallBanner() {
	// Don't show if already installed or dismissed
	if (isStandalone() || isDismissed()) {
		return;
	}

	var banner = getBanner();
	if (!banner) return;

	var instructions = banner.querySelector("[data-instructions]");
	var installBtn = banner.querySelector("[data-install-btn]");
	var dismissBtn = banner.querySelector("[data-dismiss-btn]");

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
