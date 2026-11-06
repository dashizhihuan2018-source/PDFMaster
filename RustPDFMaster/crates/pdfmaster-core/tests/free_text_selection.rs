use lopdf::{Document, Object, ObjectId, dictionary, text_string};
use pdfmaster_core::{Color, PdfMasterError, Rect, Session, TextAlignment};
use sha2::{Digest, Sha256};

fn fixture(build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(595., 842., 2).unwrap().bytes().unwrap()).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    build(&mut doc, &pages);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn annotation(da: &str, contents: &str) -> Object {
    dictionary! {
        "Subtype" => "FreeText",
        "Rect" => vec![10.into(), 20.into(), 110.into(), 60.into()],
        "Contents" => text_string(contents),
        "DA" => Object::string_literal(da),
    }
    .into()
}

fn set_annots(doc: &mut Document, page: ObjectId, entries: Vec<Object>) {
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", entries);
}

fn reference_chain(doc: &mut Document, terminal: Object, depth: usize) -> Object {
    (0..depth).fold(terminal, |value, _| doc.add_object(value).into())
}

fn assert_supported_da(da: Vec<u8>) {
    let bytes = fixture(|doc, pages| {
        let mut value = annotation("ignored", "supported");
        value
            .as_dict_mut()
            .unwrap()
            .set("DA", Object::string_literal(da));
        set_annots(doc, pages[0], vec![value]);
    });
    let rows = Session::open(&bytes).unwrap().free_texts(0).unwrap();
    assert_eq!(rows.annotations.len(), 1);
    assert_eq!(rows.unsupported_count, 0);
}

fn assert_unsupported_da(da: Vec<u8>) {
    let bytes = fixture(|doc, pages| {
        let mut value = annotation("ignored", "unsupported");
        value
            .as_dict_mut()
            .unwrap()
            .set("DA", Object::string_literal(da));
        set_annots(doc, pages[0], vec![value]);
    });
    let rows = Session::open(&bytes).unwrap().free_texts(0).unwrap();
    assert!(rows.annotations.is_empty());
    assert_eq!(rows.unsupported_count, 1);
}

#[test]
fn projects_unicode_freetext_and_preserves_exact_session_state() {
    let bytes = include_bytes!("fixtures/unicode-ap/chinese.pdf");
    let session = Session::open(bytes).unwrap();
    let before = session.info();
    let rows = session.free_texts(0).unwrap();
    assert_eq!(rows.annotations.len(), 1);
    let annotation = &rows.annotations[0];
    assert_eq!(annotation.page_index, 0);
    assert_eq!(annotation.annotation_index, 0);
    assert_eq!(annotation.page_id, (3, 0));
    assert_eq!(annotation.object_id, Some((14, 0)));
    assert_eq!(
        annotation.bounds,
        Rect {
            x: 100.,
            y: 600.,
            width: 240.,
            height: 100.,
        }
    );
    assert_eq!(annotation.contents, "中文测试");
    assert_eq!(annotation.style.font_resource, b"f0");
    assert_eq!(annotation.style.font_size, 20.);
    assert_eq!(
        annotation.style.color,
        Color {
            r: 0.,
            g: 0.,
            b: 0.
        }
    );
    assert_eq!(annotation.style.alignment, TextAlignment::Left);
    assert_eq!(annotation.flags, 4);
    assert_eq!(rows.unsupported_count, 0);
    assert_eq!(session.bytes().unwrap(), bytes.as_slice());
    assert_eq!(session.info(), before);
}

#[test]
fn projects_latin_and_astral_unicode_without_normalizing_contents() {
    for (bytes, expected) in [
        (
            include_bytes!("fixtures/unicode-ap/latin.pdf").as_slice(),
            "Café naïve",
        ),
        (
            include_bytes!("fixtures/unicode-ap/astral.pdf").as_slice(),
            "𠮷",
        ),
    ] {
        let session = Session::open(bytes).unwrap();
        let row = session.free_texts(0).unwrap().annotations.remove(0);
        assert_eq!(row.contents, expected);
        assert_eq!(row.style.font_resource, b"f0");
        assert_eq!(row.style.font_size, 20.);
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn projects_sparse_direct_and_indirect_styles_in_actual_annot_slots() {
    let bytes = fixture(|doc, pages| {
        doc.get_object_mut(pages[0])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Rotate", 270);
        let da = doc.add_object(Object::string_literal("/Gray 8 Tf .5 g"));
        let flags = doc.add_object(Object::Integer(37));
        let alignment = doc.add_object(Object::Integer(2));
        let mut indirect = annotation("ignored", " gray \n");
        let dictionary = indirect.as_dict_mut().unwrap();
        dictionary.set("DA", da);
        dictionary.set("F", flags);
        dictionary.set("Q", alignment);
        let indirect = doc.add_object(indirect);

        let mut centered = annotation(".1 .2 .3 rg /Rgb 12 Tf", "rgb");
        centered.as_dict_mut().unwrap().set("Q", 1);
        let plain = annotation("/Black 9 Tf", "black");
        set_annots(
            doc,
            pages[0],
            vec![
                dictionary! {"Subtype"=>"Stamp"}.into(),
                centered,
                indirect.into(),
                plain,
            ],
        );
    });
    let session = Session::open(&bytes).unwrap();
    let rows = session.free_texts(0).unwrap();
    assert_eq!(rows.unsupported_count, 0);
    assert_eq!(
        rows.annotations
            .iter()
            .map(|row| row.annotation_index)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(rows.annotations[0].style.font_resource, b"Rgb");
    assert_eq!(rows.annotations[0].style.font_size, 12.);
    assert_eq!(
        rows.annotations[0].style.color,
        Color {
            r: 0.1,
            g: 0.2,
            b: 0.3
        }
    );
    assert_eq!(rows.annotations[0].style.alignment, TextAlignment::Center);
    assert_eq!(rows.annotations[1].style.font_resource, b"Gray");
    assert_eq!(
        rows.annotations[1].style.color,
        Color {
            r: 0.5,
            g: 0.5,
            b: 0.5
        }
    );
    assert_eq!(rows.annotations[1].style.alignment, TextAlignment::Right);
    assert_eq!(rows.annotations[1].flags, 37);
    assert_eq!(rows.annotations[1].contents, " gray \n");
    assert!(rows.annotations[1].object_id.is_some());
    assert_eq!(
        rows.annotations[2].style.color,
        Color {
            r: 0.,
            g: 0.,
            b: 0.
        }
    );
    assert_eq!(rows.annotations[2].style.alignment, TextAlignment::Left);
}

#[test]
fn counts_only_valid_freetext_outside_the_simple_edit_profile() {
    let bytes = fixture(|doc, pages| {
        let mut entries = vec![dictionary! {"Subtype"=>"Stamp"}.into()];
        entries.push(annotation("/Ok 10 Tf", "supported"));
        let mut unsupported = vec![
            annotation("", "missing-da"),
            annotation("/A 10 Tf /B 11 Tf", "two-fonts"),
            annotation("/A 10 Tf 0 g 1 g", "two-colors"),
            annotation("/A 10 Tf 0 0 0 1 k", "cmyk"),
            annotation("/A 10 Tf q", "unknown-op"),
            annotation("/ 10 Tf", "empty-font"),
        ];
        unsupported[0].as_dict_mut().unwrap().remove(b"DA");
        let mut q = annotation("/A 10 Tf", "bad-q");
        q.as_dict_mut().unwrap().set("Q", 3);
        unsupported.push(q);
        let mut rotate = annotation("/A 10 Tf", "annotation-rotate");
        rotate.as_dict_mut().unwrap().set("Rotate", 90);
        unsupported.push(rotate);
        for key in ["RC", "DS", "CL", "LE", "RD", "IT"] {
            let mut value = annotation("/A 10 Tf", key);
            value.as_dict_mut().unwrap().set(key, Object::Null);
            unsupported.push(value);
        }
        entries.extend(unsupported);
        set_annots(doc, pages[0], entries);
    });
    let rows = Session::open(&bytes).unwrap().free_texts(0).unwrap();
    assert_eq!(rows.annotations.len(), 1);
    assert_eq!(rows.annotations[0].contents, "supported");
    assert_eq!(rows.unsupported_count, 14);
}

#[test]
fn validates_da_and_rotate_before_rejecting_an_unsupported_style_profile() {
    for profile in ["q", "rotate", "rich-text"] {
        for invalid_da in ["oversized", "non-string", "missing", "cycle", "deep"] {
            let bytes = fixture(|doc, pages| {
                let mut value = annotation("/A 10 Tf", "mixed-invalid");
                let dictionary = value.as_dict_mut().unwrap();
                match profile {
                    "q" => dictionary.set("Q", 3),
                    "rotate" => dictionary.set("Rotate", 90),
                    _ => dictionary.set("RC", Object::string_literal("rich text")),
                }
                let da = match invalid_da {
                    "oversized" => Object::string_literal(vec![b' '; 4_097]),
                    "non-string" => Object::Integer(7),
                    "missing" => Object::Reference(doc.new_object_id()),
                    "cycle" => {
                        let id = doc.new_object_id();
                        doc.objects.insert(id, Object::Reference(id));
                        Object::Reference(id)
                    }
                    _ => reference_chain(doc, Object::string_literal("/A 10 Tf"), 65),
                };
                value.as_dict_mut().unwrap().set("DA", da);
                set_annots(doc, pages[0], vec![value]);
            });
            let result = Session::open(&bytes).unwrap().free_texts(0);
            if invalid_da == "oversized" {
                assert!(
                    matches!(result, Err(PdfMasterError::Budget("FreeText DA"))),
                    "{profile}/{invalid_da}: {result:?}"
                );
            } else {
                assert!(result.is_err(), "{profile}/{invalid_da}: {result:?}");
            }
        }
    }

    let malformed_rotate = fixture(|doc, pages| {
        let mut value = annotation("/A 10 Tf", "mixed-invalid");
        value.as_dict_mut().unwrap().set("Q", 3);
        value
            .as_dict_mut()
            .unwrap()
            .set("Rotate", Object::string_literal("90"));
        set_annots(doc, pages[0], vec![value]);
    });
    assert!(
        Session::open(&malformed_rotate)
            .unwrap()
            .free_texts(0)
            .is_err()
    );
}

#[test]
fn validates_freetext_structure_before_an_unsupported_profile_short_circuit() {
    for field in ["Rect", "Contents", "F", "Q", "Rotate"] {
        let bytes = fixture(|doc, pages| {
            let mut value = annotation("/A 10 Tf", "mixed-structure");
            let dictionary = value.as_dict_mut().unwrap();
            dictionary.set("RC", Object::string_literal("rich text"));
            match field {
                "Rect" => dictionary.set("Rect", Object::Null),
                "Contents" => dictionary.set("Contents", Object::Integer(1)),
                "F" => dictionary.set("F", Object::Integer(-1)),
                "Q" => dictionary.set("Q", Object::string_literal("3")),
                _ => dictionary.set("Rotate", Object::string_literal("90")),
            }
            set_annots(doc, pages[0], vec![value]);
        });
        assert!(
            Session::open(&bytes).unwrap().free_texts(0).is_err(),
            "{field}"
        );
    }
}

#[test]
fn rejects_default_appearance_with_an_unconsumed_malformed_suffix() {
    for da in ["/A 10 Tf (unterminated", "/A 10 Tf 7"] {
        let bytes = fixture(|doc, pages| {
            set_annots(doc, pages[0], vec![annotation(da, "strict-da")]);
        });
        let rows = Session::open(&bytes).unwrap().free_texts(0).unwrap();
        assert!(rows.annotations.is_empty(), "{da:?}");
        assert_eq!(rows.unsupported_count, 1, "{da:?}");
    }
}

#[test]
fn da_profile_boundaries_are_exact_and_comments_whitespace_are_legal() {
    for da in [
        b"% comment\n /A 10 Tf \t".to_vec(),
        format!("/{} 10 Tf", "A".repeat(127)).into_bytes(),
        b"/A 1000 Tf".to_vec(),
        b"0 g /A 10 Tf".to_vec(),
        b"1 g /A 10 Tf".to_vec(),
        b"0 1 0 rg /A 10 Tf".to_vec(),
    ] {
        assert_supported_da(da);
    }
    let mut exactly_4096 = b"/A 10 Tf".to_vec();
    exactly_4096.resize(4_096, b' ');
    assert_supported_da(exactly_4096);

    for da in [
        b"0 g".to_vec(),
        b"/A 10 Tf /B 11 Tf".to_vec(),
        b"0 g /A 10 Tf 1 g".to_vec(),
        b"(A) 10 Tf".to_vec(),
        b"/A /Ten Tf".to_vec(),
        b"/A 10 2 Tf".to_vec(),
        b"/A 10 Tf q".to_vec(),
        b"/ 10 Tf".to_vec(),
        format!("/{} 10 Tf", "A".repeat(128)).into_bytes(),
        b"/A 0 Tf".to_vec(),
        b"/A -1 Tf".to_vec(),
        b"/A 1000.1 Tf".to_vec(),
        b"-0.1 g /A 10 Tf".to_vec(),
        b"1.1 g /A 10 Tf".to_vec(),
        b"0 0 1.1 rg /A 10 Tf".to_vec(),
        b"/Gray g /A 10 Tf".to_vec(),
        b"0 1 rg /A 10 Tf".to_vec(),
    ] {
        assert_unsupported_da(da);
    }

    let missing = fixture(|doc, pages| {
        let mut value = annotation("ignored", "missing");
        value.as_dict_mut().unwrap().remove(b"DA");
        set_annots(doc, pages[0], vec![value]);
    });
    let rows = Session::open(&missing).unwrap().free_texts(0).unwrap();
    assert!(rows.annotations.is_empty());
    assert_eq!(rows.unsupported_count, 1);

    let too_large = fixture(|doc, pages| {
        let mut da = b"/A 10 Tf".to_vec();
        da.resize(4_097, b' ');
        let mut value = annotation("ignored", "too-large");
        value
            .as_dict_mut()
            .unwrap()
            .set("DA", Object::string_literal(da));
        set_annots(doc, pages[0], vec![value]);
    });
    assert!(matches!(
        Session::open(&too_large).unwrap().free_texts(0),
        Err(PdfMasterError::Budget("FreeText DA"))
    ));
}

#[test]
fn field_and_annotation_references_enforce_missing_and_depth_boundaries() {
    for depth in [64, 65] {
        for field in ["Subtype", "Rect", "Contents", "F", "Q", "Rotate", "DA"] {
            let bytes = fixture(|doc, pages| {
                let mut value = annotation("/A 10 Tf", "referenced");
                let terminal = match field {
                    "Subtype" => Object::Name(b"FreeText".to_vec()),
                    "Rect" => vec![10.into(), 20.into(), 110.into(), 60.into()].into(),
                    "Contents" => text_string("referenced"),
                    "F" | "Q" | "Rotate" => Object::Integer(0),
                    _ => Object::string_literal("/A 10 Tf"),
                };
                value
                    .as_dict_mut()
                    .unwrap()
                    .set(field, reference_chain(doc, terminal, depth));
                set_annots(doc, pages[0], vec![value]);
            });
            let result = Session::open(&bytes).unwrap().free_texts(0);
            if depth == 64 {
                assert_eq!(result.unwrap().annotations.len(), 1, "{field}");
            } else {
                assert!(result.is_err(), "{field}");
            }
        }

        let bytes = fixture(|doc, pages| {
            let value = reference_chain(doc, annotation("/A 10 Tf", "entry"), depth);
            set_annots(doc, pages[0], vec![value]);
        });
        let result = Session::open(&bytes).unwrap().free_texts(0);
        if depth == 64 {
            assert_eq!(result.unwrap().annotations.len(), 1);
        } else {
            assert!(result.is_err());
        }

        let annots_root = fixture(|doc, pages| {
            let entries = vec![annotation("/A 10 Tf", "Annots root")];
            let root = reference_chain(doc, Object::Array(entries), depth);
            doc.get_object_mut(pages[0])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", root);
        });
        let result = Session::open(&annots_root).unwrap().free_texts(0);
        if depth == 64 {
            assert_eq!(result.unwrap().annotations.len(), 1);
        } else {
            assert!(result.is_err());
        }
    }

    for field in ["Subtype", "Rect", "Contents", "F", "Q", "Rotate", "DA"] {
        let bytes = fixture(|doc, pages| {
            let mut value = annotation("/A 10 Tf", "missing");
            value
                .as_dict_mut()
                .unwrap()
                .set(field, Object::Reference(doc.new_object_id()));
            set_annots(doc, pages[0], vec![value]);
        });
        assert!(
            Session::open(&bytes).unwrap().free_texts(0).is_err(),
            "{field}"
        );
    }
    let missing_entry = fixture(|doc, pages| {
        let missing = doc.new_object_id();
        set_annots(doc, pages[0], vec![Object::Reference(missing)]);
    });
    assert!(
        Session::open(&missing_entry)
            .unwrap()
            .free_texts(0)
            .is_err()
    );
    let missing_annots = fixture(|doc, pages| {
        let missing = doc.new_object_id();
        doc.get_object_mut(pages[0])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", missing);
    });
    assert!(
        Session::open(&missing_annots)
            .unwrap()
            .free_texts(0)
            .is_err()
    );
}

#[test]
fn malformed_utf16be_and_utf16le_contents_fail_closed() {
    for contents in [
        vec![0xfe, 0xff, 0x00],
        vec![0xfe, 0xff, 0xd8, 0x00],
        vec![0xff, 0xfe, 0x00],
        vec![0xff, 0xfe, 0x00, 0xd8],
    ] {
        let bytes = fixture(|doc, pages| {
            let mut value = annotation("/A 10 Tf", "ignored");
            value.as_dict_mut().unwrap().set(
                "Contents",
                Object::String(contents, lopdf::StringFormat::Hexadecimal),
            );
            set_annots(doc, pages[0], vec![value]);
        });
        assert!(Session::open(&bytes).unwrap().free_texts(0).is_err());
    }
}

#[test]
fn revision_is_the_exact_sha256_of_current_bytes_and_covers_hidden_changes() {
    let bytes = fixture(|doc, pages| {
        set_annots(
            doc,
            pages[0],
            vec![annotation("/A 10 Tf", "one"), annotation("/B 11 Tf", "two")],
        );
    });
    let first = Session::open(&bytes).unwrap();
    let rows = first.free_texts(0).unwrap().annotations;
    let expected: [u8; 32] = Sha256::digest(first.bytes().unwrap()).into();
    assert_eq!(rows[0].revision, expected);
    assert_eq!(rows[1].revision, expected);
    assert_eq!(
        Session::open(&bytes)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations[0]
            .revision,
        expected
    );

    let changed = fixture(|doc, pages| {
        set_annots(doc, pages[0], vec![annotation("/A 10 Tf", "one")]);
        doc.trailer
            .set("UnknownMetadata", Object::string_literal("hidden"));
    });
    assert_ne!(
        Session::open(&changed)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations[0]
            .revision,
        expected
    );
}

#[test]
fn malformed_structure_and_references_fail_closed_while_unrelated_types_are_skipped() {
    for case in [
        "entry", "subtype", "rect", "contents", "flags", "q", "rotate", "cycle",
    ] {
        let bytes = fixture(|doc, pages| {
            let mut value = annotation("/A 10 Tf", "ok");
            match case {
                "entry" => value = Object::Integer(7),
                "subtype" => {
                    value.as_dict_mut().unwrap().remove(b"Subtype");
                }
                "rect" => value.as_dict_mut().unwrap().set("Rect", Object::Null),
                "contents" => value
                    .as_dict_mut()
                    .unwrap()
                    .set("Contents", Object::Integer(1)),
                "flags" => value.as_dict_mut().unwrap().set("F", -1),
                "q" => value
                    .as_dict_mut()
                    .unwrap()
                    .set("Q", Object::string_literal("1")),
                "rotate" => value
                    .as_dict_mut()
                    .unwrap()
                    .set("Rotate", Object::string_literal("0")),
                _ => {
                    let id = doc.new_object_id();
                    doc.objects.insert(id, Object::Reference(id));
                    value.as_dict_mut().unwrap().set("DA", id);
                }
            }
            set_annots(doc, pages[0], vec![value]);
        });
        assert!(
            Session::open(&bytes).unwrap().free_texts(0).is_err(),
            "{case}"
        );
    }

    let bytes = fixture(|doc, pages| {
        set_annots(
            doc,
            pages[0],
            vec![dictionary! {"Subtype"=>"Stamp", "Rect"=>Object::Null}.into()],
        )
    });
    assert_eq!(
        Session::open(&bytes)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .len(),
        0
    );
}

#[test]
fn enforces_per_source_decoded_total_da_and_annotation_budgets() {
    let source_too_large = fixture(|doc, pages| {
        let mut value = annotation("/A 10 Tf", "ok");
        value
            .as_dict_mut()
            .unwrap()
            .set("Contents", Object::string_literal(vec![b'a'; 131_076]));
        set_annots(doc, pages[0], vec![value]);
    });
    assert!(matches!(
        Session::open(&source_too_large).unwrap().free_texts(0),
        Err(PdfMasterError::Budget(_))
    ));

    let decoded_too_large = fixture(|doc, pages| {
        let mut value = annotation("/A 10 Tf", "ok");
        value
            .as_dict_mut()
            .unwrap()
            .set("Contents", Object::string_literal(vec![b'a'; 65_537]));
        set_annots(doc, pages[0], vec![value]);
    });
    assert!(matches!(
        Session::open(&decoded_too_large).unwrap().free_texts(0),
        Err(PdfMasterError::Budget(_))
    ));

    let da_too_large = fixture(|doc, pages| {
        let mut value = annotation("/A 10 Tf", "ok");
        let mut da = vec![b' '; 4_097];
        da.extend_from_slice(b"/A 10 Tf");
        value
            .as_dict_mut()
            .unwrap()
            .set("DA", Object::string_literal(da));
        set_annots(doc, pages[0], vec![value]);
    });
    assert!(matches!(
        Session::open(&da_too_large).unwrap().free_texts(0),
        Err(PdfMasterError::Budget(_))
    ));

    let total_too_large = fixture(|doc, pages| {
        let entries = (0..1_024)
            .map(|_| annotation("/A 10 Tf", &"x".repeat(1_025)))
            .collect();
        set_annots(doc, pages[0], entries);
    });
    assert!(matches!(
        Session::open(&total_too_large).unwrap().free_texts(0),
        Err(PdfMasterError::Budget(_))
    ));

    let too_many = fixture(|doc, pages| {
        set_annots(
            doc,
            pages[0],
            (0..1_025)
                .map(|_| dictionary! {"Subtype"=>"Stamp"}.into())
                .collect(),
        );
    });
    assert!(matches!(
        Session::open(&too_many).unwrap().free_texts(0),
        Err(PdfMasterError::Budget(_))
    ));
}

#[test]
fn page_quarter_turns_indirect_annots_and_utf16le_keep_source_coordinates() {
    for rotation in [0, 90, 180, 270] {
        let bytes = fixture(|doc, pages| {
            doc.get_object_mut(pages[0])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Rotate", rotation);
            let mut value = annotation("/A 10 Tf", "ignored");
            value.as_dict_mut().unwrap().set(
                "Contents",
                Object::String(
                    vec![0xff, 0xfe, b'A', 0, b'\n', 0, b'B', 0],
                    lopdf::StringFormat::Hexadecimal,
                ),
            );
            let annotation = doc.add_object(value);
            let annots = doc.add_object(vec![annotation.into()]);
            doc.get_object_mut(pages[0])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", annots);
        });
        let row = Session::open(&bytes)
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .remove(0);
        assert_eq!(row.contents, "A\nB");
        assert_eq!(
            row.bounds,
            Rect {
                x: 10.,
                y: 20.,
                width: 100.,
                height: 40.
            }
        );
    }
}
