use super::*;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};

fn write_to(pasteboard: &NSPasteboard, payload: &pdfmaster_core::PageClipboard) -> Reply<()> {
    let item = NSPasteboardItem::new();
    for (kind, bytes) in [
        (PAGES_TYPE, &payload.pdf),
        ("com.adobe.pdf", &payload.pdf),
        ("public.png", &payload.png),
        ("public.tiff", &payload.tiff),
    ] {
        if !item.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(kind)) {
            return Err("无法准备页面剪贴板，原剪贴板未替换。".into());
        }
    }
    let object: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*item);
    let objects = NSArray::from_slice(&[object]);
    pasteboard.clearContents();
    if !pasteboard.writeObjects(&objects) {
        return Err("系统未接受页面剪贴板；文档未剪切。".into());
    }
    Ok(())
}
pub(super) fn write(_app: &tauri::AppHandle, payload: &pdfmaster_core::PageClipboard) -> Reply<()> {
    write_to(&NSPasteboard::generalPasteboard(), payload)
}
pub(super) fn read(_app: &tauri::AppHandle) -> Reply<Vec<u8>> {
    let data = NSPasteboard::generalPasteboard()
        .dataForType(&NSString::from_str(PAGES_TYPE))
        .ok_or("剪贴板中没有复制的 PDF 页面。")?;
    if data.is_empty() || data.len() > MAX_PDF {
        return Err("剪贴板 PDF 为空或超过32MiB预算。".into());
    }
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::{AnyThread, rc::autoreleasepool};
    use objc2_app_kit::NSBitmapImageRep;
    use objc2_foundation::{NSData, NSString};
    #[test]
    fn named_pasteboard_roundtrip_offers_pdf_png_tiff_and_observes_external_replacement() {
        autoreleasepool(|_| {
            let pb = NSPasteboard::pasteboardWithUniqueName();
            let copy = Session::blank(100., 80., 2)
                .unwrap()
                .page_clipboard(&[0, 1])
                .unwrap();
            let result = write_to(&pb, &copy);
            // Never touch NSPasteboard.general in this test.
            if result.is_err() {
                unsafe {
                    let (): () = objc2::msg_send![&*pb, releaseGlobally];
                }
            }
            result.unwrap();
            assert_eq!(
                pb.dataForType(&NSString::from_str(PAGES_TYPE))
                    .unwrap()
                    .to_vec(),
                copy.pdf
            );
            assert_eq!(
                pb.dataForType(&NSString::from_str("com.adobe.pdf"))
                    .unwrap()
                    .to_vec(),
                copy.pdf
            );
            for (kind, bytes) in [("public.png", &copy.png), ("public.tiff", &copy.tiff)] {
                let data = pb.dataForType(&NSString::from_str(kind)).unwrap();
                assert_eq!(data.to_vec(), *bytes);
                let bitmap = NSBitmapImageRep::initWithData(
                    NSBitmapImageRep::alloc(),
                    &NSData::with_bytes(bytes),
                )
                .unwrap();
                assert_eq!(bitmap.pixelsWide(), copy.width as isize);
                assert_eq!(bitmap.pixelsHigh(), copy.height as isize);
            }
            pb.clearContents();
            pb.setString_forType(
                &NSString::from_str("synthetic replacement"),
                &NSString::from_str("public.utf8-plain-text"),
            );
            assert!(pb.dataForType(&NSString::from_str(PAGES_TYPE)).is_none());
            unsafe {
                let (): () = objc2::msg_send![&*pb, releaseGlobally];
            }
        });
    }
}
