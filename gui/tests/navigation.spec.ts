import { test, expect, type Page } from "@playwright/test";

// Exercise the real desktop UI; only native IPC and file data are substituted.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const entries = (path: string) => [
      { name: "Folder", path: `${path === "/" ? "" : path}/Folder`, is_dir: true, size: 0, modified: null },
      ...["Alpha.txt", "Beta.txt", "Gamma.txt"].map((name) => ({
        name, path: `${path === "/" ? "" : path}/${name}`, is_dir: false, size: 10, modified: null,
      })),
    ];
    const state = window as unknown as Record<string, unknown>;
    state.__pendingListings = 0;
    state.__copies = [];
    state.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: () => 1,
      unregisterCallback: () => {},
      invoke: async (command: string, args: { path?: string } = {}) => {
        switch (command) {
          case "load_ui_state": return {
            "desktop.locations": JSON.stringify([
              { id: "cloud", name: "Saved cloud", kind: "cloud", account: "two", remote: "/Folder" },
              { id: "local", name: "Saved local", kind: "local", local: "/Folder" },
              { id: "pair", name: "Saved pair", kind: "pair", account: "two", remote: "/Folder", local: "/Folder" },
            ]),
          };
          case "list_accounts": return [
            { id: "one", name: "Disk One", kind: "webdav" },
            { id: "two", name: "Disk Two", kind: "webdav" },
          ];
          case "list_dir": {
            state.__pendingListings = Number(state.__pendingListings) + 1;
            try {
              // Leave time to interact with the other pane during a request.
              if (args.path?.includes("Folder")) await new Promise((resolve) => setTimeout(resolve, 500));
              return entries(args.path ?? "/");
            } finally {
              state.__pendingListings = Number(state.__pendingListings) - 1;
            }
          }
          case "list_local": return {
            path: args.path ?? "", parent: args.path ? "" : null,
            crumbs: args.path ? [{ name: "Folder", path: args.path }] : [],
            entries: args.path ? entries(args.path) : [{ name: "Local disk", path: "/Folder", is_dir: true, size: 0, modified: null, icon: "drive" }],
          };
          case "cloud_copy": case "download": case "upload":
            (state.__copies as unknown[]).push({ command, ...args }); return 1;
          case "local_conflicts": return [];
          case "plugin:dialog|message": return "Заменить";
          case "list_transfers": return { items: [], counts: { queued: 0, running: 0, done: 0, failed: 0, cancelled: 0 } };
          case "cloud_quota": return null;
          case "get_desktop_settings": return { windows: true, close_to_tray: false, autostart: false };
          case "plugin:app|version": return "1.3.1";
          case "plugin:event|listen": return 1;
          case "remember_ui_state": case "app_ready": case "plugin:event|unlisten": return;
          default: throw new Error(`Unexpected IPC command: ${command}`);
        }
      },
    };
    state.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  });
  await page.goto("/");
  await expect(page.locator('[data-pane="left"] .row')).toHaveCount(3);
  await page.locator('[data-pane="left"] .row[data-path="cloud://one/"]').dblclick();
  await expect(page.locator('[data-pane="left"] .row')).toHaveCount(4);
});

test.afterEach(async ({ page }) => {
  // Any unhandled IPC or runtime rejection is a test failure, too.
  const errors = await page.pageErrors();
  expect(errors).toEqual([]);
});

const pane = (page: Page, side = "cloud") => page.locator(`[data-pane="${side === "cloud" ? "left" : side === "local" ? "right" : side}"]`);
const grid = (page: Page, side = "cloud") => pane(page, side).getByRole("grid");

async function expectKeyboard(page: Page, side = "cloud") {
  await expect(grid(page, side)).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(pane(page, side).locator(".row.selected")).toHaveCount(1);
  await expect(pane(page, side).locator(".row.cursor")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Tab");
  await expect(grid(page, side === "cloud" ? "local" : "cloud")).toBeFocused();
}

test("account menu returns focus to the replacement pane", async ({ page }) => {
  await page.keyboard.press("Alt+F1");
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page).getByRole("navigation").getByRole("button", { name: "Disk Two", exact: true })).toBeVisible();
  await expectKeyboard(page);
});

test("selecting the same account opens its root and restores focus", async ({ page }) => {
  await pane(page).locator('.row[data-path$="/Folder"]').dblclick();
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await page.keyboard.press("Alt+F1");
  await page.getByRole("menuitem", { name: "Disk One", exact: true }).click();
  await expect(pane(page).locator('.row[data-path$="/Folder"]')).toBeVisible();
  await expectKeyboard(page);
});

test("opening a folder from the context menu restores keyboard navigation", async ({ page }) => {
  await pane(page).locator('.row[data-path$="/Folder"]').click({ button: "right" });
  await page.getByRole("menuitem", { name: "Открыть Enter", exact: true }).click();
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expectKeyboard(page);
});

test("closing a tab with the mouse focuses the remaining pane", async ({ page }) => {
  await page.getByLabel("Новая вкладка", { exact: true }).first().click();
  await page.getByLabel("Закрыть вкладку", { exact: true }).last().click();
  await expectKeyboard(page);
});

test("row clicks, folder entry, parent, history and tab selection keep the grid focused", async ({ page }) => {
  await pane(page).locator('.row[data-path$="/Alpha.txt"]').click();
  await page.keyboard.press("ArrowDown");
  await expect(pane(page).locator(".row.cursor")).toHaveAttribute("data-path", "cloud://one/Beta.txt");
  await expect(grid(page)).toBeFocused();
  await pane(page).locator('.row[data-path$="/Folder"]').dblclick();
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expect(grid(page)).toBeFocused();
  await page.keyboard.press("Backspace");
  await expect(pane(page).locator('.row[data-path$="/Folder"]')).toHaveAttribute("aria-selected", "true");
  await pane(page).getByLabel("Назад", { exact: true }).click();
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expect(grid(page)).toBeFocused();
  await page.getByRole("tab").first().click();
  await expectKeyboard(page);
});

test("a slow folder load does not take focus from the other pane", async ({ page }) => {
  await pane(page).locator('.row[data-path$="/Folder"]').dblclick();
  await pane(page, "local").getByRole("textbox", { name: "Поиск в текущей папке" }).fill("Alpha");
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expect(pane(page, "local").getByRole("textbox")).toBeFocused();
});

test("Backspace edits search text and rename text without navigating", async ({ page }) => {
  const search = pane(page).getByRole("textbox", { name: "Поиск в текущей папке" });
  await search.fill("Alpha");
  await page.keyboard.press("Backspace");
  await expect(search).toHaveValue("Alph");
  await page.keyboard.press("Escape");
  await pane(page).locator('.row[data-path$="/Alpha.txt"]').click();
  await page.keyboard.press("F2");
  const rename = pane(page).locator("input.rename");
  await expect(rename).toBeFocused();
  await page.keyboard.press("Backspace");
  await expect(rename).toHaveValue(".txt");
  await page.keyboard.press("Escape");
  await expect(grid(page)).toBeFocused();
});

test("cancelling account and context menus returns to the file list", async ({ page }) => {
  await page.keyboard.press("Alt+F1");
  await page.keyboard.press("Escape");
  await expect(grid(page)).toBeFocused();
  await pane(page).locator('.row[data-path$="/Folder"]').click({ button: "right" });
  await page.keyboard.press("Escape");
  await expectKeyboard(page);
});

test("new tabs, Ctrl+Tab and Ctrl+W keep keyboard navigation working", async ({ page }) => {
  await grid(page).focus();
  await page.keyboard.press("Control+t");
  await expect(page.getByRole("tab", { selected: true })).toHaveCount(2);
  await expect(grid(page)).toBeFocused();
  await page.keyboard.press("Control+Tab");
  await expect(grid(page)).toBeFocused();
  await page.keyboard.press("Control+w");
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expectKeyboard(page);
});

test("opening a local drive from its context menu keeps focus in the local panel", async ({ page }) => {
  await pane(page, "local").locator('.row[data-path$="/Folder"]').click({ button: "right" });
  await page.getByRole("menuitem", { name: "Открыть Enter", exact: true }).click();
  await expect(pane(page, "local").locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expectKeyboard(page, "local");
});

test("an old account response cannot change the path of the new account", async ({ page }) => {
  await pane(page).locator('.row[data-path$="/Folder"]').dblclick();
  await page.keyboard.press("Alt+F1");
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page).locator('.row[data-path$="/Folder"]')).toBeVisible();
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__pendingListings === 0);
  await expect(page.getByRole("tab", { selected: true }).first()).toHaveAttribute("title", "Disk Two: /");
  await expectKeyboard(page);
});

test("address entry, breadcrumbs and opening a folder in a new tab restore focus", async ({ page }) => {
  await grid(page).focus();
  await page.keyboard.press("Control+l");
  await pane(page).getByRole("textbox", { name: "Путь к папке" }).fill("/Folder");
  await page.keyboard.press("Enter");
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expect(grid(page)).toBeFocused();
  await pane(page).getByRole("navigation", { name: "Путь", exact: true }).getByRole("button", { name: "Disk One", exact: true }).click();
  await expect(pane(page).locator('.row[data-path$="/Folder"]')).toBeVisible();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Control+Enter");
  await expect(page.getByRole("tab")).toHaveCount(3);
  await expect(pane(page).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
  await expectKeyboard(page);
});

test("closing settings returns focus to the panel that opened it", async ({ page }) => {
  await grid(page, "local").focus();
  await page.getByRole("button", { name: "Меню", exact: true }).click();
  await page.getByRole("menuitem", { name: "Настройки", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Настройки" })).toBeVisible();
  await page.getByRole("button", { name: "Закрыть", exact: true }).click();
  await expectKeyboard(page, "local");
});

for (const kind of ["cloud", "local", "pair"] as const) {
  test(`opening a saved ${kind} location focuses its file list`, async ({ page }) => {
    await page.getByRole("button", { name: "Избранное", exact: true }).click();
    if (kind === "pair") await page.getByRole("button", { name: "Пары папок (1)", exact: true }).click();
    await page.getByRole("button", { name: new RegExp(`^Saved ${kind} `) }).click();
    const side = "cloud";
    await expect(pane(page, side).locator('.row[data-path$="/Folder/Folder"]')).toBeVisible();
    await expectKeyboard(page, side);
  });
}

test("a new tab starts at the combined root with clouds before local disks", async ({ page }) => {
  await grid(page).focus();
  await page.keyboard.press("Control+t");
  await expect(pane(page).locator(".row .label")).toHaveText(["Disk One", "Disk Two", "Local disk"]);
  await expect(pane(page).getByLabel("Вверх", { exact: true })).toBeDisabled();
  await expect(grid(page)).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(pane(page).locator('.row[data-path="cloud://one/Alpha.txt"]')).toBeVisible();
  await page.keyboard.press("Backspace");
  await expect(pane(page).locator('.row[data-path="cloud://one/"]')).toHaveAttribute("aria-selected", "true");
  await expectKeyboard(page);
});

test("Alt+F2 opens a cloud on the right without renaming the selected file", async ({ page }) => {
  await pane(page).locator('.row[data-path="cloud://one/Alpha.txt"]').click();
  await page.keyboard.press("Alt+F2");
  await expect(page.getByRole("menuitem", { name: "Local disk", exact: true })).toBeVisible();
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page, "local").locator('.row[data-path="cloud://two/Alpha.txt"]')).toBeVisible();
  await expect(pane(page).locator("input.rename")).toHaveCount(0);
  await expectKeyboard(page, "local");
});

test("F5 copies between two clouds in either direction using native paths", async ({ page }) => {
  await page.keyboard.press("Alt+F2");
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page, "local").locator('.row[data-path="cloud://two/Alpha.txt"]')).toBeVisible();
  await pane(page).locator('.row[data-path="cloud://one/Alpha.txt"]').click();
  await page.keyboard.press("F5");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __copies: unknown[] }).__copies)).toMatchObject([
    { command: "cloud_copy", accountId: "one", targetAccountId: "two", targetDir: "/", items: [{ path: "/Alpha.txt" }] },
  ]);
  await pane(page, "local").locator('.row[data-path="cloud://two/Beta.txt"]').click();
  await page.keyboard.press("F5");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __copies: unknown[] }).__copies)).toMatchObject([
    { command: "cloud_copy" },
    { command: "cloud_copy", accountId: "two", targetAccountId: "one", targetDir: "/", items: [{ path: "/Beta.txt" }] },
  ]);
});

test("cloud clipboard paste targets the cloud in the other pane", async ({ page }) => {
  await pane(page).locator('.row[data-path="cloud://one/Gamma.txt"]').click();
  await page.keyboard.press("Control+c");
  await page.keyboard.press("Alt+F2");
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page, "local").locator('.row[data-path="cloud://two/Gamma.txt"]')).toBeVisible();
  await page.keyboard.press("Control+v");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __copies: unknown[] }).__copies)).toMatchObject([
    { command: "cloud_copy", accountId: "one", targetAccountId: "two", targetDir: "/", items: [{ path: "/Gamma.txt" }] },
  ]);
});

test("cloud download and upload work with the local pane on the left", async ({ page }) => {
  await page.keyboard.press("Alt+F1");
  await page.getByRole("menuitem", { name: "Local disk", exact: true }).click();
  await expect(pane(page).locator('.row[data-path="/Folder/Alpha.txt"]')).toBeVisible();
  await page.keyboard.press("Alt+F2");
  await page.getByRole("menuitem", { name: "Disk Two", exact: true }).click();
  await expect(pane(page, "local").locator('.row[data-path="cloud://two/Alpha.txt"]')).toBeVisible();
  await pane(page).locator('.row[data-path="/Folder/Alpha.txt"]').click();
  await page.keyboard.press("F5");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __copies: unknown[] }).__copies)).toMatchObject([
    { command: "upload", accountId: "two", localPaths: ["/Folder/Alpha.txt"], remoteDir: "/" },
  ]);
  await pane(page, "local").locator('.row[data-path="cloud://two/Beta.txt"]').click();
  await page.keyboard.press("F5");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __copies: unknown[] }).__copies)).toMatchObject([
    { command: "upload" },
    { command: "download", accountId: "two", localDir: "/Folder", items: [{ path: "/Beta.txt" }] },
  ]);
});
