//! SteamVR integration.

pub mod dashboard;
pub mod manifest;
pub mod openvr;
pub mod session;

/// What the UI shows about SteamVR.
#[derive(Debug, Clone, Default)]
pub struct VrStatus {
    /// Connected to a running SteamVR.
    pub connected: bool,
    /// Registered as a SteamVR startup app (None = not checked yet).
    pub registered: Option<bool>,
    /// A register/unregister call is running.
    pub busy: bool,
    pub error: Option<String>,
}
