const { expect, test } = require("../base-test");
const { navigateAndWait, watchPageErrors } = require("../helpers");

async function getActiveWorker(page, context) {
	await navigateAndWait(page, "/chats");
	await page.evaluate(async () => {
		await navigator.serviceWorker.ready;
	});
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

async function seedNotifications(worker, notifications) {
	await worker.evaluate((seeds) => {
		for (const seed of seeds) {
			const notification = {
				...seed,
				close() {
					self.__moltisNotifications = self.__moltisNotifications.filter((candidate) => candidate !== notification);
				},
			};
			self.__moltisNotifications.push(notification);
		}
	}, notifications);
}

async function installBadgeHarness(worker) {
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
}

async function installPushManagerHarness(page, options = {}) {
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: the browser harness models native subscription persistence modes.
	await page.addInitScript((config) => {
		const persistedEndpoint = config.persistAcrossReload ? sessionStorage.getItem("__moltisPushHarnessEndpoint") : null;
		const sharedEndpointKey = "__moltisPushHarnessSharedEndpoint";
		const keyBytes = config.applicationServerKey || [1, 2, 3];
		const makeSubscription = (endpoint, applicationServerKey = keyBytes) => {
			const subscription = {
				endpoint,
				options: { applicationServerKey: new Uint8Array(applicationServerKey).buffer },
				getKey() {
					return new Uint8Array([4, 5, 6]).buffer;
				},
				unsubscribe() {
					window.__moltisPushHarness.unsubscribeCount += 1;
					if (window.__moltisPushHarness.current === subscription) window.__moltisPushHarness.current = null;
					if (config.persistAcrossReload) sessionStorage.removeItem("__moltisPushHarnessEndpoint");
					if (config.sharedAcrossTabs && localStorage.getItem(sharedEndpointKey) === endpoint) {
						localStorage.removeItem(sharedEndpointKey);
					}
					return Promise.resolve(true);
				},
				toJSON() {
					return { endpoint, keys: { p256dh: "BAUG", auth: "BAUG" } };
				},
			};
			return subscription;
		};
		if (config.sharedAcrossTabs && config.endpoint && !localStorage.getItem(sharedEndpointKey)) {
			localStorage.setItem(sharedEndpointKey, config.endpoint);
		}
		const initialEndpoint =
			(config.sharedAcrossTabs ? localStorage.getItem(sharedEndpointKey) : null) ||
			persistedEndpoint ||
			config.endpoint;
		const initialKey = persistedEndpoint ? [1, 2, 3] : keyBytes;

		window.__moltisPushHarness = {
			current: initialEndpoint ? makeSubscription(initialEndpoint, initialKey) : null,
			freshEndpoint: config.freshEndpoint || "https://push.example.com/fresh",
			freshEndpoints: config.freshEndpoints || [],
			subscribeCount: 0,
			unsubscribeCount: 0,
			getSubscriptionCount: 0,
		};
		if (config.enabled) localStorage.setItem("moltis-push-enabled", "1");
		Object.defineProperty(Notification, "permission", {
			configurable: true,
			get: () => "granted",
		});
		Object.defineProperty(Notification, "requestPermission", {
			configurable: true,
			value: () => Promise.resolve("granted"),
		});
		Object.defineProperty(PushManager.prototype, "getSubscription", {
			configurable: true,
			value: () => {
				window.__moltisPushHarness.getSubscriptionCount += 1;
				if (config.sharedAcrossTabs) {
					const endpoint = localStorage.getItem(sharedEndpointKey);
					return Promise.resolve(endpoint ? makeSubscription(endpoint) : null);
				}
				return Promise.resolve(window.__moltisPushHarness.current);
			},
		});
		Object.defineProperty(PushManager.prototype, "subscribe", {
			configurable: true,
			value: (subscribeOptions) => {
				window.__moltisPushHarness.subscribeCount += 1;
				const applicationServerKey = Array.from(new Uint8Array(subscribeOptions.applicationServerKey));
				const endpoint =
					window.__moltisPushHarness.freshEndpoints[window.__moltisPushHarness.subscribeCount - 1] ||
					window.__moltisPushHarness.freshEndpoint;
				const subscription = makeSubscription(endpoint, applicationServerKey);
				window.__moltisPushHarness.current = subscription;
				if (config.sharedAcrossTabs) localStorage.setItem(sharedEndpointKey, endpoint);
				if (config.persistAcrossReload) {
					sessionStorage.setItem("__moltisPushHarnessEndpoint", subscription.endpoint);
				}
				return Promise.resolve(subscription);
			},
		});
	}, options);
}

async function mockPushApi(
	page,
	{ presenceStatus = 200, presenceHandler, subscribeStatus = () => 200, requests = [], completedRequests = [] } = {},
) {
	await page.route("**/api/push/vapid-key", (route) => route.fulfill({ json: { public_key: "AQID" } }));
	await page.route("**/api/push/status", (route) =>
		route.fulfill({ json: { enabled: true, subscription_count: 0, subscriptions: [] } }),
	);
	await page.route("**/api/push/presence", (route) =>
		presenceHandler ? presenceHandler(route) : route.fulfill({ status: presenceStatus, body: "" }),
	);
	await page.route("**/api/push/unsubscribe", (route) => route.fulfill({ status: 200, body: "" }));
	await page.route("**/api/push/subscribe", async (route) => {
		const body = route.request().postDataJSON();
		requests.push(body);
		await route.fulfill({ status: subscribeStatus(requests.length), body: "" });
		completedRequests.push(body);
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

async function readPushMarker(page) {
	return await page.evaluate(async () => {
		const response = await (await caches.open("moltis-state")).match("/__moltis__/push-rotation-pending");
		return response ? await response.json() : null;
	});
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

	test("waits for old clients to close before activating a new worker", async ({ page }) => {
		await navigateAndWait(page, "/chats");

		// The worker script must not call skipWaiting() at install time, or an
		// update reloads the app mid-conversation.
		const source = await (await page.request.get("/sw.js")).text();
		const installBlock = source.slice(source.indexOf('addEventListener("install"'));
		const activateIndex = installBlock.indexOf('addEventListener("activate"');
		const installBody = activateIndex === -1 ? installBlock : installBlock.slice(0, activateIndex);
		expect(installBody).not.toContain("skipWaiting");

		// No page message may bypass the normal activation lifecycle either.
		expect(source).not.toContain("SKIP_WAITING");
		const appSource = await (await page.request.get("/assets/dist/main.js")).text();
		expect(appSource).not.toContain("sw-update-available");
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

	test("disables worker-side rotation when the server reports revocation", async ({ page, context }) => {
		const worker = await getActiveWorker(page, context);
		const result = await worker.evaluate(async () => {
			const cache = await caches.open("moltis-state");
			await Promise.all([
				cache.put("/__moltis__/push-enabled", new Response("1")),
				cache.delete("/__moltis__/push-disabled"),
				cache.delete("/__moltis__/push-rotation-pending"),
			]);
			let unsubscribeCount = 0;
			let requestBody;
			const originalFetch = self.fetch;
			self.fetch = (input, init) => {
				if (String(input).endsWith("/api/push/subscribe")) {
					requestBody = JSON.parse(init.body);
					return Promise.resolve(new Response("", { status: 410 }));
				}
				return originalFetch(input, init);
			};
			try {
				const pending = [];
				const event = new Event("pushsubscriptionchange");
				Object.defineProperties(event, {
					oldSubscription: { value: { endpoint: "https://push.example.com/old" } },
					newSubscription: {
						value: {
							endpoint: "https://push.example.com/fresh",
							toJSON: () => ({ keys: { p256dh: "key", auth: "auth" } }),
							unsubscribe: () => {
								unsubscribeCount += 1;
								return Promise.resolve(true);
							},
						},
					},
					waitUntil: { value: (promise) => pending.push(promise) },
				});
				self.dispatchEvent(event);
				await Promise.all(pending);
			} finally {
				self.fetch = originalFetch;
			}
			return {
				disabled: Boolean(await cache.match("/__moltis__/push-disabled")),
				enabled: Boolean(await cache.match("/__moltis__/push-enabled")),
				pending: Boolean(await cache.match("/__moltis__/push-rotation-pending")),
				requestBody,
				unsubscribeCount,
			};
		});

		expect(result).toEqual({
			disabled: true,
			enabled: false,
			pending: false,
			requestBody: {
				endpoint: "https://push.example.com/fresh",
				keys: { p256dh: "key", auth: "auth" },
				replaces: "https://push.example.com/old",
				revive: false,
			},
			unsubscribeCount: 1,
		});
	});

	test("preserves the earliest replacement across consecutive worker rotations", async ({ page, context }) => {
		const worker = await getActiveWorker(page, context);
		const result = await worker.evaluate(async () => {
			const cache = await caches.open("moltis-state");
			await Promise.all([
				cache.put("/__moltis__/push-enabled", new Response("1")),
				cache.delete("/__moltis__/push-disabled"),
				cache.delete("/__moltis__/push-rotation-pending"),
			]);
			const requests = [];
			const originalFetch = self.fetch;
			self.fetch = (input, init) => {
				if (String(input).endsWith("/api/push/subscribe")) {
					requests.push(JSON.parse(init.body));
					return Promise.resolve(new Response("", { status: 500 }));
				}
				return originalFetch(input, init);
			};
			const rotate = async (oldEndpoint, newEndpoint) => {
				const pending = [];
				const event = new Event("pushsubscriptionchange");
				Object.defineProperties(event, {
					oldSubscription: { value: { endpoint: oldEndpoint } },
					newSubscription: {
						value: {
							endpoint: newEndpoint,
							toJSON: () => ({ keys: { p256dh: "key", auth: "auth" } }),
						},
					},
					waitUntil: { value: (promise) => pending.push(promise) },
				});
				self.dispatchEvent(event);
				await Promise.all(pending);
			};
			try {
				await rotate("https://push.example.com/a", "https://push.example.com/b");
				await rotate("https://push.example.com/b", "https://push.example.com/c");
			} finally {
				self.fetch = originalFetch;
			}
			const marker = await cache.match("/__moltis__/push-rotation-pending");
			return { requests, marker: marker ? await marker.json() : null };
		});

		expect(result.requests).toEqual([
			expect.objectContaining({
				endpoint: "https://push.example.com/b",
				replaces: "https://push.example.com/a",
				revive: false,
			}),
			expect.objectContaining({
				endpoint: "https://push.example.com/c",
				replaces: "https://push.example.com/a",
				revive: false,
			}),
		]);
		expect(result.marker).toEqual({ replaces: "https://push.example.com/a", revive: false });
	});

	test("worker rotation preserves explicit revival authorization", async ({ page, context }) => {
		const worker = await getActiveWorker(page, context);
		const request = await worker.evaluate(async () => {
			const cache = await caches.open("moltis-state");
			await Promise.all([
				cache.put("/__moltis__/push-enabled", new Response("1")),
				cache.delete("/__moltis__/push-disabled"),
				cache.put(
					"/__moltis__/push-rotation-pending",
					new Response(JSON.stringify({ replaces: "https://push.example.com/revoked", revive: true })),
				),
			]);
			let requestBody;
			const originalFetch = self.fetch;
			self.fetch = (input, init) => {
				if (String(input).endsWith("/api/push/subscribe")) {
					requestBody = JSON.parse(init.body);
					return Promise.resolve(new Response("", { status: 201 }));
				}
				return originalFetch(input, init);
			};
			try {
				const pending = [];
				const event = new Event("pushsubscriptionchange");
				Object.defineProperties(event, {
					oldSubscription: { value: { endpoint: "https://push.example.com/intermediate" } },
					newSubscription: {
						value: {
							endpoint: "https://push.example.com/fresh",
							toJSON: () => ({ keys: { p256dh: "key", auth: "auth" } }),
						},
					},
					waitUntil: { value: (promise) => pending.push(promise) },
				});
				self.dispatchEvent(event);
				await Promise.all(pending);
			} finally {
				self.fetch = originalFetch;
			}
			return requestBody;
		});

		expect(request).toMatchObject({
			endpoint: "https://push.example.com/fresh",
			replaces: "https://push.example.com/revoked",
			revive: true,
		});
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

	test("does not redeliver an older ordered push after its notification is cleared", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);
		await worker.evaluate(async () => {
			const cache = await caches.open("moltis-state");
			const response = await cache.match("/__moltis__/push-notification-order");
			const orders = response ? await response.json() : {};
			delete orders["ordered-chat"];
			await cache.put("/__moltis__/push-notification-order", new Response(JSON.stringify(orders)));
		});

		await deliverPush(page, context, { body: "Newer", sessionKey: "ordered-chat", order: 20 });
		await expect
			.poll(() =>
				worker.evaluate(() => self.__moltisNotifications.some((notification) => notification.body === "Newer")),
			)
			.toBe(true);
		await clearNotifications(worker);
		await deliverPush(page, context, { body: "Older", sessionKey: "ordered-chat", order: 10 });
		await deliverPush(page, context, { body: "Queue drained", sessionKey: "order-sentinel" });
		await expect
			.poll(() =>
				worker.evaluate(() => self.__moltisNotifications.some((notification) => notification.body === "Queue drained")),
			)
			.toBe(true);

		const result = await worker.evaluate(async () => {
			const marker = await (await caches.open("moltis-state")).match("/__moltis__/push-notification-order");
			return {
				bodies: self.__moltisNotifications.map((notification) => notification.body),
				orders: marker ? await marker.json() : {},
			};
		});
		expect(result.bodies).toEqual(["Queue drained"]);
		expect(result.orders["ordered-chat"]).toBe(20);
	});

	test("rolls legacy-tagged notifications into the current session notification", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);
		await installBadgeHarness(worker);
		await seedNotifications(worker, [
			{
				tag: "legacy-session-tag",
				body: "Older result",
				data: { sessionKey: "legacy-chat", count: 2 },
			},
		]);

		await deliverPush(page, context, {
			body: "Newest result",
			notificationId: "current-result",
			sessionKey: "legacy-chat",
		});

		await expect
			.poll(() =>
				worker.evaluate(() =>
					self.__moltisNotifications.map((notification) => ({
						body: notification.body,
						count: notification.data.count,
						tag: notification.tag,
					})),
				),
			)
			.toEqual([
				{
					body: "Newest result\n… and 2 earlier messages",
					count: 3,
					tag: "moltis:session:legacy-chat",
				},
			]);
		await expect.poll(() => worker.evaluate(() => self.__moltisBadgeCount)).toBe(3);
	});

	test("clears legacy-tagged notifications by their stored session", async ({ page, context }) => {
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);
		await installBadgeHarness(worker);
		await seedNotifications(worker, [
			{ tag: "old-worker-a", data: { sessionKey: "viewed-chat", count: 2 } },
			{ tag: "old-worker-b", data: { sessionKey: "other-chat", count: 1 } },
		]);

		await page.evaluate(async () => {
			const registration = await navigator.serviceWorker.ready;
			registration.active?.postMessage({ type: "CLEAR_NOTIFICATIONS", sessionKey: "viewed-chat" });
		});

		await expect
			.poll(() =>
				worker.evaluate(() =>
					self.__moltisNotifications.map((notification) => ({
						count: notification.data.count,
						sessionKey: notification.data.sessionKey,
					})),
				),
			)
			.toEqual([{ count: 1, sessionKey: "other-chat" }]);
		await expect.poll(() => worker.evaluate(() => self.__moltisBadgeCount)).toBe(1);
	});

	for (const eventType of ["notificationclick", "notificationclose"]) {
		test(`${eventType} excludes a legacy notification while close propagation lags`, async ({ page, context }) => {
			const worker = await getActiveWorker(page, context);
			await clearNotifications(worker);
			await installNotificationHarness(worker);
			await installBadgeHarness(worker);
			await seedNotifications(worker, [
				{ tag: "legacy-lagging", data: { sessionKey: "lagging-chat", count: 2 } },
				{ tag: "other-notification", data: { sessionKey: "other-chat", count: 1 } },
			]);

			const result = await worker.evaluate(async (type) => {
				const notification = self.__moltisNotifications.find((candidate) => candidate.tag === "legacy-lagging");
				notification.close = () => {
					// Simulate the browser retaining a just-closed notification briefly.
				};
				const pending = [];
				const event = new Event(type);
				Object.defineProperties(event, {
					action: { value: "dismiss" },
					notification: { value: notification },
					waitUntil: { value: (promise) => pending.push(promise) },
				});
				self.dispatchEvent(event);
				await Promise.all(pending);
				return {
					badge: self.__moltisBadgeCount,
					storedCount: self.__moltisNotifications.reduce((sum, candidate) => sum + (candidate.data.count || 1), 0),
				};
			}, eventType);

			expect(result).toEqual({ badge: 1, storedCount: 3 });
		});
	}

	test("closing a modern notification does not exclude its newer same-session replacement", async ({
		page,
		context,
	}) => {
		const worker = await getActiveWorker(page, context);
		await clearNotifications(worker);
		await installNotificationHarness(worker);
		await installBadgeHarness(worker);
		await seedNotifications(worker, [
			{
				tag: "moltis:session:modern-chat",
				data: { sessionKey: "modern-chat", notificationId: "new", count: 1 },
			},
		]);

		const badge = await worker.evaluate(async () => {
			const closing = {
				tag: "moltis:session:modern-chat",
				data: { sessionKey: "modern-chat", notificationId: "old", count: 1 },
			};
			const pending = [];
			const event = new Event("notificationclose");
			Object.defineProperties(event, {
				notification: { value: closing },
				waitUntil: { value: (promise) => pending.push(promise) },
			});
			self.dispatchEvent(event);
			await Promise.all(pending);
			return self.__moltisBadgeCount;
		});

		expect(badge).toBe(1);
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

	test("explicit enable revives a remotely removed endpoint", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page);
		const requests = [];
		await mockPushApi(page, { requests });
		await navigateAndWait(page, "/settings/notifications");

		await page.getByRole("button", { name: "Enable push notifications" }).click();
		await expect(page.getByRole("button", { name: "Disable push notifications" })).toBeVisible();
		expect(requests[0]).toMatchObject({ endpoint: "https://push.example.com/fresh", revive: true });
		await expect.poll(() => readPushMarker(page)).toBeNull();
	});

	test("failed explicit revival recovers through presence with revival authorization", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			freshEndpoints: ["https://push.example.com/revival-b", "https://push.example.com/revival-c"],
		});
		const requests = [];
		await mockPushApi(page, {
			presenceStatus: 404,
			requests,
			subscribeStatus: (attempt) => (attempt === 1 ? 500 : 200),
		});
		await navigateAndWait(page, "/settings/notifications");

		await page.getByRole("button", { name: "Enable push notifications" }).click();
		await expect(page.getByText("Server rejected subscription (500)", { exact: true })).toBeVisible();
		await expect.poll(() => readPushMarker(page)).toEqual({ revive: true });
		await page.evaluate(() => window.dispatchEvent(new Event("focus")));
		await expect
			.poll(() => page.evaluate(() => window.__moltisPushHarness.current?.endpoint))
			.toBe("https://push.example.com/revival-c");
		await expect.poll(() => readPushMarker(page)).toBeNull();
		expect(requests).toEqual([
			expect.objectContaining({ endpoint: "https://push.example.com/revival-b", revive: true }),
			expect.objectContaining({
				endpoint: "https://push.example.com/revival-c",
				replaces: "https://push.example.com/revival-b",
				revive: true,
			}),
		]);
	});

	test("presence revocation persists disabled intent without automatic recovery", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			enabled: true,
			endpoint: "https://push.example.com/revoked",
		});
		const requests = [];
		await mockPushApi(page, { presenceStatus: 410, requests });
		await navigateAndWait(page, "/chats");

		await expect.poll(() => page.evaluate(() => localStorage.getItem("moltis-push-enabled"))).toBe("0");
		await expect.poll(() => page.evaluate(() => window.__moltisPushHarness.unsubscribeCount)).toBe(1);
		const state = await page.evaluate(async () => {
			const cache = await caches.open("moltis-state");
			return {
				disabled: Boolean(await cache.match("/__moltis__/push-disabled")),
				pending: Boolean(await cache.match("/__moltis__/push-rotation-pending")),
			};
		});
		expect(state).toEqual({ disabled: true, pending: false });
		expect(requests).toEqual([]);
	});

	test("a stale cross-tab presence revocation cannot disable a newer explicit revival", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			enabled: true,
			endpoint: "https://push.example.com/cross-tab-a",
			sharedAcrossTabs: true,
		});
		let stalePresenceRoute;
		const refreshedPresence = [];
		await mockPushApi(page, {
			presenceHandler: (route) => {
				if (!stalePresenceRoute) {
					stalePresenceRoute = route;
					return;
				}
				refreshedPresence.push(route.request().postDataJSON());
				return route.fulfill({ status: 200, body: "" });
			},
		});
		await navigateAndWait(page, "/settings/notifications");
		await expect.poll(() => Boolean(stalePresenceRoute)).toBe(true);
		const staleGeneration = stalePresenceRoute.request().postDataJSON().intent_generation;

		await page.getByRole("button", { name: "Disable push notifications" }).click();
		await expect(page.getByRole("button", { name: "Enable push notifications" })).toBeVisible();

		const revivalPage = await context.newPage();
		await installPushManagerHarness(revivalPage, {
			freshEndpoint: "https://push.example.com/cross-tab-b",
			sharedAcrossTabs: true,
		});
		await mockPushApi(revivalPage);
		await navigateAndWait(revivalPage, "/settings/notifications");
		await revivalPage.getByRole("button", { name: "Enable push notifications" }).click();
		await expect(revivalPage.getByRole("button", { name: "Disable push notifications" })).toBeVisible();
		await expect.poll(() => readPushMarker(revivalPage)).toBeNull();

		const readsBeforeRevocation = await page.evaluate(() => window.__moltisPushHarness.getSubscriptionCount);
		await stalePresenceRoute.fulfill({ status: 410, body: "" });
		await expect
			.poll(() => page.evaluate(() => window.__moltisPushHarness.getSubscriptionCount))
			.toBeGreaterThan(readsBeforeRevocation);
		await expect
			.poll(() =>
				revivalPage.evaluate(async () => ({
					endpoint: localStorage.getItem("__moltisPushHarnessSharedEndpoint"),
					enabled: localStorage.getItem("moltis-push-enabled"),
					generation: Number.parseInt(
						await (await (await caches.open("moltis-state")).match("/__moltis__/push-intent-generation")).text(),
						10,
					),
				})),
			)
			.toEqual({
				endpoint: "https://push.example.com/cross-tab-b",
				enabled: "1",
				generation: staleGeneration + 2,
			});
		await expect
			.poll(() =>
				refreshedPresence.some(
					(payload) =>
						payload.endpoint === "https://push.example.com/cross-tab-b" &&
						payload.intent_generation === staleGeneration + 2,
				),
			)
			.toBe(true);
	});

	test("worker revocation marker overrides stale page enabled intent", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			enabled: true,
			endpoint: "https://push.example.com/revoked",
		});
		const requests = [];
		await mockPushApi(page, { requests });
		await navigateAndWait(page, "/chats");
		requests.length = 0;
		await page.evaluate(async () => {
			localStorage.setItem("moltis-push-enabled", "1");
			await navigator.locks.request("moltis-push-state", async () => {
				const cache = await caches.open("moltis-state");
				await cache.put("/__moltis__/push-disabled", new Response("1"));
				await cache.delete("/__moltis__/push-enabled");
			});
		});

		await page.reload();
		await expect.poll(() => page.evaluate(() => window.__moltisPushHarness.unsubscribeCount)).toBe(1);
		expect(requests).toEqual([]);
	});

	test("presence 404 replaces the unknown browser capability without reviving", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			enabled: true,
			endpoint: "https://push.example.com/unknown",
		});
		const requests = [];
		await mockPushApi(page, { presenceStatus: 404, requests });
		await navigateAndWait(page, "/chats");

		await expect
			.poll(() => page.evaluate(() => window.__moltisPushHarness.current?.endpoint))
			.toBe("https://push.example.com/fresh");
		await expect.poll(() => readPushMarker(page)).toBeNull();
		expect(requests[0]).toMatchObject({
			endpoint: "https://push.example.com/fresh",
			replaces: "https://push.example.com/unknown",
			revive: false,
		});
		expect(await page.evaluate(() => window.__moltisPushHarness.unsubscribeCount)).toBe(1);
	});

	test("retries failed VAPID rotation with the fresh endpoint and original replacement", async ({ page, context }) => {
		await context.grantPermissions(["notifications"]);
		await installPushManagerHarness(page, {
			enabled: true,
			endpoint: "https://push.example.com/stale",
			applicationServerKey: [9, 9, 9],
			freshEndpoint: "https://push.example.com/rotated",
			persistAcrossReload: true,
		});
		const requests = [];
		const completedRequests = [];
		await mockPushApi(page, {
			presenceStatus: 200,
			requests,
			completedRequests,
			subscribeStatus: (attempt) => (attempt === 1 ? 500 : 200),
		});
		await navigateAndWait(page, "/chats");

		await expect.poll(() => completedRequests.length).toBe(1);
		await expect
			.poll(() => readPushMarker(page))
			.toEqual({
				replaces: "https://push.example.com/stale",
				revive: false,
			});
		await page.reload();
		await expect.poll(() => completedRequests.length).toBe(2);
		await expect.poll(() => readPushMarker(page)).toBeNull();
		expect(requests).toEqual([
			expect.objectContaining({
				endpoint: "https://push.example.com/rotated",
				replaces: "https://push.example.com/stale",
				revive: false,
			}),
			expect.objectContaining({
				endpoint: "https://push.example.com/rotated",
				replaces: "https://push.example.com/stale",
				revive: false,
			}),
		]);
		const state = await page.evaluate(async () => ({
			currentEndpoint: window.__moltisPushHarness.current?.endpoint,
			pending: Boolean(await (await caches.open("moltis-state")).match("/__moltis__/push-rotation-pending")),
		}));
		expect(state).toEqual({ currentEndpoint: "https://push.example.com/rotated", pending: false });
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
