use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Color, Edit, PageRange, PageRangeError, PdfMasterError, Rect, Session};
use serde_json::json;

fn assert_unchanged(session: &Session, bytes: &[u8], info: &pdfmaster_core::DocumentInfo) {
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(&session.info(), info);
}

fn with_page_entry(bytes: &[u8], page: usize, key: &str, value: Object) -> Vec<u8> {
    let mut document = Document::load_mem(bytes).unwrap();
    let page_id = document.get_pages()[&((page + 1) as u32)];
    document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(key, value);
    let mut output = Vec::new();
    document.save_to(&mut output).unwrap();
    output
}

fn with_acro_form(bytes: &[u8]) -> Vec<u8> {
    let mut document = Document::load_mem(bytes).unwrap();
    let form = document.add_object(dictionary! {
        "Fields" => Vec::<Object>::new(),
    });
    document.catalog_mut().unwrap().set("AcroForm", form);
    let mut output = Vec::new();
    document.save_to(&mut output).unwrap();
    output
}

#[test]
fn page_range_is_one_based_sorted_and_unique() {
    assert_eq!(
        PageRange::parse(" 1, 3-5, 5, 7 ", 7).unwrap(),
        vec![0, 2, 3, 4, 6]
    );
    assert_eq!(PageRange::parse("3-1, 2", 3).unwrap(), vec![0, 1, 2]);
    assert_eq!(PageRange::parse("+2, 1-+3", 3).unwrap(), vec![0, 1, 2]);
    assert_eq!(PageRange::parse("1-4096,4096", 4096).unwrap().len(), 4096);
}

#[test]
fn page_range_rejects_each_invalid_class_explicitly() {
    assert_eq!(PageRange::parse("", 3), Err(PageRangeError::Empty));
    assert_eq!(PageRange::parse(" \n\t", 3), Err(PageRangeError::Empty));
    assert_eq!(
        PageRange::parse("1", 0),
        Err(PageRangeError::InvalidPageCount)
    );
    assert_eq!(PageRange::parse("0", 3), Err(PageRangeError::Zero));
    assert_eq!(PageRange::parse("+0", 3), Err(PageRangeError::Zero));
    assert_eq!(
        PageRange::parse("4", 3),
        Err(PageRangeError::OutOfBounds {
            page: 4,
            page_count: 3,
        })
    );
    for malformed in ["1,,2", "1-x", "2-", "-2", "1--2", "1-2-3", "1 2", "++1"] {
        assert_eq!(
            PageRange::parse(malformed, 3),
            Err(PageRangeError::Malformed),
            "{malformed}"
        );
    }
    let overflow = format!("{}0", usize::MAX);
    assert_eq!(
        PageRange::parse(&overflow, usize::MAX),
        Err(PageRangeError::Overflow)
    );
    assert_eq!(
        PageRange::parse(&format!("+{overflow}"), usize::MAX),
        Err(PageRangeError::Overflow)
    );
    let oversized = "1".repeat(PageRange::MAX_INPUT_BYTES + 1);
    assert_eq!(
        PageRange::parse(&oversized, 3),
        Err(PageRangeError::InputBudget {
            max_bytes: PageRange::MAX_INPUT_BYTES,
        })
    );
    assert_eq!(
        PageRange::parse("1-4097", 4097),
        Err(PageRangeError::SelectionBudget { max_pages: 4096 })
    );
}

#[test]
fn batch_edit_serde_tags_are_stable() {
    assert_eq!(
        serde_json::to_value(Edit::RotatePages {
            pages: vec![0, 2],
            degrees: 90,
        })
        .unwrap(),
        json!({"kind":"rotate_pages","pages":[0,2],"degrees":90})
    );
    assert_eq!(
        serde_json::to_value(Edit::DeletePages { pages: vec![1, 3] }).unwrap(),
        json!({"kind":"delete_pages","pages":[1,3]})
    );
}

#[test]
fn invalid_batches_preserve_bytes_info_and_redo_history() {
    let mut session = Session::blank(612.0, 792.0, 3).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    assert!(before_info.can_redo);

    for edit in [
        Edit::RotatePages {
            pages: vec![],
            degrees: 90,
        },
        Edit::RotatePages {
            pages: vec![0, 0],
            degrees: 90,
        },
        Edit::RotatePages {
            pages: vec![0, 3],
            degrees: 90,
        },
        Edit::RotatePages {
            pages: vec![0, 1],
            degrees: 45,
        },
        Edit::RotatePages {
            pages: vec![0, 1],
            degrees: 360,
        },
        Edit::DeletePages { pages: vec![] },
        Edit::DeletePages { pages: vec![1, 1] },
        Edit::DeletePages { pages: vec![0, 3] },
        Edit::DeletePages {
            pages: vec![0, 1, 2],
        },
    ] {
        assert!(session.apply(edit).is_err());
        assert_unchanged(&session, &before_bytes, &before_info);
    }
}

#[test]
fn oversized_batch_is_rejected_before_untrusted_capacity_is_allocated() {
    let mut session = Session::blank(612.0, 792.0, 3).unwrap();
    let before_bytes = session.bytes().unwrap();
    let before_info = session.info();
    assert!(matches!(
        session.apply(Edit::RotatePages {
            pages: vec![0; 4097],
            degrees: 90,
        }),
        Err(PdfMasterError::Budget("page batch length"))
    ));
    assert_unchanged(&session, &before_bytes, &before_info);
}

#[test]
fn batch_rotation_is_one_transaction_and_extreme_angles_do_not_overflow() {
    const LARGE_POSITIVE_90: i32 = 2_147_483_610;
    const LARGE_NEGATIVE_90: i32 = -2_147_483_610;

    let mut single = Session::blank(612.0, 792.0, 1).unwrap();
    single
        .apply(Edit::Rotate {
            page: 0,
            degrees: 270,
        })
        .unwrap();
    single
        .apply(Edit::Rotate {
            page: 0,
            degrees: LARGE_POSITIVE_90,
        })
        .unwrap();
    assert_eq!(single.info().pages[0].rotation, 0);

    let mut negative = Session::blank(612.0, 792.0, 1).unwrap();
    negative
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    negative
        .apply(Edit::Rotate {
            page: 0,
            degrees: LARGE_NEGATIVE_90,
        })
        .unwrap();
    assert_eq!(negative.info().pages[0].rotation, 0);

    let mut prepared = Session::blank(612.0, 792.0, 4).unwrap();
    for page in [0, 2] {
        prepared.apply(Edit::Rotate { page, degrees: 270 }).unwrap();
    }
    let baseline = prepared.bytes().unwrap();
    let mut session = Session::open(&baseline).unwrap();
    let original = session.bytes().unwrap();
    session
        .apply(Edit::RotatePages {
            pages: vec![2, 0],
            degrees: LARGE_POSITIVE_90,
        })
        .unwrap();
    assert_eq!(
        session
            .info()
            .pages
            .iter()
            .map(|page| page.rotation)
            .collect::<Vec<_>>(),
        vec![0, 0, 0, 0]
    );

    let rotated = session.bytes().unwrap();
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), original);
    assert!(matches!(session.undo(), Err(PdfMasterError::NoUndo)));
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), rotated);

    let reopened = Session::open(&rotated).unwrap();
    assert_eq!(reopened.info().page_count, 4);
    assert_eq!(reopened.info().pages[0].rotation, 0);
    assert_eq!(reopened.info().pages[2].rotation, 0);
}

#[test]
fn batch_delete_uses_original_indices_and_is_one_transaction() {
    let mut prepared = Session::blank(612.0, 792.0, 4).unwrap();
    prepared
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    prepared
        .apply(Edit::Rotate {
            page: 1,
            degrees: 180,
        })
        .unwrap();
    prepared
        .apply(Edit::Rotate {
            page: 2,
            degrees: 270,
        })
        .unwrap();
    let baseline = prepared.bytes().unwrap();
    let mut session = Session::open(&baseline).unwrap();

    session
        .apply(Edit::DeletePages { pages: vec![0, 2] })
        .unwrap();
    assert_eq!(session.info().page_count, 2);
    assert_eq!(
        session
            .info()
            .pages
            .iter()
            .map(|page| page.rotation)
            .collect::<Vec<_>>(),
        vec![180, 0]
    );
    let deleted = session.bytes().unwrap();

    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), baseline);
    assert!(matches!(session.undo(), Err(PdfMasterError::NoUndo)));
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), deleted);

    let reopened = Session::open(&deleted).unwrap();
    assert_eq!(reopened.info().page_count, 2);
    assert_eq!(reopened.info().pages[0].rotation, 180);
    assert_eq!(reopened.info().pages[1].rotation, 0);
}

#[test]
fn batch_delete_preserves_annotation_structure_and_acroform_guards() {
    let mut annotated = Session::blank(612.0, 792.0, 3).unwrap();
    annotated
        .apply(Edit::AddNote {
            page: 1,
            bounds: Rect {
                x: 20.0,
                y: 20.0,
                width: 30.0,
                height: 30.0,
            },
            contents: "keep".into(),
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 0.0,
            },
        })
        .unwrap();
    let before_bytes = annotated.bytes().unwrap();
    let before_info = annotated.info();
    assert!(
        annotated
            .apply(Edit::DeletePages { pages: vec![0, 1] })
            .is_err()
    );
    assert_unchanged(&annotated, &before_bytes, &before_info);

    let base = Session::blank(612.0, 792.0, 3).unwrap().bytes().unwrap();
    let structured_bytes = with_page_entry(&base, 1, "StructParents", 0_i64.into());
    let mut structured = Session::open(&structured_bytes).unwrap();
    let before_bytes = structured.bytes().unwrap();
    let before_info = structured.info();
    assert!(
        structured
            .apply(Edit::DeletePages { pages: vec![0, 1] })
            .is_err()
    );
    assert_unchanged(&structured, &before_bytes, &before_info);

    let form_bytes = with_acro_form(&base);
    let mut form = Session::open(&form_bytes).unwrap();
    let before_bytes = form.bytes().unwrap();
    let before_info = form.info();
    assert!(form.apply(Edit::DeletePages { pages: vec![0] }).is_err());
    assert_unchanged(&form, &before_bytes, &before_info);
}
