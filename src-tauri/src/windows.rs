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
// macOS implementation via CoreGraphics + NSRunningApplication
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    use super::WindowInfo;

    use core::ffi::c_void;
    use core_foundation::array::CFArrayGetValueAtIndex;
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryGetValue, CFDictionaryRef};
    use core_foundation::number::{CFNumber, CFNumberRef};
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly,
    };

    // Extract a f64 from a CFDictionary using a string key (for CGWindowBounds sub-dict).
    fn dict_f64(dict: &CFDictionary, key: &str) -> f64 {
        let cf_key = CFString::new(key);
        let raw: *const c_void = unsafe {
            CFDictionaryGetValue(
                dict.as_concrete_TypeRef(),
                cf_key.as_CFTypeRef() as *const c_void,
            )
        };
        if raw.is_null() {
            return 0.0;
        }
        let num: CFNumber =
            unsafe { TCFType::wrap_under_get_rule(raw as CFNumberRef) };
        num.to_f64().unwrap_or(0.0)
    }

    // Extract an i64 from a CFDictionary using a raw CFStringRef key.
    fn dict_i64(dict: &CFDictionary, key_ref: CFStringRef) -> i64 {
        let raw: *const c_void = unsafe {
            CFDictionaryGetValue(
                dict.as_concrete_TypeRef(),
                key_ref as *const c_void,
            )
        };
        if raw.is_null() {
            return 0;
        }
        let num: CFNumber = unsafe { TCFType::wrap_under_get_rule(raw as CFNumberRef) };
        num.to_i64().unwrap_or(0)
    }

    // Extract a String from a CFDictionary using a raw CFStringRef key.
    fn dict_string(dict: &CFDictionary, key_ref: CFStringRef) -> Option<String> {
        let raw: *const c_void = unsafe {
            CFDictionaryGetValue(
                dict.as_concrete_TypeRef(),
                key_ref as *const c_void,
            )
        };
        if raw.is_null() {
            return None;
        }
        let cf_str: CFString =
            unsafe { TCFType::wrap_under_get_rule(raw as CFStringRef) };
        Some(cf_str.to_string())
    }

    /// Map a key character/name to a macOS virtual key code.
    fn char_to_keycode(s: &str) -> Option<CGKeyCode> {
        let code: CGKeyCode = match s {
            "a" => 0,
            "s" => 1,
            "d" => 2,
            "f" => 3,
            "h" => 4,
            "g" => 5,
            "z" => 6,
            "x" => 7,
            "c" => 8,
            "v" => 9,
            "b" => 11,
            "q" => 12,
            "w" => 13,
            "e" => 14,
            "r" => 15,
            "y" => 16,
            "t" => 17,
            "1" => 18,
            "2" => 19,
            "3" => 20,
            "4" => 21,
            "6" => 22,
            "5" => 23,
            "9" => 25,
            "7" => 26,
            "8" => 28,
            "0" => 29,
            "o" => 31,
            "u" => 32,
            "i" => 34,
            "p" => 35,
            "l" => 37,
            "j" => 38,
            "k" => 40,
            "n" => 45,
            "m" => 46,
            "tab" => 48,
            "return" | "enter" => 0x24,
            "space" => 0x31,
            "delete" | "backspace" => 0x33,
            "escape" | "esc" => 0x35,
            "left" => 0x7B,
            "right" => 0x7C,
            "down" => 0x7D,
            "up" => 0x7E,
            _ => return None,
        };
        Some(code)
    }

    pub fn send_key_combo(combo: &str) {
        let lower = combo.to_lowercase();
        let parts: Vec<&str> = lower.split('+').collect();

        let mut flags = CGEventFlags::CGEventFlagNull;
        let mut keycode: Option<CGKeyCode> = None;

        for part in &parts {
            match part.trim() {
                "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
                "cmd" | "command" | "meta" => flags |= CGEventFlags::CGEventFlagCommand,
                "alt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
                "shift" => flags |= CGEventFlags::CGEventFlagShift,
                s => {
                    if keycode.is_none() {
                        keycode = char_to_keycode(s);
                    }
                }
            }
        }

        let vk = match keycode {
            Some(k) => k,
            None => {
                eprintln!("[CE] send_key_combo: '{}' → no key parsed!", combo);
                return;
            }
        };

        eprintln!(
            "[CE] send_key_combo: '{}' → flags={:?}, keycode={}",
            combo, flags, vk
        );

        let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[CE] send_key_combo: failed to create event source");
                return;
            }
        };

        // Key down
        if let Ok(down) = CGEvent::new_keyboard_event(source.clone(), vk, true) {
            if flags != CGEventFlags::CGEventFlagNull {
                down.set_flags(flags);
            }
            down.post(CGEventTapLocation::HID);
        }

        // Key up
        if let Ok(up) = CGEvent::new_keyboard_event(source, vk, false) {
            if flags != CGEventFlags::CGEventFlagNull {
                up.set_flags(flags);
            }
            up.post(CGEventTapLocation::HID);
        }
    }

    pub fn focus_window(hwnd: u64) -> bool {
        // Find the PID for this window ID by re-querying the window list.
        let windows = list_windows_raw();
        let pid = windows.iter().find(|w| w.hwnd == hwnd).map(|w| w.pid);

        let pid = match pid {
            Some(p) => p,
            None => {
                eprintln!("[CE] focus_window: window {} not found in list", hwnd);
                return false;
            }
        };

        eprintln!("[CE] focus_window: window={} pid={}", hwnd, pid);

        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32);
        match app {
            Some(app) => {
                #[allow(deprecated)]
                let opts = NSApplicationActivationOptions::ActivateIgnoringOtherApps;
                let result = app.activateWithOptions(opts);
                eprintln!("[CE] focus_window: activateWithOptions={}", result);
                result
            }
            None => {
                eprintln!("[CE] focus_window: no NSRunningApplication for pid={}", pid);
                false
            }
        }
    }

    fn list_windows_raw() -> Vec<WindowInfo> {
        use core_graphics::window::{
            kCGWindowBounds, kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
            kCGWindowOwnerPID,
        };

        let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let array = match copy_window_info(options, kCGNullWindowID) {
            Some(a) => a,
            None => return Vec::new(),
        };

        let count = array.len() as usize;
        let mut results = Vec::new();

        for i in 0..count {
            let raw_ptr: *const c_void = unsafe {
                CFArrayGetValueAtIndex(array.as_concrete_TypeRef(), i as isize)
            };
            if raw_ptr.is_null() {
                continue;
            }

            // Each element is a CFDictionary<CFString, CFType>
            let dict: CFDictionary =
                unsafe { TCFType::wrap_under_get_rule(raw_ptr as CFDictionaryRef) };

            // Filter: layer 0 = normal windows
            let layer = dict_i64(&dict, unsafe { kCGWindowLayer });
            if layer != 0 {
                continue;
            }

            // Title (kCGWindowName) — skip if empty or absent
            let title = match dict_string(&dict, unsafe { kCGWindowName }) {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            // Window ID → hwnd
            let window_id = dict_i64(&dict, unsafe { kCGWindowNumber }) as u64;

            // Owning PID
            let pid = dict_i64(&dict, unsafe { kCGWindowOwnerPID }) as u32;

            // App name used as class_name (no window class concept on macOS)
            let class_name =
                dict_string(&dict, unsafe { kCGWindowOwnerName }).unwrap_or_default();

            // Bounds — kCGWindowBounds is a nested CFDictionary with X/Y/Width/Height
            let bounds_raw: *const c_void = unsafe {
                CFDictionaryGetValue(
                    dict.as_concrete_TypeRef(),
                    kCGWindowBounds as *const c_void,
                )
            };

            let (x, y, width, height) = if !bounds_raw.is_null() {
                let bounds: CFDictionary =
                    unsafe { TCFType::wrap_under_get_rule(bounds_raw as CFDictionaryRef) };
                let bx = dict_f64(&bounds, "X") as i32;
                let by = dict_f64(&bounds, "Y") as i32;
                let bw = dict_f64(&bounds, "Width").max(0.0) as u32;
                let bh = dict_f64(&bounds, "Height").max(0.0) as u32;
                (bx, by, bw, bh)
            } else {
                (0, 0, 0, 0)
            };

            // Skip zero-size windows
            if width == 0 && height == 0 {
                continue;
            }

            results.push(WindowInfo {
                hwnd: window_id,
                title,
                pid,
                x,
                y,
                width,
                height,
                class_name,
            });
        }

        results
    }

    pub fn list_windows() -> Vec<WindowInfo> {
        list_windows_raw()
    }
}

// ---------------------------------------------------------------------------
// Stub implementation (non-Windows, non-macOS)
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
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
