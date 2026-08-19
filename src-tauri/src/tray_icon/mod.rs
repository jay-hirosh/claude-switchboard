//! Dynamic tray icon renderer. Selects between the macOS dual-pie and the
//! Windows concentric design at compile time and produces fresh PNG bytes
//! on every call.

use serde::{Deserialize, Serialize};

pub mod digits;
pub mod shared;
#[cfg(not(target_os = "windows"))]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// User-selectable tray icon layout. `Dual` is the original design (both
/// buckets, always drawn); `Primary` and `Minimal` trade off detail for a
/// smaller/quieter icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    /// Both buckets, one pie/ring each. The original design.
    #[default]
    Dual,
    /// A single pie/ring for whichever bucket (5h or 7d) is more urgent.
    Primary,
    /// A plain status-colored dot — no digits, smallest visual footprint.
    Minimal,
}

/// Renders the tray icon for the given usage state. Returns PNG bytes ready
/// for `tauri::image::Image::from_bytes`. The output dimensions are
/// platform-dependent: 88×44 on macOS, 32×32 on Windows.
pub fn render(five_hour: Option<f64>, seven_day: Option<f64>, paused: bool, style: IconStyle) -> Vec<u8> {
    #[cfg(target_os = "macos")]
    {
        macos::render(five_hour, seven_day, paused, style)
    }
    #[cfg(target_os = "windows")]
    {
        windows::render(five_hour, seven_day, paused, style)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Other platforms (Linux, etc.) get the macOS layout as a reasonable default.
        macos::render(five_hour, seven_day, paused, style)
    }
}
