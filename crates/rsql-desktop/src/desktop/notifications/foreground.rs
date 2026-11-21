use gpui_kit::App;

#[cfg(target_os = "macos")]
pub fn is_background(_: &App) -> bool {
    use objc2_app_kit::NSApplication;
    objc2_foundation::MainThreadMarker::new()
        .is_some_and(|mtm| !NSApplication::sharedApplication(mtm).isActive())
}

#[cfg(windows)]
pub fn is_background(_: &App) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // Both calls inspect a borrowed system window. A missing window or owner is indeterminate.
    let window = unsafe { GetForegroundWindow() };
    let mut process = 0;
    !window.is_invalid()
        && unsafe { GetWindowThreadProcessId(window, Some(&mut process)) } != 0
        && process != 0
        && process != std::process::id()
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_background(_: &App) -> bool {
    false
}
