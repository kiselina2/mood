use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::ScreenCapture;

/// A single captured screen frame in RGBA format.
#[derive(Clone)]
pub struct Frame {
    /// Raw pixel data in RGBA format, 4 bytes per pixel.
    pub data: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
}
