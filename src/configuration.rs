use std::path::{Path, PathBuf};

use signal_system::{SystemBackend, SystemDaemonConfiguration};
use triad_runtime::{BindingSurface, SocketMode as RuntimeSocketMode};

use crate::error::{Error, Result};

/// System's hand-written daemon configuration, wrapping the
/// `signal-system` startup contract that the Persona manager encodes when it
/// spawns `system-daemon`. The contract `SystemDaemonConfiguration` is the
/// externally-consumed boundary; this type adds the cached `PathBuf`s the
/// emitted daemon shell binds from through `triad_runtime::BindingSurface`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Configuration {
    raw: SystemDaemonConfiguration,
    system_socket_path: PathBuf,
    supervision_socket_path: PathBuf,
    state_dir: PathBuf,
}

impl Configuration {
    pub fn from_raw(raw: SystemDaemonConfiguration) -> Self {
        let system_socket_path = PathBuf::from(raw.system_socket_path.as_str());
        let supervision_socket_path = PathBuf::from(raw.supervision_socket_path.as_str());
        let state_dir = system_socket_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            raw,
            system_socket_path,
            supervision_socket_path,
            state_dir,
        }
    }

    /// Read and decode the binary rkyv configuration from the daemon's single
    /// startup-argument file path. Daemons never parse NOTA — the contract is
    /// signal-encoded before it reaches the process.
    pub fn from_binary_path(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::ConfigurationRead {
            path: path.to_path_buf(),
            source,
        })?;
        let raw = SystemDaemonConfiguration::from_rkyv_bytes(&bytes)
            .map_err(|_| Error::ConfigurationArchiveDecode)?;
        Ok(Self::from_raw(raw))
    }

    pub fn raw(&self) -> &SystemDaemonConfiguration {
        &self.raw
    }

    pub fn backend(&self) -> SystemBackend {
        self.raw.backend
    }

    fn system_socket_mode(&self) -> RuntimeSocketMode {
        RuntimeSocketMode::new(self.raw.system_socket_mode.into_u32())
    }

    fn supervision_socket_mode(&self) -> RuntimeSocketMode {
        RuntimeSocketMode::new(self.raw.supervision_socket_mode.into_u32())
    }
}

impl BindingSurface for Configuration {
    fn socket_path(&self) -> &Path {
        &self.system_socket_path
    }

    fn socket_mode(&self) -> Option<RuntimeSocketMode> {
        Some(self.system_socket_mode())
    }

    fn meta_socket_path(&self) -> Option<&Path> {
        Some(&self.supervision_socket_path)
    }

    fn meta_socket_mode(&self) -> Option<RuntimeSocketMode> {
        Some(self.supervision_socket_mode())
    }

    fn database_path(&self) -> &Path {
        // `system` is stateless — it opens no durable store. The emitted shell
        // never binds this path; the accessor exists only because the trait
        // requires it, so it points at the daemon's state directory.
        &self.state_dir
    }
}

impl From<SystemDaemonConfiguration> for Configuration {
    fn from(raw: SystemDaemonConfiguration) -> Self {
        Self::from_raw(raw)
    }
}
