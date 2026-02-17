const { expect, test } = require("@playwright/test");
const { watchPageErrors } = require("../helpers");

function isVisible(locator) {
	return locator.isVisible().catch(() => false);
}

async function clickFirstVisibleButton(page, roleQuery) {
	const buttons = page.locator(".onboarding-card").getByRole("button", roleQuery);
	const count = await buttons.count();
	for (let i = 0; i < count; i++) {
		const button = buttons.nth(i);
		if (!(await isVisible(button))) continue;
		await button.click();
		return true;
	}
	return false;
}

async function detectOnboardingStep(page) {
	const pathname = new URL(page.url()).pathname;
	if (/^\/chats\//.test(pathname)) return "chats";

	const currentHeading = page.locator(".onboarding-card h2").first();
	if (!(await isVisible(currentHeading))) return "pending";
	const headingText = ((await currentHeading.textContent()) || "").trim();

	if (headingText === "Secure your instance") return "auth";
	if (headingText === "Set up your identity") return "identity";
	if (/^(Add LLMs|Add providers)$/.test(headingText)) return "providers";
	if (headingText === "Voice (optional)") return "voice";
	if (headingText === "Connect Telegram") return "channel";
	if (headingText === "Setup Summary") return "summary";
	return "pending";
}

async function waitForStableStep(page) {
	await expect.poll(() => detectOnboardingStep(page), { timeout: 10_000 }).not.toBe("pending");
	return detectOnboardingStep(page);
}

async function advanceToIdentityStep(page) {
	for (let i = 0; i < 6; i++) {
		const step = await detectOnboardingStep(page);
		if (step === "identity" || step === "chats" || step === "summary") return step;
		if (step === "auth") {
			await completePasswordAuthStep(page);
			continue;
		}
		if (step === "providers" || step === "voice" || step === "channel") {
			if (await clickFirstVisibleButton(page, { name: /skip for now/i })) continue;
			if (await clickFirstVisibleButton(page, { name: /continue/i })) continue;
		}
		break;
	}
	return detectOnboardingStep(page);
}

async function completePasswordAuthStep(page) {
	if (await clickFirstVisibleButton(page, { name: /^Next$/ })) return true;

	const setupCodeInput = page.getByPlaceholder("6-digit code from terminal");
	if (await isVisible(setupCodeInput)) {
		await setupCodeInput.fill("123456");

		const passwordCard = page.locator(".backend-card").filter({ hasText: "Password" }).first();
		if (await isVisible(passwordCard)) {
			await passwordCard.click({ timeout: 1_500 }).catch(() => {});
		}
	}

	const inputs = page.locator(".onboarding-card input[type='password']");
	if ((await inputs.count()) >= 2) {
		await inputs.first().fill("testpassword1");
		await inputs.nth(1).fill("testpassword1");

		if (await clickFirstVisibleButton(page, { name: /^Set password(?: & continue)?$/ })) return true;
		if (await clickFirstVisibleButton(page, { name: /^Skip$/ })) return true;
	}

	if (await clickFirstVisibleButton(page, { name: /skip for now/i })) return true;
	return false;
}

async function completeIdentityStep(page) {
	await page.getByPlaceholder("e.g. Alice").fill("TestUser");
	const botNameInput = page.getByPlaceholder("e.g. Rex");
	if (await isVisible(botNameInput)) {
		await botNameInput.fill("TestBot");
	}
	await page.getByRole("button", { name: "Continue", exact: true }).click();
}

/**
 * Onboarding tests for remote access (auth required).
 *
 * Uses a gateway started with MOLTIS_BEHIND_PROXY=true (simulates remote)
 * and MOLTIS_E2E_SETUP_CODE=123456 (deterministic setup code).
 * The test verifies that after completing auth, the WebSocket reconnects
 * immediately so subsequent RPC calls (identity save) succeed.
 */
test.describe("Onboarding with forced auth (remote)", () => {
	test.describe.configure({ mode: "serial" });

	test("completes auth and identity steps via WebSocket", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		// Fresh runs should land on /onboarding. Retries can land on /login if a
		// previous attempt already configured auth but did not finish onboarding.
		await page.goto("/");
		await expect
			.poll(() => new URL(page.url()).pathname, { timeout: 15_000 })
			.toMatch(/^\/(?:onboarding|login|chats\/.+)$/);

		let pathname = new URL(page.url()).pathname;

		if (pathname === "/login") {
			await page.getByPlaceholder("Enter password").fill("testpassword1");
			await page.getByRole("button", { name: "Sign in", exact: true }).click();
			await expect
				.poll(() => new URL(page.url()).pathname, { timeout: 15_000 })
				.toMatch(/^\/(?:onboarding|chats\/.+)$/);
			pathname = new URL(page.url()).pathname;
		}

		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		let step = await waitForStableStep(page);

		if (step === "chats") {
			expect(pageErrors).toEqual([]);
			return;
		}

		if (step === "auth") {
			await completePasswordAuthStep(page);
			step = await waitForStableStep(page);
		}

		if (step !== "identity") {
			step = await advanceToIdentityStep(page);
			if (step !== "identity") {
				expect(pageErrors).toEqual([]);
				return;
			}
		}

		// Fill identity and save — proves WS is connected (uses sendRpc)
		await completeIdentityStep(page);

		// After identity save, onboarding should advance (summary in current flow).
		await expect.poll(() => detectOnboardingStep(page), { timeout: 10_000 }).toMatch(/^(summary|chats)$/);
		if ((await detectOnboardingStep(page)) === "summary") {
			await expect(page.getByText("You: TestUser Agent:", { exact: false })).toBeVisible();
		}

		expect(pageErrors).toEqual([]);
	});
});
