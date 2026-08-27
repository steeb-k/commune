//! Where data lives on disk.
//!
//! The application derives these per platform (`GLib`'s XDG answers, macOS's
//! `Library`, Android's `no_backup` next to `getFilesDir()` — see the long
//! comment on the application's `utils::DataType::base_dir_path`). The core
//! takes the two roots from [`crate::config`] instead: whoever embeds the
//! core already knows the right answer for its platform.

use std::path::PathBuf;

use crate::config;

/// The type of data.
#[derive(Debug, Clone, Copy)]
pub enum DataType {
    /// Data that should not be deleted.
    Persistent,
    /// Cache that can be deleted freely.
    Cache,
}

impl DataType {
    /// The path of the directory where data should be stored, depending on
    /// this type.
    #[must_use]
    pub fn dir_path(self) -> PathBuf {
        let config = config::get();
        match self {
            DataType::Persistent => config.data_dir.clone(),
            DataType::Cache => config.cache_dir.clone(),
        }
    }
}
