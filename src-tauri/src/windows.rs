use serde::{Deserialize, Serialize};

/// Information about a visible top-level window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Window handle as u64 (safe for JSON serialization across FFI boundary).
    pub hwnd: u64,
    /// Window title text.
    pub title: String,
    /// Process ID that owns the window.
    pub pid: u32,
    /// Left edge in physical pixels.
    pub x: i32,
    /// Top edge in physical pixels.
    pub y: i32,
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// Win32 window class name (e.g. "Chrome_WidgetWin_1").
    pub class_name: String,
}

// ---------------------------------------------------------------------------
// Win32 FFI implementation (Windows only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use super::WindowInfo;

    // Win32 type aliases
    type HWND = isize;
    type BOOL = i32;
    type LPARAM = isize;
    type DWORD = u32;

    #[repr(C)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(lpEnumFunc: unsafe extern "system" fn(HWND, LPARAM) -> BOOL, lParam: LPARAM) -> BOOL;
        fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
        fn GetWindowTextLengthW(hWnd: HWND) -> i32;
        fn IsWindowVisible(hWnd: HWND) -> BOOL;
        fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
        fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;
        fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
    }

    /// Callback invoked by `EnumWindows` for each top-level window.
    ///
    /// `lparam` is a pointer to `Vec<WindowInfo>` reinterpreted as `isize`.
    unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // Skip invisible windows.
        if IsWindowVisible(hwnd) == 0 {
            return 1; // TRUE — continue enumeration
        }

        // Skip windows with no title text.
        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return 1;
        }

        // Read the title.
        let buf_size = (title_len + 1) as usize;
        let mut title_buf: Vec<u16> = vec![0u16; buf_size];
        let copied = GetWindowTextW(hwnd, title_buf.as_mut_ptr(), buf_size as i32);
        if copied <= 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&title_buf[..copied as usize]);
        if title.is_empty() {
            return 1;
        }

        // Read the window rectangle.
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return 1;
        }

        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;

        // Skip zero-size windows.
        if width == 0 && height == 0 {
            return 1;
        }

        // Get the owning process ID.
        let mut pid: DWORD = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);

        // Read the window class name.
        let mut class_buf: Vec<u16> = vec![0u16; 256];
        let class_len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), 256);
        let class_name = if class_len > 0 {
            String::from_utf16_lossy(&class_buf[..class_len as usize])
        } else {
            String::new()
        };

        // Push the result into the caller-provided Vec.
        let results = &mut *(lparam as *mut Vec<WindowInfo>);
        results.push(WindowInfo {
            hwnd: hwnd as u64,
            title,
            pid,
            x: rect.left,
            y: rect.top,
            width,
            height,
            class_name,
        });

        1 // TRUE — continue enumeration
    }

    /// Enumerate all visible top-level windows with non-empty titles and
    /// non-zero size.
    pub fn list_windows() -> Vec<WindowInfo> {
        let mut results: Vec<WindowInfo> = Vec::new();
        unsafe {
            EnumWindows(
                enum_callback,
                &mut results as *mut Vec<WindowInfo> as LPARAM,
            );
        }
        results
    }
}

// ---------------------------------------------------------------------------
// Stub implementation (non-Windows)
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::WindowInfo;

    /// Returns an empty list on non-Windows platforms.
    pub fn list_windows() -> Vec<WindowInfo> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enumerate all visible top-level windows.
///
/// On non-Windows platforms this returns an empty `Vec`.
pub fn list_windows() -> Vec<WindowInfo> {
    platform::list_windows()
}

/// Filter visible windows by case-insensitive substring match on the title.
///
/// An empty `query` returns all windows (equivalent to `list_windows()`).
pub fn find_windows(query: &str) -> Vec<WindowInfo> {
    if query.is_empty() {
        return list_windows();
    }
    let query_lower = query.to_lowercase();
    list_windows()
        .into_iter()
        .filter(|w| w.title.to_lowercase().contains(&query_lower))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_windows_empty_query_returns_all() {
        // On non-Windows this is an empty Vec; on Windows it matches list_windows().
        let all = list_windows();
        let found = find_windows("");
        assert_eq!(all.len(), found.len());
    }

    #[test]
    fn window_info_serializes_to_json() {
        let info = WindowInfo {
            hwnd: 0x0001_ABCD,
            title: "Test Window".to_string(),
            pid: 1234,
            x: 100,
            y: 200,
            width: 800,
            height: 600,
            class_name: "TestClass".to_string(),
        };

        let json = serde_json::to_string(&info).expect("serialization failed");
        assert!(json.contains("\"hwnd\":109517"));
        assert!(json.contains("\"title\":\"Test Window\""));
        assert!(json.contains("\"pid\":1234"));
        assert!(json.contains("\"x\":100"));
        assert!(json.contains("\"y\":200"));
        assert!(json.contains("\"width\":800"));
        assert!(json.contains("\"height\":600"));
        assert!(json.contains("\"class_name\":\"TestClass\""));

        // Round-trip
        let deserialized: WindowInfo =
            serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.hwnd, info.hwnd);
        assert_eq!(deserialized.title, info.title);
        assert_eq!(deserialized.pid, info.pid);
        assert_eq!(deserialized.width, info.width);
    }
}
