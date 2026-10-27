use lopdf::{Document, Object, StringFormat, dictionary, text_string};
use pdfmaster_core::{Edit, Session};

fn fixture(count: usize, tree: Option<Object>) -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(200., 120., count).unwrap().bytes().unwrap()).unwrap();
    if let Some(tree) = tree {
        doc.catalog_mut().unwrap().set("PageLabels", tree);
    }
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn labeled() -> Vec<u8> {
    fixture(
        4,
        Some(
            dictionary! {"Nums"=>vec![
                0.into(), dictionary!{"S"=>"r"}.into(),
                2.into(), dictionary!{"S"=>"D","St"=>7,"P"=>text_string("正文-")}.into(),
            ]}
            .into(),
        ),
    )
}

#[test]
fn every_structural_edit_preserves_surviving_labels_through_reopen_undo_and_redo() {
    let bytes = labeled();
    for (edit, expected) in [
        (Edit::DeletePage { page: 0 }, vec!["ii", "正文-7", "正文-8"]),
        (
            Edit::DeletePages { pages: vec![3, 0] },
            vec!["ii", "正文-7"],
        ),
        (
            Edit::MovePage { from: 0, to: 3 },
            vec!["ii", "正文-7", "正文-8", "i"],
        ),
        (
            Edit::DuplicatePage { page: 2, index: 0 },
            vec!["正文-7", "i", "ii", "正文-7", "正文-8"],
        ),
        (
            Edit::InsertBlank {
                index: 1,
                width: 160.,
                height: 100.,
            },
            vec!["i", "2", "ii", "正文-7", "正文-8"],
        ),
    ] {
        let mut session = Session::open(&bytes).unwrap();
        session.apply(edit).unwrap();
        assert_eq!(session.page_labels().unwrap(), expected);
        let after = session.bytes().unwrap();
        let reopened = Session::open(&after).unwrap();
        assert_eq!(reopened.page_labels().unwrap(), expected);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), bytes);
        assert_eq!(
            session.page_labels().unwrap(),
            ["i", "ii", "正文-7", "正文-8"]
        );
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
        assert_eq!(session.page_labels().unwrap(), expected);
    }
}

#[test]
fn extraction_retains_selected_labels_in_source_order_without_mutating_source() {
    let bytes = labeled();
    let session = Session::open(&bytes).unwrap();
    let before = session.info();
    let extracted = Session::open(&session.extract_pages(&[3, 1]).unwrap()).unwrap();
    assert_eq!(extracted.page_labels().unwrap(), ["ii", "正文-8"]);
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.info(), before);
}

#[test]
fn insertion_keeps_each_inputs_labels_and_assigns_current_numbers_to_unlabeled_pages() {
    let bytes = labeled();
    let unlabeled = fixture(2, None);
    let mut session = Session::open(&bytes).unwrap();
    session.insert_pdfs(&[&unlabeled, &bytes], 2).unwrap();
    let expected = [
        "i", "ii", "3", "4", "i", "ii", "正文-7", "正文-8", "正文-7", "正文-8",
    ];
    assert_eq!(session.page_labels().unwrap(), expected);
    let after = session.bytes().unwrap();
    assert_eq!(
        Session::open(&after).unwrap().page_labels().unwrap(),
        expected
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), after);
    let mut plain_target = Session::open(&unlabeled).unwrap();
    plain_target.insert_pdf(&bytes, 1).unwrap();
    assert_eq!(
        plain_target.page_labels().unwrap(),
        ["1", "i", "ii", "正文-7", "正文-8", "6"]
    );
}

#[test]
fn multi_file_insertion_numbers_implicit_target_pages_only_after_final_order_is_known() {
    let bytes = fixture(2, None);
    let first = fixture(
        1,
        Some(dictionary! {"Nums"=>vec![0.into(),dictionary!{"P"=>text_string("A")}.into()]}.into()),
    );
    let second = fixture(
        1,
        Some(dictionary! {"Nums"=>vec![0.into(),dictionary!{"P"=>text_string("B")}.into()]}.into()),
    );
    let implicit = fixture(1, None);
    let mut session = Session::open(&bytes).unwrap();
    session
        .insert_pdfs(&[&first, &implicit, &second], 0)
        .unwrap();
    let expected = ["A", "2", "B", "4", "5"];
    assert_eq!(session.page_labels().unwrap(), expected);
    let after = session.bytes().unwrap();
    assert_eq!(
        Session::open(&after).unwrap().page_labels().unwrap(),
        expected
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), bytes);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), after);
}

#[test]
fn documents_without_labels_keep_implicit_physical_numbering_without_inventing_metadata() {
    for tree in [None, Some(Object::Null)] {
        let bytes = fixture(3, tree);
        let mut session = Session::open(&bytes).unwrap();
        session.apply(Edit::MovePage { from: 0, to: 2 }).unwrap();
        session
            .apply(Edit::InsertBlank {
                index: 0,
                width: 100.,
                height: 100.,
            })
            .unwrap();
        session
            .apply(Edit::DuplicatePage { page: 2, index: 4 })
            .unwrap();
        session
            .apply(Edit::DeletePages { pages: vec![0, 3] })
            .unwrap();
        session.insert_pdf(&fixture(1, None), 1).unwrap();
        assert_eq!(session.page_labels().unwrap(), ["1", "2", "3", "4"]);
        let doc = Document::load_mem(&session.bytes().unwrap()).unwrap();
        assert!(!doc.catalog().unwrap().has(b"PageLabels"));
    }
}

#[test]
fn malformed_labels_abort_structure_and_late_batch_without_consuming_redo() {
    let bad = fixture(
        2,
        Some(dictionary! {"Nums"=>vec![0.into(),dictionary!{"S"=>"Unknown"}.into()]}.into()),
    );
    let mut session = Session::open(&bad).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    let info = session.info();
    for edit in [
        Edit::DeletePage { page: 1 },
        Edit::MovePage { from: 0, to: 1 },
        Edit::DuplicatePage { page: 1, index: 0 },
        Edit::InsertBlank {
            index: 1,
            width: 100.,
            height: 100.,
        },
    ] {
        assert!(session.apply(edit).is_err());
        assert_eq!(session.bytes().unwrap(), bad);
        assert_eq!(session.info(), info);
    }
    assert!(session.extract_pages(&[0]).is_err());
    let bytes = labeled();
    let mut target = Session::open(&bytes).unwrap();
    target
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    target.undo().unwrap();
    let before = target.info();
    assert!(target.insert_pdfs(&[&fixture(1, None), &bad], 1).is_err());
    assert_eq!(target.bytes().unwrap(), bytes);
    assert_eq!(target.info(), before);
    target.redo().unwrap();
    assert_eq!(target.info().pages[0].rotation, 90);
}

#[test]
fn unknown_label_extensions_are_readable_but_cannot_be_silently_dropped_by_rewriting() {
    for tree in [
        dictionary! {"Nums"=>vec![0.into(),dictionary!{"S"=>"D","Vendor"=>text_string("keep")}.into()]},
        dictionary! {"Nums"=>vec![0.into(),dictionary!{"S"=>"D"}.into()],"Vendor"=>text_string("keep")},
    ] {
        let bytes = fixture(2, Some(tree.into()));
        let mut session = Session::open(&bytes).unwrap();
        assert_eq!(session.page_labels().unwrap(), ["1", "2"]);
        let before = session.info();
        assert!(session.apply(Edit::DeletePage { page: 1 }).is_err());
        assert!(session.extract_pages(&[1]).is_err());
        assert_eq!(session.bytes().unwrap(), bytes);
        assert_eq!(session.info(), before);
    }
}

#[test]
fn rewriting_does_not_bypass_encoded_prefix_budget_or_destroy_redo_on_failure() {
    let mut raw = vec![0xfe, 0xff];
    for _ in 0..4095 {
        raw.extend_from_slice(&[0, b'x']);
    }
    let prefix = Object::String(raw, StringFormat::Hexadecimal);
    let nums = (0..128)
        .flat_map(|i| [i.into(), dictionary! {"P"=>prefix.clone(),"S"=>"D"}.into()])
        .collect::<Vec<_>>();
    let bytes = fixture(128, Some(dictionary! {"Nums"=>nums}.into()));
    let mut session = Session::open(&bytes).unwrap();
    assert_eq!(session.page_labels().unwrap().len(), 128);
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    let info = session.info();
    assert!(
        session
            .apply(Edit::DuplicatePage { page: 0, index: 1 })
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.info(), info);
}

#[test]
fn indirect_prefixes_and_number_trees_are_detached_without_mutating_original_objects() {
    let mut doc = Document::load_mem(&fixture(4, None)).unwrap();
    let prefix = doc.add_object(text_string("附录-"));
    let style = doc.add_object(Object::Name(b"R".to_vec()));
    let rule = doc.add_object(dictionary! {"P"=>prefix,"S"=>style,"St"=>49});
    let first = doc.add_object(dictionary!{"Nums"=>vec![0.into(),dictionary!{"S"=>"r"}.into()],"Limits"=>vec![0.into(),0.into()]});
    let second = doc.add_object(dictionary!{"Nums"=>vec![2.into(),Object::Reference(rule)],"Limits"=>vec![2.into(),2.into()]});
    let root = doc
        .add_object(dictionary! {"Kids"=>vec![Object::Reference(first),Object::Reference(second)]});
    doc.catalog_mut().unwrap().set("PageLabels", root);
    let original: Vec<_> = [prefix, style, rule, first, second, root]
        .into_iter()
        .map(|id| (id, doc.objects[&id].clone()))
        .collect();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let mut session = Session::open(&bytes).unwrap();
    session.apply(Edit::MovePage { from: 3, to: 0 }).unwrap();
    assert_eq!(
        session.page_labels().unwrap(),
        ["附录-L", "i", "ii", "附录-XLIX"]
    );
    let result = Document::load_mem(&session.bytes().unwrap()).unwrap();
    for (id, object) in original {
        assert_eq!(result.objects[&id], object);
    }
    let extracted = Session::open(&session.extract_pages(&[3, 0]).unwrap()).unwrap();
    assert_eq!(extracted.page_labels().unwrap(), ["附录-L", "附录-XLIX"]);
    let mut target = Session::blank(100., 100., 1).unwrap();
    target.insert_pdf(&session.bytes().unwrap(), 0).unwrap();
    assert_eq!(
        target.page_labels().unwrap(),
        ["附录-L", "i", "ii", "附录-XLIX", "5"]
    );
}

#[test]
fn empty_prefix_only_and_repeated_letter_labels_survive_permutation_and_extraction() {
    let bytes = fixture(
        5,
        Some(
            dictionary! {"Nums"=>vec![
                0.into(),dictionary!{}.into(),
                1.into(),dictionary!{"P"=>text_string("封面")}.into(),
                2.into(),dictionary!{"S"=>"A","St"=>26}.into(),
            ]}
            .into(),
        ),
    );
    let mut session = Session::open(&bytes).unwrap();
    session.apply(Edit::MovePage { from: 4, to: 0 }).unwrap();
    assert_eq!(
        session.page_labels().unwrap(),
        ["BB", "", "封面", "Z", "AA"]
    );
    let output = Session::open(&session.extract_pages(&[4, 0, 1]).unwrap()).unwrap();
    assert_eq!(output.page_labels().unwrap(), ["BB", "", "AA"]);
}
