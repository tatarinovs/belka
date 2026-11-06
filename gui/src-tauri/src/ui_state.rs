//! UI preferences live outside the disposable browser profile.
use crate::error::CmdResult;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;

type Values = BTreeMap<String, String>;

pub struct UiState {
    file: PathBuf,
    values: Mutex<Values>,
}

impl UiState {
    pub fn new(dir: &Path) -> Result<Self> {
        let file = dir.join("ui-state.json");
        let values = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Cannot read UI settings")?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Values::new(),
            Err(err) => return Err(err).context("Cannot read UI settings"),
        };
        Ok(Self {
            file,
            values: Mutex::new(values),
        })
    }

    pub fn close_to_tray(&self) -> bool {
        self.values
            .lock()
            .unwrap()
            .get("desktop.closeToTray")
            .is_some_and(|v| v == "true")
    }

    pub fn set_close_to_tray(&self, enabled: bool) -> Result<()> {
        let mut current = self.values.lock().unwrap();
        let mut next = current.clone();
        next.insert("desktop.closeToTray".into(), enabled.to_string());
        self.save(&next)?;
        *current = next;
        Ok(())
    }

    fn save(&self, values: &Values) -> Result<()> {
        std::fs::create_dir_all(self.file.parent().unwrap())?;
        let temporary = self.file.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(values)?)?;
        std::fs::rename(&temporary, &self.file).context("Cannot save UI settings")
    }
}

#[tauri::command]
pub fn load_ui_state(state: State<'_, UiState>) -> Values {
    state.values.lock().unwrap().clone()
}

#[tauri::command]
pub fn remember_ui_state(state: State<'_, UiState>, key: String, value: String) -> CmdResult<()> {
    let mut current = state.values.lock().unwrap();
    let mut next = current.clone();
    next.insert(key, value);
    state.save(&next)?;
    *current = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_tray_preference_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = UiState::new(dir.path()).unwrap();
        assert!(!state.close_to_tray());
        state.set_close_to_tray(true).unwrap();
        let state = UiState::new(dir.path()).unwrap();
        assert!(state.close_to_tray());
        state.set_close_to_tray(false).unwrap();
        assert!(!UiState::new(dir.path()).unwrap().close_to_tray());
    }

    #[test]
    fn settings_survive_browser_profile_disposal() {
        let dir = tempfile::tempdir().unwrap();
        let state = UiState::new(dir.path()).unwrap();
        assert!(state.values.lock().unwrap().is_empty());
        let values = BTreeMap::from([("tabs".into(), "{\"path\":\"D:\\\\Фото\"}".into())]);
        state.save(&values).unwrap();
        let state = UiState::new(dir.path()).unwrap();
        assert_eq!(&*state.values.lock().unwrap(), &values);
        state.save(&BTreeMap::new()).unwrap();
        assert!(UiState::new(dir.path())
            .unwrap()
            .values
            .lock()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn broken_settings_are_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ui-state.json"), b"broken").unwrap();
        assert!(UiState::new(dir.path()).is_err());
    }
}
