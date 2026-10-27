use lopdf::{Document, Object, dictionary, text_string};
use pdfmaster_core::{Edit, Session};
use serde_json::json;

fn fixture() -> Vec<u8> {
    let mut doc =
        Document::load_mem(&Session::blank(200., 120., 5).unwrap().bytes().unwrap()).unwrap();
    let ids: Vec<_> = doc.get_pages().into_values().collect();
    for (page, id) in ids.iter().enumerate() {
        let annotation = doc.add_object(dictionary! {
            "Type"=>"Annot", "Subtype"=>"Link", "Rect"=>vec![0.into(),0.into(),20.into(),20.into()],
            "Dest"=>vec![ids[0].into(),"Fit".into()],
            "Contents"=>text_string(&format!("Link-{page}")),
            "P"=>*id,
        });
        doc.get_object_mut(*id).unwrap().as_dict_mut().unwrap().set(
            "MediaBox",
            vec![0.into(), 0.into(), (200 + page as i64).into(), 120.into()],
        );
        doc.get_object_mut(*id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![annotation.into()]);
    }
    // Distinct explicit labels prove labels travel with the surviving page IDs.
    let labels: Vec<Object> = (0..5)
        .flat_map(|page| {
            [
                page.into(),
                dictionary! {"P"=>text_string(&format!("Source-{page}"))}.into(),
            ]
        })
        .collect();
    doc.catalog_mut()
        .unwrap()
        .set("PageLabels", dictionary! {"Nums"=>labels});
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn movement(pages: &[usize], insertion: usize) -> Edit {
    serde_json::from_value(json!({"kind":"move_pages","pages":pages,"insertion":insertion}))
        .expect("the overview move protocol must support a whole selection")
}

#[test]
fn overview_moves_use_source_order_and_original_page_boundaries() {
    let bytes = fixture();
    for (boundary, expected) in [
        (0, [1, 3, 0, 2, 4]),
        (2, [0, 1, 3, 2, 4]),
        (5, [0, 2, 4, 1, 3]),
    ] {
        let mut session = Session::open(&bytes).unwrap();
        session.apply(movement(&[3, 1], boundary)).unwrap();
        let expected_labels: Vec<_> = expected
            .iter()
            .map(|page| format!("Source-{page}"))
            .collect();
        assert_eq!(session.page_labels().unwrap(), expected_labels);
        assert_eq!(
            session
                .info()
                .pages
                .iter()
                .map(|page| page.width as i32)
                .collect::<Vec<_>>(),
            expected.map(|page| 200 + page)
        );
        let after = session.bytes().unwrap();
        assert_ne!(after, bytes);
        let original = Document::load_mem(&bytes).unwrap();
        let reordered = Document::load_mem(&after).unwrap();
        let original_ids: Vec<_> = original.get_pages().into_values().collect();
        let reordered_ids: Vec<_> = reordered.get_pages().into_values().collect();
        assert_eq!(
            reordered_ids,
            expected.map(|page| original_ids[page as usize])
        );
        for id in reordered_ids {
            let original_page = original.get_object(id).unwrap().as_dict().unwrap();
            let page = reordered.get_object(id).unwrap().as_dict().unwrap();
            let original_annotation = original_page.get(b"Annots").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap();
            let annotation = page.get(b"Annots").unwrap().as_array().unwrap()[0]
                .as_reference()
                .unwrap();
            assert_eq!(annotation, original_annotation);
            assert_eq!(
                reordered.get_object(annotation).unwrap(),
                original.get_object(annotation).unwrap()
            );
        }
        assert_eq!(
            Session::open(&after).unwrap().page_labels().unwrap(),
            expected_labels
        );
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), bytes);
        assert!(
            !session.info().can_undo,
            "a selection move is exactly one undo step"
        );
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
    }
}

#[test]
fn every_small_selection_and_original_boundary_matches_a_reference_sequence() {
    let bytes = fixture();
    for mask in 1u32..32 {
        let selected: Vec<usize> = (0..5)
            .rev()
            .filter(|page| mask & (1 << page) != 0)
            .collect();
        for insertion in 0..=5 {
            let mut expected: Vec<usize> = (0..5).filter(|page| !selected.contains(page)).collect();
            let start = insertion - selected.iter().filter(|&&page| page < insertion).count();
            expected.splice(start..start, (0..5).filter(|page| selected.contains(page)));
            let mut session = Session::open(&bytes).unwrap();
            let result = session.apply(movement(&selected, insertion));
            if expected == [0, 1, 2, 3, 4] {
                assert!(result.is_err(), "noop {selected:?} to {insertion}");
                assert_eq!(session.bytes().unwrap(), bytes);
            } else {
                result.unwrap();
                assert_eq!(
                    session.page_labels().unwrap(),
                    expected
                        .iter()
                        .map(|page| format!("Source-{page}"))
                        .collect::<Vec<_>>()
                );
                session.undo().unwrap();
                assert_eq!(session.bytes().unwrap(), bytes);
            }
        }
    }
}

#[test]
fn invalid_or_noop_overview_moves_preserve_bytes_dirty_flag_and_redo() {
    let bytes = fixture();
    for (pages, insertion) in [
        (vec![], 0),
        (vec![1, 1], 0),
        (vec![5], 0),
        (vec![1], 6),
        (vec![1, 2], 1),
        (vec![2, 1], 2),
        (vec![1, 2], 3),
        (vec![4, 3, 2, 1, 0], 5),
    ] {
        let mut session = Session::open(&bytes).unwrap();
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let rotated = session.bytes().unwrap();
        session.undo().unwrap();
        let before_info = session.info();
        assert!(
            session.apply(movement(&pages, insertion)).is_err(),
            "{pages:?} to {insertion}"
        );
        assert_eq!(session.bytes().unwrap(), bytes);
        assert_eq!(session.info(), before_info);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), rotated);
    }
}
