//! Putting text on the system clipboard.

use anyhow::Result;

#[cfg(windows)]
pub fn set_text(text: &str) -> Result<()> {
    use anyhow::bail;
    use std::os::raw::c_void;
    use std::time::Duration;

    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(mem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(mem: *mut c_void) -> i32;
        fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    }

    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // Another program may hold the clipboard for a moment.
    // SAFETY: plain Win32 clipboard calls; the clipboard owns the memory
    // once SetClipboardData succeeds, and it is freed here otherwise.
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !opened {
            bail!("The clipboard is busy");
        }
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        let copied = !mem.is_null() && {
            let ptr = GlobalLock(mem).cast::<u16>();
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                GlobalUnlock(mem);
            }
            !ptr.is_null()
                && EmptyClipboard() != 0
                && !SetClipboardData(CF_UNICODETEXT, mem).is_null()
        };
        if !copied && !mem.is_null() {
            GlobalFree(mem);
        }
        CloseClipboard();
        if !copied {
            bail!("Failed to copy to the clipboard");
        }
    }
    Ok(())
}

/// pbcopy on macOS; wl-copy or xclip elsewhere.
#[cfg(not(windows))]
pub fn set_text(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };
    for (tool, args) in tools {
        let Ok(mut child) = Command::new(tool)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }
    anyhow::bail!("No clipboard tool found (pbcopy, wl-copy or xclip); use --no-clipboard")
}
