import { test, expect, type Page } from "@playwright/test";

async function boot(page: Page, options: { storageError?: boolean; listingError?: boolean; suspendFrames?: boolean; savedNas?: boolean } = {}) {
  await page.addInitScript((options) => {
    const state = window as unknown as Record<string, any>;
    const listings: { command: string; resolve: () => void }[] = [];
    state.startup = { pending: 0, shown: [] };
    state.releaseListing = (command?: string) => {
      const at = command ? listings.findIndex((listing) => listing.command === command) : 0;
      if (at >= 0) listings.splice(at, 1)[0]?.resolve();
    };
    if (options.suspendFrames) window.requestAnimationFrame = () => 0;
    state.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => 1,
      unregisterCallback: () => {},
      invoke: async (command: string) => {
        switch (command) {
          case "load_ui_state":
            if (options.storageError) throw new Error("Settings unavailable");
            return options.savedNas ? { tabs: JSON.stringify({
              left: { tabs: [{ id: "local", path: "" }], active: "local" },
              right: { tabs: [{ id: "nas", path: "cloud://nas/Slow" }], active: "nas" },
            }) } : {};
          case "list_accounts": return options.savedNas ? [{ id: "nas", name: "Slow NAS", kind: "synology" }] : [];
          case "plugin:app|version": return "1.3.1";
          case "list_local": case "list_dir": {
            state.startup.pending++;
            await new Promise<void>((resolve) => listings.push({ command, resolve }));
            if (options.listingError) throw new Error("Disk unavailable");
            if (command === "list_dir") return [
              { name: "nas-file.txt", path: "/Slow/nas-file.txt", is_dir: false, size: 10, modified: null },
            ];
            return { path: "", parent: null, crumbs: [], entries: [
              { name: "Local disk", path: "C:\\", is_dir: true, size: 0, modified: null, icon: "drive" },
            ] };
          }
          case "app_ready":
            state.startup.shown.push({
              rows: document.querySelectorAll(".row").length,
              grids: document.querySelectorAll('[role="grid"]').length,
              loading: document.querySelectorAll('[role="grid"][aria-busy="true"]').length,
              errors: [...document.querySelectorAll('[role="alert"]')].map((el) => el.textContent),
              retry: [...document.querySelectorAll("button")].some((el) => el.textContent === "Повторить"),
              height: document.querySelector("main")?.getBoundingClientRect().height,
              viewport: window.innerHeight,
            });
            return;
          case "list_transfers": return { items: [], counts: { queued: 0, running: 0, done: 0, failed: 0, cancelled: 0 } };
          case "cloud_quota": return null;
          case "plugin:event|listen": return 1;
          case "remember_ui_state": case "plugin:event|unlisten": return;
          default: throw new Error(`Unexpected IPC command: ${command}`);
        }
      },
    };
    state.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  }, options);
  await page.goto("/");
}

const startup = (page: Page) => page.evaluate(() => (window as any).startup);
const release = (page: Page, command?: string) => page.evaluate((command) => (window as any).releaseListing(command), command);

test.afterEach(async ({ page }) => {
  expect(await page.pageErrors()).toEqual([]);
});

test("startup shows the painted layout while both initial listings are still pending", async ({ page }) => {
  await boot(page);
  await expect.poll(async () => (await startup(page)).pending).toBe(2);
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  const [shown] = (await startup(page)).shown;
  expect(shown).toMatchObject({ rows: 0, grids: 2, loading: 2, errors: [] });
  expect(shown.height).toBe(shown.viewport);
  await release(page);
  await release(page);
  await expect(page.locator(".row")).toHaveCount(2);

  // Opening another tab must not signal startup readiness again.
  await page.getByLabel("Новая вкладка", { exact: true }).first().click();
  await expect.poll(async () => (await startup(page)).pending).toBe(3);
  await release(page);
  await expect(page.locator(".row")).toHaveCount(2);
  expect((await startup(page)).shown).toHaveLength(1);
});

test("a failed initial listing renders an error in the already shown window", async ({ page }) => {
  await boot(page, { listingError: true });
  await expect.poll(async () => (await startup(page)).pending).toBe(2);
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  await release(page);
  await release(page);
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  expect((await startup(page)).shown[0]).toMatchObject({ grids: 2, rows: 0 });
  await expect(page.getByText("Disk unavailable", { exact: true })).toHaveCount(2);
});

test("a restored slow NAS tab does not delay startup or block the other pane", async ({ page }) => {
  await boot(page, { savedNas: true });
  await expect.poll(async () => (await startup(page)).pending).toBe(2);
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  const nas = page.locator('[data-pane="right"]');
  await expect(nas.getByRole("grid")).toHaveAttribute("aria-busy", "true");
  await expect(nas.getByText("Загрузка списка…", { exact: true })).toBeVisible();
  await release(page, "list_local");
  const local = page.locator('[data-pane="left"]');
  await expect(local.locator(".row")).toHaveCount(2);
  await local.getByRole("textbox", { name: "Поиск в текущей папке" }).fill("Local disk");
  await expect(local.locator(".row")).toHaveCount(1);
  await expect(nas.getByRole("grid")).toHaveAttribute("aria-busy", "true");
  await release(page, "list_dir");
  await expect(nas.getByText("nas-file.txt", { exact: true })).toBeVisible();
  expect((await startup(page)).shown).toHaveLength(1);
});

test("a settings failure shows a usable retry page", async ({ page }) => {
  await boot(page, { storageError: true });
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  expect((await startup(page)).shown[0]).toMatchObject({ grids: 0, retry: true });
  await expect(page.getByRole("alert")).toContainText("Settings unavailable");
});

test("suspended animation frames in a hidden WebView do not prevent showing a ready UI", async ({ page }) => {
  await boot(page, { suspendFrames: true });
  await expect.poll(async () => (await startup(page)).pending).toBe(2);
  await expect.poll(async () => (await startup(page)).shown.length).toBe(1);
  expect((await startup(page)).shown[0]).toMatchObject({ grids: 2, rows: 0, loading: 2 });
});
