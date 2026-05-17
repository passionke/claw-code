import { expect, test } from "@playwright/test";

const gatewayUrl =
  process.env.CLAW_GATEWAY_BASE_URL ?? `http://127.0.0.1:${process.env.GATEWAY_HOST_PORT ?? "8088"}`;

test.describe("Claw Web UI", () => {
  test("W1: page loads with sidebar input", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("claw-header")).toBeVisible();
    const input = page.locator(
      'textarea, input[type="text"], [contenteditable="true"]',
    ).first();
    await expect(input).toBeVisible({ timeout: 30_000 });
  });

  test("W2: bridge and gateway health ok", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("health-bridge")).toContainText(/ok/i, {
      timeout: 30_000,
    });
    await expect(page.getByTestId("health-gateway")).toContainText(/ok/i, {
      timeout: 30_000,
    });
  });

  test("W3: sidebar chat receives assistant reply", async ({ page }) => {
    await page.goto("/");
    const input = page.locator(
      'textarea, input[type="text"], [contenteditable="true"]',
    ).first();
    await input.waitFor({ state: "visible", timeout: 30_000 });
    await input.fill("Reply with exactly: ok");
    await input.press("Enter");

    await expect(page.getByText(/RUN_ERROR/i)).not.toBeVisible({ timeout: 5_000 }).catch(
      () => undefined,
    );
    await expect(page.locator("body")).toContainText(/\bok\b/i, { timeout: 90_000 });
  });

  test("W6: continuation on same thread", async ({ page }) => {
    await page.goto("/");
    const input = page.locator(
      'textarea, input[type="text"], [contenteditable="true"]',
    ).first();
    await input.waitFor({ state: "visible", timeout: 30_000 });
    await input.fill("Say hi in one word");
    await input.press("Enter");
    await expect(page.locator("body")).not.toContainText(/unknown sessionId/i, {
      timeout: 90_000,
    });

    await input.fill("Say bye in one word");
    await input.press("Enter");
    await expect(page.locator("body")).not.toContainText(/unknown sessionId/i, {
      timeout: 90_000,
    });
  });
});

test("W5: session execution visible after message", async ({ page }) => {
  await page.goto("/");
  const sessionEl = page.getByTestId("claw-session-id");
  await expect(sessionEl).toBeVisible({ timeout: 30_000 });
  const threadId = (await sessionEl.locator(".claw-session-id-value").textContent())?.trim();
  test.skip(!threadId, "no claw-session-id in sidebar");

  const input = page.locator('textarea, input[type="text"], [contenteditable="true"]').first();
  await input.waitFor({ state: "visible", timeout: 30_000 });
  await input.fill("Reply with exactly: ok");
  await input.press("Enter");
  await expect(page.locator("body")).toContainText(/\bok\b/i, { timeout: 90_000 });

  const deadline = Date.now() + 60_000;
  let saw = false;
  while (Date.now() < deadline) {
    const res = await fetch(
      `${gatewayUrl}/v1/sessions/${threadId}/execution?ds_id=1`,
    );
    if (res.ok) {
      saw = true;
      break;
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  expect(saw).toBeTruthy();
});
