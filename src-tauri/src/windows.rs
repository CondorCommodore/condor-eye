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

    const SW_RESTORE: i32 = 9;
    const VK_MENU: u8 = 0x12; // Alt key
    const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
    const KEYEVENTF_KEYUP: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(
            lpEnumFunc: unsafe extern "system" fn(HWND, LPARAM) -> BOOL,
            lParam: LPARAM,
        ) -> BOOL;
        fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
        fn GetWindowTextLengthW(hWnd: HWND) -> i32;
        fn IsWindowVisible(hWnd: HWND) -> BOOL;
        fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
        fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;
        fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
        fn SetForegroundWindow(hWnd: HWND) -> BOOL;
        fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
        fn IsIconic(hWnd: HWND) -> BOOL;
        fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
        fn SetWindowPos(
            hWnd: HWND,
            hWndInsertAfter: HWND,
            X: i32,
            Y: i32,
            cx: i32,
            cy: i32,
            uFlags: u32,
        ) -> BOOL;
        fn GetForegroundWindow() -> HWND;
        fn GetCurrentThreadId() -> DWORD;
        fn AttachThreadInput(idAttach: DWORD, idAttachTo: DWORD, fAttach: BOOL) -> BOOL;
        fn BringWindowToTop(hWnd: HWND) -> BOOL;
    }

    const HWND_TOPMOST: HWND = -1;
    const HWND_NOTOPMOST: HWND = -2;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOSIZE: u32 = 0x0001;

    const VK_CONTROL: u8 = 0x11;
    const VK_TAB: u8 = 0x09;

    /// Send a key combo like "ctrl+3" or "ctrl+tab" to the focused window.
    pub fn send_key_combo(combo: &str) {
        let lower = combo.to_lowercase();
        let parts: Vec<&str> = lower.split('+').collect();

        let mut modifiers: Vec<u8> = Vec::new();
        let mut key: Option<u8> = None;

        for part in &parts {
            match part.trim() {
                "ctrl" => modifiers.push(VK_CONTROL),
                "alt" => modifiers.push(VK_MENU),
                "tab" => key = Some(VK_TAB),
                s if s.len() == 1 => {
                    if let Some(c) = s.chars().next() {
                        if c.is_ascii_digit() {
                            // '1' = 0x31, '9' = 0x39
                            key = Some(c as u8);
                        } else if c.is_ascii_alphabetic() {
                            // 'a' = 0x41
                            key = Some(c.to_ascii_uppercase() as u8);
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(vk) = key {
            eprintln!(
                "[CE] send_key_combo: '{}' → modifiers={:02X?}, key=0x{:02X}, fg=0x{:X}",
                combo,
                modifiers,
                vk,
                unsafe { GetForegroundWindow() } as u64
            );
            unsafe {
                // Press modifiers
                for &m in &modifiers {
                    keybd_event(m, 0, 0, 0);
                }
                // Press and release key
                keybd_event(vk, 0, 0, 0);
                keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
                // Release modifiers
                for &m in modifiers.iter().rev() {
                    keybd_event(m, 0, KEYEVENTF_KEYUP, 0);
                }
            }
        } else {
            eprintln!("[CE] send_key_combo: '{}' → no key parsed!", combo);
        }
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

    /// Bring a window to the foreground by its HWND, transferring both
    /// visual Z-order AND keyboard input focus.
    ///
    /// Uses the AttachThreadInput pattern: attach our thread to the
    /// foreground window's thread, which lets SetForegroundWindow succeed
    /// even from a background process.
    pub fn focus_window(hwnd: u64) -> bool {
        let h = hwnd as HWND;
        unsafe {
            // Restore if minimized
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }

            // Get the current foreground window's thread and our thread
            let fg_hwnd = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg_hwnd, std::ptr::null_mut());
            let our_thread = GetCurrentThreadId();

            eprintln!(
                "[CE] focus_window: target=0x{:X}, fg=0x{:X}, fg_thread={}, our_thread={}",
                hwnd, fg_hwnd as u64, fg_thread, our_thread
            );

            // Attach our input to the foreground window's thread
            // This allows SetForegroundWindow to succeed from a background process
            let attached = if fg_thread != our_thread {
                AttachThreadInput(our_thread, fg_thread, 1) != 0
            } else {
                false
            };

            // Force to top of Z-order
            let flags = SWP_NOMOVE | SWP_NOSIZE;
            SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, flags);
            SetWindowPos(h, HWND_NOTOPMOST, 0, 0, 0, 0, flags);

            // Bring to top and set foreground (keyboard focus)
            BringWindowToTop(h);
            let fg_result = SetForegroundWindow(h);

            // Detach thread input
            if attached {
                AttachThreadInput(our_thread, fg_thread, 0);
            }

            let new_fg = GetForegroundWindow();
            eprintln!(
                "[CE] focus_window: SetForegroundWindow={}, new_fg=0x{:X}, match={}",
                fg_result,
                new_fg as u64,
                new_fg == h
            );

            fg_result != 0
        }
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

    pub fn list_windows() -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn focus_window(_hwnd: u64) -> bool {
        false
    }

    pub fn send_key_combo(_combo: &str) {}
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

/// Bring a window to the foreground by HWND.
///
/// Restores minimized windows before focusing. Returns true on success.
pub fn focus_window(hwnd: u64) -> bool {
    platform::focus_window(hwnd)
}

/// Send a keyboard shortcut to the currently focused window.
///
/// Combo format: "ctrl+1", "ctrl+tab", "ctrl+w", "alt+f4", etc.
/// Works with Firefox, Chrome, and Edge tab shortcuts (Ctrl+1-9).
pub fn send_key_combo(combo: &str) {
    platform::send_key_combo(combo);
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
        let deserialized: WindowInfo = serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(deserialized.hwnd, info.hwnd);
        assert_eq!(deserialized.title, info.title);
        assert_eq!(deserialized.pid, info.pid);
        assert_eq!(deserialized.width, info.width);
    }
}
