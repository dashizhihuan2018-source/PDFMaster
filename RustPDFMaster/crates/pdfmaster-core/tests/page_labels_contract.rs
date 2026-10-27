use lopdf::{Document, Object, dictionary, text_string};
use pdfmaster_core::{PdfMasterError, Session};

fn fixture(pages: usize, build: impl FnOnce(&mut Document) -> Option<Object>) -> Vec<u8> {
    let mut document =
        Document::load_mem(&Session::blank(200., 120., pages).unwrap().bytes().unwrap()).unwrap();
    if let Some(labels) = build(&mut document) {
        document.catalog_mut().unwrap().set("PageLabels", labels);
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn labels(pages: usize, nums: Vec<Object>) -> Vec<u8> {
    fixture(pages, |_| Some(dictionary! {"Nums"=>nums}.into()))
}

fn read_only(bytes: &[u8]) -> Vec<String> {
    let session = Session::open(bytes).unwrap();
    let before = session.info();
    let result = session.page_labels().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.info(), before);
    result
}

#[test]
fn absent_and_null_labels_use_physical_page_numbers_without_editing() {
    for tree in [None, Some(Object::Null)] {
        assert_eq!(read_only(&fixture(3, |_| tree)), ["1", "2", "3"]);
    }
}

#[test]
fn range_prefixes_unicode_and_all_five_numbering_styles_follow_the_spec() {
    let bytes = labels(
        10,
        vec![
            0.into(),
            dictionary! {"S"=>"r"}.into(),
            2.into(),
            dictionary! {"S"=>"D","St"=>7,"P"=>text_string("正文-")}.into(),
            4.into(),
            dictionary! {"S"=>"R","St"=>49}.into(),
            6.into(),
            dictionary! {"S"=>"A","St"=>26}.into(),
            8.into(),
            dictionary! {"S"=>"a","St"=>52}.into(),
        ],
    );
    assert_eq!(
        read_only(&bytes),
        [
            "i", "ii", "正文-7", "正文-8", "XLIX", "L", "Z", "AA", "zz", "aaa"
        ]
    );
}

#[test]
fn prefix_only_and_empty_labels_do_not_invent_a_numbering_style() {
    let bytes = labels(
        4,
        vec![
            0.into(),
            dictionary! {"P"=>text_string("封面  ")}.into(),
            2.into(),
            dictionary! {}.into(),
        ],
    );
    assert_eq!(read_only(&bytes), ["封面  ", "封面  ", "", ""]);
}

#[test]
fn nested_number_trees_and_indirect_fields_are_resolved_in_order() {
    let bytes = fixture(4, |document| {
        let style = document.add_object(Object::Name(b"D".to_vec()));
        let prefix = document.add_object(text_string("附录-"));
        let start = document.add_object(11);
        let label = document
            .add_object(dictionary! {"Type"=>"PageLabel","S"=>style,"P"=>prefix,"St"=>start});
        let one = document.add_object(dictionary! {"Nums"=>vec![0.into(),dictionary!{"S"=>"r"}.into()],"Limits"=>vec![0.into(),0.into()]});
        let two = document.add_object(dictionary! {"Nums"=>vec![2.into(),Object::Reference(label)],"Limits"=>vec![2.into(),2.into()]});
        let middle = document.add_object(
            dictionary! {"Kids"=>vec![Object::Reference(two)],"Limits"=>vec![2.into(),2.into()]},
        );
        Some(Object::Reference(document.add_object(
            dictionary! {"Kids"=>vec![Object::Reference(one),Object::Reference(middle)]},
        )))
    });
    assert_eq!(read_only(&bytes), ["i", "ii", "附录-11", "附录-12"]);
}

#[test]
fn malformed_and_ambiguous_tree_shapes_fail_without_mutating() {
    for tree in [
        Object::Null,
        dictionary! {}.into(),
        dictionary! {"Nums"=>vec![0.into()]}.into(),
        dictionary! {"Nums"=>vec![1.into(),dictionary!{"S"=>"D"}.into()]}.into(),
        dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into(),0.into(),dictionary!{}.into()]}.into(),
        dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into(),2.into(),dictionary!{}.into(),1.into(),dictionary!{}.into()]}.into(),
        dictionary! {"Nums"=>vec![(-1).into(),dictionary!{}.into()]}.into(),
            dictionary! {"Nums"=>vec![Object::Real(0.5),dictionary!{}.into()]}.into(),
        dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into()],"Kids"=>Vec::<Object>::new()}.into(),
        dictionary! {"Kids"=>Vec::<Object>::new()}.into(),
    ].into_iter().skip(1) {
        let bytes = fixture(3, |_| Some(tree));
        let session = Session::open(&bytes).unwrap();
        let before = session.info();
        assert!(session.page_labels().is_err());
        assert_eq!(session.bytes().unwrap(), bytes);
        assert_eq!(session.info(), before);
    }
}

#[test]
fn invalid_label_dictionary_fields_fail_instead_of_fabricating_labels() {
    for label in [
        dictionary! {"S"=>"Unknown"},
        dictionary! {"S"=>7},
        dictionary! {"St"=>0},
        dictionary! {"St"=>-1},
        dictionary! {"St"=>Object::Real(1.5)},
        dictionary! {"P"=>"NameIsNotText"},
        dictionary! {"Type"=>"NotPageLabel"},
    ] {
        let bytes = labels(1, vec![0.into(), label.into()]);
        assert!(Session::open(&bytes).unwrap().page_labels().is_err());
    }
}

#[test]
fn number_tree_cycles_reference_cycles_and_shared_nodes_fail_bounded() {
    for case in 0..3 {
        let bytes = fixture(2, |document| {
            let node = document.new_object_id();
            let value = match case {
                0 => Object::Reference(node),
                1 => dictionary! {"Kids"=>vec![Object::Reference(node)]}.into(),
                _ => {
                    let leaf = document.add_object(dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into()],"Limits"=>vec![0.into(),0.into()]});
                    dictionary! {"Kids"=>vec![Object::Reference(leaf),Object::Reference(leaf)]}
                        .into()
                }
            };
            document.set_object(node, value);
            Some(Object::Reference(node))
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.page_labels().is_err(), "case {case}");
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn false_limits_do_not_hide_malformed_or_overlapping_ranges() {
    for bounds in [
        vec![0.into()],
        vec![1.into(), 1.into()],
        vec![1.into(), 0.into()],
    ] {
        let bytes = fixture(2, |document| {
            let leaf = document.add_object(
                dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into()],"Limits"=>bounds},
            );
            Some(dictionary! {"Kids"=>vec![Object::Reference(leaf)]}.into())
        });
        assert!(Session::open(&bytes).unwrap().page_labels().is_err());
    }
}

#[test]
fn very_large_starts_cannot_expand_roman_or_letter_strings_unbounded() {
    for style in ["R", "r", "A", "a"] {
        let bytes = labels(
            1,
            vec![0.into(), dictionary! {"S"=>style,"St"=>i64::MAX}.into()],
        );
        assert!(matches!(
            Session::open(&bytes).unwrap().page_labels(),
            Err(PdfMasterError::Budget(_))
        ));
    }
    let bytes = labels(
        2,
        vec![0.into(), dictionary! {"S"=>"D","St"=>i64::MAX}.into()],
    );
    assert_eq!(
        read_only(&bytes),
        [i64::MAX.to_string(), (i64::MAX as u64 + 1).to_string()]
    );
}

#[test]
fn exact_prefix_and_total_output_budgets_are_enforced_before_expansion() {
    let prefix = "x".repeat(8192);
    assert_eq!(
        read_only(&labels(
            128,
            vec![0.into(), dictionary! {"P"=>text_string(&prefix)}.into()]
        ))
        .len(),
        128
    );
    for bytes in [
        labels(
            129,
            vec![0.into(), dictionary! {"P"=>text_string(&prefix)}.into()],
        ),
        labels(
            1,
            vec![
                0.into(),
                dictionary! {"P"=>text_string(&format!("{prefix}x"))}.into(),
            ],
        ),
        labels(
            1,
            vec![
                0.into(),
                dictionary! {"P"=>text_string(&prefix),"S"=>"D"}.into(),
            ],
        ),
    ] {
        assert!(matches!(
            Session::open(&bytes).unwrap().page_labels(),
            Err(PdfMasterError::Budget(_))
        ));
    }
}

#[test]
fn tree_depth_and_child_queue_are_bounded() {
    for depth in [64, 65] {
        let bytes = fixture(1, |document| {
            let mut node=document.add_object(dictionary!{"Nums"=>vec![0.into(),dictionary!{}.into()],"Limits"=>vec![0.into(),0.into()]});
            for _ in 0..depth {
                node=document.add_object(dictionary!{"Kids"=>vec![Object::Reference(node)],"Limits"=>vec![0.into(),0.into()]});
            }
            Some(Object::Reference(node))
        });
        let result = Session::open(&bytes).unwrap().page_labels();
        if depth == 64 {
            assert_eq!(result.unwrap(), [""]);
        } else {
            assert!(matches!(result, Err(PdfMasterError::Budget(_))));
        }
    }
    let bytes = fixture(1, |document| {
        let child=document.add_object(dictionary!{"Nums"=>vec![0.into(),dictionary!{}.into()],"Limits"=>vec![0.into(),0.into()]});
        Some(dictionary! {"Kids"=>vec![Object::Reference(child);4097]}.into())
    });
    assert!(matches!(
        Session::open(&bytes).unwrap().page_labels(),
        Err(PdfMasterError::Budget("page label children"))
    ));
}

#[test]
fn shared_prefix_payloads_still_charge_each_range_to_the_raw_budget() {
    for count in [128, 129] {
        let bytes = fixture(count, |document| {
            let rule = document.add_object(dictionary! {"P"=>text_string(&"x".repeat(8192))});
            let pairs = (0..count)
                .flat_map(|i| [Object::Integer(i as i64), Object::Reference(rule)])
                .collect::<Vec<_>>();
            Some(dictionary! {"Nums"=>pairs}.into())
        });
        let result = Session::open(&bytes).unwrap().page_labels();
        if count == 128 {
            assert_eq!(result.unwrap().len(), 128);
        } else {
            assert!(matches!(
                result,
                Err(PdfMasterError::Budget("page label prefix bytes"))
            ));
        }
    }
}

#[test]
fn utf16_encoded_prefix_bytes_have_an_independent_cumulative_budget() {
    for count in [128, 129] {
        let bytes = fixture(count, |document| {
            let mut encoded = vec![0xfe, 0xff];
            for _ in 0..4095 {
                encoded.extend_from_slice(&[0, b'x']);
            }
            assert_eq!(encoded.len(), 8192);
            let rule = document.add_object(dictionary! {"P"=>Object::string_literal(encoded)});
            let pairs = (0..count)
                .flat_map(|i| [Object::Integer(i as i64), Object::Reference(rule)])
                .collect::<Vec<_>>();
            Some(dictionary! {"Nums"=>pairs}.into())
        });
        let result = Session::open(&bytes).unwrap().page_labels();
        if count == 128 {
            assert_eq!(result.unwrap().len(), 128);
        } else {
            assert!(matches!(
                result,
                Err(PdfMasterError::Budget("page label prefix bytes"))
            ));
        }
    }
}

#[test]
fn malformed_prefix_encodings_are_not_silently_shortened() {
    for encoded in [
        vec![0xfe, 0xff, 0, 65, 65],
        vec![0xfe, 0xff, 0xd8, 0],
        vec![0xfe, 0xff, 0xdc, 0],
        vec![b'A', 0x7f, b'B'],
        vec![0xef, 0xbb, 0xbf, 0xff],
    ] {
        let bytes = labels(
            1,
            vec![
                0.into(),
                dictionary! {"P"=>Object::string_literal(encoded)}.into(),
            ],
        );
        let session = Session::open(&bytes).unwrap();
        assert!(matches!(
            session.page_labels(),
            Err(PdfMasterError::Malformed(_))
        ));
        assert_eq!(session.bytes().unwrap(), bytes);
    }
    for (encoded, expected) in [
        (vec![b'A', 0x80, b'B'], "A•B"),
        (vec![0xfe, 0xff, 0xd8, 0x3d, 0xde, 0], "😀"),
        (
            [vec![0xef, 0xbb, 0xbf], "中文".as_bytes().to_vec()].concat(),
            "中文",
        ),
    ] {
        assert_eq!(
            read_only(&labels(
                1,
                vec![
                    0.into(),
                    dictionary! {"P"=>Object::string_literal(encoded)}.into()
                ]
            )),
            [expected]
        );
    }
}

#[test]
fn reference_depth_accepts_exactly_sixty_four_hops_and_rejects_sixty_five() {
    for hops in [63, 64, 65] {
        let bytes = fixture(1, |document| {
            let mut node =
                document.add_object(dictionary! {"Nums"=>vec![0.into(),dictionary!{}.into()]});
            for _ in 1..hops {
                node = document.add_object(Object::Reference(node));
            }
            Some(Object::Reference(node))
        });
        let result = Session::open(&bytes).unwrap().page_labels();
        if hops <= 64 {
            assert_eq!(result.unwrap(), [""]);
        } else {
            assert!(matches!(
                result,
                Err(PdfMasterError::Budget("page label reference depth"))
            ));
        }
    }
}

#[test]
fn structural_edits_do_not_claim_to_migrate_catalog_label_ranges() {
    let bytes = labels(
        2,
        vec![
            0.into(),
            dictionary! {"S"=>"D"}.into(),
            1.into(),
            dictionary! {"P"=>text_string("Appendix")}.into(),
        ],
    );
    let mut session = Session::open(&bytes).unwrap();
    assert_eq!(session.page_labels().unwrap(), ["1", "Appendix"]);
    session
        .apply(pdfmaster_core::Edit::DeletePage { page: 1 })
        .unwrap();
    let after = session.bytes().unwrap();
    assert!(matches!(
        session.page_labels(),
        Err(PdfMasterError::Malformed(_))
    ));
    assert_eq!(session.bytes().unwrap(), after);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.page_labels().unwrap(), ["1", "Appendix"]);
}
