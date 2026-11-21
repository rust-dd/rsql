//! Main-thread vetoes for AppKit's termination request, before shutdown begins.
use objc2::{
    MainThreadMarker,
    runtime::{AnyObject, Sel},
};
use objc2_app_kit::NSApplicationTerminateReply;
use std::{cell::RefCell, rc::Rc};

type Predicate = Rc<dyn Fn() -> bool>;

thread_local! {
    static PREDICATES: RefCell<Vec<Predicate>> = const { RefCell::new(Vec::new()) };
}

/// Keeps the predicate registered until its owner drops this handle.
pub struct TerminationGuard(Predicate);

impl TerminationGuard {
    pub fn new(_: MainThreadMarker, allow_termination: impl Fn() -> bool + 'static) -> Self {
        let predicate: Predicate = Rc::new(allow_termination);
        PREDICATES.with_borrow_mut(|predicates| predicates.push(predicate.clone()));
        Self(predicate)
    }
}

impl Drop for TerminationGuard {
    fn drop(&mut self) {
        PREDICATES.with_borrow_mut(|predicates| predicates.retain(|p| !Rc::ptr_eq(p, &self.0)));
    }
}

pub(crate) extern "C-unwind" fn should_terminate(
    _: &AnyObject,
    _: Sel,
    _: *mut AnyObject,
) -> NSApplicationTerminateReply {
    let predicates = PREDICATES.with_borrow(Clone::clone);
    if predicates.iter().all(|allow| allow()) {
        NSApplicationTerminateReply::TerminateNow
    } else {
        NSApplicationTerminateReply::TerminateCancel
    }
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn delegate_allows_termination(mtm: MainThreadMarker) -> bool {
    use objc2::{msg_send, rc::Retained, runtime::AnyClass, sel};
    use objc2_app_kit::NSApplication;
    let class = AnyClass::get(c"GPUIApplicationDelegate").expect("registered GPUI delegate");
    class
        .verify_sel::<(&NSApplication,), NSApplicationTerminateReply>(
            sel!(applicationShouldTerminate:),
        )
        .expect("AppKit termination selector signature");
    let method = class
        .instance_method(sel!(applicationShouldTerminate:))
        .unwrap();
    assert_eq!(
        method.implementation() as *const (),
        should_terminate as *const ()
    );
    let app = NSApplication::sharedApplication(mtm);
    // This isolated delegate receives only the termination request. No windows or quit events.
    let delegate: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let reply: NSApplicationTerminateReply =
        unsafe { msg_send![&*delegate, applicationShouldTerminate: &*app] };
    reply == NSApplicationTerminateReply::TerminateNow
}
