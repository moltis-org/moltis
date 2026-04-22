const { expect, test } = require("../base-test");
const { watchPageErrors } = require("../helpers");

const LLM_STEP_HEADING = /^(Add LLMs|Add providers)$/;

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

async function waitForOnboardingStepLoaded(page) {
	await expect(page.locator(".onboarding-card")).toBeVisible();
	await expect(page.getByText("Loading…")).toHaveCount(0, { timeout: 10_000 });
	await expect(page.getByText("Scanning OpenClaw installation…", { exact: true })).not.toBeVisible({
		timeout: 10_000,
	});
}

async function visibleOnboardingHeadingText(page) {
	const headings = page.locator(".onboarding-card h2");
	const count = await headings.count();
	for (let i = 0; i < count; i++) {
		const heading = headings.nth(i);
		if (!(await isVisible(heading))) continue;
		const text = (await heading.textContent())?.trim();
		if (text) return text;
	}
	return null;
}

async function waitForOnboardingHeadingAdvance(page, previousHeading) {
	if (!previousHeading) return true;
	try {
		await expect.poll(() => visibleOnboardingHeadingText(page), { timeout: 10_000 }).not.toBe(previousHeading);
		return true;
	} catch {
		return false;
	}
}

async function waitForLlmStepReady(page) {
	const llmLoading = page.getByText("Loading LLMs…", { exact: true });
	if (await isVisible(llmLoading)) {
		await expect(llmLoading).not.toBeVisible({ timeout: 10_000 });
	}

	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	await expect(llmHeading).toBeVisible({ timeout: 10_000 });
}

async function waitForStepToDisappear(locator) {
	await expect.poll(() => isVisible(locator), { timeout: 10_000 }).toBeFalsy();
}

async function maybeSkipAuth(page) {
	const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
	if (!(await isVisible(authHeading))) return false;

	const clicked = await clickFirstVisibleButton(page, { name: /skip/i });
	expect(clicked).toBeTruthy();
	await waitForOnboardingStepLoaded(page);
	await waitForStepToDisappear(authHeading);
	return true;
}

async function maybeCompleteIdentity(page) {
	const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
	if (!(await isVisible(identityHeading))) return false;

	const userNameInput = page.getByPlaceholder("e.g. Alice");
	if (!(await isVisible(userNameInput))) return false;
	try {
		await userNameInput.fill("E2E User");
	} catch (error) {
		const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
		if (await isVisible(llmHeading)) return false;
		throw error;
	}

	const agentNameInput = page.getByPlaceholder("e.g. Rex");
	if ((await agentNameInput.count()) > 0 && (await isVisible(agentNameInput))) {
		await agentNameInput.fill("E2E Bot");
	}

	await page.getByRole("button", { name: "Continue", exact: true }).click();
	await waitForOnboardingStepLoaded(page);
	await waitForStepToDisappear(identityHeading);
	return true;
}

async function maybeSkipOpenClawImport(page) {
	const importHeading = page.getByRole("heading", { name: "Import from OpenClaw", exact: true });
	if (!(await isVisible(importHeading))) return false;
	const headingBefore = await visibleOnboardingHeadingText(page);

	const card = page.locator(".onboarding-card");
	const skipForNow = card.getByText("Skip for now", { exact: true });
	const skipButton = card.getByRole("button", { name: "Skip", exact: true });
	const continueButton = card.getByRole("button", { name: "Continue", exact: true });

	await expect
		.poll(
			async () => {
				return (await isVisible(skipForNow)) || (await isVisible(skipButton)) || (await isVisible(continueButton));
			},
			{ timeout: 10_000 },
		)
		.toBeTruthy();

	if (await isVisible(skipForNow)) {
		await skipForNow.click();
	} else if (await isVisible(skipButton)) {
		await skipButton.click();
	} else if (await isVisible(continueButton)) {
		await continueButton.click();
	} else {
		return false;
	}
	await waitForOnboardingStepLoaded(page);
	if (await waitForOnboardingHeadingAdvance(page, headingBefore)) return true;

	await expect
		.poll(
			async () => {
				if (await isVisible(importHeading)) return "import";
				const heading = await visibleOnboardingHeadingText(page);
				if (heading) return heading;
				const loadingLlms = page.getByText("Loading LLMs…", { exact: true });
				if (await isVisible(loadingLlms)) return "loading-llm";
				return "transitioning";
			},
			{ timeout: 10_000 },
		)
		.not.toBe("import");
	return true;
}

async function maybeWaitForLlmLoading(page) {
	const loadingLlms = page.getByText("Loading LLMs…", { exact: true });
	if (!(await isVisible(loadingLlms))) return false;
	await expect(loadingLlms).toHaveCount(0, { timeout: 10_000 });
	return true;
}

async function moveToLlmStep(page) {
	const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
	// Onboarding step order can vary by environment, and the OpenClaw import step
	// is populated asynchronously after the card first appears. Keep polling until
	// a real pre-LLM step is visible and can advance.
	for (let i = 0; i < 40; i++) {
		await waitForOnboardingStepLoaded(page);
		if (await isVisible(llmHeading)) {
			await waitForLlmStepReady(page);
			return true;
		}
		if (await maybeWaitForLlmLoading(page)) {
			await waitForLlmStepReady(page);
			return true;
		}

		if (await maybeSkipOpenClawImport(page)) continue;
		if (await maybeSkipAuth(page)) continue;
		if (await maybeCompleteIdentity(page)) continue;

		const backBtn = page.getByRole("button", { name: "Back", exact: true }).first();
		if (await isVisible(backBtn)) {
			await backBtn.click();
			continue;
		}

		// Wait for a step transition instead of a fixed delay
		await waitForOnboardingStepLoaded(page);
	}
	await waitForLlmStepReady(page);
	return true;
}

async function moveToVoiceStep(page) {
	const reachedLlm = await moveToLlmStep(page);
	if (!reachedLlm) return false;

	const voiceHeading = page.getByRole("heading", { name: "Voice (optional)", exact: true });
	if (await isVisible(voiceHeading)) return true;

	const skipped = await clickFirstVisibleButton(page, { name: "Skip for now", exact: true });
	if (!skipped) return false;

	// Voice step may not exist in the current onboarding flow — return false
	// gracefully instead of throwing when the heading never appears.
	for (let i = 0; i < 20; i++) {
		if (await isVisible(voiceHeading)) return true;
		await page.waitForTimeout(500);
	}
	return false;
}

async function moveToChannelStep(page) {
	const reachedLlm = await moveToLlmStep(page);
	if (!reachedLlm) return false;

	const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
	if (await isVisible(channelHeading)) return true;

	for (let i = 0; i < 6; i++) {
		if (await clickFirstVisibleButton(page, { name: "Skip for now", exact: true })) {
			if (await isVisible(channelHeading)) return true;
			continue;
		}

		if (!(await clickFirstVisibleButton(page, { name: "Continue", exact: true }))) break;
		if (await isVisible(channelHeading)) return true;
	}

	return isVisible(channelHeading);
}

async function moveToIdentityStep(page) {
	await waitForOnboardingStepLoaded(page);

	const identityHeading = page.getByRole("heading", {
		name: "Set up your identity",
		exact: true,
	});
	if (await isVisible(identityHeading)) return { reached: true, blockedByAuth: false };

	const authHeading = page.getByRole("heading", {
		name: "Secure your instance",
		exact: true,
	});
	if (await isVisible(authHeading)) {
		const authSkippable = await clickFirstVisibleButton(page, { name: "Skip for now", exact: true });
		if (!authSkippable) return { reached: false, blockedByAuth: true };
	}

	for (let i = 0; i < 6; i++) {
		if (await isVisible(identityHeading)) return { reached: true, blockedByAuth: false };

		if (await clickFirstVisibleButton(page, { name: /skip/i })) continue;
		if (await clickFirstVisibleButton(page, { name: /continue/i })) continue;
		break;
	}

	return { reached: await isVisible(identityHeading), blockedByAuth: false };
}

function horizontalOverflowPx(page) {
	return page.evaluate(() => Math.max(0, document.documentElement.scrollWidth - document.documentElement.clientWidth));
}

function firstVisibleOnboardingInputFontSizePx(page) {
	return page.evaluate(() => {
		const inputs = Array.from(document.querySelectorAll(".onboarding-card .provider-key-input"));
		const input = inputs.find((el) => {
			const rect = el.getBoundingClientRect();
			const style = window.getComputedStyle(el);
			return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
		});
		if (!input) return 0;
		return Number.parseFloat(window.getComputedStyle(input).fontSize || "0");
	});
}

/**
 * Onboarding tests run against a server started WITHOUT seeded
 * IDENTITY.md and USER.md, so the app enters onboarding mode.
 * These use the "onboarding" Playwright project which points at
 * a separate gateway instance on port 18790.
 */
test.describe("Onboarding wizard", () => {
	test.describe.configure({ mode: "serial" });

	test("onboarding gon includes voice_enabled flag", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		const pathname = new URL(page.url()).pathname;
		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const voiceEnabledType = await page.evaluate(() => typeof window.__MOLTIS__?.voice_enabled);
		expect(voiceEnabledType).toBe("boolean");
		expect(pageErrors).toEqual([]);
	});

	test("redirects to /onboarding on first run", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/");

		await expect(page).toHaveURL(/\/onboarding/, { timeout: 15_000 });
		expect(pageErrors).toEqual([]);
	});

	test("server started footer timestamp is hydrated", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const startedTime = page.locator(".onboarding-card time[data-epoch-ms]").first();
		await expect(startedTime).toBeVisible();
		await expect.poll(async () => ((await startedTime.textContent()) || "").trim(), { timeout: 10_000 }).not.toBe("");
		await expect(page.locator(".onboarding-card")).toContainText("Version v");

		expect(pageErrors).toEqual([]);
	});

	test("step indicator shows first step", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-step-dot").first()).toHaveClass(/active/);
		const activeStepLabel = (
			await page.locator(".onboarding-step.active .onboarding-step-label").first().textContent()
		)?.trim();
		expect(["Security", "Import", "LLM"]).toContain(activeStepLabel);
	});

	test("step indicator orders Import before LLM when import is available", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const labels = (await page.locator(".onboarding-step-label").allTextContents()).map((v) => v.trim());
		const importIdx = labels.indexOf("Import");
		const llmIdx = labels.indexOf("LLM");
		if (importIdx === -1 || llmIdx === -1) {
			test.skip(true, "OpenClaw import is not available in this onboarding run");
		}

		expect(importIdx).toBeLessThan(llmIdx);
	});

	test("step indicator orders Remote before Channel", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const labels = (await page.locator(".onboarding-step-label").allTextContents()).map((value) => value.trim());
		const remoteAccessIdx = labels.indexOf("Remote");
		const channelIdx = labels.indexOf("Channel");

		expect(remoteAccessIdx).toBeGreaterThan(-1);
		expect(channelIdx).toBeGreaterThan(-1);
		expect(remoteAccessIdx).toBeLessThan(channelIdx);
	});

	test("auth step renders actionable controls when shown", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		const isAuthStepVisible = await authHeading.isVisible().catch(() => false);

		if (!isAuthStepVisible) {
			// When auth is not needed, the wizard may show identity, OpenClaw import, or LLM step
			const anyStepHeading = page.getByRole("heading", {
				name: /^(Add LLMs|Add providers|Set up your identity|Import from OpenClaw)$/,
			});
			await expect(anyStepHeading).toBeVisible();
			return;
		}

		const passkeyCard = page.locator(".backend-card").filter({ hasText: "Passkey" }).first();
		const passwordCard = page.locator(".backend-card").filter({ hasText: "Password" }).first();
		await expect(passkeyCard).toBeVisible();
		await expect(passwordCard).toBeVisible();

		await passwordCard.click();
		const passwordInput = page.getByLabel(/^Password(?: \*)?$/);
		const confirmPasswordInput = page.getByLabel("Confirm password", { exact: true });
		await expect(passwordInput).toHaveAttribute("type", "password");
		await expect(passwordInput).toHaveAttribute("autocomplete", "new-password");
		await expect(confirmPasswordInput).toHaveAttribute("type", "password");
		await expect(confirmPasswordInput).toHaveAttribute("autocomplete", "new-password");
		await expect(page.getByRole("button", { name: /Set password|Skip/i }).first()).toBeVisible();
	});

	test("identity step has name input", async ({ page }) => {
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		const identityStep = await moveToIdentityStep(page);

		if (identityStep.blockedByAuth) {
			const authHeading = page.getByRole("heading", {
				name: "Secure your instance",
				exact: true,
			});
			await expect(authHeading).toBeVisible();
			await expect(page.locator(".backend-card").filter({ hasText: "Passkey" }).first()).toBeVisible();
			await expect(page.locator(".backend-card").filter({ hasText: "Password" }).first()).toBeVisible();
			await expect(page.getByText("Setup code", { exact: true })).toBeVisible();
			return;
		}

		if (!identityStep.reached) {
			const currentHeading = page.locator(".onboarding-card h2").first();
			await expect(currentHeading).toBeVisible();
			const headingText = (await currentHeading.textContent())?.trim() || "";
			expect(["Add LLMs", "Voice (optional)", "Remote Access", "Connect a Channel"]).toContain(headingText);
			const canSkip = await clickFirstVisibleButton(page, { name: /skip/i });
			const canContinue = await clickFirstVisibleButton(page, { name: /continue/i });
			expect(canSkip || canContinue).toBeTruthy();
			return;
		}

		await expect(identityHeading).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
		await expect(page.getByPlaceholder("e.g. Rex")).toBeVisible();
		await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeVisible();
	});

	test("mobile onboarding layout avoids horizontal overflow", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.setViewportSize({ width: 375, height: 812 });
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-card")).toBeVisible();
		await expect.poll(() => horizontalOverflowPx(page), { timeout: 10_000 }).toBeLessThan(2);
		const initialInputFontSize = await firstVisibleOnboardingInputFontSizePx(page);
		if (initialInputFontSize > 0) {
			expect(initialInputFontSize).toBeGreaterThanOrEqual(16);
		}

		const authHeading = page.getByRole("heading", { name: "Secure your instance", exact: true });
		if (await authHeading.isVisible().catch(() => false)) {
			const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true });
			if (await skipBtn.isVisible().catch(() => false)) {
				await skipBtn.click();
			}
		}

		const identityHeading = page.getByRole("heading", { name: "Set up your identity", exact: true });
		if (await identityHeading.isVisible().catch(() => false)) {
			await expect(page.getByPlaceholder("e.g. Alice")).toBeVisible();
			await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeVisible();
		}

		await expect.poll(() => horizontalOverflowPx(page), { timeout: 10_000 }).toBeLessThan(2);
		const finalInputFontSize = await firstVisibleOnboardingInputFontSizePx(page);
		if (finalInputFontSize > 0) {
			expect(finalInputFontSize).toBeGreaterThanOrEqual(16);
		}
		expect(pageErrors).toEqual([]);
	});

	test("page has no JS errors through wizard", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect(page.locator(".onboarding-card")).toBeVisible();
		await expect(page.getByText("Loading…")).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("telegram bot fields disable credential autofill", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const reachedLlm = await moveToLlmStep(page);
		expect(reachedLlm).toBeTruthy();
		await expect(page.getByRole("heading", { name: LLM_STEP_HEADING })).toBeVisible();
		await page.getByRole("button", { name: "Skip for now", exact: true }).click();

		const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
		for (let i = 0; i < 6; i++) {
			if (await channelHeading.isVisible().catch(() => false)) {
				break;
			}
			// Each step may have "Continue", "Skip for now", or both. Try either.
			const nextBtn = page.getByRole("button", { name: /^(Continue|Skip for now)$/ }).first();
			await expect(nextBtn).toBeVisible({ timeout: 5_000 });
			await nextBtn.click();
			await page.waitForTimeout(300);
		}

		await expect(channelHeading).toBeVisible();

		let telegramUserInput = page.locator('input[name="telegram_bot_username"]');
		if (!(await isVisible(telegramUserInput))) {
			const telegramSelectBtn = page.getByRole("button", { name: "Telegram", exact: true });
			if (await isVisible(telegramSelectBtn)) {
				await telegramSelectBtn.click();
			}
		}

		telegramUserInput = page.locator('input[name="telegram_bot_username"]');
		if (!(await isVisible(telegramUserInput))) {
			test.skip(true, "Telegram onboarding option is not available in this run");
			return;
		}

		await expect(telegramUserInput).toHaveAttribute("autocomplete", "off");
		await expect(telegramUserInput).toHaveAttribute("name", "telegram_bot_username");
		const tokenInput = page.locator('input[name="telegram_bot_token"]');
		await expect(tokenInput).toHaveAttribute("type", "password");
		await expect(tokenInput).toHaveAttribute("autocomplete", "new-password");
		await expect(tokenInput).toHaveAttribute("name", "telegram_bot_token");
		expect(pageErrors).toEqual([]);
	});

	test("whatsapp pairing renders SVG QR from channel event", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const reachedChannel = await moveToChannelStep(page);
		if (!reachedChannel) {
			test.skip(true, "could not reach channel step in this onboarding flow");
			return;
		}

		const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
		await expect(channelHeading).toBeVisible();

		const whatsappSelectBtn = page.getByRole("button", { name: "WhatsApp", exact: true });
		if (await isVisible(whatsappSelectBtn)) {
			await whatsappSelectBtn.click();
		}

		const accountInput = page.getByPlaceholder("e.g. my-whatsapp");
		if (!(await isVisible(accountInput))) {
			test.skip(true, "WhatsApp onboarding option is not available in this run");
			return;
		}

		const accountId = "e2e-whatsapp";

		await page.evaluate(async () => {
			const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
			if (!onboardingScript) throw new Error("onboarding-app.js script not found");
			const appUrl = new URL(onboardingScript.src, window.location.origin).href;
			const marker = "js/onboarding-app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in onboarding test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		await accountInput.fill(accountId);
		await page.getByRole("button", { name: "Start Pairing", exact: true }).click();

		await page.evaluate(
			async ({ accountIdArg }) => {
				const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
				if (!onboardingScript) throw new Error("onboarding-app.js script not found");
				const appUrl = new URL(onboardingScript.src, window.location.origin).href;
				const marker = "js/onboarding-app.js";
				const markerIdx = appUrl.indexOf(marker);
				if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
				const prefix = appUrl.slice(0, markerIdx);
				const events = await import(`${prefix}js/events.js`);
				const svg =
					"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 21 21'><rect width='21' height='21' fill='#fff'/><rect x='2' y='2' width='5' height='5' fill='#000'/></svg>";
				const listeners = events.eventListeners.channel || [];
				listeners.forEach((handler) => {
					handler({
						kind: "pairing_qr_code",
						channel_type: "whatsapp",
						account_id: accountIdArg,
						qr_data: "2@mock_payload",
						qr_svg: svg,
					});
				});
			},
			{ accountIdArg: accountId },
		);

		const qrImage = page.locator('img[alt="WhatsApp pairing QR code"]');
		await expect(qrImage).toBeVisible();
		await expect(qrImage).toHaveAttribute("src", /data:image\/svg\+xml;utf8,/);
		await expect(page.getByText("2@mock_payload")).toHaveCount(0);
		expect(pageErrors).toEqual([]);
	});

	test("whatsapp pairing falls back to polling channels.status for QR", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const reachedChannel = await moveToChannelStep(page);
		if (!reachedChannel) {
			test.skip(true, "could not reach channel step in this onboarding flow");
			return;
		}

		const whatsappSelectBtn = page.getByRole("button", { name: "WhatsApp", exact: true });
		if (await isVisible(whatsappSelectBtn)) {
			await whatsappSelectBtn.click();
		}

		const accountInput = page.getByPlaceholder("e.g. my-whatsapp");
		if (!(await isVisible(accountInput))) {
			test.skip(true, "WhatsApp onboarding option is not available in this run");
			return;
		}

		const accountId = "e2e-wa-poll";

		// Mock WebSocket: channels.add succeeds, channels.status returns QR data
		// (simulating the polling fallback path — no channel event is emitted).
		await page.evaluate(
			async ({ accountIdArg }) => {
				const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
				if (!onboardingScript) throw new Error("onboarding-app.js script not found");
				const appUrl = new URL(onboardingScript.src, window.location.origin).href;
				const marker = "js/onboarding-app.js";
				const prefix = appUrl.slice(0, appUrl.indexOf(marker));
				const state = await import(`${prefix}js/state.js`);
				const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
				state.setConnected(true);
				state.setWs({
					readyState: wsOpen,
					send(raw) {
						const req = JSON.parse(raw || "{}");
						const resolver = state.pending[req.id];
						if (!resolver) return;
						if (req.method === "channels.add") {
							resolver({ ok: true, payload: {} });
						} else if (req.method === "channels.status") {
							// Return QR data in the extra field, simulating the polling path.
							resolver({
								ok: true,
								payload: {
									channels: [
										{
											type: "whatsapp",
											account_id: accountIdArg,
											status: "disconnected",
											extra: { qr_data: "2@polled_qr_payload" },
										},
									],
								},
							});
						} else {
							resolver({ ok: false, error: { message: `unexpected rpc: ${req.method}` } });
						}
						delete state.pending[req.id];
					},
				});
			},
			{ accountIdArg: accountId },
		);

		await accountInput.fill(accountId);
		await page.getByRole("button", { name: "Start Pairing", exact: true }).click();

		// The polling fallback runs every 2s. Wait for QR data to appear.
		// It should render as raw text since no SVG was provided via polling.
		const qrFallback = page.getByText("2@polled_qr_payload");
		await expect(qrFallback).toBeVisible({ timeout: 10_000 });
		expect(pageErrors).toEqual([]);
	});

	test("whatsapp pairing with default account ID polls and renders SVG QR", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		// Navigate to "Connect a Channel" by repeatedly clicking Skip/Continue.
		const channelHeading = page.getByRole("heading", { name: "Connect a Channel", exact: true });
		await expect
			.poll(
				async () => {
					if (await isVisible(channelHeading)) return true;
					// Skip or advance whatever step is on screen.
					// All skip buttons say "Skip for now" (via i18n).
					const skipBtn = page.getByRole("button", { name: "Skip for now", exact: true }).first();
					const continueBtn = page.getByRole("button", { name: "Continue", exact: true }).first();
					const userNameInput = page.getByPlaceholder("e.g. Alice");
					if (await isVisible(userNameInput)) {
						await userNameInput.fill("E2E User");
						const agentNameInput = page.getByPlaceholder("e.g. Rex");
						if (await isVisible(agentNameInput)) await agentNameInput.fill("E2E Bot");
						if (await isVisible(continueBtn)) await continueBtn.click();
					} else if (await isVisible(skipBtn)) {
						await skipBtn.click();
					} else if (await isVisible(continueBtn)) {
						await continueBtn.click();
					}
					return false;
				},
				{ timeout: 60_000, intervals: [1000] },
			)
			.toBeTruthy();
		if (!(await isVisible(channelHeading))) {
			test.skip(true, "could not reach channel step in this onboarding flow");
			return;
		}

		// Select WhatsApp from channel type grid (may be auto-selected if only one offered)
		const whatsappSelectBtn = page.getByRole("button", { name: "WhatsApp", exact: true });
		if (await isVisible(whatsappSelectBtn)) {
			await whatsappSelectBtn.click();
		}

		// The WhatsApp form uses placeholder "main" for the Account ID field.
		const accountInput = page.getByPlaceholder("main");
		await expect(accountInput).toBeVisible({ timeout: 5_000 });

		// Leave account ID empty — the form defaults to "main".
		// The bug was that the polling useEffect used accountId.trim() (empty)
		// instead of accountId.trim() || "main", so channel matching failed.
		const svg =
			'<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100"/></svg>';

		await page.evaluate(
			async ({ svgArg }) => {
				const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
				if (!onboardingScript) throw new Error("onboarding-app.js script not found");
				const appUrl = new URL(onboardingScript.src, window.location.origin).href;
				const marker = "js/onboarding-app.js";
				const prefix = appUrl.slice(0, appUrl.indexOf(marker));
				const state = await import(`${prefix}js/state.js`);
				const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
				state.setConnected(true);
				state.setWs({
					readyState: wsOpen,
					send(raw) {
						const req = JSON.parse(raw || "{}");
						const resolver = state.pending[req.id];
						if (!resolver) return;
						if (req.method === "channels.add") {
							resolver({ ok: true, payload: {} });
						} else if (req.method === "channels.status") {
							// Return QR SVG with account_id "main" — the default
							resolver({
								ok: true,
								payload: {
									channels: [
										{
											type: "whatsapp",
											account_id: "main",
											status: "disconnected",
											extra: { qr_data: "2@default_poll", qr_svg: svgArg },
										},
									],
								},
							});
						} else {
							resolver({ ok: false, error: { message: `unexpected rpc: ${req.method}` } });
						}
						delete state.pending[req.id];
					},
				});
			},
			{ svgArg: svg },
		);

		// Do NOT fill the account input — leave it empty to test the || "main" fallback.
		await page.getByRole("button", { name: "Start Pairing", exact: true }).click();

		// The QR SVG should render as an <img> via the blob URL path.
		const qrImage = page.locator('img[alt="WhatsApp pairing QR code"]');
		await expect(qrImage).toBeVisible({ timeout: 10_000 });
		expect(pageErrors).toEqual([]);
	});

	test("matrix onboarding renders a real mask icon", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		await page.waitForLoadState("networkidle");

		await page.evaluate(() => {
			const probe = document.createElement("span");
			probe.className = "icon icon-xl icon-matrix";
			probe.id = "matrix-icon-probe";
			document.body.append(probe);
		});

		const matrixIcon = page.locator("#matrix-icon-probe");
		await expect(matrixIcon).toBeVisible();
		await expect
			.poll(() => {
				return matrixIcon.evaluate((node) => {
					const style = window.getComputedStyle(node);
					return style.maskImage || style.webkitMaskImage || "";
				});
			})
			.not.toBe("none");

		expect(pageErrors).toEqual([]);
	});

	test("onboarding channel selector styles render a 3-column grid and the nostr icon", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await page.setViewportSize({ width: 1280, height: 900 });
		await page.evaluate(() => {
			const existing = document.getElementById("channel-selector-probe");
			if (existing) existing.remove();

			const selector = document.createElement("div");
			selector.id = "channel-selector-probe";
			selector.className = "grid grid-cols-2 gap-3 md:grid-cols-3";
			const button = document.createElement("button");
			button.type = "button";
			button.className = "backend-card w-full min-h-[120px] items-center justify-center gap-4 px-4 py-8 text-center";
			const icon = document.createElement("span");
			icon.className = "icon icon-xl icon-nostr";
			const label = document.createElement("span");
			label.textContent = "Nostr";
			button.append(icon, label);
			selector.append(button);
			document.body.append(selector);
		});

		const selector = page.locator("#channel-selector-probe");
		await expect(selector).toBeVisible();

		await expect
			.poll(() =>
				selector.evaluate((node) => {
					const columns = window.getComputedStyle(node).gridTemplateColumns;
					return columns.split(" ").filter(Boolean).length;
				}),
			)
			.toBe(3);

		const icon = selector.locator(".icon.icon-nostr");
		await expect(icon).toBeVisible();
		await expect
			.poll(() =>
				icon.evaluate((node) => {
					const style = window.getComputedStyle(node);
					return style.maskImage || style.webkitMaskImage || "";
				}),
			)
			.not.toBe("none");

		expect(pageErrors).toEqual([]);
	});

	test("matrix onboarding exposes advanced config patch and storage note", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		const reachedChannel = await moveToChannelStep(page);
		if (!reachedChannel) {
			test.skip(true, "could not reach channel step in this onboarding flow");
			return;
		}

		await expect(page.getByText(/stored in Moltis's internal database \(.+moltis\.db\)/)).toBeVisible();

		const matrixSelectBtn = page.getByRole("button", { name: "Matrix", exact: true });
		if (await isVisible(matrixSelectBtn)) {
			await matrixSelectBtn.click();
		}

		const homeserverInput = page.locator('input[name="matrix_homeserver"]');
		if (!(await isVisible(homeserverInput))) {
			test.skip(true, "Matrix onboarding option is not available in this run");
			return;
		}
		await expect(page.getByText("Encrypted Matrix chats require Password auth.", { exact: false })).toBeVisible();
		await expect(
			page.getByText("Password is the default because it supports encrypted Matrix chats", { exact: false }),
		).toBeVisible();
		await expect(
			page.getByText("Use Password so Moltis creates and persists its own Matrix device keys", { exact: false }),
		).toBeVisible();
		await expect(
			page.getByText("do not transfer that device's private encryption keys into Moltis", { exact: false }),
		).toBeVisible();
		await expect(page.getByText("verify yes", { exact: false })).toBeVisible();

		await page.evaluate(async () => {
			const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
			if (!onboardingScript) throw new Error("onboarding-app.js script not found");
			const appUrl = new URL(onboardingScript.src, window.location.origin).href;
			const marker = "js/onboarding-app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__matrixOnboardingAddRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "channels.add") {
						window.__matrixOnboardingAddRequest = req.params || null;
						resolver({ ok: true, payload: {} });
					} else if (req.method === "channels.status") {
						resolver({ ok: true, payload: { channels: [] } });
					} else {
						resolver({ ok: false, error: { message: `unexpected rpc in onboarding matrix test: ${req.method}` } });
					}
					delete state.pending[req.id];
				},
			});
		});

		const authSelect = page.getByText("Authentication", { exact: true }).locator("xpath=following-sibling::select[1]");
		await expect(authSelect).toHaveValue("password");
		await homeserverInput.fill("https://matrix.example.com");
		await expect(page.getByLabel("Let Moltis own this Matrix account", { exact: true })).toBeChecked();
		await authSelect.selectOption("access_token");
		await page.locator('input[name="matrix_credential"]').fill("syt_test_token");
		await page.getByText("Advanced Config JSON", { exact: true }).click();
		await page
			.locator('textarea[name="channel_advanced_config"]')
			.fill('{"reply_to_message":true,"stream_mode":"off"}');
		await page.getByRole("button", { name: "Connect Matrix", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__matrixOnboardingAddRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__matrixOnboardingAddRequest);
		expect(sentRequest.account_id).toMatch(/^matrix-example-com-[a-z0-9]{6}$/);
		expect(sentRequest.config).toMatchObject({
			homeserver: "https://matrix.example.com",
			access_token: "syt_test_token",
			ownership_mode: "user_managed",
			otp_self_approval: true,
			otp_cooldown_secs: 300,
			reply_to_message: true,
			stream_mode: "off",
		});
		expect(pageErrors).toEqual([]);
	});

	test("llm provider api key form includes key source hint", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);

		const pathname = new URL(page.url()).pathname;
		if (/^\/chats\//.test(pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const reachedLlm = await moveToLlmStep(page);
		expect(reachedLlm).toBeTruthy();

		const llmHeading = page.getByRole("heading", { name: LLM_STEP_HEADING });
		await expect(llmHeading).toBeVisible();

		// Providers with key-source help links. The test picks the first one
		// that shows a "Configure" button (i.e. is not already configured from
		// environment variables). A broad list avoids flakes when the user has
		// several providers pre-configured locally.
		const candidates = [
			{ providerName: "OpenAI", linkName: "OpenAI Platform" },
			{ providerName: "Kimi Code", linkName: "Kimi Code Console" },
			{ providerName: "Anthropic", linkName: "Anthropic Console" },
			{ providerName: "DeepSeek", linkName: "DeepSeek Platform" },
			{ providerName: "Groq", linkName: "Groq Console" },
			{ providerName: "Mistral", linkName: "Mistral Console" },
			{ providerName: "Google Gemini", linkName: "Google AI Studio" },
			{ providerName: "xAI (Grok)", linkName: "xAI Console" },
			{ providerName: "Cerebras", linkName: "Cerebras Cloud" },
			{ providerName: "Venice", linkName: "Venice Settings" },
			{ providerName: "OpenRouter", linkName: "OpenRouter Settings" },
			{ providerName: "Moonshot", linkName: "Moonshot Platform" },
			{ providerName: "MiniMax", linkName: "MiniMax Platform" },
		];
		let matched = false;
		for (const candidate of candidates) {
			const row = page
				.locator(".onboarding-card .rounded-md.border")
				.filter({ has: page.getByText(candidate.providerName, { exact: true }) })
				.first();
			if ((await row.count()) === 0) continue;

			const configureBtn = row.getByRole("button", { name: "Configure", exact: true }).first();
			if (await configureBtn.isVisible().catch(() => false)) {
				await configureBtn.click();
				await expect(page.getByRole("link", { name: candidate.linkName })).toBeVisible();
				matched = true;
				break;
			}
		}

		// If every candidate is already configured from env, skip gracefully.
		if (!matched) {
			test.skip(true, "all API-key providers are pre-configured; cannot test key source hint");
			return;
		}
		expect(pageErrors).toEqual([]);
	});

	test("voice needs-key badge uses dedicated pill styling class", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const reachedVoice = await moveToVoiceStep(page);
		if (!reachedVoice) {
			test.skip(true, "voice step not reachable in this onboarding run");
			return;
		}

		const needsKeyBadges = page.locator(".provider-item-badge.needs-key", { hasText: "needs key" });
		const badgeCount = await needsKeyBadges.count();
		if (badgeCount === 0) {
			test.skip(true, "all voice providers already configured");
			return;
		}

		const firstBadge = needsKeyBadges.first();
		await expect(firstBadge).toBeVisible();
		const styles = await firstBadge.evaluate((el) => {
			const computed = window.getComputedStyle(el);
			return {
				background: computed.backgroundColor,
				radius: Number.parseFloat(computed.borderTopLeftRadius || "0"),
			};
		});
		expect(styles.background).not.toBe("transparent");
		expect(styles.background).not.toBe("rgba(0, 0, 0, 0)");
		expect(styles.radius).toBeGreaterThan(8);

		expect(pageErrors).toEqual([]);
	});

	test("voice onboarding saves whisper base URL without requiring an API key", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding");
		await page.waitForLoadState("networkidle");

		await expect.poll(() => new URL(page.url()).pathname, { timeout: 15_000 }).toMatch(/^\/(?:onboarding|chats\/.+)$/);
		if (/^\/chats\//.test(new URL(page.url()).pathname)) {
			expect(pageErrors).toEqual([]);
			return;
		}

		const reachedVoice = await moveToVoiceStep(page);
		if (!reachedVoice) {
			test.skip(true, "voice step not reachable in this onboarding run");
			return;
		}

		const whisperRow = page
			.locator(".onboarding-card .rounded-md.border")
			.filter({ has: page.getByText("OpenAI Whisper", { exact: true }) })
			.first();
		if (!(await isVisible(whisperRow))) {
			test.skip(true, "OpenAI Whisper row not available in this onboarding run");
			return;
		}

		await page.evaluate(async () => {
			const onboardingScript = document.querySelector('script[type="module"][src*="js/onboarding-app.js"]');
			if (!onboardingScript) throw new Error("onboarding-app.js script not found");
			const appUrl = new URL(onboardingScript.src, window.location.origin).href;
			const marker = "js/onboarding-app.js";
			const markerIdx = appUrl.indexOf(marker);
			if (markerIdx < 0) throw new Error("onboarding-app.js marker not found in script URL");
			const prefix = appUrl.slice(0, markerIdx);
			const state = await import(`${prefix}js/state.js`);
			const wsOpen = typeof WebSocket !== "undefined" ? WebSocket.OPEN : 1;
			window.__voiceOnboardingSaveSettingsRequest = null;
			state.setConnected(true);
			state.setWs({
				readyState: wsOpen,
				send(raw) {
					const req = JSON.parse(raw || "{}");
					const resolver = state.pending[req.id];
					if (!resolver) return;
					if (req.method === "voice.config.save_settings") {
						window.__voiceOnboardingSaveSettingsRequest = req.params || null;
						resolver({ ok: true, payload: { ok: true } });
					} else if (req.method === "voice.provider.toggle") {
						resolver({ ok: true, payload: { ok: true } });
					} else if (req.method === "voice.providers.all") {
						resolver({
							ok: true,
							payload: {
								stt: [
									{
										id: "whisper",
										name: "OpenAI Whisper",
										type: "stt",
										category: "cloud",
										description: "Best accuracy, handles accents and background noise",
										available: true,
										enabled: true,
										keySource: "config",
										settings: { baseUrl: "http://127.0.0.1:8001/v1" },
										capabilities: { baseUrl: true },
									},
								],
								tts: [],
							},
						});
					} else {
						resolver({
							ok: false,
							error: { message: `unexpected rpc in onboarding voice test: ${req.method}` },
						});
					}
					delete state.pending[req.id];
				},
			});
		});

		await whisperRow.getByRole("button", { name: "Configure", exact: true }).click();
		await whisperRow.locator('input[data-field="baseUrl"]').fill("http://127.0.0.1:8001/v1");
		await whisperRow.getByRole("button", { name: "Save", exact: true }).click();

		await expect.poll(() => page.evaluate(() => window.__voiceOnboardingSaveSettingsRequest)).not.toBeNull();

		const sentRequest = await page.evaluate(() => window.__voiceOnboardingSaveSettingsRequest);
		expect(sentRequest).toMatchObject({
			provider: "whisper",
			baseUrl: "http://127.0.0.1:8001/v1",
		});
		expect(pageErrors).toEqual([]);
	});
});
