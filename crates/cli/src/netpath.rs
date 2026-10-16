//! Local paths on network shares as UNC paths: a mapped drive `Z:\disk\x`
//! becomes `\\server\share\disk\x`.

use anyhow::Result;
use std::path::Path;

/// `path` with a mapped network drive replaced by its UNC root; a UNC
/// path is returned as it is.
pub fn to_unc(path: &Path) -> Result<String> {
    let text = path.to_string_lossy();
    if text.starts_with(r"\\") || text.starts_with("//") {
        return Ok(text.into_owned());
    }
    let Some(drive) = text.get(..2).filter(|d| d.ends_with(':')) else {
        anyhow::bail!("'{text}' is not a file on a network share");
    };
    let root = drive_unc(drive)?;
    Ok(format!(
        "{}{}",
        root.trim_end_matches(['\\', '/']),
        &text[2..]
    ))
}

#[cfg(windows)]
fn drive_unc(drive: &str) -> Result<String> {
    const NO_ERROR: u32 = 0;
    const ERROR_MORE_DATA: u32 = 234;

    #[link(name = "mpr")]
    extern "system" {
        fn WNetGetConnectionW(
            local_name: *const u16,
            remote_name: *mut u16,
            length: *mut u32,
        ) -> u32;
    }

    let local: Vec<u16> = drive.encode_utf16().chain([0]).collect();
    let mut buf = vec![0u16; 260];
    loop {
        let mut len = buf.len() as u32;
        // SAFETY: `local` is NUL-terminated and `buf` holds `len` u16s.
        let rc = unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) };
        match rc {
            NO_ERROR => {
                let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                return Ok(String::from_utf16_lossy(&buf[..end]));
            }
            ERROR_MORE_DATA if len as usize > buf.len() => buf.resize(len as usize, 0),
            _ => anyhow::bail!("{drive} is not a network drive (error {rc})"),
        }
    }
}

#[cfg(not(windows))]
fn drive_unc(drive: &str) -> Result<String> {
    anyhow::bail!("{drive} is not a network drive")
}
