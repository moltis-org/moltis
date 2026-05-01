// Tests for the shared copyToClipboard utility.
//
// Three behavioural cases matter after the insecure-context fix:
//   1. Clipboard API available   → writeText() is called, button shows "Copied"
//   2. Clipboard API undefined   → execCommand("copy") fallback runs, button shows "Copied"
//   3. Both APIs fail            → error toast is shown via showToast()
//
// We exercise these via the SSH "Copy Public Key" button because it:
//   - Is reachable without extra setup on the default (pre-configured) server
//   - Uses copyToClipboard() with local-state feedback (setCopiedKeyId → "Copied"
//     label) AND a non-empty failMessage, making all three cases observable.

const { test, expect } = require("../base-test");
const { navigateAndWait } = require("../helpers");

async function generateSshKey(page) {
	const suffix = Date.now().toString().slice(-6);
	const keyName = `e2e-clipboard-${suffix}`;
	await page.getByPlaceholder("production-box").fill(keyName);
	await page.getByRole("button", { name: "Generate", exact: true }).click();
	await expect(page.locator(".provider-item-name", { hasText: keyName }).first()).toBeVisible({
		timeout: 15_000,
	});
	return keyName;
}

/** Locate the copy button scoped to a specific key's container.
 *  This prevents the locator from jumping to a different key's button
 *  when the clicked button's text changes from "Copy Public Key" to "Copied". */
function copyBtnForKey(page, keyName) {
	const keyItem = page.locator(".provider-item", {
		has: page.locator(".provider-item-name", { hasText: keyName }),
	});
	return keyItem.locator("button.provider-btn-secondary").first();
}

test.describe("copyToClipboard utility", () => {
	test("copy button writes correct text via Clipboard API", async ({ page, context }) => {
		// Grant real clipboard permissions so the native Clipboard API works.
		await context.grantPermissions(["clipboard-read", "clipboard-write"]);
		await navigateAndWait(page, "/settings/ssh");
		const keyName = await generateSshKey(page);

		const copyBtn = copyBtnForKey(page, keyName);
		await expect(copyBtn).toBeVisible();
		await copyBtn.click();

		// Button label should flip to "Copied" for ~2 s then revert
		await expect(copyBtn).toHaveText("Copied", { timeout: 2_000 });

		// The written text must be the public key (begins with the key type)
		const clipText = await page.evaluate(() => navigator.clipboard.readText());
		expect(clipText.trim()).toMatch(/^ssh-/);
	});

	test("copy button falls back to execCommand when clipboard API is unavailable", async ({ page }) => {
		await navigateAndWait(page, "/settings/ssh");
		const keyName = await generateSshKey(page);

		// Simulate an insecure context where navigator.clipboard is undefined,
		// then intercept document.execCommand to confirm the fallback fires.
		await page.evaluate(() => {
			window.__execCommandCopyCalled = false;
			Object.defineProperty(Navigator.prototype, "clipboard", {
				configurable: true,
				get: () => undefined,
			});
			const orig = document.execCommand.bind(document);
			document.execCommand = (cmd, ...args) => {
				if (cmd === "copy") window.__execCommandCopyCalled = true;
				return orig(cmd, ...args);
			};
		});

		const copyBtn = copyBtnForKey(page, keyName);
		await expect(copyBtn).toBeVisible();
		await copyBtn.click();

		// execCommand path should still produce "Copied" feedback on the button
		await expect(copyBtn).toHaveText("Copied", { timeout: 2_000 });

		const execCommandWasCalled = await page.evaluate(() => window.__execCommandCopyCalled);
		expect(execCommandWasCalled).toBe(true);
	});

	test("copy button shows error toast when both clipboard and execCommand fail", async ({ page }) => {
		await navigateAndWait(page, "/settings/ssh");
		const keyName = await generateSshKey(page);

		// Exhaust all copy paths: clipboard undefined + execCommand returns false.
		// copyToClipboard() should then call showToast() with the failMessage
		// that SshSection passes: "Could not copy public key — please copy it
		// manually."
		await page.evaluate(() => {
			Object.defineProperty(Navigator.prototype, "clipboard", {
				configurable: true,
				get: () => undefined,
			});
			document.execCommand = () => false;
		});

		const copyBtn = copyBtnForKey(page, keyName);
		await expect(copyBtn).toBeVisible();
		await copyBtn.click();

		await expect(page.locator(".skills-toast-container")).toContainText("Could not copy public key", {
			timeout: 3_000,
		});

		// Button label must NOT have changed to "Copied" — the copy failed
		await expect(copyBtn).toHaveText("Copy Public Key");
	});
});
