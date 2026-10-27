use super::*;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString};
use objc2_ui_kit::{UIPasteboard, UIPasteboardOptionLocalOnly};

pub(super) fn write(_app: &tauri::AppHandle, payload: &pdfmaster_core::PageClipboard) -> Reply<()> {
    let kinds = [PAGES_TYPE, "com.adobe.pdf", "public.png", "public.tiff"].map(NSString::from_str);
    let data = [&payload.pdf, &payload.pdf, &payload.png, &payload.tiff]
        .map(|bytes| NSData::with_bytes(bytes));
    let values: Vec<&AnyObject> = data.iter().map(|d| &**d as &AnyObject).collect();
    let keys: Vec<_> = kinds.iter().map(|s| &**s).collect();
    let item = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &values);
    let items = NSArray::from_slice(&[&*item]);
    let local = NSNumber::numberWithBool(true);
    // Explicit user action only. Avoid publishing document bytes via Handoff.
    let options = NSDictionary::from_slices(
        &[unsafe { UIPasteboardOptionLocalOnly }],
        &[&*local as &AnyObject],
    );
    let pasteboard = UIPasteboard::generalPasteboard();
    unsafe {
        pasteboard.setItems_options(&items, &options);
    }
    if pasteboard.dataForPasteboardType(&kinds[0]).is_none() {
        return Err("系统未接受页面剪贴板；文档未剪切。".into());
    }
    Ok(())
}
pub(super) fn read(_app: &tauri::AppHandle) -> Reply<Vec<u8>> {
    let data = UIPasteboard::generalPasteboard()
        .dataForPasteboardType(&NSString::from_str(PAGES_TYPE))
        .ok_or("剪贴板中没有复制的 PDF 页面，或系统未允许读取。")?;
    if data.is_empty() || data.len() > MAX_PDF {
        return Err("剪贴板 PDF 为空或超过32MiB预算。".into());
    }
    Ok(data.to_vec())
}
