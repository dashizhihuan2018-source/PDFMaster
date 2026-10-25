use crate::Store;
use objc2::runtime::{AnyClass, AnyObject, MethodImplementation, Sel};
use objc2::{MainThreadMarker, ffi, sel};
use objc2_app_kit::NSApplication;
use std::{
    error::Error,
    ffi::c_char,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{OnceLock, TryLockError},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

const NS_TERMINATE_CANCEL: usize = 0;
const NS_TERMINATE_NOW: usize = 1;
const METHOD_ENCODING: &[u8; 5] = b"Q@:@\0";
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuardState {
    Clean,
    Dirty,
    Unavailable,
}

fn termination_reply(state: GuardState) -> usize {
    match state {
        GuardState::Clean => NS_TERMINATE_NOW,
        GuardState::Dirty | GuardState::Unavailable => NS_TERMINATE_CANCEL,
    }
}

fn guard_state() -> GuardState {
    let Some(app) = APP_HANDLE.get() else {
        return GuardState::Unavailable;
    };
    let Some(store) = app.try_state::<Store>() else {
        return GuardState::Unavailable;
    };
    match store.try_lock() {
        Ok(workspace) if workspace.dirty() => GuardState::Dirty,
        Ok(_) => GuardState::Clean,
        Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => GuardState::Unavailable,
    }
}

fn show_cancellation(state: GuardState) {
    let Some(app) = APP_HANDLE.get() else {
        return;
    };
    let message = match state {
        GuardState::Dirty => {
            "文档尚未保存。请先导出副本，再退出 PDF Master。未保存修改仍保留在当前进程。"
        }
        GuardState::Unavailable => {
            "暂时无法确认文档是否已安全保存。为避免丢失修改，本次退出已取消，请稍后重试。"
        }
        GuardState::Clean => return,
    };
    app.dialog()
        .message(message)
        .title("保留未保存修改")
        .show(|_| {});
}

unsafe extern "C-unwind" fn application_should_terminate(
    _delegate: *mut AnyObject,
    _selector: Sel,
    _sender: *mut AnyObject,
) -> usize {
    // Cocoa invokes applicationShouldTerminate: on the application main
    // thread. The body catches Rust panics so none can cross Objective-C; an
    // unexpected failure always cancels termination.
    catch_unwind(AssertUnwindSafe(|| {
        let state = guard_state();
        show_cancellation(state);
        termination_reply(state)
    }))
    .unwrap_or(NS_TERMINATE_CANCEL)
}

fn add_guard_method(class: &AnyClass) -> io::Result<()> {
    let selector = sel!(applicationShouldTerminate:);
    if class.instance_method(selector).is_some() {
        return Err(io::Error::other(
            "NSApplication delegate already implements applicationShouldTerminate:",
        ));
    }

    let implementation: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize =
        application_should_terminate;
    // SAFETY: `class` is a live Objective-C class with process lifetime. The
    // selector has one object argument and NSUInteger return, exactly matching
    // Q@:@ on supported 64-bit macOS. The function pointer is process-static
    // and the runtime copies the NUL-terminated encoding. Existing inherited
    // or direct implementations are rejected above, never replaced/swizzled.
    let added = unsafe {
        ffi::class_addMethod(
            class as *const _ as *mut _,
            selector,
            implementation.__imp(),
            METHOD_ENCODING.as_ptr().cast::<c_char>(),
        )
    };
    if added.as_bool() {
        Ok(())
    } else {
        Err(io::Error::other(
            "failed to add applicationShouldTerminate:",
        ))
    }
}

pub fn install(app: &AppHandle) -> Result<(), Box<dyn Error>> {
    if std::mem::size_of::<usize>() != 8 {
        return Err(io::Error::other("unsupported macOS NSUInteger width").into());
    }
    APP_HANDLE
        .set(app.clone())
        .map_err(|_| io::Error::other("macOS termination guard already initialized"))?;

    let mtm = MainThreadMarker::new()
        .ok_or_else(|| io::Error::other("termination guard must install on the main thread"))?;
    let application = NSApplication::sharedApplication(mtm);
    let delegate = application
        .delegate()
        .ok_or_else(|| io::Error::other("NSApplication delegate is unavailable"))?;
    let delegate_object: &AnyObject = AsRef::<AnyObject>::as_ref(&*delegate);
    let class = delegate_object.class();
    // AppKit owns the delegate and its class for the process lifetime. Setup
    // runs on the main thread before event dispatch, so the one-time runtime
    // method addition cannot race ordinary delegate messaging.
    add_guard_method(class)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{
        ClassType,
        runtime::{ClassBuilder, NSObject},
    };

    #[test]
    fn native_termination_contract_fails_closed_except_when_clean() {
        assert_eq!(termination_reply(GuardState::Clean), NS_TERMINATE_NOW);
        assert_eq!(termination_reply(GuardState::Dirty), NS_TERMINATE_CANCEL);
        assert_eq!(
            termination_reply(GuardState::Unavailable),
            NS_TERMINATE_CANCEL
        );
        assert_eq!(METHOD_ENCODING, b"Q@:@\0");
    }

    #[test]
    fn native_runtime_adds_once_and_refuses_an_existing_selector() {
        let builder =
            ClassBuilder::new(c"PDFMasterTerminationGuardContract", NSObject::class()).unwrap();
        let class = builder.register();
        add_guard_method(class).unwrap();
        assert!(
            class
                .instance_method(sel!(applicationShouldTerminate:))
                .is_some()
        );
        assert!(add_guard_method(class).is_err());
    }
}
