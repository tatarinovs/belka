import { test, expect } from "@playwright/test";
import { cloudAddress, location, nativeEntry, restorePanes } from "../src/lib/desktop-navigation";

const accounts = [{ id: "one", name: "Disk One", kind: "webdav" as const }];

test("cloud addresses preserve account IDs and remote paths without touching local paths", () => {
  const address = cloudAddress("account/a?#", "/Folder/файл #1.txt");
  expect(location(address)).toEqual({ kind: "cloud", account: "account/a?#", path: "/Folder/файл #1.txt" });
  expect(nativeEntry({ path: address, name: "файл #1.txt", is_dir: false, size: 3, modified: null }).path).toBe("/Folder/файл #1.txt");
  expect(location("C:\\Folder")).toEqual({ kind: "local", path: "C:\\Folder" });
  expect(location("")).toEqual({ kind: "root" });
});

test("universal tabs restore clouds on either side and filter removed accounts", () => {
  const panes = restorePanes(JSON.stringify({
    left: { tabs: [{ id: "a", path: "C:\\" }], active: "a" },
    right: { tabs: [{ id: "b", path: "cloud://one/" }, { id: "c", path: "cloud://removed/" }], active: "c" },
  }), accounts);
  expect(panes.right).toEqual({ tabs: [{ id: "b", path: "cloud://one/" }], active: "b" });
  expect(panes.left.tabs[0].path).toBe("C:\\");
});

test("missing or malformed session data starts both panes at the combined root", () => {
  for (const raw of [null, "invalid", "null", JSON.stringify({ left: { tabs: [null] } })]) {
    const panes = restorePanes(raw, accounts);
    expect(panes.left.tabs).toEqual([{ id: expect.any(String), path: "" }]);
    expect(panes.right.tabs).toEqual([{ id: expect.any(String), path: "" }]);
  }
});
