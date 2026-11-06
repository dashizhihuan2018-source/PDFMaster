//! Narrow admission of constant stroking/nonstroking alpha; hayro owns state/compositing.
use super::{AuditedResources, PreflightState, finite_number, resolve_object};
use crate::{PdfMasterError, Result};
use lopdf::{Document, Object};

fn fail(message: impl std::fmt::Display) -> PdfMasterError {
    PdfMasterError::Render(format!("ExtGState: {message}"))
}

fn resolve<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Object> {
    let resolved = resolve_object(document, value)
        .map(|(_, value)| value)
        .map_err(|error| match error {
            PdfMasterError::Render(message) => fail(message),
            other => other,
        })?;
    // The pinned backend resolves typed resource values only one reference hop.
    // Resolve first so malformed/cyclic/over-budget graphs keep their strict errors.
    if matches!(value, Object::Reference(id) if matches!(document.objects.get(id), Some(Object::Reference(_))))
    {
        return Err(fail("unsupported reference depth (maximum one hop)"));
    }
    Ok(resolved)
}

pub(super) fn audit_resources(
    document: &Document,
    value: &Object,
    state: &mut PreflightState,
    audited: &mut AuditedResources,
) -> Result<()> {
    let category = resolve(document, value)?
        .as_dict()
        .map_err(|_| fail("resources must resolve to a dictionary"))?;
    for (name, entry) in category.iter() {
        state.count_resource()?;
        state.charge_source_bytes(name.len() as u64)?;
        let parameters = resolve(document, entry)?
            .as_dict()
            .map_err(|_| fail("entry must resolve to a dictionary"))?;
        for (key, value) in parameters.iter() {
            match key.as_slice() {
                b"Type" => {
                    if resolve(document, value)?.as_name().ok() != Some(b"ExtGState") {
                        return Err(fail("Type must resolve to the name ExtGState"));
                    }
                }
                b"CA" | b"ca" => {
                    let alpha = finite_number(resolve(document, value)?)
                        .ok_or_else(|| fail("CA/ca must be finite numbers in 0..=1"))?;
                    if !(0.0..=1.0).contains(&alpha) {
                        return Err(fail("CA/ca must be finite numbers in 0..=1"));
                    }
                }
                _ => {
                    return Err(fail(format!(
                        "unsupported parameter {}",
                        String::from_utf8_lossy(key)
                    )));
                }
            }
        }
        audited.extgstates.insert(name.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn shared_resolver_accepts_64_reference_hops_and_rejects_the_65th() {
        for hops in [64, 65, 66] {
            let mut doc = Document::new();
            let mut id = doc.add_object(Object::Real(0.5));
            for _ in 1..hops {
                id = doc.add_object(Object::Reference(id));
            }
            let reference = Object::Reference(id);
            let result = resolve_object(&doc, &reference);
            if hops == 64 {
                let (first, value) = result.unwrap();
                assert_eq!(first, Some(id));
                assert_eq!(value, &Object::Real(0.5));
            } else {
                assert!(matches!(
                    result,
                    Err(PdfMasterError::Budget("source reference depth"))
                ));
            }
        }
    }

    #[test]
    fn unrepresentable_nonfinite_reals_are_rejected_by_the_actual_auditor() {
        for key in ["CA", "ca"] {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut entry = dictionary! {};
                entry.set(key, Object::Real(value));
                let category: Object = dictionary! {"A" => entry}.into();
                let error = audit_resources(
                    &Document::new(),
                    &category,
                    &mut PreflightState::default(),
                    &mut AuditedResources::default(),
                )
                .unwrap_err();
                assert!(error.to_string().contains("finite numbers"), "{error}");
            }
        }
    }

    #[test]
    fn names_are_charged_to_the_existing_source_byte_budget_before_admission() {
        let doc = Document::new();
        let category: Object = dictionary! {"Alpha" => dictionary! {}}.into();
        let mut state = PreflightState {
            source_resource_bytes: super::super::MAX_SOURCE_RESOURCE_BYTES - 5,
            ..Default::default()
        };
        let mut audited = AuditedResources::default();
        audit_resources(&doc, &category, &mut state, &mut audited).unwrap();
        assert!(audited.extgstates.contains(b"Alpha".as_slice()));
        let mut audited = AuditedResources::default();
        let error = audit_resources(&doc, &category, &mut state, &mut audited).unwrap_err();
        assert!(matches!(
            error,
            PdfMasterError::Budget("source resource bytes")
        ));
        assert!(audited.extgstates.is_empty());
    }
}
