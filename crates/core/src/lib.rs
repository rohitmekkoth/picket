//! Portable event store and synthetic alignment engine. Account collectors are not connected.

use serde::Serialize;

pub mod engine;
pub mod event;
pub mod store;

/// Read-only process details shared by the CLI and desktop shell.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub version: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub monitoring: bool,
}

/// Reports local process metadata without touching accounts or personal data.
pub fn runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        monitoring: false,
    }
}
