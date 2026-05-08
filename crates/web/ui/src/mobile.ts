// Mobile navigation and panel handling

let navPanel: HTMLElement | null = null;
let navOverlay: HTMLElement | null = null;
let sessionsPanel: HTMLElement | null = null;
let sessionsOverlay: HTMLElement | null = null;
let sessionsToggle: HTMLElement | null = null;
let burgerBtn: HTMLElement | null = null;

// Check if we're in mobile view
function isMobile(): boolean {
	return window.innerWidth < 768;
}

// Open navigation panel
function openNav(): void {
	if (!(navPanel && navOverlay)) return;
	navPanel.classList.add("open");
	navOverlay.classList.add("visible");
	document.body.style.overflow = "hidden";
}

// Close navigation panel
function closeNav(): void {
	if (!(navPanel && navOverlay)) return;
	navPanel.classList.remove("open");
	navOverlay.classList.remove("visible");
	if (!sessionsPanel?.classList.contains("open")) {
		document.body.style.overflow = "";
	}
}

// Toggle navigation panel
function toggleNav(): void {
	if (navPanel?.classList.contains("open")) {
		closeNav();
	} else {
		openNav();
	}
}

// Open sessions panel
function openSessions(): void {
	if (!(sessionsPanel && sessionsOverlay)) return;
	sessionsPanel.classList.add("open");
	sessionsPanel.classList.remove("hidden");
	sessionsOverlay.classList.add("visible");
	if (sessionsToggle) sessionsToggle.classList.add("hidden");
	document.body.style.overflow = "hidden";
}

// Close sessions panel
function closeSessions(): void {
	if (!(sessionsPanel && sessionsOverlay)) return;
	sessionsPanel.classList.remove("open");
	sessionsOverlay.classList.remove("visible");
	if (sessionsToggle) sessionsToggle.classList.remove("hidden");
	if (!navPanel?.classList.contains("open")) {
		document.body.style.overflow = "";
	}
	// On mobile, also add hidden after transition
	if (isMobile()) {
		setTimeout(() => {
			if (!sessionsPanel?.classList.contains("open")) {
				sessionsPanel?.classList.add("hidden");
			}
		}, 300);
	}
}

// Toggle sessions panel
function toggleSessions(): void {
	if (sessionsPanel?.classList.contains("open")) {
		closeSessions();
	} else {
		openSessions();
	}
}

// Handle nav link clicks - close nav on mobile after navigation
function handleNavClick(e: Event): void {
	if (isMobile() && (e.target as HTMLElement).closest(".nav-link")) {
		closeNav();
	}
}

// Handle session item clicks - close sessions on mobile after selection
function handleSessionClick(e: Event): void {
	if (isMobile() && (e.target as HTMLElement).closest(".session-item")) {
		setTimeout(closeSessions, 100);
	}
}

// Handle resize - reset states when switching between mobile/desktop
function handleResize(): void {
	if (isMobile()) {
		// Mobile view - show toggle button if sessions panel is not open
		if (!sessionsPanel?.classList.contains("open")) {
			sessionsToggle?.classList.remove("hidden");
		}
	} else {
		// Desktop view - reset mobile-specific states
		navPanel?.classList.remove("open");
		navOverlay?.classList.remove("visible");
		sessionsOverlay?.classList.remove("visible");
		sessionsToggle?.classList.add("hidden");
		document.body.style.overflow = "";

		// On desktop, sessions panel visibility is controlled differently
		// Remove open class but don't add hidden (let desktop CSS handle it)
		sessionsPanel?.classList.remove("open");
	}
}

// Initialize mobile interactions
export function initMobile(): void {
	navPanel = document.getElementById("navPanel");
	navOverlay = document.getElementById("navOverlay");
	sessionsPanel = document.getElementById("sessionsPanel");
	sessionsOverlay = document.getElementById("sessionsOverlay");
	sessionsToggle = document.getElementById("sessionsToggle");
	burgerBtn = document.getElementById("burgerBtn");

	// Burger button toggles nav
	if (burgerBtn) {
		burgerBtn.addEventListener("click", toggleNav);
	}

	// Overlay clicks close their respective panels
	if (navOverlay) {
		navOverlay.addEventListener("click", closeNav);
	}

	if (sessionsOverlay) {
		sessionsOverlay.addEventListener("click", closeSessions);
	}

	// Sessions toggle FAB
	if (sessionsToggle) {
		sessionsToggle.addEventListener("click", toggleSessions);
		// Initially hide on desktop
		if (!isMobile()) {
			sessionsToggle.classList.add("hidden");
		}
	}

	// Close nav when clicking nav links on mobile
	if (navPanel) {
		navPanel.addEventListener("click", handleNavClick);
	}

	// Close sessions when clicking session items on mobile
	if (sessionsPanel) {
		sessionsPanel.addEventListener("click", handleSessionClick);
	}

	// Handle resize events
	window.addEventListener("resize", handleResize);

	// Escape key closes panels
	document.addEventListener("keydown", (e: KeyboardEvent) => {
		if (e.key === "Escape") {
			if (navPanel?.classList.contains("open")) {
				closeNav();
			}
			if (sessionsPanel?.classList.contains("open") && isMobile()) {
				closeSessions();
			}
		}
	});

	// Touch gestures for bottom sheet (sessions panel)
	if (sessionsPanel) {
		setupBottomSheetGestures(sessionsPanel);
	}
}

// Simple swipe-down gesture to close bottom sheet
function setupBottomSheetGestures(panel: HTMLElement): void {
	let startY = 0;
	let currentY = 0;
	let isDragging = false;

	panel.addEventListener(
		"touchstart",
		(e: TouchEvent) => {
			// Only initiate drag from the top area (drag handle region)
			const touch = e.touches[0];
			const panelRect = panel.getBoundingClientRect();
			if (touch.clientY - panelRect.top < 40) {
				startY = touch.clientY;
				isDragging = true;
			}
		},
		{ passive: true },
	);

	panel.addEventListener(
		"touchmove",
		(e: TouchEvent) => {
			if (!isDragging) return;
			currentY = e.touches[0].clientY;
			const deltaY = currentY - startY;

			// Only allow dragging down
			if (deltaY > 0) {
				panel.style.transform = `translateY(${deltaY}px)`;
			}
		},
		{ passive: true },
	);

	panel.addEventListener(
		"touchend",
		() => {
			if (!isDragging) return;
			isDragging = false;

			const deltaY = currentY - startY;
			panel.style.transform = "";

			// Close if dragged more than 100px down
			if (deltaY > 100) {
				closeSessions();
			}
		},
		{ passive: true },
	);
}

// Export functions for external use
export { closeNav, closeSessions, isMobile, openNav, openSessions, toggleNav, toggleSessions };
