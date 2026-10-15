/** The Android build gets the one-pane phone layout. */
export const isMobile = /Android/i.test(navigator.userAgent);

// Shared components get touch-sized controls through `:root.mobile` rules.
if (isMobile) document.documentElement.classList.add("mobile");

/** Where downloads go unless the user picks another folder. */
export const ANDROID_DOWNLOADS = "/storage/emulated/0/Download";
