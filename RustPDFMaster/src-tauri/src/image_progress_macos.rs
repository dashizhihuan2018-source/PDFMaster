use crate::image_import::{CancelNotice, ProgressNotice};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSButton, NSLayoutAttribute, NSLayoutConstraint,
    NSLayoutConstraintOrientation, NSProgressIndicator, NSStackView, NSStackViewDistribution,
    NSTextField, NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSProgress, NSRect, NSSize, NSString,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AttemptToken(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Attempt {
    id: u32,
    token: AttemptToken,
}

static NEXT_ATTEMPT_TOKEN: AtomicU64 = AtomicU64::new(0);

fn allocate_attempt(id: u32) -> Option<Attempt> {
    let previous = NEXT_ATTEMPT_TOKEN
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()?;
    Some(Attempt {
        id,
        token: AttemptToken(previous.checked_add(1)?),
    })
}

#[derive(Default)]
struct Generation {
    latest_id: Option<u32>,
    latest_token: Option<AttemptToken>,
    cancelled_through: Option<u32>,
    active: Option<Attempt>,
}

impl Generation {
    fn begin_show(&mut self, attempt: Attempt) -> bool {
        if self
            .cancelled_through
            .is_some_and(|cancelled| attempt.id <= cancelled)
            || self.latest_id.is_some_and(|latest| attempt.id < latest)
            || self
                .latest_token
                .is_some_and(|latest| attempt.token <= latest)
        {
            return false;
        }
        self.latest_id = Some(self.latest_id.map_or(attempt.id, |id| id.max(attempt.id)));
        self.latest_token = Some(attempt.token);
        self.active = Some(attempt);
        true
    }

    fn allows(&self, attempt: Attempt) -> bool {
        self.active == Some(attempt)
    }

    fn finish(&mut self, attempt: Attempt) -> bool {
        let should_close = self.active == Some(attempt);
        if should_close {
            self.active = None;
        }
        if self
            .latest_token
            .is_none_or(|latest| attempt.token > latest)
        {
            self.latest_id = Some(self.latest_id.map_or(attempt.id, |id| id.max(attempt.id)));
            self.latest_token = Some(attempt.token);
        }
        should_close
    }

    fn cancel_wire(&mut self, id: u32) -> bool {
        self.cancelled_through = Some(
            self.cancelled_through
                .map_or(id, |cancelled| cancelled.max(id)),
        );
        self.latest_id = Some(self.latest_id.map_or(id, |latest| latest.max(id)));
        let should_close = self.active.is_some_and(|attempt| attempt.id <= id);
        if should_close {
            self.active = None;
        }
        should_close
    }
}

fn progress_text(current: usize, total: usize) -> String {
    format!("正在处理第 {current} 张图 ，共 {total} 张...")
}

struct CancelIvars {
    id: u32,
    on_cancel: tauri::ipc::Channel<CancelNotice>,
}

define_class!(
    // SAFETY: NSObject has no subclass requirements and every instance stays on the main thread.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = CancelIvars]
    struct CancelTarget;

    // SAFETY: NSObjectProtocol adds no requirements.
    unsafe impl NSObjectProtocol for CancelTarget {}

    impl CancelTarget {
        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&AnyObject>) {
            let _ = self
                .ivars()
                .on_cancel
                .send(CancelNotice { id: self.ivars().id });
        }
    }
);

impl CancelTarget {
    fn new(
        mtm: MainThreadMarker,
        id: u32,
        on_cancel: tauri::ipc::Channel<CancelNotice>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CancelIvars { id, on_cancel });
        // SAFETY: NSObject's init has this exact signature for the allocated subclass.
        unsafe { msg_send![super(this), init] }
    }
}

struct Owner {
    attempt: Attempt,
    parent: Retained<NSWindow>,
    sheet: Retained<NSWindow>,
    text: Retained<NSTextField>,
    indicator: Retained<NSProgressIndicator>,
    _target: Retained<CancelTarget>,
    _progress: Retained<NSProgress>,
}

impl Owner {
    fn close(&self) {
        self.parent.endSheet(&self.sheet);
        self.sheet.orderOut(None);
        // SAFETY: The indicator is retained in this main-thread owner and accepts a nil sender.
        unsafe { self.indicator.stopAnimation(None) };
    }
}

#[derive(Default)]
struct SheetState {
    generations: BTreeMap<String, Generation>,
    owners: BTreeMap<String, Owner>,
}

thread_local! {
    static SHEETS: RefCell<SheetState> = RefCell::new(SheetState::default());
}

fn activate(constraint: Retained<NSLayoutConstraint>) {
    constraint.setActive(true);
}

fn create_owner(
    mtm: MainThreadMarker,
    window: &tauri::Window,
    attempt: Attempt,
    on_cancel: tauri::ipc::Channel<CancelNotice>,
) -> Option<Owner> {
    let pointer = window.ns_window().ok()?.cast::<NSWindow>();
    // SAFETY: Tauri returns its live NSWindow. This runs on AppKit's main thread and retaining it
    // keeps the parent valid for the sheet owner's lifetime.
    let parent = unsafe { Retained::retain(pointer) }?;
    // SAFETY: The locked SDK initializer is called with a valid main-thread marker and values.
    let sheet = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(360., 0.)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: This Retained owner controls the window lifetime outside an NSWindowController.
    unsafe { sheet.setReleasedWhenClosed(false) };
    sheet.setHasShadow(true);

    let content = NSView::new(mtm);
    let stack = NSStackView::new(mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation(1));
    stack.setAlignment(NSLayoutAttribute(0));
    stack.setDistribution(NSStackViewDistribution(2));
    stack.setTranslatesAutoresizingMaskIntoConstraints(false);

    let text = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    text.setDrawsBackground(false);
    text.setEditable(false);
    text.setSelectable(false);
    text.setBezeled(false);
    text.setContentHuggingPriority_forOrientation(249., NSLayoutConstraintOrientation::Horizontal);

    let progress = NSProgress::progressWithTotalUnitCount(100);
    progress.setCompletedUnitCount(0);
    let indicator = NSProgressIndicator::new(mtm);
    indicator.setObservedProgress(Some(&progress));
    indicator.setMinValue(0.);
    indicator.setMaxValue(1.);
    indicator.setIndeterminate(progress.isIndeterminate());
    indicator.setDoubleValue(progress.fractionCompleted());
    indicator
        .setContentHuggingPriority_forOrientation(249., NSLayoutConstraintOrientation::Horizontal);

    let target = CancelTarget::new(mtm, attempt.id, on_cancel);
    // SAFETY: The retained target implements cancel: with the exact action signature.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("取消"),
            Some(&target),
            Some(sel!(cancel:)),
            mtm,
        )
    };
    button.setBezelStyle(NSBezelStyle(1));
    button.setTranslatesAutoresizingMaskIntoConstraints(false);
    let wrapper = NSView::new(mtm);
    wrapper.addSubview(&button);
    activate(
        button
            .trailingAnchor()
            .constraintEqualToAnchor_constant(&wrapper.trailingAnchor(), -2.),
    );
    activate(
        button
            .topAnchor()
            .constraintEqualToAnchor_constant(&wrapper.topAnchor(), 2.),
    );
    activate(
        button
            .bottomAnchor()
            .constraintEqualToAnchor_constant(&wrapper.bottomAnchor(), -2.),
    );

    stack.addArrangedSubview(&text);
    stack.addArrangedSubview(&indicator);
    stack.addArrangedSubview(&wrapper);
    sheet.setContentView(Some(&content));
    content.addSubview(&stack);
    activate(
        stack
            .leadingAnchor()
            .constraintEqualToAnchor_constant(&content.leadingAnchor(), 15.),
    );
    activate(
        stack
            .trailingAnchor()
            .constraintEqualToAnchor_constant(&content.trailingAnchor(), -15.),
    );
    activate(
        stack
            .topAnchor()
            .constraintEqualToAnchor_constant(&content.topAnchor(), 15.),
    );
    activate(
        stack
            .bottomAnchor()
            .constraintEqualToAnchor_constant(&content.bottomAnchor(), -15.),
    );

    parent.beginCriticalSheet_completionHandler(&sheet, None);
    sheet.makeKeyAndOrderFront(None);
    // SAFETY: The retained indicator is on the AppKit main thread and accepts a nil sender.
    unsafe { indicator.startAnimation(None) };

    Some(Owner {
        attempt,
        parent,
        sheet,
        text,
        indicator,
        _target: target,
        _progress: progress,
    })
}

fn show(
    window: tauri::Window,
    attempt: Attempt,
    on_cancel: tauri::ipc::Channel<CancelNotice>,
    active: &AtomicBool,
) {
    let label = window.label().to_string();
    if !active.load(Ordering::Acquire) {
        SHEETS.with(|cell| {
            cell.borrow_mut()
                .generations
                .entry(label)
                .or_default()
                .finish(attempt);
        });
        return;
    }
    let should_show = SHEETS.with(|cell| {
        cell.borrow_mut()
            .generations
            .entry(label.clone())
            .or_default()
            .begin_show(attempt)
    });
    if !should_show {
        return;
    }
    SHEETS.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(owner) = state.owners.remove(&label) {
            owner.close();
        }
    });
    let Some(mtm) = MainThreadMarker::new() else {
        SHEETS.with(|cell| {
            cell.borrow_mut()
                .generations
                .entry(label)
                .or_default()
                .finish(attempt);
        });
        return;
    };
    let owner = create_owner(mtm, &window, attempt, on_cancel);
    SHEETS.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(owner) = owner {
            state.owners.insert(label, owner);
        } else {
            state.generations.entry(label).or_default().finish(attempt);
        }
    });
}

fn update(window: tauri::Window, attempt: Attempt, current: usize, total: usize) {
    let label = window.label().to_string();
    let _ = window.run_on_main_thread(move || {
        SHEETS.with(|cell| {
            let state = cell.borrow();
            let allowed = state
                .generations
                .get(&label)
                .is_some_and(|generation| generation.allows(attempt));
            if allowed
                && let Some(owner) = state
                    .owners
                    .get(&label)
                    .filter(|owner| owner.attempt == attempt)
            {
                owner
                    .text
                    .setStringValue(&NSString::from_str(&progress_text(current, total)));
            }
        });
    });
}

fn hide_attempt(window: tauri::Window, attempt: Attempt) {
    let label = window.label().to_string();
    let _ = window.run_on_main_thread(move || {
        SHEETS.with(|cell| {
            let mut state = cell.borrow_mut();
            let should_close = state
                .generations
                .entry(label.clone())
                .or_default()
                .finish(attempt);
            if should_close
                && let Some(owner) = state.owners.get(&label)
                && owner.attempt == attempt
            {
                owner.close();
                state.owners.remove(&label);
            }
        });
    });
}

pub(super) fn hide(window: tauri::Window, id: u32) {
    let label = window.label().to_string();
    let _ = window.run_on_main_thread(move || {
        SHEETS.with(|cell| {
            let mut state = cell.borrow_mut();
            let should_close = state
                .generations
                .entry(label.clone())
                .or_default()
                .cancel_wire(id);
            if should_close && let Some(owner) = state.owners.remove(&label) {
                owner.close();
            }
        });
    });
}

pub(super) struct Session {
    window: tauri::Window,
    attempt: Attempt,
    active: Arc<AtomicBool>,
}

impl Session {
    pub(super) fn start(
        window: tauri::Window,
        id: u32,
        on_cancel: tauri::ipc::Channel<CancelNotice>,
    ) -> Option<Self> {
        let attempt = allocate_attempt(id)?;
        let active = Arc::new(AtomicBool::new(true));
        let session = Self {
            window,
            attempt,
            active,
        };
        let show_active = Arc::clone(&session.active);
        let show_window = session.window.clone();
        session
            .window
            .run_on_main_thread(move || show(show_window, attempt, on_cancel, &show_active))
            .ok()?;
        Some(session)
    }

    pub(super) fn observer(&self) -> Box<dyn FnMut(ProgressNotice) + Send> {
        let window = self.window.clone();
        let attempt = self.attempt;
        Box::new(move |notice| {
            if notice.id == attempt.id {
                update(window.clone(), attempt, notice.current, notice.total);
            }
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        hide_attempt(self.window.clone(), self.attempt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(id: u32, token: u64) -> Attempt {
        Attempt {
            id,
            token: AttemptToken(token),
        }
    }

    #[test]
    fn source_progress_text_uses_real_current_and_total() {
        assert_eq!(progress_text(2, 7), "正在处理第 2 张图 ，共 7 张...");
    }

    #[test]
    fn hide_before_queued_show_prevents_orphaned_sheet() {
        let mut generation = Generation::default();
        let queued = attempt(7, 1);
        assert!(!generation.cancel_wire(7));
        assert!(!generation.begin_show(queued));
        assert!(!generation.allows(queued));
    }

    #[test]
    fn newer_job_rejects_late_update_hide_and_duplicate_show() {
        let mut generation = Generation::default();
        let old = attempt(7, 1);
        let current = attempt(8, 2);
        assert!(generation.begin_show(old));
        assert!(generation.allows(old));
        assert!(generation.begin_show(current));
        assert!(!generation.begin_show(current));
        assert!(!generation.allows(old));
        assert!(!generation.finish(old));
        assert!(generation.allows(current));
        assert!(!generation.cancel_wire(7));
        assert!(generation.allows(current));
    }

    #[test]
    fn failed_attempt_can_retry_with_the_same_wire_id() {
        let mut generation = Generation::default();
        let failed = attempt(7, 1);
        let retry = attempt(7, 2);
        assert!(generation.begin_show(failed));
        assert!(generation.finish(failed));
        assert!(generation.begin_show(retry));
        assert!(generation.allows(retry));
        assert!(!generation.allows(failed));
        assert!(!generation.finish(failed));
        assert!(!generation.begin_show(failed));
        assert!(generation.allows(retry));
    }

    #[test]
    fn positive_cancel_blocks_queued_wire_but_not_a_newer_wire() {
        let mut generation = Generation::default();
        let cancelled = attempt(7, 1);
        let newer = attempt(8, 2);
        assert!(!generation.cancel_wire(7));
        assert!(!generation.begin_show(cancelled));
        assert!(generation.begin_show(newer));
        assert!(generation.allows(newer));
        assert!(!generation.cancel_wire(7));
        assert!(generation.allows(newer));
    }
}
