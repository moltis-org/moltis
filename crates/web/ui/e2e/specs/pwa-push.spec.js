const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

async function getActiveWorker(page, context) {
	await navigateAndWait(page, "/chats");
	await expect.poll(() => context.serviceWorkers().length).toBeGreaterThan(0);
	return context.serviceWorkers()[0];
}

async function clearNotifications(worker) {
	await worker.evaluate(async () => {
		const notifications = await self.registration.getNotifications();
		for (const notification of notifications) notification.close();
	});
}

async function installNotificationHarness(worker) {
	await worker.evaluate(() => {
		self.__moltisNotifications = [];
		Object.defineProperty(self.registration, "getNotifications", {
			configurable: true,
			value: ({ tag } = {}) =>
				Promise.resolve(
					tag
						? self.__moltisNotifications.filter((notification) => notification.tag === tag)
						: [...self.__moltisNotifications],
				),
		});
		Object.defineProperty(self.registration, "showNotification", {
			configurable: true,
			value: (title, options) => {
				if (options.silent === true && Object.hasOwn(options, "vibrate")) {
					return Promise.reject(new TypeError("silent notifications cannot specify vibrate"));
				}
				self.__moltisNotifications = self.__moltisNotifications.filter((existing) => existing.tag !== options.tag);
				const notification = {
					title,
					...options,
					close() {
						self.__moltisNotifications = self.__moltisNotifications.filter((candidate) => candidate !== notification);
					},
				};
				self.__moltisNotifications.push(notification);
				return Promise.resolve();
			},
		});
	});
}

async function deliverPush(page, context, payload) {
	const session = await context.newCDPSession(page);
	let registrationId;
	const origin = new URL(page.url()).origin;
	session.on("ServiceWorker.workerRegistrationUpdated", ({ registrations }) => {
		const registration = registrations.find((candidate) => candidate.scopeURL === `${origin}/`);
		if (registration) registrationId = registration.registrationId;
	});
	await session.send("ServiceWorker.enable");
	await expect.poll(() => registrationId).toBeTruthy();
	await session.send("ServiceWorker.deliverPushMessage", {
		origin,
		registrationId,
		data: JSON.stringify(payload),
	});
	await session.detach();
}

// These tests exercise the PWA surface that is observable without a real push
// service: the manifest contract, the offline fallback document, and the
// service worker's notification/caching logic evaluated directly in the page.

test.describe("PWA manifest", () => {
	test("manifest declares the fields installability depends on", async ({ page }) => {
		const response = await page.request.get("/manifest.json");
		expect(response.ok()).toBeTruthy();

		const manifest = await response.json();
		expect(manifest.id).toBeTruthy();
		expect(manifest.name).toBeTruthy();
		expect(manifest.start_url).toBeTruthy();
		expect(manifest.display).toBe("standalone");
		// A locked orientation makes the installed app unusable on tablets and
		// desktop, where the window is landscape by definition.
		expect(manifest.orientation).toBe("any");
		expect(manifest.launch_handler?.client_mode).toBe("navigate-existing");
	});

	test("manifest ships both maskable and any-purpose icons", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();

		const maskable = manifest.icons.filter((icon) => icon.purpose === "maskable");
		expect(maskable.length).toBeGreaterThan(0);
		expect(manifest.icons.some((icon) => !icon.purpose || icon.purpose.includes("any"))).toBeTruthy();

		// Android requires a 192px and a 512px icon for the install prompt.
		const sizes = manifest.icons.map((icon) => icon.sizes);
		expect(sizes).toContain("192x192");
		expect(sizes).toContain("512x512");
	});

	test("every icon referenced by the manifest resolves", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();
		const sources = [...new Set(manifest.icons.map((icon) => icon.src))];

		for (const src of sources) {
			const response = await page.request.get(src);
			expect(response.status(), `icon ${src} must exist`).toBe(200);
		}
	});

	test("every shortcut points at a route the SPA serves", async ({ page }) => {
		const manifest = await (await page.request.get("/manifest.json")).json();

		for (const shortcut of manifest.shortcuts || []) {
			const response = await page.request.get(shortcut.url);
			expect(response.status(), `shortcut ${shortcut.url} must resolve`).toBeLessThan(400);
		}
	});
});

test.describe("PWA offline fallback", () => {
	test("offline page is served and self-contained", async ({ page }) => {
		const response = await page.request.get("/offline.html");
		expect(response.ok()).toBeTruthy();

		const html = await response.text();
		expect(html).toContain("You're offline");
		// The offline page must not depend on any bundle, or it cannot render
		// in the exact situation it exists for.
		expect(html).not.toMatch(/<script[^>]+src=/);
	});

	test("an uncached navigation uses the offline fallback", async ({ page, context }) => {
		await navigateAndWait(page, "/chats");
		await expect.poll(() => page.evaluate(() => Boolean(navigator.serviceWorker.controller))).toBe(true);

		await context.setOffline(true);
		try {
			await page.goto(`/uncached-offline-${Date.now()}`);
			await expect(page.getByRole("heading", { name: "You're offline" })).toBeVisible();
		} finally {
			await context.setOffline(false);
		}
	});
});

test.describe("service worker", () => {
	test("registers and controls the page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		const scope = await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			return registration.scope;
		});

		expect(scope).toContain("/");
		expect(pageErrors).toEqual([]);
	});

	test("does not activate a waiting worker without the page asking", async ({ page }) => {
		await navigateAndWait(page, "/chats");

		// The worker script must not call skipWaiting() at install time, or an
		// update reloads the app mid-conversation.
		const source = await (await page.request.get("/sw.js")).text();
		const installBlock = source.slice(source.indexOf('addEventListener("install"'));
		const activateIndex = installBlock.indexOf('addEventListener("activate"');
		const installBody = activateIndex === -1 ? installBlock : installBlock.slice(0, activateIndex);
		expect(installBody).not.toContain("skipWaiting");

		// It must still honour an explicit request from the page.
		expect(source).toContain("SKIP_WAITING");
	});

	test("requires the offline shell but tolerates missing generated assets", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		expect(source).not.toContain("addAll");
		expect(source).toContain("allSettled");
		expect(source).toContain("Promise.all(");
	});

	test("handles push subscription rotation", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		// Without this the endpoint silently rotates and push stops working
		// until the user toggles it off and on again.
		expect(source).toContain("pushsubscriptionchange");
		// Open tabs must refresh their cached endpoint after the worker rotates it.
		expect(source).toContain("push-subscription-changed");
	});

	test("re-alerts instead of silently replacing a same-session notification", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		expect(source).toContain("renotify");
	});

	test("shows a valid silent notification without a vibration option", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);

		await deliverPush(page, context, {
			title: "moltis",
			body: "Silent result",
			notificationId: "silent-result",
			sessionKey: "silent-test",
			silent: true,
		});

		await expect
			.poll(() =>
				worker.evaluate(() =>
					self.__moltisNotifications
						.filter((notification) => notification.tag === "moltis:session:silent-test")
						.map((notification) => ({
							silent: notification.silent,
							body: notification.body,
						})),
				),
			)
			.toEqual([{ silent: true, body: "Silent result" }]);
	});

	test("matches the visible session by whole path, not substring", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();

		// `/chats/main-2` contains `/chats/main`, so a substring test would treat
		// a different chat as on-screen and silence a notification for one the
		// user cannot see.
		expect(source).toContain("new URL(client.url).pathname");
		expect(source).not.toMatch(/client\.url\.includes\(`\/chats\//);
	});

	test("badges the app icon only when it is installed, and never blocks on it", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();

		// The Badging API neither resolves nor rejects where there is no badge
		// target — it hangs. Awaiting it, or gating a waitUntil on it, wedges the
		// worker and the push handler never shows its notification.
		expect(source).toContain("isInstalled()");
		expect(source).not.toMatch(/await\s+nav\.(set|clear)AppBadge/);
	});

	test("badges the total across chats and decrements only the viewed chat", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);
		await worker.evaluate(async () => {
			Object.defineProperty(navigator, "setAppBadge", {
				configurable: true,
				value: (count) => {
					self.__moltisBadgeCount = count;
					return Promise.resolve();
				},
			});
			Object.defineProperty(navigator, "clearAppBadge", {
				configurable: true,
				value: () => {
					self.__moltisBadgeCount = 0;
					return Promise.resolve();
				},
			});
			const cache = await caches.open("moltis-state");
			await cache.put("/__moltis__/installed", new Response("1"));
		});

		await deliverPush(page, context, { body: "First", notificationId: "badge-a", sessionKey: "badge-a" });
		await deliverPush(page, context, { body: "Second", notificationId: "badge-b", sessionKey: "badge-b" });
		await expect.poll(() => worker.evaluate(() => self.__moltisBadgeCount)).toBe(2);
		await page.evaluate(() => {
			history.pushState(null, "", "/settings");
			window.dispatchEvent(new PopStateEvent("popstate"));
		});
		await expect(page).toHaveURL(/\/settings(?:\/profile)?$/);
		await expect.poll(() => worker.evaluate(() => self.__moltisBadgeCount)).toBe(2);

		await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			registration.active?.postMessage({ type: "CLEAR_NOTIFICATIONS", sessionKey: "badge-a" });
		});
		await expect.poll(() => worker.evaluate(() => self.__moltisBadgeCount)).toBe(1);
	});

	test("acknowledges notification routing without reloading the SPA", async ({ page }) => {
		await navigateAndWait(page, "/settings");
		const result = await page.evaluate(async () => {
			window.__moltisNavigationMarker = "still-loaded";
			const channel = new MessageChannel();
			const acknowledged = new Promise((resolve) => {
				channel.port1.onmessage = (event) => resolve(event.data?.handled === true);
			});
			navigator.serviceWorker.dispatchEvent(
				new MessageEvent("message", {
					data: { type: "notification-click", url: "/chats/main" },
					ports: [channel.port2],
				}),
			);
			return {
				acknowledged: await acknowledged,
				pathname: window.location.pathname,
				marker: window.__moltisNavigationMarker,
			};
		});

		expect(result).toEqual({ acknowledged: true, pathname: "/chats/main", marker: "still-loaded" });
	});

	test("focuses a background client before requesting in-place routing", async ({ page }) => {
		const source = await (await page.request.get("/sw.js")).text();
		const routeClient = source.slice(
			source.indexOf("async function routeClient"),
			source.indexOf("async function openNotificationUrl"),
		);
		expect(routeClient.indexOf("client.focus()")).toBeGreaterThan(-1);
		expect(routeClient.indexOf("client.focus()")).toBeLessThan(routeClient.indexOf("client.postMessage"));
	});

	test("a badge update leaves the service worker responsive", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		// Drive the badge path the same way the app does on focus.
		await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			registration.active?.postMessage({ type: "CLEAR_NOTIFICATIONS", sessionKey: "main" });
		});

		// A wedged worker shows up as the next navigation dying, which is exactly
		// how the original badge bug surfaced.
		await page.reload();
		await expect(page.locator("body")).toBeVisible();
		expect(pageErrors).toEqual([]);
	});
});

test.describe("push settings", () => {
	test("notifications section renders its current state", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/notifications");

		await expect(page.getByText("Notifications", { exact: true }).first()).toBeVisible();
		expect(pageErrors).toEqual([]);
	});

	test("booting the app does not query pushManager without permission", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/chats");

		// Regression guard: initPWA runs presence reporting on every page load.
		// Reaching for pushManager.getSubscription() there — with no granted
		// permission and no push backend — crashes the renderer, taking down the
		// whole app rather than just push. The permission check must short-circuit.
		const state = await page.evaluate(() => ({
			permission: typeof Notification === "undefined" ? "unsupported" : Notification.permission,
			alive: document.readyState,
		}));

		expect(state.permission).not.toBe("granted");
		expect(state.alive).toBe("complete");
		expect(pageErrors).toEqual([]);
	});

	test("presence endpoint rejects an endpoint the server does not know", async ({ page }) => {
		const response = await page.request.post("/api/push/presence", {
			data: {
				endpoint: "https://push.example.com/definitely-not-registered",
				client_id: "e2e-tab",
				sequence: 1,
				session_key: "main",
				visible: true,
			},
		});

		// 404 tells the client its subscription is stale; 501 means the server
		// was built without the push-notifications feature.
		expect([404, 501]).toContain(response.status());
	});
});
