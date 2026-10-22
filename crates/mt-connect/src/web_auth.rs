//! Sign-in through `ASWebAuthenticationSession` (macOS and iOS).
//!
//! The session shows the Voltrus authorize page in the system's in-app browser sheet and hands back the
//! redirect (`voltrus-<slug>://connect?...`) without ever opening the default browser. Everything here
//! runs on the main thread: [`start`] must be called from there (tauri's `with_webview` closure is), and
//! AuthenticationServices calls the completion handler there too.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_authentication_services::{
    ASWebAuthenticationPresentationContextProviding, ASWebAuthenticationSession, ASWebAuthenticationSessionErrorCode,
    ASWebAuthenticationSessionErrorDomain,
};
use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSString, NSURL};
use url::Url;

/// How a session ended.
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
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
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
        let outcome = classify(url, error);
        // A session that was replaced by a newer one reports nothing.
        let mine = ACTIVE.with(|a| {
            let mut a = a.borrow_mut();
            let mine = a.as_ref().is_some_and(|s| s.id == id);
            mine.then(|| a.take()).flatten()
        });
        if mine.is_some() {
            on_done(outcome);
        }
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
    ACTIVE.with(|a| *a.borrow_mut() = Some(Active { id, session: session.clone(), _provider: provider }));
    if !unsafe { session.start() } {
        ACTIVE.with(|a| a.borrow_mut().take());
        return Err("Could not open the sign-in window.".into());
    }
    Ok(())
}
