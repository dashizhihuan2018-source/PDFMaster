//! Admission for the direct Span/ActualText marked-content profile.
use super::{MAX_RENDER_CONTENT_BYTES, operation_error};
use crate::{PdfMasterError, Result};
use lopdf::content::Operation;

pub(super) fn validate_actual_text(operation: &Operation, owner: &str) -> Result<()> {
    let fail = || operation_error(owner, operation, "requires direct Span/ActualText UTF16BE");
    if operation.operands.len() != 2
        || operation.operands[0].as_name().ok() != Some(b"Span".as_slice())
    {
        return Err(fail());
    }
    let properties = operation.operands[1].as_dict().map_err(|_| fail())?;
    if properties.len() != 1 {
        return Err(fail());
    }
    let bytes = properties
        .get(b"ActualText")
        .and_then(lopdf::Object::as_str)
        .map_err(|_| fail())?;
    validate_payload(bytes, operation, owner)
}

fn validate_payload(bytes: &[u8], operation: &Operation, owner: &str) -> Result<()> {
    if bytes.len() > MAX_RENDER_CONTENT_BYTES {
        return Err(PdfMasterError::Budget("ActualText bytes"));
    }
    let fail = || operation_error(owner, operation, "requires valid UTF16BE ActualText");
    if !bytes.starts_with(&[0xfe, 0xff]) || !bytes.len().is_multiple_of(2) {
        return Err(fail());
    }
    let units = bytes[2..]
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    for scalar in char::decode_utf16(units) {
        scalar.map_err(|_| fail())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Object, dictionary};

    #[test]
    fn actual_text_direct_helper_rejects_indirect_properties_and_values() {
        let reference = Object::Reference((5000, 0));
        let outcomes: Vec<_> = [reference.clone(), dictionary! { "ActualText" => reference }.into()]
            .into_iter().map(|properties| {
                let op = Operation::new("BDC", vec![Object::Name(b"Span".to_vec()), properties]);
                matches!(validate_actual_text(&op, "test"), Err(PdfMasterError::Render(message)) if message.contains("operator BDC") && message.contains("direct Span/ActualText"))
            }).collect();
        assert_eq!(outcomes, vec![true; 2]);
    }

    #[test]
    fn actual_text_payload_enforces_exact_production_budget_before_encoding() {
        let op = Operation::new("BDC", vec![]);
        let mut bytes = vec![0; super::super::MAX_RENDER_CONTENT_BYTES];
        bytes[..2].copy_from_slice(&[0xfe, 0xff]);
        assert!(validate_payload(&bytes, &op, "test").is_ok());
        bytes.push(0); // Also odd: budget must win.
        assert!(matches!(
            validate_payload(&bytes, &op, "test"),
            Err(PdfMasterError::Budget("ActualText bytes"))
        ));
    }

    #[test]
    fn actual_text_payload_accepts_valid_unicode_without_normalization() {
        let op = Operation::new("BDC", vec![]);
        for bytes in [
            vec![0xfe, 0xff],
            vec![0xfe, 0xff, 0xd8, 0x42, 0xdf, 0xb7],
            vec![0xfe, 0xff, 0x20, 0x0d],
        ] {
            assert!(validate_payload(&bytes, &op, "test").is_ok());
        }
    }

    #[test]
    fn actual_text_payload_rejects_malformed_encoding_with_bounded_context() {
        let op = Operation::new("BDC", vec![]);
        let outcomes: Vec<_> = [vec![], vec![0, 65], vec![0xff, 0xfe, 65, 0], vec![0xfe, 0xff, 0], vec![0xfe, 0xff, 0xd8, 0], vec![0xfe, 0xff, 0xdc, 0], vec![0xfe, 0xff, 0xd8, 0, 0, 65]]
            .into_iter().map(|bytes| {
                matches!(validate_payload(&bytes, &op, "test"), Err(PdfMasterError::Render(message)) if message.contains("operator BDC") && message.contains("valid UTF16BE") && message.len() < 200)
            }).collect();
        assert_eq!(outcomes, vec![true; 7]);
    }
}
