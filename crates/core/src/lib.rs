//! Portable domain layer. Account access and monitoring are not implemented yet.

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub version: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub monitoring: bool,
}

pub fn runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        monitoring: false,
    }
}
