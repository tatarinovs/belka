//! Cloud storage providers and the HTTP plumbing behind them. Shared by the
//! `cloud-uploader` CLI and the desktop GUI.

pub mod client;
pub mod progress;
pub mod provider;
pub mod storage;
pub mod utils;

pub use progress::Progress;
pub use provider::ProviderConfig;
pub use storage::{RemoteFileInfo, StorageProvider};
