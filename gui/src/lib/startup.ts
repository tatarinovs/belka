import { tick } from "svelte";
import { api } from "./api";

/** Show only after Svelte's updates have reached the browser's paint cycle. */
export async function showWhenDrawn() {
  await tick();
  // Hidden WebViews can suspend animation frames. Flush layout there too,
  // and bound only the paint wait, never the application's initialization.
  document.documentElement.getBoundingClientRect();
  await new Promise<void>((resolve) => {
    let frame = 0;
    const finish = () => {
      clearTimeout(timer);
      cancelAnimationFrame(frame);
      resolve();
    };
    const timer = setTimeout(finish, 250);
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(finish);
    });
  });
  await api.appReady();
}
