use super::Permission;
use block2::RcBlock;
use objc2_foundation::{NSBundle, NSString};
use objc2_user_notifications::{
    UNAuthorizationStatus, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSettings, UNUserNotificationCenter,
};
use std::{ptr::NonNull, sync::Mutex};

fn matching_bundle() -> bool {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .is_some_and(|id| id.to_string() == crate::profile::ID)
}

pub(super) async fn permission() -> Permission {
    if !matching_bundle() {
        return Permission::Unavailable;
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    {
        let send = Mutex::new(Some(send));
        let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // Foundation keeps the settings object alive for the duration of this callback.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            if let Some(tx) = send.lock().unwrap().take() {
                let _ = tx.send(if status == UNAuthorizationStatus::Authorized {
                    Permission::Granted
                } else {
                    Permission::Unavailable
                });
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .getNotificationSettingsWithCompletionHandler(&completion);
    }
    receive.await.unwrap_or(Permission::Unavailable)
}

pub(super) fn deliver(notice: &gpui_kit::SystemNotification) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notice.title));
    content.setBody(&NSString::from_str(&notice.body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&notice.tag),
        &content,
        None,
    );
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, None);
}
