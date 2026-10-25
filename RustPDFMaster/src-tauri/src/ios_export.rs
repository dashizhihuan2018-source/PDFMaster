//! Rust UIKit adapter: export a prewritten PDF, never an empty placeholder.
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_ui_kit::{UIDocumentPickerDelegate, UIDocumentPickerViewController, UIViewController};
use std::{
    cell::RefCell,
    fs::File,
    io::Read,
    sync::{Arc, Mutex},
};
use tauri::Manager;
use tokio::sync::oneshot;

use crate::export_staging::{PreparedExport, verify_completion};
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
            let result = (|| {
                let url = urls.firstObject().ok_or("系统未返回导出文件")?;
                // SAFETY: URL was supplied by this picker; balance scope on this same main thread.
                let scoped = unsafe { url.startAccessingSecurityScopedResource() };
                let readback = (|| {
                    let path = url
                        .path()
                        .ok_or("导出提供器没有可回读的本地路径")?
                        .to_string();
                    let mut bytes = Vec::new();
                    File::open(path)
                        .map_err(|_| "无法回读导出提供器中的文件")?
                        .take(32 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| "回读导出文件失败")?;
                    Ok::<_, String>(bytes)
                })();
                if scoped {
                    unsafe { url.stopAccessingSecurityScopedResource() };
                }
                let bytes = readback?;
                verify_completion(true, Some(&bytes), &self.ivars().expected)
            })();
            finish(&self.ivars().sender, result);
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
                    true,
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
