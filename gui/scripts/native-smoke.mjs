// Opt-in: connect to a running test GUI with WebView2 CDP enabled. Uses only
// a newly generated cloud folder and a caller-provided local test directory.
import { chromium } from "@playwright/test";
import { mkdir, writeFile, readFile, unlink } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { resolve, join } from "node:path";
import assert from "node:assert/strict";

assert(process.env.BELKA_NATIVE_SMOKE === "1", "Set BELKA_NATIVE_SMOKE=1 to allow temporary cloud writes");
const local = resolve(process.env.BELKA_SMOKE_DIR ?? "../target/gui-smoke-profile/local");
await mkdir(local, { recursive: true });
const source = join(local, "fixture.txt");
const data = Buffer.from("Belka native round trip\n".repeat(5000));
const version = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8")).version;
await writeFile(source, data);
const browser = await chromium.connectOverCDP(process.env.BELKA_GUI_CDP ?? "http://127.0.0.1:9223");
const page = browser.contexts()[0].pages()[0];
const invoke = (cmd, args = {}) => page.evaluate(({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args), { cmd, args });
const errors = [];
page.on("pageerror", error => errors.push(error.message));
let root;
let account;
try {
  await page.waitForSelector('[data-pane="left"]');
  const accounts = await invoke("list_accounts");
  const backup = join(local, "smoke-accounts.belka");
  const password = randomBytes(32).toString("hex");
  try {
    assert.equal(await invoke("export_accounts", { path: backup, password }), accounts.length);
    const imported = await invoke("import_accounts", { path: backup, password });
    assert.equal(imported.added, 0);
    assert.equal(imported.skipped, accounts.length);
  } finally {
    await unlink(backup).catch(() => {});
  }
  account = accounts.find(a => a.kind === "yandex");
  assert(account, "A saved Yandex account is required for this smoke test");
  await page.keyboard.press("Alt+F1");
  await page.getByRole("menuitem", { name: "Облака и диски", exact: true }).click();
  const name = `belka-gui-smoke-${Date.now()}`;
  root = await invoke("cloud_mkdir", { accountId: account.id, parent: "/", name });
  assert.equal(root, `/${name}`);
  const left = page.locator('[data-pane="left"]');
  await left.getByText(account.name, { exact: true }).dblclick();
  await left.getByText(name, { exact: true }).waitFor({ timeout: 60000 });
  await left.getByText(name, { exact: true }).dblclick();

  async function settled(minDone = 0) {
    for (let n = 0; n < 450; n++) {
      const list = await invoke("list_transfers");
      if (!list.counts.running && !list.counts.queued && list.counts.done >= minDone) {
        assert.equal(list.counts.failed, 0, JSON.stringify(list.items.filter(x => x.state === "failed")));
        return list;
      }
      await new Promise(r => setTimeout(r, 200));
    }
    throw new Error("Transfer queue did not settle");
  }
  // Two separately queued operations deliberately target the same path.
  await Promise.all([0, 1].map(() => invoke("upload", { accountId: account.id, localPaths: [source], remoteDir: root })));
  await settled();
  await left.getByRole("button", { name: "Обновить", exact: true }).click();
  await left.getByText("fixture.txt", { exact: true }).waitFor({ timeout: 60000 });
  const entries = await invoke("list_dir", { accountId: account.id, path: root });
  assert.equal(entries.length, 1);
  const dest = join(local, "download");
  await Promise.all([0, 1].map(() => invoke("download", { accountId: account.id, items: entries, localDir: dest })));
  let queue = await settled();
  assert.deepEqual(await readFile(join(dest, "fixture.txt")), data);
  // Exercise the real frontend's F5 handler and polling too, rather than
  // relying only on direct IPC for the deliberately duplicated jobs above.
  const uiDestination = join(local, `ui-download-${name}`);
  await mkdir(uiDestination, { recursive: true });
  const right = page.locator('[data-pane="right"]');
  await right.getByRole("button", { name: "Ввести путь", exact: true }).click();
  const address = right.getByRole("textbox", { name: "Путь к папке", exact: true });
  await address.fill(uiDestination);
  await address.press("Enter");
  await right.getByText("Папка пуста", { exact: true }).waitFor();
  const breadcrumbs = right.getByRole("navigation", { name: "Путь", exact: true });
  const layout = await breadcrumbs.evaluate(nav => {
    const bounds = nav.getBoundingClientRect();
    const items = [...nav.children];
    const current = items.at(-1).getBoundingClientRect();
    return {
      currentVisible: current.left >= bounds.left - 1 && current.right <= bounds.right + 1,
      separated: items.every((item, i) => !i || item.getBoundingClientRect().left >= items[i - 1].getBoundingClientRect().right),
      arrowsIntact: [...nav.querySelectorAll(":scope > svg")].every(svg => svg.getBoundingClientRect().width === 12),
    };
  });
  assert.deepEqual(layout, { currentVisible: true, separated: true, arrowsIntact: true });
  assert.equal(await breadcrumbs.getByRole("button").last().getAttribute("title"), uiDestination);
  await left.getByText("fixture.txt", { exact: true }).click();
  await page.keyboard.press("F5");
  queue = await settled(queue.counts.done + 1);
  assert.deepEqual(await readFile(join(uiDestination, "fixture.txt")), data);
  await right.getByText("fixture.txt", { exact: true }).waitFor();
  const preview = await invoke("preview_file", { accountId: account.id, path: entries[0].path,
    size: entries[0].size, modified: entries[0].modified, text: true });
  assert.equal(preview.text, data.toString());
  assert.equal(await invoke("plugin:app|version"), version);
  await page.getByRole("button", { name: "Меню", exact: true }).click();
  await page.getByRole("menuitem", { name: "О приложении" }).click();
  await page.getByRole("dialog").filter({ hasText: `Версия ${version}` }).waitFor();
  await page.keyboard.press("Escape");
  assert.deepEqual(errors, []);
  await page.screenshot({ path: join(local, "native-smoke.png") });
  console.log(JSON.stringify({ nativeGUI: "OK", breadcrumbs: "OK", transfers: queue.counts, preview: "OK", about: "OK", pageErrors: errors }));
} finally {
  if (root && account && /^\/belka-gui-smoke-\d+$/.test(root)) {
    await invoke("cloud_delete", { accountId: account.id, items: [{ path: root, is_dir: true }] });
    console.log("Temporary cloud folder removed");
  }
  await browser.close();
}
