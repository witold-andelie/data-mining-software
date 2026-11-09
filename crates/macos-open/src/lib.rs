//! Files macOS asks the application to open: a `.str` double-clicked in Finder, chosen with
//! "Open With", or dropped on the Dock icon. macOS delivers them to the application delegate
//! (`application:openURLs:`), which winit 0.30 leaves to the application, and declaring an
//! Objective-C class needs `unsafe`.
//!
//! **This is the only crate in the workspace that may use `unsafe`** (owner decision,
//! 2026-11-09). It is confined to `mod macos`, which only declares the delegate class and
//! forwards file paths to [`take_paths`]. Everything else here, and on other platforms all of
//! it, is safe code.
//!
//! Usage: call [`install`] on the main thread after the winit event loop is created and before
//! it runs (files that launch the app arrive while it finishes launching); set a waker with
//! [`set_waker`]; call [`take_paths`] once per frame.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

type Waker = Box<dyn Fn() + Send + Sync>;

/// Upper bound on paths kept between two `take_paths` calls (a mass "Open With" cannot grow
/// memory without limit).
const MAX_PENDING: usize = 1024;

#[derive(Default)]
struct Shared {
    paths: Vec<PathBuf>,
    waker: Option<Waker>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(Shared::default()))
}

/// Installs the handler. Returns whether it is active: `false` off macOS, off the main thread,
/// or when another application delegate is already set.
pub fn install() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::install()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Called after paths arrive (from the main thread), for example to request a repaint.
pub fn set_waker(waker: Option<Waker>) {
    let mut shared = shared().lock().unwrap_or_else(|e| e.into_inner());
    shared.waker = waker;
}

/// The paths received since the last call, oldest first.
pub fn take_paths() -> Vec<PathBuf> {
    let mut shared = shared().lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut shared.paths)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn push(paths: impl IntoIterator<Item = PathBuf>) {
    let mut shared = shared().lock().unwrap_or_else(|e| e.into_inner());
    for path in paths {
        if shared.paths.len() < MAX_PENDING {
            shared.paths.push(path);
        }
    }
    if let Some(waker) = &shared.waker {
        waker();
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSApplicationDelegate};
    use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSURL};
    use std::cell::RefCell;
    use std::path::PathBuf;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and this class implements no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DmsOpenDocumentsDelegate"]
        struct OpenDelegate;

        // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject subclass.
        unsafe impl NSObjectProtocol for OpenDelegate {}

        // SAFETY: the method has the signature AppKit declares for `application:openURLs:`
        // (see `objc2_app_kit::NSApplicationDelegate::application_openURLs`).
        unsafe impl NSApplicationDelegate for OpenDelegate {
            #[unsafe(method(application:openURLs:))]
            fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
                super::push(
                    urls.iter()
                        .filter(|url| url.isFileURL())
                        .filter_map(|url| url.path())
                        .map(|path| PathBuf::from(path.to_string())),
                );
            }
        }
    );

    impl OpenDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` is NSObject's designated initializer and takes no arguments.
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// AppKit holds the delegate weakly; this keeps it alive for the life of the app.
        static DELEGATE: RefCell<Option<Retained<OpenDelegate>>> = const { RefCell::new(None) };
    }

    pub(super) fn install() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        if app.delegate().is_some() {
            return false;
        }
        let delegate = OpenDelegate::new(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.with(|slot| *slot.borrow_mut() = Some(delegate));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn paths_queue_wake_and_drain_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        set_waker(Some(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        push([PathBuf::from("a.str"), PathBuf::from("Výběr/b.str")]);
        assert_eq!(
            take_paths(),
            [PathBuf::from("a.str"), PathBuf::from("Výběr/b.str")]
        );
        assert!(take_paths().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        push((0..MAX_PENDING + 10).map(|i| PathBuf::from(format!("{i}.str"))));
        assert_eq!(take_paths().len(), MAX_PENDING);
        set_waker(None);
    }

    #[test]
    fn install_needs_the_main_thread() {
        // Test threads are not the main thread, so nothing is installed here.
        assert!(!install());
    }
}
