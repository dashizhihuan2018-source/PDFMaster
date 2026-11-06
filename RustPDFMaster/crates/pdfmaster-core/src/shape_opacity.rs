//! Private, exact recognizer for our own uniform-alpha shape appearances.
use crate::{PageInfo, PdfMasterError, Result, ShapeAnnotation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

const MARKER: &[u8] = b"PDFMaster_ShapeOpacity";
fn unsupported() -> PdfMasterError {
    PdfMasterError::InvalidOperation("shape alpha appearance is not canonical")
}
fn malformed() -> PdfMasterError {
    PdfMasterError::Malformed("invalid shape alpha structure".into())
}

// Use the accepted bounded/cycle-checking resolver first, then enforce the
// backend's direct/one-hop terminal profile at every resolved value.
fn terminal<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Object> {
    let resolved = resolved(document, value)?;
    if let Object::Reference(id) = value
        && matches!(document.objects.get(id), Some(Object::Reference(_)))
    {
        return Err(unsupported());
    }
    Ok(resolved)
}
// Structural traversal must finish before any valid-but-unsupported reference
// depth can hide malformed siblings. Keep the accepted bounded resolver here;
// terminal() applies the narrower backend profile only after this pass.
fn resolved<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Object> {
    crate::render::resolve_object(document, value).map(|(_, value)| value)
}
fn structural_number(document: &Document, value: &Object) -> Result<f32> {
    let value = resolved(document, value)?
        .as_float()
        .map_err(|_| malformed())?;
    if !value.is_finite() {
        return Err(malformed());
    }
    Ok(value)
}
fn structural_dict<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Dictionary> {
    resolved(document, value)?
        .as_dict()
        .map_err(|_| malformed())
}
fn number(document: &Document, value: &Object) -> Result<f32> {
    let v = terminal(document, value)?
        .as_float()
        .map_err(|_| malformed())?;
    if !v.is_finite() {
        return Err(malformed());
    }
    Ok(v)
}
fn dict<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Dictionary> {
    terminal(document, value)?
        .as_dict()
        .map_err(|_| malformed())
}
fn get<'a>(d: &'a Dictionary, name: &[u8]) -> Result<&'a Object> {
    d.get(name).map_err(|_| malformed())
}
fn only(d: &Dictionary, allowed: &[&[u8]]) -> Result<()> {
    if d.iter().any(|(key, _)| !allowed.contains(&key.as_slice())) {
        return Err(unsupported());
    }
    Ok(())
}
fn name(document: &Document, value: &Object, expected: &[u8]) -> Result<()> {
    if terminal(document, value)?
        .as_name()
        .map_err(|_| malformed())?
        != expected
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(crate) fn validate_occurrence(
    document: &Document,
    entry: &Object,
    annotation: &Dictionary,
) -> Result<()> {
    if annotation.has(MARKER) || annotation.has(b"CA") {
        terminal(document, entry)?;
    }
    Ok(())
}

pub(crate) fn validate_known_values(document: &Document, a: &Dictionary) -> Result<()> {
    for key in [b"CA".as_slice(), b"ca"] {
        if let Ok(value) = a.get(key)
            && !(0. ..=1.).contains(&structural_number(document, value)?)
        {
            return Err(malformed());
        }
    }
    if let Ok(value) = a.get(MARKER) {
        resolved(document, value)?
            .as_i64()
            .map_err(|_| malformed())?;
    }
    if a.has(MARKER) || a.has(b"CA") {
        let rect = resolved(document, get(a, b"Rect")?)?
            .as_array()
            .map_err(|_| malformed())?;
        if rect.len() != 4 {
            return Err(malformed());
        }
        for value in rect {
            structural_number(document, value)?;
        }
        // Traverse known AP fields before any valid-but-unsupported nominal
        // style can short circuit parsing. Semantic exactness is checked later.
        let Ok(ap) = a.get(b"AP") else {
            return Ok(());
        };
        let ap = structural_dict(document, ap)?;
        for key in [b"N".as_slice(), b"R", b"D"] {
            if let Ok(value) = ap.get(key) {
                match resolved(document, value)? {
                    Object::Stream(stream) => validate_stream_structure(document, stream)?,
                    Object::Dictionary(states) => {
                        for (_, value) in states {
                            let stream = resolved(document, value)?
                                .as_stream()
                                .map_err(|_| malformed())?;
                            validate_stream_structure(document, stream)?;
                        }
                    }
                    _ => return Err(malformed()),
                }
            }
        }
    }
    Ok(())
}

fn validate_stream_structure(document: &Document, stream: &Stream) -> Result<()> {
    for key in [b"Type".as_slice(), b"Subtype"] {
        if let Ok(value) = stream.dict.get(key) {
            resolved(document, value)?
                .as_name()
                .map_err(|_| malformed())?;
        }
    }
    if let Ok(value) = stream.dict.get(b"FormType") {
        resolved(document, value)?
            .as_i64()
            .map_err(|_| malformed())?;
    }
    if let Ok(value) = stream.dict.get(b"BBox") {
        let bbox = resolved(document, value)?
            .as_array()
            .map_err(|_| malformed())?;
        if bbox.len() != 4 {
            return Err(malformed());
        }
        for value in bbox {
            structural_number(document, value)?;
        }
    }
    if let Ok(value) = stream.dict.get(b"Length")
        && resolved(document, value)?
            .as_i64()
            .map_err(|_| malformed())?
            != stream.content.len() as i64
    {
        return Err(malformed());
    }
    if let Ok(value) = stream.dict.get(b"Resources") {
        let resources = structural_dict(document, value)?;
        if let Ok(value) = resources.get(b"ExtGState") {
            let states = structural_dict(document, value)?;
            if let Ok(value) = states.get(b"PMShapeAlpha") {
                let state = structural_dict(document, value)?;
                if let Ok(value) = state.get(b"Type") {
                    resolved(document, value)?
                        .as_name()
                        .map_err(|_| malformed())?;
                }
                for key in [b"CA".as_slice(), b"ca"] {
                    if let Ok(value) = state.get(key)
                        && !(0. ..=1.).contains(&structural_number(document, value)?)
                    {
                        return Err(malformed());
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn require_normal_appearance_if_present(
    document: &Document,
    a: &Dictionary,
) -> Result<()> {
    if a.has(b"AS") {
        return Err(unsupported());
    }
    let Ok(ap) = a.get(b"AP") else {
        return Ok(());
    };
    let ap = dict(document, ap)?;
    only(ap, &[b"N"])?;
    terminal(document, get(ap, b"N")?)?
        .as_stream()
        .map_err(|_| unsupported())?;
    Ok(())
}

fn content(model: &ShapeAnnotation) -> Result<Vec<u8>> {
    let raw = if let Some(line) = &model.line {
        crate::lines::appearance(
            line,
            model.bounds,
            model.stroke,
            model.fill,
            model.line_width,
        )?
    } else {
        crate::shapes::appearance(
            model.bounds.width,
            model.bounds.height,
            model.shape,
            model.stroke,
            model.fill,
            model.line_width,
        )?
    };
    let rest = raw.strip_prefix(b"q\n").ok_or_else(unsupported)?;
    let mut result = b"q\n/PMShapeAlpha gs\n".to_vec();
    result.extend_from_slice(rest);
    Ok(result)
}

pub(crate) fn validate(
    document: &Document,
    annotation: &Dictionary,
    model: &ShapeAnnotation,
) -> Result<f32> {
    validate_known_values(document, annotation)?;
    if annotation.has(b"ca") {
        return Err(unsupported());
    }
    if !annotation.has(MARKER) && !annotation.has(b"CA") {
        return Ok(1.);
    }
    if !annotation.has(MARKER) || !annotation.has(b"CA") {
        return Err(unsupported());
    }
    // The backend does not render otherwise canonical alpha shapes whose
    // annotation Rect array or scalar coordinates require multiple hops. Do
    // not normalize them into a fresh appearance during an ordinary edit.
    let rect = terminal(document, get(annotation, b"Rect")?)?
        .as_array()
        .map_err(|_| malformed())?;
    for value in rect {
        terminal(document, value)?;
    }
    if terminal(document, get(annotation, MARKER)?)?
        .as_i64()
        .map_err(|_| malformed())?
        != 1
    {
        return Err(unsupported());
    }
    let alpha = number(document, get(annotation, b"CA")?)?;
    if alpha == 1. {
        return Err(unsupported());
    }
    require_normal_appearance_if_present(document, annotation)?;
    let ap = dict(document, get(annotation, b"AP")?)?;
    let stream = terminal(document, get(ap, b"N")?)?
        .as_stream()
        .map_err(|_| malformed())?;
    let d = &stream.dict;
    only(
        d,
        &[
            b"Type",
            b"Subtype",
            b"FormType",
            b"BBox",
            b"Resources",
            b"Length",
        ],
    )?;
    name(document, get(d, b"Type")?, b"XObject")?;
    name(document, get(d, b"Subtype")?, b"Form")?;
    // Unchanged renderer preflight requires a direct XObject Subtype. An
    // equivalent one-hop name is not a currently renderable editable profile.
    if !matches!(get(d, b"Subtype")?, Object::Name(_)) {
        return Err(unsupported());
    }
    if terminal(document, get(d, b"FormType")?)?
        .as_i64()
        .map_err(|_| malformed())?
        != 1
    {
        return Err(unsupported());
    }
    let bbox = terminal(document, get(d, b"BBox")?)?
        .as_array()
        .map_err(|_| malformed())?;
    if bbox.len() != 4 {
        return Err(malformed());
    }
    let actual = bbox
        .iter()
        .map(|v| number(document, v))
        .collect::<Result<Vec<_>>>()?;
    if actual != [0., 0., model.bounds.width, model.bounds.height] {
        return Err(unsupported());
    }
    if let Ok(length) = d.get(b"Length")
        && terminal(document, length)?
            .as_i64()
            .map_err(|_| malformed())?
            != stream.content.len() as i64
    {
        return Err(malformed());
    }
    if stream.content.len() > 65536 {
        return Err(PdfMasterError::Budget("shape alpha appearance content"));
    }
    let resources = dict(document, get(d, b"Resources")?)?;
    only(resources, &[b"ExtGState"])?;
    let states = dict(document, get(resources, b"ExtGState")?)?;
    only(states, &[b"PMShapeAlpha"])?;
    let state = dict(document, get(states, b"PMShapeAlpha")?)?;
    only(state, &[b"Type", b"CA", b"ca"])?;
    name(document, get(state, b"Type")?, b"ExtGState")?;
    // Parse both before comparing either; malformed known values cannot hide.
    let stroke = number(document, get(state, b"CA")?)?;
    let fill = number(document, get(state, b"ca")?)?;
    if stroke != alpha || fill != alpha || stream.content != content(model)? {
        return Err(unsupported());
    }
    Ok(alpha)
}

pub(crate) fn apply_to_generated(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    annotation_index: usize,
    opacity: f32,
) -> Result<()> {
    if !opacity.is_finite() || !(0. ..=1.).contains(&opacity) {
        return Err(malformed());
    }
    let page_id = crate::document::page_object_ids(document)?[page];
    let entries = crate::notes::validated_annots_for_add(document, page_id, &pages[page])?;
    let (_, id) = crate::notes::resolve_with_id(document, &entries[annotation_index])?;
    let id = id.ok_or_else(unsupported)?;
    // This is a newly created/COW root only. Never unmark the shared source.
    let mut nominal = document.clone();
    let a = nominal
        .get_object_mut(id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| malformed())?;
    a.remove(b"CA");
    a.remove(MARKER);
    require_normal_appearance_if_present(
        &nominal,
        nominal
            .get_object(id)
            .and_then(Object::as_dict)
            .map_err(|_| malformed())?,
    )?;
    let model = crate::shapes::inspect_canvas_shapes(&nominal, pages, page)?
        .shapes
        .into_iter()
        .find(|m| m.annotation_index == annotation_index)
        .ok_or_else(unsupported)?;
    let ap = if opacity == 1. {
        None
    } else {
        let stream = Stream::new(
            dictionary! {
                "Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1,
                "BBox"=>vec![0.into(), 0.into(), model.bounds.width.into(), model.bounds.height.into()],
                "Resources"=>dictionary! {"ExtGState"=>dictionary! {"PMShapeAlpha"=>dictionary! {"Type"=>"ExtGState", "CA"=>opacity, "ca"=>opacity}}},
            },
            content(&model)?,
        );
        Some(document.add_object(stream))
    };
    let a = document
        .get_object_mut(id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| malformed())?;
    a.remove(b"CA");
    a.remove(MARKER);
    if let Some(ap) = ap {
        a.set("AP", dictionary! {"N"=>ap});
        a.set("CA", opacity);
        a.set(MARKER, 1);
    }
    validate(
        document,
        document
            .get_object(id)
            .and_then(Object::as_dict)
            .map_err(|_| malformed())?,
        &model,
    )?;
    crate::notes::validate_notes_page(document, page_id, &pages[page])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanvasShapeDraft, CanvasShapeGeometry, Color, Rect, Session};

    #[test]
    fn raw_nonfinite_and_length_fields_cannot_be_serialization_normalized_away() {
        let mut session = Session::blank(200., 120., 1).unwrap();
        session
            .create_canvas_shape(
                0,
                &CanvasShapeDraft {
                    geometry: CanvasShapeGeometry::Rectangle {
                        bounds: Rect {
                            x: 20.,
                            y: 20.,
                            width: 100.,
                            height: 60.,
                        },
                    },
                    stroke: Color {
                        r: 1.,
                        g: 0.,
                        b: 0.,
                    },
                    fill: None,
                    line_width: 2.,
                    opacity: 0.5,
                },
            )
            .unwrap();
        let selection = session.canvas_shapes(0).unwrap().annotations.remove(0);
        let root = selection.object_id.unwrap();
        let model = selection.original;
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut d = session.document.clone();
            d.get_object_mut(root)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("CA", Object::Real(invalid));
            assert!(matches!(
                validate(&d, d.get_object(root).unwrap().as_dict().unwrap(), &model),
                Err(PdfMasterError::Malformed(_))
            ));
            // Nonfinite PDF numbers cannot be faithfully serialized. Exercise
            // the real strict projector on raw objects, not fabricated wire.
            for coordinate in 0..4 {
                for hops in 0..=2 {
                    let mut d = session.document.clone();
                    let mut value = Object::Real(invalid);
                    for _ in 0..hops {
                        value = Object::Reference(d.add_object(value));
                    }
                    let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
                    a.get_mut(b"Rect").unwrap().as_array_mut().unwrap()[coordinate] = value;
                    a.set("BE", dictionary! {"S"=>"C"});
                    assert!(matches!(
                        crate::shapes::inspect_canvas_shapes(&d, &session.pages, 0),
                        Err(PdfMasterError::Malformed(_))
                    ));
                }
            }
        }
        for length in [Object::Integer(1), Object::Real(42.), Object::Null] {
            let mut d = session.document.clone();
            let ap = d
                .get_object(root)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"AP")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"N")
                .unwrap()
                .as_reference()
                .unwrap();
            d.get_object_mut(ap)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .dict
                .set("Length", length);
            assert!(matches!(
                validate(&d, d.get_object(root).unwrap().as_dict().unwrap(), &model),
                Err(PdfMasterError::Malformed(_))
            ));
        }
    }
}
