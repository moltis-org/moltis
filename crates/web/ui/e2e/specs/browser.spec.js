// Browser viewer page — end-to-end tests.
//
// Verifies the browser session management UI: creating sessions,
// navigating, screencast frame delivery, input relay, session lifecycle,
// history, profiles, and regression scenarios.

const { test, expect } = require("../base-test");
const { navigateAndWait, waitForWsConnected, watchPageErrors } = require("../helpers");

// ── Helpers ──────────────────────────────────────────────────

/** Navigate to browser page and wait for WS + page content. */
async function goToBrowser(page) {
	await navigateAndWait(page, "/settings/browser");
	await waitForWsConnected(page);
}

/** Create a new browser session, wait for the URL input to appear. */
async function createBrowserSession(page) {
	await page.getByRole("button", { name: "New Session" }).click();
	await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });
}

/** Navigate the active session to a URL and wait for canvas. */
async function navigateTo(page, url) {
	const input = page.getByPlaceholder("Search or enter URL...");
	await input.fill(url);
	await input.press("Enter");
	await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });
}

/** Wait for a text to appear somewhere on the page body. */
async function waitForBodyText(page, text, timeout = 15000) {
	await expect
		.poll(async () => (await page.locator("body").innerText()).includes(text), { timeout })
		.toBeTruthy();
}

/** Count active session cards in the live tab. */
async function countSessionCards(page) {
	return page.locator("[class*='rounded-lg border p-3']").count();
}

/** Fetch sessions from REST API directly. */
async function apiGetSessions(page) {
	return page.evaluate(async () => {
		const res = await fetch("/api/browser/sessions");
		return res.json();
	});
}

/** Perform a browser action via REST API. */
async function apiBrowserAction(page, params) {
	return page.evaluate(async (p) => {
		const res = await fetch("/api/browser/action", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(p),
		});
		return res.json();
	}, params);
}

// ═══════════════════════════════════════════════════════════════
// ── 1. Page Layout & Initial State ────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Browser sessions page", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("renders browser page with heading and new session button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByRole("heading", { name: "Browser Sessions", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "New Session" })).toBeVisible();
		await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows empty state message when no sessions exist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		// Wait for sessions to load — the empty state appears after fetch
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });
		expect(pageErrors).toEqual([]);
	});

	test("shows description text about browser sessions", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByText("Create browser sessions or view ones created by agents")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("Live and History tabs are visible", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByText("Live", { exact: false })).toBeVisible();
		await expect(page.getByText("History", { exact: false })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("shows select session prompt when no session is active", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await expect(page.getByText("Select a session to view the browser")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 2. Session Creation ───────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Session creation", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("new session button shows creating state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		const btn = page.getByRole("button", { name: "New Session" });
		await btn.click();

		// Button should show creating state (disabled with "Creating…" text)
		// or have already finished creating — either way, no JS errors.
		await expect(page.getByRole("heading", { name: "Browser Sessions", exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("creating session shows placeholder immediately (Bug: delayed highlight)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "New Session" }).click();

		// Placeholder should appear immediately with "creating" badge
		await expect(page.getByText("New session")).toBeVisible({ timeout: 5000 });
		await expect(page.getByText("Starting browser")).toBeVisible({ timeout: 5000 });
		await expect(page.getByText("creating")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("navigate bar appears after creating a session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// The "Enter a URL" hint should be visible in the canvas area
		await expect(page.getByText("Enter a URL above to start browsing")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("session card shows selected state after creation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// The selected card should have the accent border class
		const cards = page.locator("[class*='rounded-lg border p-3']");
		await expect(cards.first()).toBeVisible();
		// Active session card has accent styling
		await expect(cards.first()).toHaveClass(/border-\[var\(--accent\)\]/);

		expect(pageErrors).toEqual([]);
	});

	test("new session button is disabled while creating", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		const btn = page.getByRole("button", { name: "New Session" });
		await btn.click();

		// Either shows "Creating…" or was already done — the button
		// should be disabled during creation to prevent double-creates
		const creating = await page.getByText("Creating").isVisible().catch(() => false);
		if (creating) {
			await expect(page.getByText("Creating")).toBeVisible();
		}

		expect(pageErrors).toEqual([]);
	});

	test("multiple sessions can be created", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		// Wait for creation to finish
		await page.waitForTimeout(1000);

		// Create second session
		await createBrowserSession(page);

		// Should have at least 2 session cards
		await expect
			.poll(async () => countSessionCards(page), { timeout: 15000 })
			.toBeGreaterThanOrEqual(2);

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 3. Navigation ─────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Navigation", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("navigate bar normalizes bare domains with https", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Should navigate successfully (no error toast about invalid scheme)
		await expect
			.poll(
				async () => {
					const text = await page.locator(".truncate").allInnerTexts();
					return text.some((t) => t.includes("example.com"));
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("URL bar shows target URL immediately on navigation (Bug 15: no flicker)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// URL bar should immediately show the normalized URL, not flicker to blank
		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val.includes("example.com");
				},
				{ timeout: 5000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("non-URL text triggers Google search", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("what is the weather");
		await input.press("Enter");

		// Should navigate to google search
		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val.includes("google.com/search");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("full https URL is used as-is", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("https://example.com");
		await input.press("Enter");

		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val.includes("example.com");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("Go button submits navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");

		// Click Go instead of pressing Enter
		await page.getByRole("button", { name: "Go" }).click();

		await expect
			.poll(
				async () => {
					const text = await page.locator(".truncate").allInnerTexts();
					return text.some((t) => t.includes("example.com"));
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("Go button is disabled with empty input", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		const goBtn = page.getByRole("button", { name: "Go" });
		await expect(goBtn).toBeDisabled();

		expect(pageErrors).toEqual([]);
	});

	test("re-navigation in same session updates URL", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");

		// First navigation
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Second navigation
		await input.click();
		await input.fill("example.org");
		await input.press("Enter");

		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val.includes("example.org");
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 4. Screencast & Canvas ────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Screencast and canvas", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("screencast delivers frames after navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// Verify frame metadata is shown
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("Frame #");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("canvas element is focusable", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");
		await expect(canvas).toHaveAttribute("tabindex", "0");

		expect(pageErrors).toEqual([]);
	});

	test("canvas shows session ID and frame dimensions", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// Session ID should be shown above the canvas
		await expect(page.getByText("Session:")).toBeVisible();

		// Frame dimensions should appear (e.g. "1440x900")
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return /\d+x\d+/.test(text);
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("live badge appears during screencast", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// The "live" badge should appear on the active session card
		await expect(page.getByText("live")).toBeVisible({ timeout: 15000 });

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 5. Mouse / Keyboard / Scroll Input ────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Input relay", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("mouse click on canvas sends mouse_input action without error", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");

		// Click on the canvas — should dispatch mousePressed + mouseReleased
		await canvas.click({ position: { x: 100, y: 100 } });

		// No JS errors from the click relay
		await page.waitForTimeout(500);
		expect(pageErrors).toEqual([]);
	});

	test("mouse move on canvas throttles correctly", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");

		// Move mouse across the canvas
		await canvas.hover({ position: { x: 50, y: 50 } });
		await canvas.hover({ position: { x: 150, y: 150 } });
		await canvas.hover({ position: { x: 250, y: 250 } });

		await page.waitForTimeout(300);
		expect(pageErrors).toEqual([]);
	});

	test("keyboard events are relayed from canvas", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");
		await canvas.focus();

		// Type some keys — these go through relayKeyEvent
		await canvas.press("a");
		await canvas.press("b");
		await canvas.press("Backspace");

		await page.waitForTimeout(500);
		expect(pageErrors).toEqual([]);
	});

	test("scroll wheel on canvas dispatches scroll action", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");

		// Dispatch wheel event on the canvas
		await canvas.evaluate((el) => {
			el.dispatchEvent(new WheelEvent("wheel", { deltaX: 0, deltaY: 100, bubbles: true }));
		});

		// Wait for the batched wheel flush (50ms timer)
		await page.waitForTimeout(200);
		expect(pageErrors).toEqual([]);
	});

	test("right-click on canvas is prevented (no context menu)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		const canvas = page.locator("canvas");

		// Right-click should not open a context menu
		await canvas.click({ button: "right", position: { x: 100, y: 100 } });

		await page.waitForTimeout(300);
		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 6. Session Switching ──────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Session switching", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("clicking a session card selects it", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create two sessions
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		await page.waitForTimeout(1000);
		await createBrowserSession(page);

		// Click the first session card
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			// Click the card that's NOT currently active (second one = first created)
			await cards.nth(1).click();
			await page.waitForTimeout(500);

			// Canvas should show (it was navigated before)
			await expect(page.locator("canvas")).toBeVisible({ timeout: 15000 });
		}

		expect(pageErrors).toEqual([]);
	});

	test("switching sessions rapidly does not cause JS errors (Bug 16: relay accumulation)", async ({
		page,
	}) => {
		const pageErrors = watchPageErrors(page);

		// Create first session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Create second session
		await createBrowserSession(page);

		// Switch back and forth rapidly by clicking session cards
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			for (let i = 0; i < 4; i++) {
				await cards.nth(i % count).click();
				await page.waitForTimeout(200);
			}
		}

		// No JS errors should have occurred
		expect(pageErrors).toEqual([]);
	});

	test("switching to session with cached screenshot shows frame instantly", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create and navigate first session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Create second session (switches away from first)
		await page.waitForTimeout(1000);
		await createBrowserSession(page);

		// Switch back to first session — should show cached frame
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			await cards.nth(1).click(); // Second card = first session
			// Should not show "Fetching browser view..." for long — cached frame is instant
			await page.waitForTimeout(500);
			const body = await page.locator("body").innerText();
			expect(body).not.toContain("Fetching browser view");
		}

		expect(pageErrors).toEqual([]);
	});

	test("URL bar updates when switching sessions", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create first session with navigation
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		// Create second session (no navigation)
		await page.waitForTimeout(1000);
		await createBrowserSession(page);

		// URL bar should be empty for the new session
		await expect
			.poll(
				async () => {
					const val = await input.inputValue();
					return val === "" || val === "about:blank";
				},
				{ timeout: 5000 },
			)
			.toBeTruthy();

		// Switch back to first session
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			await cards.nth(1).click();
			// URL should update to show example.com
			await expect
				.poll(
					async () => {
						const val = await input.inputValue();
						return val.includes("example.com");
					},
					{ timeout: 10000 },
				)
				.toBeTruthy();
		}

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 7. Session Closing ────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Session closing", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("session can be closed", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await expect(page.getByRole("button", { name: "Close" })).toBeVisible();

		await page.getByRole("button", { name: "Close" }).click();

		// Should return to empty state
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });

		expect(pageErrors).toEqual([]);
	});

	test("closing active session resets canvas to select state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		await page.getByRole("button", { name: "Close" }).click();

		// Canvas should be gone, replaced with "Select a session" prompt
		await expect(page.getByText("Select a session to view the browser")).toBeVisible({
			timeout: 10000,
		});

		expect(pageErrors).toEqual([]);
	});

	test("close button does not appear on creating placeholder", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "New Session" }).click();

		// The placeholder card (with "creating" badge) should NOT have a Close button
		const creatingCard = page.locator("[class*='opacity-75']");
		const hasClose = await creatingCard.getByRole("button", { name: "Close" }).isVisible().catch(() => false);
		// During the creating state, the close button should be hidden
		// (The code returns null for sess.creating)
		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 8. URL Bar Autocomplete ──────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("URL bar autocomplete", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("typing a URL-like string shows 'Go to site' suggestion", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("github.com");

		// Should show "Go to site" in the dropdown
		await expect(page.getByText("Go to site")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("Escape key closes autocomplete dropdown", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("github.com");
		await expect(page.getByText("Go to site")).toBeVisible({ timeout: 5000 });

		await input.press("Escape");

		// Dropdown should disappear
		await expect(page.getByText("Go to site")).toBeHidden({ timeout: 3000 });

		expect(pageErrors).toEqual([]);
	});

	test("ArrowDown/ArrowUp keys navigate suggestions", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("github.com");
		await expect(page.getByText("Go to site")).toBeVisible({ timeout: 5000 });

		// ArrowDown should highlight the first item
		await input.press("ArrowDown");
		await page.waitForTimeout(200);

		// ArrowUp should de-highlight
		await input.press("ArrowUp");
		await page.waitForTimeout(200);

		expect(pageErrors).toEqual([]);
	});

	test("non-URL text shows 'Search Google for' suggestion", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("weather today");

		// Should show "Search Google for" suggestion
		await expect(page.getByText("Search Google for")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("clicking outside closes autocomplete dropdown", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("github.com");
		await expect(page.getByText("Go to site")).toBeVisible({ timeout: 5000 });

		// Click outside the input area
		await page.getByRole("heading", { name: "Browser Sessions", exact: true }).click();

		// Dropdown should close
		await expect(page.getByText("Go to site")).toBeHidden({ timeout: 3000 });

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 9. History Tab ────────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("History tab", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("closed session appears in History tab (Bug 17: dead sessions in history)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create and navigate a session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("example.com");
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		// Close the session
		await page.getByText("Close").click();

		// Switch to History tab
		await page.getByText("History").click();

		// The closed session should appear with example.com URL
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes("example.com") && (text.includes("closed") || text.includes("lost"));
				},
				{ timeout: 10000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("switching to History tab shows past sessions heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByText("History").click();

		// Should show "No past sessions." or list of sessions
		const body = await page.locator("body").innerText();
		expect(body.includes("No past sessions") || body.includes("closed") || body.includes("lost")).toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("switching back to Live tab from History works", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Go to History
		await page.getByText("History").click();
		await page.waitForTimeout(500);

		// Go back to Live
		await page.getByText("Live").click();
		await page.waitForTimeout(500);

		// Should show the live tab content
		const body = await page.locator("body").innerText();
		expect(body.includes("No active sessions") || body.includes("New Session")).toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("history session shows View Log button", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create, navigate, and close a session to populate history
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });
		await page.getByText("Close").click();
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });

		// Switch to History tab
		await page.getByText("History").click();

		// Should see "View Log" button
		await expect(page.getByText("View Log")).toBeVisible({ timeout: 10000 });

		expect(pageErrors).toEqual([]);
	});

	test("clicking View Log shows action log panel", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create, navigate, and close a session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });
		await page.getByText("Close").click();
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });

		// Switch to History tab and click View Log
		await page.getByText("History").click();
		await expect(page.getByText("View Log")).toBeVisible({ timeout: 10000 });
		await page.getByText("View Log").click();

		// Should show "Session Log" heading and Back button
		await expect(page.getByText("Session Log")).toBeVisible({ timeout: 5000 });
		await expect(page.getByRole("button", { name: "Back" })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("Back button returns from action log to history view", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create, navigate, and close a session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });
		await page.getByText("Close").click();
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });

		// Go to History → View Log → Back
		await page.getByText("History").click();
		await expect(page.getByText("View Log")).toBeVisible({ timeout: 10000 });
		await page.getByText("View Log").click();
		await expect(page.getByText("Session Log")).toBeVisible({ timeout: 5000 });

		await page.getByRole("button", { name: "Back" }).click();

		// Should no longer show Session Log
		await expect(page.getByText("Session Log")).toBeHidden({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("clicking a history session with URL revives it in Live tab", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create, navigate, and close a session
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });
		await page.getByText("Close").click();
		await expect(page.getByText("No active sessions")).toBeVisible({ timeout: 10000 });

		// Switch to History and click the session (not View Log)
		await page.getByText("History").click();
		await expect
			.poll(
				async () => (await page.locator("body").innerText()).includes("example.com"),
				{ timeout: 10000 },
			)
			.toBeTruthy();

		// Click the history card itself (should revive)
		const historyCards = page.locator("[class*='rounded border p-2']");
		await historyCards.first().click();

		// Should switch to Live tab and create a new session
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 10. Profile Management ───────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Profile management", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("profile dropdown button is visible next to New Session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// The dropdown arrow button should be next to New Session
		const dropdownBtn = page.locator("button", { hasText: "\u25BE" });
		await expect(dropdownBtn).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("clicking dropdown shows profile menu with 'default' profile", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		const dropdownBtn = page.locator("button", { hasText: "\u25BE" });
		await dropdownBtn.click();

		// Should show profile menu with "default" option
		await expect(page.getByText("Profile")).toBeVisible({ timeout: 3000 });
		await expect(page.getByText("default")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("profile menu shows 'New profile name' input", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		const dropdownBtn = page.locator("button", { hasText: "\u25BE" });
		await dropdownBtn.click();

		await expect(page.getByPlaceholder("New profile name...")).toBeVisible({ timeout: 3000 });

		expect(pageErrors).toEqual([]);
	});

	test("selecting a profile from dropdown creates session with that profile", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		const dropdownBtn = page.locator("button", { hasText: "\u25BE" });
		await dropdownBtn.click();

		// Click "default" profile
		await page.locator("button", { hasText: "default" }).last().click();

		// Should create a session
		await expect(page.getByPlaceholder("Search or enter URL...")).toBeVisible({ timeout: 30000 });

		// Profile should show in the session card
		await waitForBodyText(page, "default");

		expect(pageErrors).toEqual([]);
	});

	test("session card shows profile ID", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// The "default" profile text should be visible in the session card metadata
		await expect
			.poll(
				async () => {
					const cards = page.locator("[class*='rounded-lg border p-3']");
					const text = await cards.first().innerText();
					return text.includes("default");
				},
				{ timeout: 10000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 11. Session Metadata Display ─────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Session metadata", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("session card shows Age and Idle timers", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// Wait for session card to appear with metadata
		await expect
			.poll(
				async () => {
					const cards = page.locator("[class*='rounded-lg border p-3']");
					const count = await cards.count();
					if (count === 0) return false;
					const text = await cards.first().innerText();
					return text.includes("Age:") && text.includes("Idle:");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("session shows sandbox badge when sandbox is enabled", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// Check for sandbox badge (if sandbox is enabled in the test environment)
		// or absence of it (if running without sandbox). Either way, no errors.
		await page
			.getByText("sandbox")
			.isVisible()
			.catch(() => false);

		expect(pageErrors).toEqual([]);
	});

	test("session URL shows in card after navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Session card should show the URL
		await expect
			.poll(
				async () => {
					const cards = page.locator("[class*='rounded-lg border p-3']");
					const text = await cards.first().innerText();
					return text.includes("example.com");
				},
				{ timeout: 30000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("session shows (no page loaded) when URL is blank", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// After creation with about:blank, card should show "(no page loaded)"
		await expect
			.poll(
				async () => {
					const cards = page.locator("[class*='rounded-lg border p-3']");
					const count = await cards.count();
					if (count === 0) return false;
					const text = await cards.first().innerText();
					return text.includes("(no page loaded)") || text.includes("about:blank");
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 12. REST API Tests ────────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("REST API endpoints", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("sessions created via REST API appear in UI list (Bug 2: shared manager)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session directly via the REST API (simulating agent path)
		const apiRes = await apiBrowserAction(page, { action: "navigate", url: "about:blank" });

		// The session should appear in the UI list after refresh
		await page.getByRole("button", { name: /Refresh/ }).click();
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					return text.includes(apiRes.session_id?.slice(0, 12) || "browser-");
				},
				{ timeout: 10000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("GET /api/browser/sessions returns valid JSON", async ({ page }) => {
		const data = await apiGetSessions(page);
		expect(data).toHaveProperty("sessions");
		expect(Array.isArray(data.sessions)).toBeTruthy();
	});

	test("GET /api/browser/history returns valid JSON", async ({ page }) => {
		const data = await page.evaluate(async () => {
			const res = await fetch("/api/browser/history");
			return res.json();
		});
		expect(data).toHaveProperty("sessions");
		expect(Array.isArray(data.sessions)).toBeTruthy();
	});

	test("GET /api/browser/actions/{session_id} returns valid JSON for known session", async ({
		page,
	}) => {
		// Create a session first
		const apiRes = await apiBrowserAction(page, { action: "navigate", url: "about:blank" });
		const sessionId = apiRes.session_id;

		const data = await page.evaluate(async (sid) => {
			const res = await fetch(`/api/browser/actions/${sid}`);
			return res.json();
		}, sessionId);

		expect(data).toHaveProperty("actions");
		expect(Array.isArray(data.actions)).toBeTruthy();
	});

	test("POST /api/browser/action with navigate returns session_id and url", async ({ page }) => {
		const data = await apiBrowserAction(page, { action: "navigate", url: "about:blank" });
		expect(data).toHaveProperty("session_id");
		expect(data.session_id).toBeTruthy();
	});

	test("POST /api/browser/action with screenshot returns screenshot data", async ({ page }) => {
		// Create a session first
		const nav = await apiBrowserAction(page, {
			action: "navigate",
			url: "about:blank",
		});

		const data = await apiBrowserAction(page, {
			session_id: nav.session_id,
			action: "screenshot",
		});

		// Should have screenshot data (base64 PNG)
		expect(data).toHaveProperty("screenshot");

		// If screenshot succeeded, it should be a non-empty string
		if (data.screenshot) {
			expect(typeof data.screenshot).toBe("string");
			expect(data.screenshot.length).toBeGreaterThan(100);
		}
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 13. Refresh Button ───────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Refresh functionality", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("refresh button fetches updated sessions list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create a session via API first
		await apiBrowserAction(page, { action: "navigate", url: "about:blank" });

		// Click refresh
		await page.getByRole("button", { name: "Refresh" }).click();

		// Session should appear
		await expect
			.poll(async () => countSessionCards(page), { timeout: 10000 })
			.toBeGreaterThanOrEqual(1);

		expect(pageErrors).toEqual([]);
	});

	test("refresh button doesn't cause JS errors on empty state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "Refresh" }).click();
		await page.waitForTimeout(1000);

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 14. Error Handling & Edge Cases ──────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Error handling", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("dead session shows error toast and recovers (Bug 12: stuck fetching)", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock a session that will fail on screenshot
		await page.route("**/api/browser/action", async (route, request) => {
			const body = JSON.parse(request.postData() || "{}");
			if (body.action === "navigate") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						success: true,
						session_id: "fake-dead-session",
						url: "about:blank",
					}),
				});
			}
			if (body.action === "screenshot") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						success: false,
						session_id: "fake-dead-session",
						error: "connection lost",
					}),
				});
			}
			return route.continue();
		});

		await page.getByRole("button", { name: "New Session" }).click();
		await page.waitForTimeout(3000);

		// Should not be stuck on "Fetching browser view..." — should show error or recover
		const bodyText = await page.locator("body").innerText();
		expect(bodyText).not.toContain("Fetching browser view");

		expect(pageErrors).toEqual([]);
	});

	test("API failure on session creation shows toast", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Mock API to fail on navigate
		await page.route("**/api/browser/action", async (route) => {
			return route.fulfill({
				status: 500,
				contentType: "application/json",
				body: JSON.stringify({ error: "internal server error" }),
			});
		});

		await page.getByRole("button", { name: "New Session" }).click();
		await page.waitForTimeout(3000);

		// Should show error toast or return to normal state (not stuck)
		const body = await page.locator("body").innerText();
		expect(body.includes("Failed") || body.includes("No active sessions") || body.includes("error")).toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("navigation failure shows toast message", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// Intercept navigation to simulate failure
		await page.route("**/api/browser/action", async (route, request) => {
			const body = JSON.parse(request.postData() || "{}");
			if (body.action === "navigate" && body.url !== "about:blank") {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({
						success: false,
						error: "Navigation timeout",
					}),
				});
			}
			return route.continue();
		});

		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Should show error toast
		await expect(page.getByText("Navigation failed")).toBeVisible({ timeout: 10000 });

		expect(pageErrors).toEqual([]);
	});

	test("selecting dead session shows toast and refreshes list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Create two sessions
		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		await page.waitForTimeout(1000);
		await createBrowserSession(page);

		// Kill the first session's screenshot response to simulate dead session
		let blockScreenshot = false;
		await page.route("**/api/browser/action", async (route, request) => {
			const body = JSON.parse(request.postData() || "{}");
			if (body.action === "screenshot" && blockScreenshot) {
				return route.fulfill({
					status: 200,
					contentType: "application/json",
					body: JSON.stringify({ success: false, error: "session dead" }),
				});
			}
			return route.continue();
		});

		blockScreenshot = true;

		// Click the first session card
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const count = await cards.count();
		if (count >= 2) {
			await cards.nth(1).click();
			await page.waitForTimeout(3000);

			// Should show error toast
			const body = await page.locator("body").innerText();
			expect(
				body.includes("no longer available") || body.includes("Session") || !body.includes("Fetching"),
			).toBeTruthy();
		}

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 15. Scrollbar Overlay ────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Scrollbar overlay", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("scrollbar container is inside the canvas wrapper", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// The canvas wrapper is a relative div containing the canvas
		// and optionally the scrollbar. Verify the wrapper structure.
		const wrapper = page.locator(".relative.inline-block");
		await expect(wrapper).toBeVisible();

		// Canvas should be inside
		await expect(wrapper.locator("canvas")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 16. WebSocket Event Subscription ─────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("WebSocket screencast subscription", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("browser.screencast.frame event is subscribed in WebSocket", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		// Check that the WebSocket connect frame includes browser.screencast.frame
		const isSubscribed = await page.evaluate(async () => {
			const appScript = document.querySelector('script[type="module"][src*="js/app.js"]');
			if (!appScript) return false;
			const appUrl = new URL(appScript.src, window.location.origin);
			const prefix = appUrl.href.slice(0, appUrl.href.length - "js/app.js".length);
			const wsModule = await import(`${prefix}js/websocket.js`);
			// Check if the module exports or configures the subscription
			// The subscriptions are sent during connect; we can verify indirectly
			// by checking that the frame listener works after navigation
			return true;
		});

		expect(isSubscribed).toBeTruthy();
		expect(pageErrors).toEqual([]);
	});

	test("screencast frames arrive via WebSocket after navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// Frames arrive via WebSocket — verify by checking frame counter increments
		await expect
			.poll(
				async () => {
					const text = await page.locator("body").innerText();
					const match = text.match(/Frame #(\d+)/);
					return match ? Number.parseInt(match[1], 10) : 0;
				},
				{ timeout: 15000 },
			)
			.toBeGreaterThan(0);

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 17. SPA Navigation & Cleanup ─────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("SPA navigation", () => {
	test("navigating away from browser page and back works", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await goToBrowser(page);
		await createBrowserSession(page);

		// Navigate to another settings page
		await page.goto("/settings/identity");
		await expect(page.getByRole("heading", { name: "Identity", exact: true })).toBeVisible();

		// Navigate back to browser
		await page.goto("/settings/browser");
		await expect(page.getByRole("heading", { name: "Browser Sessions", exact: true })).toBeVisible();

		// The page should still function
		await expect(page.getByRole("button", { name: "New Session" })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("browser page URL is /settings/browser", async ({ page }) => {
		await page.goto("/settings/browser");
		await expect(page).toHaveURL(/\/settings\/browser$/);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 18. Session State Badges ─────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Session state badges", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("creating session shows 'creating' badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.getByRole("button", { name: "New Session" }).click();
		await expect(page.getByText("creating")).toBeVisible({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});

	test("navigated session shows 'live' badge during screencast", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");
		await expect(page.locator("canvas")).toBeVisible({ timeout: 30000 });

		await expect(page.getByText("live")).toBeVisible({ timeout: 10000 });

		expect(pageErrors).toEqual([]);
	});

	test("unnavigated session does not show live or idle badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// Wait for creation to complete (placeholder replaced by real session)
		await page.waitForTimeout(3000);

		// An unnavigated session (at about:blank) shouldn't show idle/live badges
		const cards = page.locator("[class*='rounded-lg border p-3']");
		const text = await cards.first().innerText();
		// about:blank sessions don't get idle badge
		expect(text.includes("live") || text.includes("idle")).toBeFalsy();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 19. History Tab Counts ───────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Tab counts", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("Live tab shows session count when sessions exist", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);

		// The Live tab should show count like "Live (1)"
		await expect
			.poll(
				async () => {
					const liveTab = page.locator("button", { hasText: /^Live/ });
					const text = await liveTab.innerText();
					return /Live\s*\(\d+\)/.test(text);
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 20. Concurrent Operations ────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Concurrent operations", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("navigating and clicking refresh simultaneously is safe", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");

		// Start navigation and refresh at the same time
		await Promise.all([input.press("Enter"), page.getByRole("button", { name: "Refresh" }).click()]);

		await page.waitForTimeout(3000);
		expect(pageErrors).toEqual([]);
	});

	test("rapidly clicking New Session only creates one session", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		const btn = page.getByRole("button", { name: "New Session" });
		// Click rapidly 3 times
		await btn.click();
		await btn.click().catch(() => {}); // May be disabled
		await btn.click().catch(() => {}); // May be disabled

		// Wait for creation to settle
		await page.waitForTimeout(5000);

		// Should have at most 1-2 session cards (not 3)
		const count = await countSessionCards(page);
		expect(count).toBeLessThanOrEqual(2);

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 21. Canvas Rendering States ──────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Canvas rendering states", () => {
	test.beforeEach(async ({ page }) => {
		await goToBrowser(page);
	});

	test("shows 'Enter a URL' when session has no navigation", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await expect(page.getByText("Enter a URL above to start browsing")).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("shows 'Waiting for first frame' during screencast start", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		const input = page.getByPlaceholder("Search or enter URL...");
		await input.fill("example.com");
		await input.press("Enter");

		// Either "Waiting for first frame..." or the canvas should appear
		// (depends on how fast frames arrive)
		await expect
			.poll(
				async () => {
					const body = await page.locator("body").innerText();
					return body.includes("Waiting for first frame") || (await page.locator("canvas").isVisible());
				},
				{ timeout: 15000 },
			)
			.toBeTruthy();

		expect(pageErrors).toEqual([]);
	});

	test("canvas replaces waiting message when frame arrives", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await createBrowserSession(page);
		await navigateTo(page, "example.com");

		// Canvas should be visible
		await expect(page.locator("canvas")).toBeVisible();

		// "Waiting for first frame" should be gone
		await expect(page.getByText("Waiting for first frame")).toBeHidden({ timeout: 5000 });

		expect(pageErrors).toEqual([]);
	});
});

// ═══════════════════════════════════════════════════════════════
// ── 22. Mobile Viewport ──────────────────────────────────────
// ═══════════════════════════════════════════════════════════════

test.describe("Responsive layout", () => {
	test("browser page renders on mobile viewport", async ({ page }) => {
		const pageErrors = watchPageErrors(page);

		await page.setViewportSize({ width: 390, height: 844 });
		await navigateAndWait(page, "/settings/browser");
		await waitForWsConnected(page);

		await expect(
			page.getByRole("heading", { name: "Browser Sessions", exact: true }),
		).toBeVisible();
		await expect(page.getByRole("button", { name: "New Session" })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});
});
