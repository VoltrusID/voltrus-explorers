//! Sign-in through `ASWebAuthenticationSession` (macOS and iOS).
//!
//! The session shows the Voltrus authorize page in the system's in-app browser sheet and hands back the
//! redirect (`voltrus-<slug>://connect?...`) without ever opening the default browser. [`start`] must be
//! called on the main thread (tauri's `with_webview` closure is), where the running session is kept.
//! AuthenticationServices calls the completion handler on a background queue, not the main thread
//! (macOS 26, Safari and default-browser providers alike), so the result is sent back to the main
//! queue before it is matched against the running session.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use block2::{DynBlock, RcBlock};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_authentication_services::{
    ASWebAuthenticationPresentationContextProviding, ASWebAuthenticationSession, ASWebAuthenticationSessionErrorCode,
    ASWebAuthenticationSessionErrorDomain,
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString, NSURL};
use url::Url;

/// How a session ended. `Send`: it crosses from AuthenticationServices' queue to the main thread.
#[derive(Debug)]
pub enum Outcome {
    /// The authorize page redirected to the app's callback scheme.
    Callback(Url),
    /// The user closed the sheet (`ASWebAuthenticationSessionErrorCodeCanceledLogin`).
    Cancelled,
    Failed(String),
}

struct AnchorIvars {
    window: Retained<NSObject>,
}

#[cfg(target_os = "macos")]
define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AnchorIvars]
    struct AnchorProvider;

    unsafe impl NSObjectProtocol for AnchorProvider {}

    unsafe impl ASWebAuthenticationPresentationContextProviding for AnchorProvider {
        #[allow(non_snake_case)]
        #[unsafe(method_id(presentationAnchorForWebAuthenticationSession:))]
        fn presentationAnchorForWebAuthenticationSession(&self, _session: &ASWebAuthenticationSession) -> Retained<NSObject> {
            self.ivars().window.clone()
        }
    }
);

// objc2-authentication-services only declares the protocol's method for macOS; on iOS the selector is
// implemented directly and the protocol conformance is declared empty.
#[cfg(target_os = "ios")]
define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AnchorIvars]
    struct AnchorProvider;

    impl AnchorProvider {
        #[unsafe(method_id(presentationAnchorForWebAuthenticationSession:))]
        fn __anchor(&self, _session: &ASWebAuthenticationSession) -> Retained<NSObject> {
            self.ivars().window.clone()
        }
    }

    unsafe impl NSObjectProtocol for AnchorProvider {}

    unsafe impl ASWebAuthenticationPresentationContextProviding for AnchorProvider {}
);

impl AnchorProvider {
    fn new(mtm: MainThreadMarker, window: Retained<NSObject>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(AnchorIvars { window });
        unsafe { msg_send![super(this), init] }
    }
}

/// The running session. `presentationContextProvider` is a weak property, so the provider is kept here.
struct Active {
    id: u64,
    session: Retained<ASWebAuthenticationSession>,
    _provider: Retained<AnchorProvider>,
    on_done: Box<dyn Fn(Outcome)>,
}

// Main-thread state: `start` and `finish` only ever run there.
thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

/// Main thread: reports the end of session `id`, unless a newer session replaced it (then it is dropped).
fn finish(id: u64, outcome: Outcome) {
    let mine = ACTIVE.with(|a| a.borrow_mut().take_if(|s| s.id == id));
    if let Some(active) = mine {
        (active.on_done)(outcome);
    }
}

fn classify(url: *mut NSURL, error: *mut NSError) -> Outcome {
    // SAFETY: AuthenticationServices passes either nil or a valid object for each argument.
    if let Some(url) = unsafe { url.as_ref() } {
        return match url.absoluteString().map(|s| s.to_string()).map(|s| Url::parse(&s)) {
            Some(Ok(parsed)) => Outcome::Callback(parsed),
            _ => Outcome::Failed("The sign-in page returned an unreadable address.".into()),
        };
    }
    // SAFETY: as above.
    let Some(error) = (unsafe { error.as_ref() }) else {
        return Outcome::Failed("Sign-in ended without a result.".into());
    };
    let domain_matches = error.domain().to_string() == unsafe { ASWebAuthenticationSessionErrorDomain }.to_string();
    if domain_matches && error.code() == ASWebAuthenticationSessionErrorCode::CanceledLogin.0 {
        return Outcome::Cancelled;
    }
    Outcome::Failed(error.localizedDescription().to_string())
}

/// Starts a session on the main thread. `window` is the `NSWindow` / `UIWindow` the sheet is anchored to
/// (the app's main window); `on_done` runs once, on the main thread, when the session ends. A session
/// that is still open is cancelled and replaced; its completion is dropped.
pub fn start(window: *mut c_void, url: &Url, callback_scheme: &str, on_done: impl Fn(Outcome) + 'static) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("Sign-in must start on the main thread.")?;
    // SAFETY: a non-null pointer from tauri's platform webview is a live NSView/UIView (retained here).
    let view = unsafe { (window as *mut AnyObject).as_ref() }.ok_or("The main window is not available.")?;
    let window: Option<Retained<NSObject>> = unsafe { msg_send![view, window] };
    let window = window.ok_or("The main window is not on screen.")?;

    let ns_url = NSURL::URLWithString(&NSString::from_str(url.as_str())).ok_or("The sign-in address is invalid.")?;
    let scheme = NSString::from_str(callback_scheme);

    if let Some(old) = ACTIVE.with(|a| a.borrow_mut().take()) {
        unsafe { old.session.cancel() };
    }
    let id = NEXT_ID.with(|n| n.replace(n.get() + 1));

    let completion = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
        // Runs on an AuthenticationServices queue, where `ACTIVE` (thread-local) is always empty.
        let outcome = classify(url, error);
        DispatchQueue::main().exec_async(move || finish(id, outcome));
    });

    let provider = AnchorProvider::new(mtm, window);
    // SAFETY: `completion` is a valid block with the handler's signature; the session copies it.
    #[allow(deprecated)]
    let session = unsafe {
        ASWebAuthenticationSession::initWithURL_callbackURLScheme_completionHandler(
            ASWebAuthenticationSession::alloc(),
            &ns_url,
            Some(&scheme),
            &*completion as *const DynBlock<dyn Fn(*mut NSURL, *mut NSError)> as *mut _,
        )
    };
    unsafe {
        session.setPresentationContextProvider(Some(ProtocolObject::from_ref(&*provider)));
        // Share the user's Safari login with the sheet.
        session.setPrefersEphemeralWebBrowserSession(false);
    }
    // Registered before `start` so a completion that fires straight away still finds its session.
    ACTIVE.with(|a| *a.borrow_mut() = Some(Active { id, session: session.clone(), _provider: provider, on_done: Box::new(on_done) }));
    if !unsafe { session.start() } {
        ACTIVE.with(|a| a.borrow_mut().take());
        return Err("Could not open the sign-in window.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ptr::null_mut;

    use super::*;

    fn ptr<T: objc2::Message>(obj: &Retained<T>) -> *mut T {
        Retained::as_ptr(obj) as *mut T
    }

    #[test]
    fn outcome_crosses_to_the_main_queue() {
        // The completion handler runs on an AuthenticationServices queue; `exec_async` needs `Send`.
        fn send<T: Send + 'static>() {}
        send::<Outcome>();
    }

    #[test]
    fn completion_classifies_off_the_main_thread() {
        // What AuthenticationServices does: call the handler from a background thread.
        let outcome = std::thread::spawn(|| {
            let url = NSURL::URLWithString(&NSString::from_str("voltrus-bacnet-explorer://connect?code=c1&state=s1")).unwrap();
            classify(ptr(&url), null_mut())
        })
        .join()
        .unwrap();
        match outcome {
            Outcome::Callback(url) => assert_eq!(url.as_str(), "voltrus-bacnet-explorer://connect?code=c1&state=s1"),
            other => panic!("expected the callback, got {other:?}"),
        }
    }

    #[test]
    fn closing_the_sheet_is_a_cancel_and_other_errors_fail() {
        let domain = unsafe { ASWebAuthenticationSessionErrorDomain };
        let cancelled = NSError::new(ASWebAuthenticationSessionErrorCode::CanceledLogin.0, domain);
        assert!(matches!(classify(null_mut(), ptr(&cancelled)), Outcome::Cancelled));
        let other = NSError::new(ASWebAuthenticationSessionErrorCode::PresentationContextInvalid.0, domain);
        assert!(matches!(classify(null_mut(), ptr(&other)), Outcome::Failed(_)));
        assert!(matches!(classify(null_mut(), null_mut()), Outcome::Failed(_)));
    }
}
