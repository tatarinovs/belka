import { test, expect, type Page } from "@playwright/test";

test.use({
  userAgent: "Mozilla/5.0 (Linux; Android 16) AppleWebKit/537.36 Chrome/140.0.0.0 Mobile Safari/537.36",
  viewport: { width: 412, height: 892 },
  hasTouch: true,
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const state = window as any;
    state.calls = [];
    state.listingDelay = 0;
    const entries = (path: string) => [
      { name: "Folder", path: `${path === "/" ? "" : path}/Folder`, is_dir: true, size: 0, modified: null },
      { name: "file.txt", path: `${path === "/" ? "" : path}/file.txt`, is_dir: false, size: 5, modified: null },
    ];
    state.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => 1,
      unregisterCallback: () => {},
      invoke: async (command: string, args: any = {}) => {
        switch (command) {
          case "load_ui_state": return {};
          case "list_accounts": return [
            { id: "one", name: "Disk One", kind: "yandex" },
            { id: "two", name: "Disk Two", kind: "webdav" },
          ];
          case "list_dir":
            await new Promise((resolve) => setTimeout(resolve, state.listingDelay));
            if (state.failListings) throw new Error("Unavailable");
            return entries(args.path);
          case "list_local": return {
            path: args.path, parent: args.path ? args.path.slice(0, args.path.lastIndexOf("/")) : null,
            crumbs: [], entries: entries(args.path),
          };
          case "cloud_copy": case "download": case "upload":
            state.calls.push({ command, ...args }); return 1;
          case "local_conflicts": return [];
          case "plugin:dialog|message": return "Заменить";
          case "storage_access": return true;
          case "check_update": case "cloud_quota": return null;
          case "plugin:app|version": return "1.3.2";
          case "list_transfers": return { items: [], counts: { queued: 0, running: 0, done: 0, failed: 0, cancelled: 0 } };
          case "plugin:event|listen": return 1;
          case "remember_ui_state": case "app_ready": case "plugin:event|unlisten": return;
          default: throw new Error(`Unexpected IPC command: ${command}`);
        }
      },
    };
    state.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  });
  await page.goto("/");
  await expect(page.locator(".row")).toHaveCount(2);
});

test.afterEach(async ({ page }) => expect(await page.pageErrors()).toEqual([]));

const systemBack = (page: Page) => page.evaluate(() =>
  !window.dispatchEvent(new Event("belka-back", { cancelable: true })),
);

async function selectFolder(page: Page) {
  await page.locator(".row").filter({ hasText: "Folder" }).dispatchEvent("contextmenu");
  await page.getByRole("button", { name: "Ещё", exact: true }).click();
}

test("menus have icons and WebView tap highlights are transparent", async ({ page }) => {
  await page.getByRole("button", { name: "Меню", exact: true }).click();
  for (const item of await page.getByRole("menuitem").all()) {
    await expect(item.locator(".sheet-ico svg")).toHaveCount(1);
    expect(await item.evaluate((el) => getComputedStyle(el).getPropertyValue("-webkit-tap-highlight-color"))).toBe("rgba(0, 0, 0, 0)");
  }
  expect(await systemBack(page)).toBe(true);
  await selectFolder(page);
  for (const item of await page.getByRole("menuitem").all()) {
    await expect(item.locator(".sheet-ico svg")).toHaveCount(1);
  }
});

test("copy cloud folder to phone uses download and preserves source", async ({ page }) => {
  await selectFolder(page);
  await page.getByRole("menuitem", { name: "Копировать в другую папку или облако" }).click();
  await page.getByRole("button", { name: "Телефон", exact: true }).click();
  await expect(page.locator(".title small")).toHaveText("/storage/emulated/0/Download");
  await page.locator(".row").filter({ hasText: "Folder" }).click();
  await expect(page.locator(".title small")).toHaveText("/storage/emulated/0/Download/Folder");
  await page.getByRole("button", { name: "Копировать сюда" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls)).toMatchObject([
    { command: "download", accountId: "one", localDir: "/storage/emulated/0/Download/Folder", items: [{ path: "/Folder", is_dir: true }] },
  ]);
});

test("copy can switch back from phone to another cloud", async ({ page }) => {
  await selectFolder(page);
  await page.getByRole("menuitem", { name: "Копировать в другую папку или облако" }).click();
  await page.getByRole("button", { name: "Телефон", exact: true }).click();
  await page.locator(".tabs button").first().click();
  await page.locator(".tabs button").first().click();
  await page.getByRole("menuitem", { name: "W Disk Two", exact: true }).click();
  await page.getByRole("button", { name: "Копировать сюда" }).click();
  await expect.poll(() => page.evaluate(() => (window as any).calls)).toMatchObject([
    { command: "cloud_copy", accountId: "one", targetAccountId: "two", targetDir: "/", items: [{ path: "/Folder" }] },
  ]);
});

test("Back closes menu, clears selection and then ascends folders", async ({ page }) => {
  await page.locator(".row").filter({ hasText: "Folder" }).click();
  await expect(page.locator(".title small")).toHaveText("Disk One: /Folder");
  await selectFolder(page);
  expect(await systemBack(page)).toBe(true);
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.getByText("Выбрано: 1")).toBeVisible();
  expect(await systemBack(page)).toBe(true);
  await expect(page.getByText("Выбрано: 1")).toHaveCount(0);
  expect(await systemBack(page)).toBe(true);
  await expect(page.locator(".title strong")).toHaveText("Disk One");
  expect(await systemBack(page)).toBe(true);
  await expect(page.getByRole("status")).toContainText("ещё раз");
  expect(await systemBack(page)).toBe(false);
});

test("Back during loading cannot exit and navigation resets exit confirmation", async ({ page }) => {
  expect(await systemBack(page)).toBe(true);
  await page.evaluate(() => (window as any).listingDelay = 800);
  await page.locator(".row").filter({ hasText: "Folder" }).click();
  expect(await systemBack(page)).toBe(true);
  await expect(page.locator(".title small")).toHaveText("Disk One: /Folder");
  expect(await systemBack(page)).toBe(true);
  await expect(page.locator(".title strong")).toHaveText("Disk One");
  expect(await systemBack(page)).toBe(true);
  expect(await systemBack(page)).toBe(false);
});

test("an unavailable restored cloud folder still allows Back to its parent", async ({ page }) => {
  await page.locator(".row").filter({ hasText: "Folder" }).click();
  await expect(page.locator(".title small")).toHaveText("Disk One: /Folder");
  await page.getByRole("button", { name: "Телефон", exact: true }).click();
  await page.evaluate(() => (window as any).failListings = true);
  await page.locator(".tabs button").first().click();
  await expect(page.locator(".error.banner")).toContainText("Unavailable");
  await expect(page.locator(".title small")).toHaveText("Disk One: /Folder");
  await page.evaluate(() => (window as any).failListings = false);
  expect(await systemBack(page)).toBe(true);
  await expect(page.locator(".title strong")).toHaveText("Disk One");
  await expect(page.locator(".row")).toHaveCount(2);
});
