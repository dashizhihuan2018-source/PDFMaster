//! Rust UIKit adapter: export a prewritten PDF, never an empty placeholder.
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSFileCoordinator, NSFileCoordinatorReadingOptions, NSObject,
    NSObjectProtocol, NSString, NSURL,
};
use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController, UIViewController};
use std::{
    cell::RefCell,
    path::Path,
    ptr::NonNull,
    sync::{Arc, Mutex},
};
use tauri::Manager;
use tokio::sync::oneshot;

use crate::export_staging::{PreparedExport, readback_pdf, verify_completion};
type Completion = Arc<Mutex<Option<oneshot::Sender<Result<bool, String>>>>>;

struct ExportIvars {
    sender: Completion,
    expected: Vec<u8>,
}
thread_local! { static DELEGATE: RefCell<Option<Retained<ExportDelegate>>> = const { RefCell::new(None) }; }

fn finish(sender: &Completion, result: Result<bool, String>) {
    if let Ok(mut sender) = sender.lock()
        && let Some(sender) = sender.take()
    {
        let _ = sender.send(result);
    }
}

/// Coordinate only the system URL; no directory probing, filename guessing or scope bypass.
fn coordinated_readback(url: &NSURL) -> Result<Vec<u8>, String> {
    // SAFETY: This URL came from our document picker. Scope is balanced on this worker thread.
    let scoped = unsafe { url.startAccessingSecurityScopedResource() };
    if !scoped {
        return Err("导出提供器未授予回读权限；文档仍保留未保存状态。".into());
    }
    struct Scope<'a>(&'a NSURL, bool);
    impl Drop for Scope<'_> {
        fn drop(&mut self) {
            if self.1 {
                unsafe { self.0.stopAccessingSecurityScopedResource() };
            }
        }
    }
    let _scope = Scope(url, scoped);
    let result = RefCell::new(None);
    let reader = block2::StackBlock::new(|coordinated_url: NonNull<NSURL>| {
        // SAFETY: NSFileCoordinator supplies a live URL for the synchronous accessor's lifetime.
        let coordinated_url = unsafe { coordinated_url.as_ref() };
        let read = coordinated_url
            .path()
            .ok_or_else(|| "导出提供器没有可回读的本地路径".to_string())
            .and_then(|path| readback_pdf(Path::new(&path.to_string())));
        *result.borrow_mut() = Some(read);
    });
    let mut error = None;
    NSFileCoordinator::new().coordinateReadingItemAtURL_options_error_byAccessor(
        url,
        NSFileCoordinatorReadingOptions::empty(),
        Some(&mut error),
        &reader,
    );
    if error.is_some() {
        return Err("导出提供器无法协调回读；文档仍保留未保存状态。".into());
    }
    result
        .into_inner()
        .ok_or_else(|| "导出提供器未允许回读；文档仍保留未保存状态。".to_string())?
}

define_class!(
    // SAFETY: NSObject has no subclass requirements; all UIKit use is main-thread-only.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ExportIvars]
    struct ExportDelegate;

    // SAFETY: NSObjectProtocol adds no requirements.
    unsafe impl NSObjectProtocol for ExportDelegate {}
    // SAFETY: Methods match UIKit's documented delegate signatures, and never panic.
    unsafe impl UIDocumentPickerDelegate for ExportDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _picker: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let Some(url) = urls.firstObject() else {
                finish(&self.ivars().sender, Err("系统未返回导出文件".into()));
                return;
            };
            let sender = Arc::clone(&self.ivars().sender);
            let expected = self.ivars().expected.clone();
            // A provider may need I/O. Do not block UIKit's main thread while coordinating it.
            let spawned = std::thread::Builder::new()
                .name("pdf-export-readback".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        objc2::rc::autoreleasepool(|_| coordinated_readback(&url))
                            .and_then(|bytes| verify_completion(true, Some(&bytes), &expected))
                    }))
                    .unwrap_or_else(|_| Err("导出回读被中断；本地草稿仍保留。".into()));
                    finish(&sender, result);
                });
            if spawned.is_err() {
                finish(
                    &self.ivars().sender,
                    Err("无法启动导出回读；本地草稿仍保留。".into()),
                );
            }
        }
        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            finish(&self.ivars().sender, Ok(false));
        }
    }
);

impl ExportDelegate {
    fn new(mtm: MainThreadMarker, sender: Completion, expected: Vec<u8>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ExportIvars { sender, expected });
        // SAFETY: NSObject init has this exact signature and initializes our allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

pub async fn export(app: &tauri::AppHandle, name: &str, bytes: &[u8]) -> Result<bool, String> {
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let staged = PreparedExport::new(&cache, name, bytes)?;
    let path = staged
        .path
        .to_str()
        .ok_or("导出路径不能转换为系统字符串")?
        .to_string();
    let expected = bytes.to_vec();
    let (sender, receiver) = oneshot::channel();
    let sender: Completion = Arc::new(Mutex::new(Some(sender)));
    let callback = Arc::clone(&sender);
    let window = app.get_webview_window("main").ok_or("导出窗口不存在")?;
    window
        .with_webview(move |webview| {
            let result = (|| {
                let mtm = MainThreadMarker::new().ok_or("导出必须在主线程展示")?;
                let ptr = webview.view_controller().cast::<UIViewController>();
                // SAFETY: Tauri documents view_controller as the live UIKit UIViewController,
                // and with_webview executes while it is retained on the UI thread.
                let host = unsafe { ptr.as_ref() }.ok_or("导出控制器不可用")?;
                if host.presentedViewController().is_some() {
                    return Err("已有系统窗口，请关闭后重试".into());
                }
                let pending = DELEGATE.with(|cell| {
                    cell.borrow()
                        .as_ref()
                        .is_some_and(|d| d.ivars().sender.lock().map_or(true, |s| s.is_some()))
                });
                if pending {
                    return Err("上一次导出尚未结束".into());
                }
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
                let urls = NSArray::from_retained_slice(&[url]);
                let picker = UIDocumentPickerViewController::initForExportingURLs_asCopy(
                    UIDocumentPickerViewController::alloc(mtm),
                    &urls,
                    // Move only the disposable PreparedExport payload, never the source PDF.
                    // Unlike export-copy, move supplies a security-scoped destination URL.
                    false,
                );
                let delegate = ExportDelegate::new(mtm, Arc::clone(&callback), expected);
                picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                picker.setModalInPresentation(true);
                DELEGATE.with(|cell| *cell.borrow_mut() = Some(delegate));
                host.presentViewController_animated_completion(&picker, true, None);
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                finish(&callback, Err(error));
            }
        })
        .map_err(|e| e.to_string())?;
    let result = receiver
        .await
        .map_err(|_| "系统导出被中断；本地草稿仍保留".to_string())?;
    drop(staged);
    result
}
