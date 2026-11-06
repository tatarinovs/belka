use crate::{error::CmdResult, ui_state::UiState};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct DesktopSettings {
    windows: bool,
    close_to_tray: bool,
    autostart: bool,
}

#[tauri::command]
pub fn get_desktop_settings(state: State<'_, UiState>) -> CmdResult<DesktopSettings> {
    Ok(DesktopSettings {
        windows: cfg!(target_os = "windows"),
        close_to_tray: state.close_to_tray(),
        autostart: autostart_enabled()?,
    })
}

#[tauri::command]
pub fn set_close_to_tray(state: State<'_, UiState>, enabled: bool) -> CmdResult<()> {
    Ok(state.set_close_to_tray(enabled)?)
}

#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
fn autostart_enabled() -> anyhow::Result<bool> {
    use winreg::HKCU;
    let value = HKCU
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>("Belka"));
    match value {
        Ok(value) => Ok(!value.is_empty()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => {
            Err(anyhow::Error::from(err).context("Не удалось прочитать настройки автозапуска"))
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn autostart_enabled() -> anyhow::Result<bool> {
    Ok(false)
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> CmdResult<()> {
    #[cfg(target_os = "windows")]
    {
        use anyhow::Context;
        use winreg::HKCU;
        let (key, _) = HKCU
            .create_subkey(RUN_KEY)
            .context("Не удалось открыть настройки автозапуска")?;
        if enabled {
            let exe = std::env::current_exe().context("Не удалось определить путь к Belka")?;
            key.set_value("Belka", &format!("\"{}\"", exe.display()))
                .context("Не удалось включить автозапуск")?;
        } else if let Err(err) = key.delete_value("Belka") {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(anyhow::Error::from(err)
                    .context("Не удалось отключить автозапуск")
                    .into());
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enabled;
        Err(anyhow::anyhow!("Автозапуск доступен только в Windows").into())
    }
}
