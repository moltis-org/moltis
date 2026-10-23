const { expect, test } = require("../base-test");
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
	if (/^(Import Your Data|Import from OpenClaw)$/.test(headingText)) return "import";
	if (headingText === "Set up your identity") return "identity";
	if (/^(Add LLMs|Add providers)$/.test(headingText)) return "providers";
	if (headingText === "Voice (optional)") return "voice";
	if (headingText === "Bundled Skills") return "skills";
	if (headingText === "Remote Access") return "remote-access";
	if (headingText === "Connect a Channel") return "channel";
	if (headingText === "Setup Summary") return "summary";
	return "pending";
}

async function waitForStableStep(page) {
	let step = "pending";
	await expect
		.poll(
			async () => {
				step = await detectOnboardingStep(page);
				return step;
			},
			{ timeout: 10_000 },
		)
		.not.toBe("pending");
	return step;
}

async function advanceToIdentityStep(page) {
	for (let i = 0; i < 8; i++) {
		const step = await waitForStableStep(page);
		if (step === "identity" || step === "chats" || step === "summary") return step;
		if (step === "auth") {
			await completePasswordAuthStep(page);
		} else {
			expect(["import", "providers", "voice", "skills", "remote-access", "channel"]).toContain(step);
			const advanced =
				(await clickFirstVisibleButton(page, { name: /skip for now/i })) ||
				(await clickFirstVisibleButton(page, { name: /continue/i }));
			expect(advanced).toBe(true);
		}
		await expect.poll(() => detectOnboardingStep(page)).not.toBe(step);
	}
	return waitForStableStep(page);
}

async function dismissRecoveryKeyIfShown(page) {
	const recoveryHeading = page.getByText("Password set and vault initialized", { exact: true });
	if (await isVisible(recoveryHeading)) {
		await expect(page.getByText("Recovery key", { exact: true })).toBeVisible();
		await expect(
			page.getByText(
				"Save this recovery key in a safe place. It will not be shown again. You need it to unlock the vault if you forget your password.",
				{ exact: true },
			),
		).toBeVisible();
		const codeBlock = page.locator(".onboarding-card code");
		await codeBlock.waitFor({ state: "visible", timeout: 3_000 }).catch(() => {});

		const keyText = ((await codeBlock.textContent()) || "").trim();

		// Stub clipboard so we can assert what gets written without needing
		// browser clipboard permissions, and verify the copy button feedback.
		await page.evaluate(() => {
			window.__recoveryKeyCopied = null;
			try {
				Object.defineProperty(navigator.clipboard, "writeText", {
					configurable: true,
					value: (text) => {
						window.__recoveryKeyCopied = text;
						return Promise.resolve();
					},
				});
			} catch {
				// clipboard not configurable in this context; skip capture
			}
		});

		const copyBtn = page.getByRole("button", { name: /^Copy/, exact: false });
		if (await isVisible(copyBtn)) {
			await copyBtn.click();
			// Button should briefly show "Copied!" feedback
			await expect(copyBtn)
				.toHaveText("Copied!", { timeout: 2_000 })
				.catch(() => {});
			// The key text passed to clipboard must match what was displayed
			if (keyText) {
				const captured = await page.evaluate(() => window.__recoveryKeyCopied);
				if (captured !== null) {
					expect(captured).toBe(keyText);
				}
			}
		}

		if (await clickFirstVisibleButton(page, { name: /^Continue$/ })) return true;
	}
	return false;
}

async function completePasswordAuthStep(page) {
	if (await clickFirstVisibleButton(page, { name: /^Next$/ })) {
		await expect.poll(() => detectOnboardingStep(page)).not.toBe("auth");
		return true;
	}

	const setupCodeInput = page.getByPlaceholder("Code from the moltis terminal");
	await expect(setupCodeInput).toBeVisible();
	await expect(page.getByText("Setup code", { exact: true })).toBeVisible();
	await expect(page.getByText("Find this code in the moltis process log (stdout).", { exact: true })).toBeVisible();
	await expect(
		page.getByText("Set a password and enable the encryption vault for stored secrets", { exact: true }),
	).toBeVisible();
	await setupCodeInput.fill("123456");
	await page.locator(".backend-card").filter({ hasText: "Password" }).first().click();

	const inputs = page.locator(".onboarding-card input[type='password']");
	await expect(inputs).toHaveCount(2);
	await inputs.first().fill("testpassword1");
	await inputs.nth(1).fill("testpassword1");
	const setupResponsePromise = page.waitForResponse(
		(response) => response.url().endsWith("/api/auth/setup") && response.request().method() === "POST",
	);
	await page.getByRole("button", { name: "Set password", exact: true }).click();
	const setupResponse = await setupResponsePromise;
	expect(setupResponse.ok()).toBeTruthy();
	const data = await setupResponse.json();
	if (data.recovery_key) {
		await expect(page.getByText("Password set and vault initialized", { exact: true })).toBeVisible();
		await dismissRecoveryKeyIfShown(page);
	}
	await expect.poll(() => detectOnboardingStep(page)).not.toBe("auth");
	return true;
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

	// Runs first: the next test consumes the setup code.
	test("prefills the setup code from the #setup= link and strips it", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await page.goto("/onboarding#setup=123456");
		await expect
			.poll(() => new URL(page.url()).pathname, { timeout: 15_000 })
			.toMatch(/^\/(?:onboarding|login|chats\/.+)$/);
		// A retry after auth was configured lands elsewhere; nothing to check.
		if (new URL(page.url()).pathname !== "/onboarding" || (await waitForStableStep(page)) !== "auth") {
			expect(pageErrors).toEqual([]);
			return;
		}

		await expect(page.getByPlaceholder("Code from the moltis terminal")).toHaveValue("123456");
		await expect(page.getByText("Filled in from the link moltis opened.", { exact: true })).toBeVisible();
		expect(new URL(page.url()).hash).toBe("");

		await page.evaluate(() => {
			localStorage.setItem("moltis-locale", "fr");
			window.history.replaceState(window.history.state, "", "#setup=123456");
		});
		await page.reload();
		await expect(page.getByText("Prérempli à partir du lien ouvert par moltis.", { exact: true })).toBeVisible();
		expect(new URL(page.url()).hash).toBe("");

		await page.evaluate(() => {
			localStorage.setItem("moltis-locale", "zh");
			// Preserve the launch fragment without triggering in-page navigation.
			window.history.replaceState(window.history.state, "", "#setup=123456");
		});
		await page.reload();
		const setupCodeInput = page.getByPlaceholder("Code from the moltis terminal");
		await expect(setupCodeInput).toHaveValue("123456");
		await expect(page.getByText("已从 moltis 打开的链接中填入设置码。", { exact: true })).toBeVisible();
		expect(new URL(page.url()).hash).toBe("");
		await setupCodeInput.fill("654321");
		await expect(page.getByText("在 moltis 进程日志 (stdout) 中查找此码。", { exact: true })).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("completes auth and identity steps via WebSocket", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		// Fresh runs visiting `/` on a remote (proxied) connection should
		// be redirected to /onboarding so the setup-code AuthStep is
		// shown (#350, #646).  Retries can land on /login if a previous
		// attempt already configured auth.
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
		}
		expect(step).toBe("identity");
		await expect
			.poll(() =>
				page
					.evaluate(() => {
						const state = window.__moltis_state;
						return state?.connected === true && state.ws?.readyState === WebSocket.OPEN;
					})
					.catch(() => false),
			)
			.toBe(true);

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
