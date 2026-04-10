import { describe, it, expect, beforeEach } from "@playwright/test";
import { navigateAndWait, waitForWsConnected, watchPageErrors } from "../helpers.js";

describe("Projects Page", () => {
  beforeEach(async ({ page }) => {
    await navigateAndWait(page, "/settings/projects");
    await waitForWsConnected(page);
  });

  it("renders the projects page header", async ({ page }) => {
    const pageErrors = watchPageErrors(page);

    await expect(page.getByRole("heading", { name: "Repositories" })).toBeVisible();

    expect(pageErrors).toHaveLength(0);
  });

  it("shows the projects section in the settings sidebar", async ({ page }) => {
    const pageErrors = watchPageErrors(page);

    await expect(page.getByRole("link", { name: "Projects" })).toBeVisible();

    expect(pageErrors).toHaveLength(0);
  });

  it("shows auto-detect and clear buttons", async ({ page }) => {
    const pageErrors = watchPageErrors(page);

    await expect(page.getByRole("button", { name: "Auto-detect" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Clear All" })).toBeVisible();

    expect(pageErrors).toHaveLength(0);
  });

  it("shows directory input and add button", async ({ page }) => {
    const pageErrors = watchPageErrors(page);

    await expect(page.getByPlaceholder("Directory path...")).toBeVisible();
    await expect(page.getByRole("button", { name: "Add" })).toBeVisible();

    expect(pageErrors).toHaveLength(0);
  });
});
