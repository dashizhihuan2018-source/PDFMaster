use lopdf::{Document, Object, ObjectId, dictionary, text_string};
use pdfmaster_core::{Edit, Session, TextExportFormat as F};

fn fixture(count: usize, build: impl FnOnce(&mut Document, &[ObjectId])) -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(200., 120., count).unwrap().bytes().unwrap()).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    build(&mut doc, &pages);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}
fn annots(doc: &mut Document, page: ObjectId, rows: Vec<Object>) {
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", rows);
}

#[test]
fn text_export_all_annotations_escaping_and_readonly_history() {
    let bytes = fixture(2, |doc, pages| {
        let rows = ["Text", "Link", "Popup", "Widget"]
            .into_iter()
            .map(|kind| {
                dictionary! {
                    "Subtype" => Object::Name(kind.as_bytes().to_vec()),
                    "Contents" => text_string("中文<&>\"'\\*_`[]|\r\n尾"),
                    "T" => text_string("作者")
                }
                .into()
            })
            .chain([dictionary! {}.into()])
            .collect();
        annots(doc, pages[0], rows);
        annots(doc, pages[1], vec![dictionary!{"Subtype" => Object::Name(b"Text".to_vec()), "Contents" => text_string("second")}.into()]);
    });
    let mut session = Session::open(&bytes).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    let before = session.bytes().unwrap();
    let info = session.info();
    let txt = session.export_text(F::SummaryText).unwrap();
    for kind in ["Text", "Link", "Popup", "Widget", "Unknown"] {
        assert!(txt.contains(&format!("Page 1 | {kind} |")));
    }
    assert!(txt.ends_with("Page 2 | Text | second | "));
    let md = session.export_text(F::SummaryMarkdown).unwrap();
    assert!(
        md.contains("中文&lt;&amp;&gt;&quot;&#39;\\\\\\*\\_\\`\\[\\]\\| 尾"),
        "{md}"
    );
    let html = session.export_text(F::SummaryHtml).unwrap();
    assert!(html.starts_with("<!doctype html><html><head><meta charset=\"utf-8\"></head>"));
    assert!(html.contains("<td>作者</td>"));
    assert!(html.contains("&lt;&amp;&gt;&quot;&#39;"));
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
    session.redo().unwrap();
}

#[test]
fn text_export_plain_uses_ordered_reader_and_legitimate_empty_results() {
    let bytes = fixture(2, |doc, pages| {
        let font =
            doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
        for (index, page) in pages.iter().enumerate() {
            let content = doc.add_object(lopdf::Stream::new(
                lopdf::Dictionary::new(),
                format!("BT /F1 12 Tf 10 20 Td (Page {}) Tj ET", index + 1).into_bytes(),
            ));
            let d = doc.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
            d.set("Contents", content);
            d.set("Resources", dictionary! {"Font"=>dictionary!{"F1"=>font}});
        }
    });
    let session = Session::open(&bytes).unwrap();
    let reader = session.text_reader().unwrap();
    let expected = (0..session.info().page_count)
        .map(|page| reader.text(page).unwrap())
        .collect::<Vec<_>>()
        .join("\n\n");
    assert_eq!(session.export_text(F::PlainText).unwrap(), expected);
    assert!(!expected.is_empty());
    let blank = Session::blank(200., 120., 1).unwrap();
    assert_eq!(blank.export_text(F::PlainText).unwrap(), "");
    assert_eq!(blank.export_text(F::SummaryText).unwrap(), "");
    assert_eq!(blank.export_text(F::SummaryMarkdown).unwrap(), "");
    assert!(
        blank
            .export_text(F::SummaryHtml)
            .unwrap()
            .contains("<tbody></tbody>")
    );
}

#[test]
fn text_export_rejects_malformed_and_budget_without_partial_success() {
    for case in 0..7 {
        let bytes = fixture(if case == 6 { 5 } else { 1 }, |doc, pages| {
            let mut row = dictionary! {"Subtype"=>Object::Name(b"Text".to_vec())};
            match case {
                0 => row.set("Contents", Object::Reference((9999, 0))),
                1 => row.set("Contents", 42),
                2 => row.set(
                    "T",
                    Object::String(vec![0xfe, 0xff, 0], lopdf::StringFormat::Hexadecimal),
                ),
                3 => row.set("Subtype", 42),
                4 => row.set("Contents", text_string(&"x".repeat(65537))),
                5 => {}
                _ => {}
            }
            for &page in pages {
                annots(
                    doc,
                    page,
                    vec![
                        row.clone().into();
                        if case >= 5 {
                            4096 + usize::from(case == 5)
                        } else {
                            1
                        }
                    ],
                );
            }
        });
        let session = Session::open(&bytes).unwrap();
        for f in [F::SummaryText, F::SummaryMarkdown, F::SummaryHtml] {
            assert!(session.export_text(f).is_err(), "case {case} {f:?}");
        }
    }
    let bytes = fixture(1, |doc, pages| {
        annots(
            doc,
            pages[0],
            vec![dictionary! {"Contents"=>text_string(&"x".repeat(65536))}.into(); 33],
        )
    });
    assert!(
        Session::open(&bytes)
            .unwrap()
            .export_text(F::SummaryText)
            .is_err()
    );
}

#[test]
fn text_export_pdfdoc_utf16le_cr_normalization_and_ignored_actions() {
    let bytes = fixture(1, |doc, pages| {
        let cyclic = doc.new_object_id();
        doc.set_object(cyclic, Object::Reference(cyclic));
        let mut le = vec![0xff, 0xfe];
        for unit in "作者".encode_utf16() {
            le.extend_from_slice(&unit.to_le_bytes());
        }
        annots(
            doc,
            pages[0],
            vec![
                dictionary! {
                    "Subtype"=>Object::Name(b"Link".to_vec()),
                    "Contents"=>Object::string_literal(b"one\rtwo\r\nthree\nfour\x80".to_vec()),
                    "T"=>Object::String(le,lopdf::StringFormat::Hexadecimal),
                    "A"=>cyclic,"AP"=>cyclic,"FS"=>cyclic
                }
                .into(),
            ],
        );
    });
    let session = Session::open(&bytes).unwrap();
    assert_eq!(
        session.export_text(F::SummaryMarkdown).unwrap(),
        "- Page 1 | Link | one two three four• | 作者"
    );
    assert!(
        session
            .export_text(F::SummaryText)
            .unwrap()
            .contains("one\rtwo\r\nthree\nfour•")
    );
    let boundary = fixture(1, |doc, pages| {
        annots(
            doc,
            pages[0],
            vec![dictionary! {"Contents"=>text_string(&"x".repeat(65536))}.into()],
        )
    });
    assert!(
        Session::open(&boundary)
            .unwrap()
            .export_text(F::SummaryText)
            .is_ok()
    );
}
