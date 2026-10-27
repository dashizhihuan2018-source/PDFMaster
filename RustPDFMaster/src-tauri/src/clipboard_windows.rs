use super::*;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
};

struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
struct Memory(HGLOBAL);
impl Drop for Memory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}
impl Memory {
    fn new(bytes: &[u8]) -> Reply<Self> {
        unsafe {
            let memory = Self(GlobalAlloc(GMEM_MOVEABLE, bytes.len()));
            if memory.0.is_null() {
                return Err("无法准备剪贴板内存，原剪贴板未替换。".into());
            }
            let ptr = GlobalLock(memory.0);
            if ptr.is_null() {
                return Err("无法访问剪贴板内存，原剪贴板未替换。".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
            GlobalUnlock(memory.0);
            Ok(memory)
        }
    }
}
fn format(name: &str) -> Reply<u32> {
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    let id = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if id == 0 {
        Err("无法注册系统剪贴板格式。".into())
    } else {
        Ok(id)
    }
}
pub(super) fn write(app: &tauri::AppHandle, payload: &pdfmaster_core::PageClipboard) -> Reply<()> {
    let window = app
        .get_webview_window("main")
        .ok_or("找不到页面剪贴板所属窗口。")?;
    let hwnd = window.hwnd().map_err(|_| "无法取得剪贴板窗口句柄。")?.0;
    if hwnd.is_null() {
        return Err("剪贴板窗口句柄无效。".into());
    }
    let formats = [
        format(PAGES_TYPE)?,
        format("Portable Document Format")?,
        format("PNG")?,
        6,
        8,
    ];
    let mut memory = [
        &payload.pdf,
        &payload.pdf,
        &payload.png,
        &payload.tiff,
        &payload.dib,
    ]
    .into_iter()
    .map(|b| Memory::new(b))
    .collect::<Reply<Vec<_>>>()?;
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err("系统剪贴板正忙；文档未剪切，请重试。".into());
        }
        let _guard = Clipboard;
        if EmptyClipboard() == 0 {
            return Err("无法替换系统剪贴板；文档未剪切。".into());
        }
        for (format, memory) in formats.into_iter().zip(&mut memory) {
            if SetClipboardData(format, memory.0).is_null() {
                return Err("系统未接受全部页面格式；文档未剪切，请重新复制。".into());
            }
            // Ownership passes to the OS only on success. Failed/unsubmitted
            // allocations are freed by Memory; clipboard-owned handles are not.
            memory.0 = null_mut();
        }
    }
    Ok(())
}
pub(super) fn read(_app: &tauri::AppHandle) -> Reply<Vec<u8>> {
    let kind = format(PAGES_TYPE)?;
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
            return Err("系统剪贴板正忙，请重试。".into());
        }
        let _guard = Clipboard;
        let memory = GetClipboardData(kind);
        if memory.is_null() {
            return Err("剪贴板中没有复制的 PDF 页面。".into());
        }
        let size = GlobalSize(memory);
        if size == 0 || size > MAX_PDF {
            return Err("剪贴板 PDF 为空或超过32MiB预算。".into());
        }
        let ptr = GlobalLock(memory);
        if ptr.is_null() {
            return Err("无法读取剪贴板 PDF。".into());
        }
        let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec();
        GlobalUnlock(memory);
        Ok(bytes)
    }
}
