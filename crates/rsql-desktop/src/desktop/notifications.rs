use gpui_kit::{App, SystemNotification};
mod count;
pub(crate) use count::localized as localized_row_count;
mod foreground;
pub use foreground::is_background;
use rsql_desktop::execution::{Mode, Outcome, Status};

#[cfg(target_os = "macos")]
mod macos;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Permission {
    Granted,
    Unavailable,
}

pub fn eligible(outcome: &Outcome) -> bool {
    match &outcome.status {
        Status::Rows(rows) => {
            outcome.mode == Mode::Query && rows.elapsed_ms.is_finite() && rows.elapsed_ms >= 5_000.
        }
        Status::Error(_) => outcome.elapsed.as_millis() >= 5_000,
        _ => false,
    }
}

pub fn notification_for(
    outcome: &Outcome,
    focused: bool,
    permission: Permission,
) -> Option<SystemNotification> {
    if focused || permission != Permission::Granted || !eligible(outcome) {
        return None;
    }
    let (title, detail) = match &outcome.status {
        Status::Rows(rows) => {
            let count = localized_row_count(rows.total_rows);
            let seconds = crate::workspace::numbers::fixed(f64::from(rows.elapsed_ms) / 1_000., 1);
            ("Query Complete", format!("{count} rows in {seconds}s"))
        }
        Status::Error(_) => {
            let seconds = crate::workspace::numbers::fixed(outcome.elapsed.as_secs_f64(), 1);
            ("Query Failed", format!("Query failed after {seconds}s"))
        }
        _ => return None,
    };
    let mut preview = String::new();
    let mut units = 0;
    for c in outcome.sql.chars() {
        units += c.len_utf16();
        if units > 60 {
            break;
        }
        preview.push(if c == '\n' { ' ' } else { c });
    }
    Some(SystemNotification {
        tag: uuid::Uuid::new_v4().to_string().into(),
        title: title.into(),
        body: format!("{preview}\n{detail}").into(),
        actions: Vec::new(),
    })
}

pub fn permission(cx: &App) -> impl Future<Output = Permission> + use<> {
    let _ = cx;
    async move { native_permission().await }
}

#[cfg(target_os = "macos")]
async fn native_permission() -> Permission {
    macos::permission().await
}

#[cfg(windows)]
async fn native_permission() -> Permission {
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    use windows::Win32::{
        Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER},
        Storage::Packaging::Appx::GetCurrentPackageFullName,
    };
    let mut length = 0;
    // A null output buffer queries only the identity length of this process.
    let identity = unsafe { GetCurrentPackageFullName(&mut length, None) };
    let notifier = match identity {
        ERROR_INSUFFICIENT_BUFFER => ToastNotificationManager::CreateToastNotifier(),
        APPMODEL_ERROR_NO_PACKAGE => {
            ToastNotificationManager::CreateToastNotifierWithId(&crate::profile::ID.into())
        }
        _ => return Permission::Unavailable,
    };
    let setting = notifier.and_then(|notifier| notifier.Setting());
    if setting.is_ok_and(|status| status == NotificationSetting::Enabled) {
        Permission::Granted
    } else {
        Permission::Unavailable
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
async fn native_permission() -> Permission {
    Permission::Unavailable
}

pub fn deliver(notice: SystemNotification, cx: &App) {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        macos::deliver(&notice);
    }
    #[cfg(not(target_os = "macos"))]
    cx.show_system_notification(notice);
}
