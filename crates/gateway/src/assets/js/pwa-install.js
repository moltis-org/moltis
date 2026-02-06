// PWA Install Banner - handles "Add to Homescreen" prompts

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

// Create share icon SVG element
function createShareIcon() {
	var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
	svg.setAttribute("width", "16");
	svg.setAttribute("height", "16");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "1.5");
	svg.classList.add("inline-block", "text-[var(--accent)]");

	var path = document.createElementNS("http://www.w3.org/2000/svg", "path");
	path.setAttribute(
		"d",
		"M9 8.25H7.5a2.25 2.25 0 0 0-2.25 2.25v9a2.25 2.25 0 0 0 2.25 2.25h9a2.25 2.25 0 0 0 2.25-2.25v-9a2.25 2.25 0 0 0-2.25-2.25H15m0-3-3-3m0 0-3 3m3-3v12",
	);
	svg.appendChild(path);
	return svg;
}

// Create menu icon SVG element
function createMenuIcon() {
	var svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
	svg.setAttribute("width", "16");
	svg.setAttribute("height", "16");
	svg.setAttribute("viewBox", "0 0 24 24");
	svg.setAttribute("fill", "none");
	svg.setAttribute("stroke", "currentColor");
	svg.setAttribute("stroke-width", "2");
	svg.classList.add("inline-block", "text-[var(--accent)]");

	var circles = [
		{ cx: "12", cy: "5" },
		{ cx: "12", cy: "12" },
		{ cx: "12", cy: "19" },
	];
	for (var c of circles) {
		var circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
		circle.setAttribute("cx", c.cx);
		circle.setAttribute("cy", c.cy);
		circle.setAttribute("r", "1");
		svg.appendChild(circle);
	}
	return svg;
}

// Render iOS-specific instructions
function renderIOSInstructions(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = "Install moltis on your device";
	container.appendChild(title);

	var steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	var step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode("Tap the "));
	var strong1 = document.createElement("strong");
	strong1.textContent = "Share";
	step1.appendChild(strong1);
	step1.appendChild(document.createTextNode(" button "));
	step1.appendChild(createShareIcon());
	steps.appendChild(step1);

	var step2 = document.createElement("li");
	step2.textContent = 'Scroll down and tap "Add to Home Screen"';
	steps.appendChild(step2);

	container.appendChild(steps);

	if (!isIOSSafari()) {
		var note = document.createElement("p");
		note.className = "text-xs text-[var(--muted)] mt-2";
		note.textContent = "Tip: Open this page in Safari for the best experience.";
		container.appendChild(note);
	}
}

// Render Android-specific instructions (for non-Chrome browsers)
function renderAndroidInstructions(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)] mb-2";
	title.textContent = "Install moltis on your device";
	container.appendChild(title);

	var steps = document.createElement("ol");
	steps.className = "text-xs text-[var(--text)] space-y-1.5 list-decimal list-inside";

	var step1 = document.createElement("li");
	step1.className = "flex items-center gap-1.5";
	step1.appendChild(document.createTextNode("Tap the menu button "));
	step1.appendChild(createMenuIcon());
	steps.appendChild(step1);

	var step2 = document.createElement("li");
	step2.textContent = 'Select "Add to Home Screen" or "Install App"';
	steps.appendChild(step2);

	container.appendChild(steps);
}

// Render native install prompt (Android Chrome)
function renderNativePrompt(container) {
	while (container.firstChild) container.removeChild(container.firstChild);

	var title = document.createElement("p");
	title.className = "text-sm font-medium text-[var(--text-strong)]";
	title.textContent = "Install moltis for quick access";
	container.appendChild(title);

	var desc = document.createElement("p");
	desc.className = "text-xs text-[var(--muted)] mt-1";
	desc.textContent = "Get a native app experience with offline support.";
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
