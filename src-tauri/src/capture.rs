use image::{DynamicImage, RgbaImage};
use screenshots::Screen;
use std::io::Cursor;

#[derive(Debug)]
pub enum CaptureError {
    NoScreen,
    ScreenshotFailed(String),
    EncodeFailed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoScreen => write!(f, "No screen found for capture region"),
            Self::ScreenshotFailed(s) => write!(f, "Screenshot failed: {}", s),
            Self::EncodeFailed(s) => write!(f, "PNG encode failed: {}", s),
        }
    }
}

/// Screen region in physical pixels.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Capture the entire primary screen as PNG bytes.
/// Returns (png_bytes, region) where region is the full screen dimensions.
pub fn capture_full_screen() -> Result<(Vec<u8>, Region), CaptureError> {
    let screens = Screen::all().map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;
    let screen = screens.into_iter().next().ok_or(CaptureError::NoScreen)?;
    let di = screen.display_info;
    let region = Region {
        x: di.x,
        y: di.y,
        width: di.width,
        height: di.height,
    };

    let full = screen
        .capture()
        .map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;

    let w = full.width();
    let h = full.height();
    let rgba_data = full.into_raw();
    let rgba_img = RgbaImage::from_raw(w, h, rgba_data)
        .ok_or(CaptureError::EncodeFailed("Failed to create RGBA image".into()))?;
    let img = DynamicImage::ImageRgba8(rgba_img);

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| CaptureError::EncodeFailed(e.to_string()))?;

    Ok((buf, region))
}

/// Capture a screen region as PNG bytes.
///
/// Coordinates are in **physical pixels** (Tauri 2's outer_position/outer_size
/// already return physical values, so no DPI scaling needed by caller).
/// The capture uses the `screenshots` crate which calls Win32 GDI on Windows.
pub fn capture_region(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    let screens = Screen::all().map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;

    // Find the screen containing the top-left corner of the capture region
    let screen = screens
        .into_iter()
        .find(|s| {
            let di = s.display_info;
            x >= di.x && x < di.x + di.width as i32
                && y >= di.y && y < di.y + di.height as i32
        })
        .ok_or(CaptureError::NoScreen)?;

    // Capture the full screen
    let full = screen
        .capture()
        .map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;

    // Convert screenshots ImageBuffer to our image crate's DynamicImage.
    // screenshots 0.8 returns image::RgbaImage directly; bridge via raw bytes
    // in case the re-exported image version differs from ours.
    let w = full.width();
    let h = full.height();
    let rgba_data = full.into_raw();
    let rgba_img = RgbaImage::from_raw(w, h, rgba_data)
        .ok_or(CaptureError::EncodeFailed("Failed to create RGBA image from buffer".into()))?;
    let img = DynamicImage::ImageRgba8(rgba_img);

    // Crop to the requested region
    let di = screen.display_info;
    let crop_x = (x - di.x) as u32;
    let crop_y = (y - di.y) as u32;
    let cropped = img.crop_imm(crop_x, crop_y, width, height);

    // Encode to PNG
    let mut buf = Vec::new();
    cropped
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| CaptureError::EncodeFailed(e.to_string()))?;

    Ok(buf)
}

#[cfg(test)]
mod tests {
    // Screen capture requires a display — these tests run manually on Windows,
    // not in CI. Keeping the module for future integration tests.

    #[test]
    fn capture_error_display() {
        use super::CaptureError;
        let e = CaptureError::NoScreen;
        assert_eq!(format!("{}", e), "No screen found for capture region");
    }
}
