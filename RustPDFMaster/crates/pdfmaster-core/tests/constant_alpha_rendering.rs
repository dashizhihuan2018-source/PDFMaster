use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use pdfmaster_core::Session;

const PAINT: &[u8] = b"0 0 1 rg 0 0 120 100 re f q /A gs 1 0 0 rg 1 0 0 RG 6 w 20 20 80 60 re B Q";

fn save(mut doc: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn alpha_pdf(stroke: f32, fill: f32) -> Vec<u8> {
    carrier(
        dictionary! {"ExtGState" => dictionary! {
            "A" => dictionary! {"Type" => "ExtGState", "CA" => Object::Real(stroke), "ca" => Object::Real(fill)}
        }}.into(),
        PAINT,
        Placement::Page,
        0,
    )
}

#[derive(Clone, Copy, Debug)]
enum Placement {
    Page,
    Form,
    Appearance,
}

fn carrier(resources: Object, paint: &[u8], placement: Placement, rotation: i32) -> Vec<u8> {
    let source = Session::blank(120., 100., 1).unwrap().bytes().unwrap();
    let mut doc = Document::load_mem(&source).unwrap();
    let page_id = doc.get_pages()[&1];
    let (page_resources, page_paint) = match placement {
        Placement::Page => (resources, paint.to_vec()),
        Placement::Form | Placement::Appearance => {
            let form = doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
                    "BBox" => vec![0.into(),0.into(),120.into(),100.into()],
                    "Resources" => resources,
                },
                paint.to_vec(),
            ));
            match placement {
                Placement::Form => (
                    dictionary! {"XObject" => dictionary! {"Carrier" => form}}.into(),
                    b"/Carrier Do".to_vec(),
                ),
                Placement::Appearance => {
                    let annot = doc.add_object(dictionary! {
                        "Type" => "Annot", "Subtype" => "Square", "F" => 4,
                        "Rect" => vec![0.into(),0.into(),120.into(),100.into()],
                        "AP" => dictionary! {"N" => form},
                    });
                    doc.get_object_mut(page_id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Annots", vec![Object::Reference(annot)]);
                    (dictionary! {}.into(), Vec::new())
                }
                Placement::Page => unreachable!(),
            }
        }
    };
    let content = doc.add_object(Stream::new(dictionary! {}, page_paint));
    let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
    page.set("Resources", page_resources);
    page.set("Contents", content);
    page.set("Rotate", rotation);
    save(doc)
}

fn resources(states: Dictionary) -> Object {
    dictionary! {"ExtGState" => states}.into()
}

fn indirect_field_pdf(field: &str, hops: usize, placement: Placement, alpha: f32) -> Vec<u8> {
    let state =
        dictionary! {"Type" => "ExtGState","CA" => Object::Real(alpha),"ca" => Object::Real(alpha)};
    let bytes = carrier(
        resources(dictionary! {"A" => state.clone()}),
        PAINT,
        placement,
        0,
    );
    let mut doc = Document::load_mem(&bytes).unwrap();
    let terminal: Object = match field {
        "category" => dictionary! {"A" => state.clone()}.into(),
        "entry" => state.clone().into(),
        "Type" => Object::Name(b"ExtGState".to_vec()),
        _ => Object::Real(alpha),
    };
    let mut id = doc.add_object(terminal);
    for _ in 1..hops {
        id = doc.add_object(Object::Reference(id));
    }
    let category = match field {
        "category" => Object::Reference(id),
        "entry" => dictionary! {"A" => id}.into(),
        key => {
            let mut state = state;
            state.set(key, id);
            dictionary! {"A" => state}.into()
        }
    };
    let resource: Object = dictionary! {"ExtGState" => category}.into();
    match placement {
        Placement::Page => {
            let page_id = doc.get_pages()[&1];
            doc.get_object_mut(page_id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Resources", resource);
        }
        _ => {
            let target = doc
                .objects
                .values_mut()
                .find_map(|object| match object {
                    Object::Stream(stream) if stream.content == PAINT => Some(stream),
                    _ => None,
                })
                .unwrap();
            target.dict.set("Resources", resource);
        }
    }
    save(doc)
}

fn rgba(bytes: &[u8]) -> image::RgbaImage {
    let s = Session::open(bytes).unwrap();
    let before = (s.bytes().unwrap(), s.info());
    assert_eq!(before.0, bytes);
    let png = s.render_png(0, 1.).unwrap();
    assert_eq!((s.bytes().unwrap(), s.info()), before);
    image::load_from_memory(&png).unwrap().to_rgba8()
}

fn pixel(image: &image::RgbaImage, x: u32, y: u32, expected: [u8; 4]) {
    let actual = image.get_pixel(x, y).0;
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
        "({x},{y}): {actual:?} != {expected:?}"
    );
    assert_eq!(actual[3], 255);
}

fn rejected(bytes: &[u8], context: &str) {
    let s = Session::open(bytes).unwrap();
    let before = (s.bytes().unwrap(), s.info());
    let error = s.render_png(0, 1.).unwrap_err().to_string();
    assert!(
        error.contains(context),
        "{error:?} must contain {context:?}"
    );
    assert_eq!((s.bytes().unwrap(), s.info()), before);
}

#[test]
fn half_alpha_is_actual_pdf_compositing_after_reopen() {
    let bytes = alpha_pdf(0.5, 0.5);
    let s = Session::open(&bytes).unwrap();
    let before = (s.bytes().unwrap(), s.info());
    let png = s.render_png(0, 1.).unwrap();
    let rgba = image::load_from_memory(&png).unwrap().to_rgba8();
    let p = rgba.get_pixel(60, 50).0;
    assert!(
        p[0].abs_diff(128) <= 2 && p[1] <= 2 && p[2].abs_diff(128) <= 2 && p[3] == 255,
        "{p:?}"
    );
    assert_eq!((s.bytes().unwrap(), s.info()), before);
}

#[test]
fn numeric_boundaries_and_half_alpha_have_mathematical_pixels() {
    for (alpha, expected) in [
        (0., [0, 0, 255, 255]),
        (0.5, [128, 0, 128, 255]),
        (1., [255, 0, 0, 255]),
    ] {
        let image = rgba(&alpha_pdf(alpha, alpha));
        pixel(&image, 60, 50, expected);
        pixel(&image, 18, 50, expected); // outside the fill, inside the stroke
        pixel(&image, 5, 5, [0, 0, 255, 255]);
    }
    let image = rgba(&carrier(
        resources(dictionary! {"A" => dictionary! {"CA" => 0, "ca" => 1}}),
        PAINT,
        Placement::Page,
        0,
    ));
    pixel(&image, 60, 50, [255, 0, 0, 255]);
    pixel(&image, 18, 50, [0, 0, 255, 255]);
}

#[test]
fn independent_stroke_fill_and_cumulative_missing_components() {
    for (states, ops, fill, stroke) in [
        (
            dictionary! {"A" => dictionary! {"CA" => 0.5}},
            PAINT.to_vec(),
            [255, 0, 0, 255],
            [128, 0, 128, 255],
        ),
        (
            dictionary! {"A" => dictionary! {"ca" => 0.5}},
            PAINT.to_vec(),
            [128, 0, 128, 255],
            [255, 0, 0, 255],
        ),
        (
            dictionary! {"A" => dictionary! {"ca" => 0.5}, "S" => dictionary! {"CA" => 0}, "E" => dictionary! {}},
            b"0 0 1 rg 0 0 120 100 re f /A gs /E gs /S gs 1 0 0 rg 1 0 0 RG 6 w 20 20 80 60 re B"
                .to_vec(),
            [128, 0, 128, 255],
            [0, 0, 255, 255],
        ),
        (
            dictionary! {"A" => dictionary! {"CA" => 0.5}, "F" => dictionary! {"ca" => 0}},
            b"0 0 1 rg 0 0 120 100 re f /A gs /F gs 1 0 0 rg 1 0 0 RG 6 w 20 20 80 60 re B"
                .to_vec(),
            [0, 0, 255, 255],
            [128, 0, 128, 255],
        ),
    ] {
        let image = rgba(&carrier(resources(states), &ops, Placement::Page, 0));
        pixel(&image, 60, 50, fill);
        pixel(&image, 18, 50, stroke);
    }
}

#[test]
fn graphics_save_restore_restores_alpha_before_outside_redraw() {
    let bytes = carrier(
        resources(dictionary! {"A" => dictionary! {"ca" => 0.5}}),
        b"0 0 1 rg 0 0 120 100 re f 1 0 0 rg q /A gs 10 20 30 60 re f Q 70 20 30 60 re f",
        Placement::Page,
        0,
    );
    let image = rgba(&bytes);
    pixel(&image, 25, 50, [128, 0, 128, 255]);
    pixel(&image, 85, 50, [255, 0, 0, 255]);
}

#[test]
fn page_form_and_normal_appearance_render_all_rotations() {
    for placement in [Placement::Page, Placement::Form, Placement::Appearance] {
        for (rotation, x, y) in [(0, 40, 60), (90, 40, 40), (180, 80, 40), (270, 60, 80)] {
            let bytes = carrier(
                resources(dictionary! {"A" => dictionary! {"CA" => 0.5, "ca" => 0.5}}),
                PAINT,
                placement,
                rotation,
            );
            let image = rgba(&bytes);
            pixel(&image, x, y, [128, 0, 128, 255]);
            let dimensions = if rotation == 90 || rotation == 270 {
                (100, 120)
            } else {
                (120, 100)
            };
            assert_eq!(image.dimensions(), dimensions, "{placement:?}/{rotation}");
        }
    }
}

#[test]
fn indirect_category_entry_type_and_numeric_scalars_are_rendered() {
    let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
    let page_id = doc.get_pages()[&1];
    let t = doc.add_object(Object::Name(b"ExtGState".to_vec()));
    let ca = doc.add_object(Object::Real(0.5));
    let entry = doc.add_object(dictionary! {"Type" => t, "CA" => ca, "ca" => ca});
    let category = doc.add_object(dictionary! {"A" => entry});
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", dictionary! {"ExtGState" => category});
    pixel(&rgba(&save(doc)), 60, 50, [128, 0, 128, 255]);
}

#[test]
fn each_one_hop_field_preserves_exact_alpha_pixels_in_each_scope() {
    for placement in [Placement::Page, Placement::Form, Placement::Appearance] {
        for field in ["category", "entry", "Type", "CA", "ca"] {
            for (alpha, expected) in [
                (0., [0, 0, 255, 255]),
                (0.5, [128, 0, 128, 255]),
                (1., [255, 0, 0, 255]),
            ] {
                let image = rgba(&indirect_field_pdf(field, 1, placement, alpha));
                pixel(&image, 60, 50, expected);
                pixel(&image, 18, 50, expected);
            }
        }
    }
}

#[test]
fn each_two_hop_field_is_refused_in_each_scope() {
    for placement in [Placement::Page, Placement::Form, Placement::Appearance] {
        for field in ["category", "entry", "Type", "CA", "ca"] {
            rejected(
                &indirect_field_pdf(field, 2, placement, 0.5),
                "ExtGState: unsupported reference depth",
            );
        }
    }
}

#[test]
fn empty_category_and_parameter_dictionary_are_valid_noops() {
    pixel(
        &rgba(&carrier(
            resources(dictionary! {}),
            b"0 0 1 rg 0 0 120 100 re f",
            Placement::Page,
            0,
        )),
        60,
        50,
        [0, 0, 255, 255],
    );
    pixel(
        &rgba(&carrier(
            resources(dictionary! {"A" => dictionary! {}}),
            PAINT,
            Placement::Page,
            0,
        )),
        60,
        50,
        [255, 0, 0, 255],
    );
}

#[test]
fn malformed_categories_entries_types_and_scalars_are_rejected() {
    let wrong = [
        Object::Null,
        Object::Integer(3),
        Object::Name(b"Bad".to_vec()),
        Object::string_literal("0.5"),
        Object::Array(vec![]),
    ];
    for value in &wrong {
        rejected(
            &carrier(
                dictionary! {"ExtGState" => value.clone()}.into(),
                b"",
                Placement::Page,
                0,
            ),
            "ExtGState",
        );
        rejected(
            &carrier(
                resources(dictionary! {"Unused" => value.clone()}),
                b"",
                Placement::Page,
                0,
            ),
            "ExtGState",
        );
    }
    // Streams must be indirect: inline streams would be malformed PDF syntax,
    // which tests the document parser instead of the ExtGState auditor.
    for entry in [false, true] {
        let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
        let page_id = doc.get_pages()[&1];
        let stream = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        let category = if entry {
            dictionary! {"A" => stream}.into()
        } else {
            Object::Reference(stream)
        };
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", dictionary! {"ExtGState" => category});
        rejected(&save(doc), "ExtGState");
    }
    for value in [
        Object::Name(b"Other".to_vec()),
        Object::Integer(1),
        Object::Null,
    ] {
        rejected(
            &carrier(
                resources(dictionary! {"A" => dictionary! {"Type" => value}}),
                PAINT,
                Placement::Page,
                0,
            ),
            "ExtGState",
        );
    }
    for key in ["CA", "ca"] {
        for value in [
            Object::Null,
            Object::string_literal("0.5"),
            Object::Array(vec![0.into()]),
            Object::Name(b"Bad".to_vec()),
            Object::Real(-0.1),
            Object::Real(1.1),
            Object::Integer(-1),
            Object::Integer(2),
        ] {
            let mut entry = dictionary! {};
            entry.set(key, value);
            rejected(
                &carrier(
                    resources(dictionary! {"A" => entry}),
                    PAINT,
                    Placement::Page,
                    0,
                ),
                "ExtGState",
            );
        }
    }
}

#[test]
fn every_unadmitted_parameter_is_rejected_even_when_unused() {
    for key in [
        "BM",
        "SMask",
        "AIS",
        "TK",
        "Font",
        "TR",
        "OP",
        "LW",
        "UnknownExtension",
    ] {
        let mut entry = dictionary! {};
        entry.set(key, Object::Null);
        rejected(
            &carrier(
                resources(dictionary! {"A" => dictionary! {}, "Unused" => entry}),
                PAINT,
                Placement::Page,
                0,
            ),
            "ExtGState",
        );
    }
}

#[test]
fn gs_arity_type_and_missing_name_are_checked_even_inside_actual_text() {
    for ops in [
        "gs",
        "/A /A gs",
        "1 gs",
        "(A) gs",
        "/Missing gs",
        "/Span << /ActualText <FEFF0041> >> BDC /Missing gs EMC",
    ] {
        rejected(
            &carrier(
                resources(dictionary! {"A" => dictionary! {}}),
                ops.as_bytes(),
                Placement::Page,
                0,
            ),
            "gs",
        );
    }
    pixel(&rgba(&carrier(resources(dictionary! {"A" => dictionary! {"ca" => 0.5}}),b"/Span << /ActualText <FEFF0041> >> BDC 0 0 1 rg 0 0 120 100 re f /A gs 1 0 0 rg 20 20 80 60 re f EMC",Placement::Page,0)),60,50,[128,0,128,255]);
}

#[test]
fn page_and_sibling_names_do_not_leak_into_form_or_appearance() {
    for placement in [Placement::Form, Placement::Appearance] {
        for (page_name, sibling_name) in [(true, false), (false, true), (true, true)] {
            let bytes = carrier(resources(dictionary! {}), PAINT, placement, 0);
            let mut doc = Document::load_mem(&bytes).unwrap();
            let page_id = doc.get_pages()[&1];
            let sibling = doc.add_object(Stream::new(dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(),0.into(),120.into(),100.into()],
            "Resources" => resources(dictionary! {"A" => dictionary! {"ca" => 0.5}}),
        }, b"/A gs".to_vec()));
            let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
            let res = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
            if page_name {
                res.set("ExtGState", dictionary! {"A" => dictionary! {"ca" => 0.5}});
            }
            if sibling_name {
                if !res.has(b"XObject") {
                    res.set("XObject", dictionary! {});
                }
                res.get_mut(b"XObject")
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Sibling", sibling);
            }
            rejected(&save(doc.clone()), "ExtGState resource name is not linked");
            // The exact same graph becomes valid by adding only the missing local name.
            for object in doc.objects.values_mut() {
                if let Object::Stream(stream) = object
                    && stream.content == PAINT
                {
                    stream.dict.set(
                        "Resources",
                        resources(dictionary! {"A" => dictionary! {"CA" => 0.5,"ca" => 0.5}}),
                    );
                }
            }
            pixel(&rgba(&save(doc)), 60, 50, [128, 0, 128, 255]);
        }
    }
}

#[test]
fn missing_and_cyclic_indirect_alpha_objects_are_rejected() {
    for cycle in [false, true] {
        for level in ["category", "entry", "scalar"] {
            let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
            let page_id = doc.get_pages()[&1];
            let id = doc.new_object_id();
            if cycle {
                doc.set_object(id, Object::Reference(id));
            }
            let value = Object::Reference(id);
            let category = match level {
                "category" => value,
                "entry" => dictionary! {"A" => value}.into(),
                _ => dictionary! {"A" => dictionary! {"ca" => value}}.into(),
            };
            doc.get_object_mut(page_id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Resources", dictionary! {"ExtGState" => category});
            rejected(&save(doc), "ExtGState");
        }
    }
}

#[test]
fn indirect_alpha_resolution_retains_the_64_step_budget() {
    for hops in [64, 65, 66] {
        let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
        let page_id = doc.get_pages()[&1];
        let mut scalar = doc.add_object(Object::Real(0.5));
        for _ in 1..hops {
            scalar = doc.add_object(Object::Reference(scalar));
        }
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "Resources",
                resources(dictionary! {"A" => dictionary! {"ca" => scalar}}),
            );
        let bytes = save(doc);
        if hops == 64 {
            rejected(&bytes, "ExtGState: unsupported reference depth");
        } else {
            rejected(&bytes, "source reference depth");
        }
    }
}

#[test]
fn two_hop_alpha_scalar_is_explicitly_refused_before_wrong_backend_pixels() {
    let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
    let page_id = doc.get_pages()[&1];
    let scalar = doc.add_object(Object::Real(0.5));
    let alias = doc.add_object(Object::Reference(scalar));
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Resources",
            resources(dictionary! {"A" => dictionary! {"CA" => alias,"ca" => alias}}),
        );
    rejected(&save(doc), "ExtGState: unsupported reference depth");
}

#[test]
fn nonfinite_serialization_reports_reparsed_reality() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let bytes = carrier(
            resources(dictionary! {"A" => dictionary! {"ca" => Object::Real(value)}}),
            PAINT,
            Placement::Page,
            0,
        );
        let doc = Document::load_mem(&bytes).unwrap();
        let page = doc.get_object(doc.get_pages()[&1]);
        eprintln!("serialized {value:?}: reparsed page={page:?}");
        assert!(matches!(page, Err(lopdf::Error::ObjectNotFound(_))));
        let opened = Session::open(&bytes);
        assert!(opened.is_err(), "not a renderable nonfinite-float carrier");
        eprintln!(
            "serialized {value:?}: Session open error={}",
            opened.err().unwrap()
        );
    }
}

#[test]
fn aliases_count_separately_at_the_resource_object_budget() {
    for count in [4096, 4097] {
        let mut doc = Document::load_mem(&alpha_pdf(0.5, 0.5)).unwrap();
        let page_id = doc.get_pages()[&1];
        let entry = doc.add_object(dictionary! {});
        let mut category = dictionary! {};
        for index in 0..count {
            category.set(format!("A{index}"), entry);
        }
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"0 0 1 rg 0 0 120 100 re f /A0 gs".to_vec(),
        ));
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        page.set("Contents", content);
        page.set("Resources", dictionary! {"ExtGState" => category});
        let bytes = save(doc);
        if count == 4096 {
            pixel(&rgba(&bytes), 60, 50, [0, 0, 255, 255]);
        } else {
            rejected(&bytes, "source resource object count");
        }
    }
}
