//! Files macOS asks the application to open: a `.str` double-clicked in Finder, chosen with
//! "Open With", or dropped on the Dock icon. macOS sends them as an "open documents" Apple Event.
//! winit 0.30 neither handles it nor lets the application replace its own application
//! delegate (which drives the event loop), so this crate registers an Apple Event handler of its
//! own. Declaring the Objective-C handler class and registering it need `unsafe`.
//!
//! **This is the only crate in the workspace that may use `unsafe`** (owner decision,
//! 2026-11-09). It is confined to `mod macos`, which only declares the handler class, registers
//! it, and forwards file paths to [`take_paths`]. Everything else here, and on other platforms
//! all of it, is safe code.
//!
//! Usage: call [`install`] on the main thread after the winit event loop is created and before
//! it runs (files that launch the app arrive while it finishes launching); set a waker with
//! [`set_waker`]; call [`take_paths`] once per frame.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

type Waker = Box<dyn Fn() + Send + Sync>;

/// First system language preference. Other platforms currently resolve System to English.
pub fn preferred_language() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::NSLocale::preferredLanguages()
            .firstObject()
            .map(|language| language.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

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
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject,
    };
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::ptr::NonNull;

    /// Four-character codes from the Apple Event Manager (`AppleEvents.h`).
    const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const K_AE_OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
    /// Upper bound on files read from one event.
    const MAX_ITEMS: isize = 1024;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and this class implements no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "DmsOpenDocumentsHandler"]
        struct OpenHandler;

        impl OpenHandler {
            // SAFETY: the signature NSAppleEventManager calls handlers with
            // (`- (void)handleEvent:(NSAppleEventDescriptor *)event
            //    withReplyEvent:(NSAppleEventDescriptor *)reply`).
            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle_open_documents(
                &self,
                event: &NSAppleEventDescriptor,
                _reply: &NSAppleEventDescriptor,
            ) {
                let Some(list) = event.paramDescriptorForKeyword(KEY_DIRECT_OBJECT) else {
                    return;
                };
                let count = list.numberOfItems().clamp(0, MAX_ITEMS);
                // Apple Event lists are 1-based.
                super::push(
                    (1..=count)
                        .filter_map(|index| list.descriptorAtIndex(index))
                        .filter_map(|item| item.fileURLValue())
                        .filter_map(|url| url.path())
                        .map(|path| PathBuf::from(path.to_string())),
                );
            }
        }

        // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject subclass.
        unsafe impl NSObjectProtocol for OpenHandler {}
    );

    impl OpenHandler {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` is NSObject's designated initializer and takes no arguments.
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// The handler and the notification observer live as long as the application.
        static KEEP: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>> =
            const { RefCell::new(Vec::new()) };
        static HANDLER: RefCell<Option<Retained<OpenHandler>>> = const { RefCell::new(None) };
    }

    fn register(handler: &OpenHandler) {
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // SAFETY: `handler` is an object of a class that implements the selector with the
        // signature the Apple Event Manager calls (see `handle_open_documents`), and it is kept
        // alive in `HANDLER` for the life of the application.
        unsafe {
            manager.setEventHandler_andSelector_forEventClass_andEventID(
                handler,
                sel!(handleOpenDocuments:withReplyEvent:),
                K_CORE_EVENT_CLASS,
                K_AE_OPEN_DOCUMENTS,
            );
        }
    }

    pub(super) fn install() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let handler = OpenHandler::new(mtm);
        HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
        // AppKit installs its default "open documents" handler while the application finishes
        // launching, so ours goes in at "will finish launching": after AppKit's, and before the
        // documents that launched the application are delivered.
        let block = RcBlock::new(|_notification: NonNull<NSNotification>| {
            HANDLER.with(|slot| {
                if let Some(handler) = slot.borrow().as_ref() {
                    register(handler);
                }
            });
        });
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: reading an AppKit constant (an immutable NSString); the block is only run on
        // the main thread (the notification is posted there) and owns no thread-bound data;
        // the observer token is kept alive in `KEEP`.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
                None,
                &block,
            )
        };
        KEEP.with(|keep| keep.borrow_mut().push(observer));
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
