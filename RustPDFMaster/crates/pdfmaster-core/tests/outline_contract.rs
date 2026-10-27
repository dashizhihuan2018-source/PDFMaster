use lopdf::{Document, Object, ObjectId, dictionary, text_string};
use pdfmaster_core::{Edit, OutlineItem, Session};

fn fixture(build: impl FnOnce(&mut Document, ObjectId, &[ObjectId])) -> Vec<u8> {
    let blank = Session::blank(200., 120., 2).unwrap().bytes().unwrap();
    let mut doc = Document::load_mem(&blank).unwrap();
    let pages = doc.get_pages().values().copied().collect::<Vec<_>>();
    let root = doc.add_object(dictionary! {"Type"=>"Outlines"});
    doc.catalog_mut().unwrap().set("Outlines", root);
    build(&mut doc, root, &pages);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn first(doc: &mut Document, root: ObjectId, node: ObjectId) {
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("First", node);
}

fn fit(page: ObjectId) -> Object {
    vec![Object::Reference(page), Object::Name(b"Fit".to_vec())].into()
}

#[test]
fn projects_unicode_nested_groups_and_local_goto_without_executing_external_actions() {
    let bytes = fixture(|doc, root, pages| {
        let child = doc.add_object(dictionary! {
            "Title"=>text_string("子节"), "A"=>dictionary!{"S"=>"GoTo","D"=>fit(pages[0])},
        });
        let external = doc.add_object(dictionary! {
            "Title"=>text_string("外部网页"),"A"=>dictionary!{"S"=>"URI","URI"=>text_string("https://example.invalid/")},
        });
        let parent = doc.add_object(dictionary!{
            "Title"=>text_string("目录·第一章"),"Dest"=>fit(pages[1]),"First"=>child,"Next"=>external,
        });
        first(doc, root, parent);
    });
    let mut session = Session::open(&bytes).unwrap();
    let original_info = session.info();
    assert_eq!(
        session.outline().unwrap(),
        vec![
            OutlineItem {
                title: "目录·第一章".into(),
                depth: 0,
                path: vec![0],
                page_index: Some(1)
            },
            OutlineItem {
                title: "子节".into(),
                depth: 1,
                path: vec![0, 0],
                page_index: Some(0)
            },
            OutlineItem {
                title: "外部网页".into(),
                depth: 0,
                path: vec![1],
                page_index: None
            },
        ]
    );
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(session.info(), original_info);
    session.apply(Edit::MovePage { from: 1, to: 0 }).unwrap();
    assert_eq!(session.outline().unwrap()[0].page_index, Some(0));
    session.undo().unwrap();
    assert_eq!(session.outline().unwrap()[0].page_index, Some(1));
}

#[test]
fn missing_titles_and_nonlocal_or_ambiguous_targets_remain_nonnavigable_rows() {
    let bytes = fixture(|doc, root, pages| {
        let remote = doc.add_object(dictionary!{"Title"=>text_string("远程"),"A"=>dictionary!{"S"=>"GoToR","D"=>fit(pages[0])}});
        let ambiguous = doc.add_object(dictionary!{"Title"=>text_string("冲突"),"Dest"=>fit(pages[0]),"A"=>dictionary!{"S"=>"GoTo","D"=>fit(pages[1])},"Next"=>remote});
        let group = doc.add_object(dictionary! {"Next"=>ambiguous});
        first(doc, root, group);
    });
    let rows = Session::open(&bytes).unwrap().outline().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].title, "未命名");
    assert!(rows.iter().all(|row| row.page_index.is_none()));
}

#[test]
fn resolves_legacy_name_and_modern_byte_string_destinations_without_aliasing_their_keys() {
    let bytes = fixture(|doc, root, pages| {
        doc.catalog_mut().unwrap().set(
            "Dests",
            dictionary! {"same"=>dictionary!{"D"=>fit(pages[0])}},
        );
        let modern = doc.add_object(dictionary! {"Names"=>vec![text_string("same"),fit(pages[1])]});
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! {"Dests"=>dictionary!{"Kids"=>vec![Object::Reference(modern)]}},
        );
        let second =
            doc.add_object(dictionary! {"Title"=>text_string("现代"),"Dest"=>text_string("same")});
        let one = doc.add_object(dictionary!{"Title"=>text_string("传统"),"Dest"=>Object::Name(b"same".to_vec()),"Next"=>second});
        first(doc, root, one);
    });
    let rows = Session::open(&bytes).unwrap().outline().unwrap();
    assert_eq!(
        rows.iter().map(|r| r.page_index).collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
}

#[test]
fn linked_cycles_and_reference_cycles_fail_without_mutating_the_session() {
    for reference_cycle in [false, true] {
        let bytes = fixture(|doc, root, _| {
            let node = doc.new_object_id();
            if reference_cycle {
                doc.set_object(node, Object::Reference(node));
            } else {
                doc.set_object(
                    node,
                    dictionary! {"Title"=>text_string("cycle"),"Next"=>node},
                );
            }
            first(doc, root, node);
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.outline().is_err());
        assert_eq!(session.bytes().unwrap(), bytes);
        assert!(!session.info().dirty);
    }
}

#[test]
fn oversized_titles_depth_and_item_count_are_bounded_failures() {
    for (nodes, children, title) in [
        (1, false, "x".repeat(8193)),
        (129, true, "x".into()),
        (4097, false, "x".into()),
    ] {
        let bytes = fixture(|doc, root, _| {
            let mut next = None;
            for _ in 0..nodes {
                let mut row = dictionary! {"Title"=>text_string(&title)};
                if let Some(next) = next {
                    row.set(
                        if children { "First" } else { "Next" },
                        Object::Reference(next),
                    );
                }
                next = Some(doc.add_object(row));
            }
            first(doc, root, next.unwrap());
        });
        let session = Session::open(&bytes).unwrap();
        assert!(
            session.outline().is_err(),
            "nodes={nodes},children={children},title={}",
            title.len()
        );
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn documents_without_outlines_have_an_empty_read_only_projection() {
    assert!(
        Session::blank(200., 120., 2)
            .unwrap()
            .outline()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn outline_projection_has_a_public_read_only_contract() {
    assert!(include_str!("../src/lib.rs").contains("pub use outline::OutlineItem;"));
}

#[test]
fn direct_targets_do_not_parse_unrelated_broken_named_destinations() {
    let bytes = fixture(|doc, root, pages| {
        doc.catalog_mut().unwrap().set("Names", 7);
        let node =
            doc.add_object(dictionary! {"Title"=>text_string("直接目标"),"Dest"=>fit(pages[1])});
        first(doc, root, node);
    });
    let session = Session::open(&bytes).unwrap();
    assert_eq!(session.outline().unwrap()[0].page_index, Some(1));
    assert_eq!(session.bytes().unwrap(), bytes);
}

#[test]
fn local_goto_action_type_can_be_an_indirect_name() {
    let bytes = fixture(|doc, root, pages| {
        let kind = doc.add_object(Object::Name(b"GoTo".to_vec()));
        let node = doc.add_object(dictionary! {"Title"=>text_string("间接动作"),"A"=>dictionary!{"S"=>kind,"D"=>fit(pages[0])}});
        first(doc, root, node);
    });
    assert_eq!(
        Session::open(&bytes).unwrap().outline().unwrap()[0].page_index,
        Some(0)
    );
}

#[test]
fn valid_standard_destination_modes_project_only_the_current_local_page() {
    let bytes = fixture(|doc, root, pages| {
        let mut next = None;
        for (mode, parameters) in [
            ("Fit", vec![]),
            ("FitB", vec![]),
            ("FitH", vec![Object::Null]),
            ("FitV", vec![10.into()]),
            ("FitBH", vec![Object::Null]),
            ("FitBV", vec![10.into()]),
            ("XYZ", vec![Object::Null, Object::Null, 0.into()]),
            ("FitR", vec![0.into(), 0.into(), 100.into(), 100.into()]),
        ]
        .into_iter()
        .rev()
        {
            let mut parts = vec![
                Object::Reference(pages[1]),
                Object::Name(mode.as_bytes().to_vec()),
            ];
            parts.extend(parameters);
            let mut node = dictionary! {"Title"=>text_string(mode),"Dest"=>parts};
            if let Some(next) = next {
                node.set("Next", Object::Reference(next));
            }
            next = Some(doc.add_object(node));
        }
        first(doc, root, next.unwrap());
    });
    let rows = Session::open(&bytes).unwrap().outline().unwrap();
    assert_eq!(rows.len(), 8);
    assert!(rows.iter().all(|row| row.page_index == Some(1)));
}

#[test]
fn malformed_and_remote_destination_arrays_never_become_local_navigation() {
    let bytes = fixture(|doc, root, pages| {
        let mut next = None;
        for target in [
            vec![],
            vec![Object::Reference(pages[0])],
            vec![0.into(), Object::Name(b"Fit".to_vec())],
            vec![
                Object::Reference((999999, 0)),
                Object::Name(b"Fit".to_vec()),
            ],
            vec![
                Object::Reference(pages[0]),
                Object::Name(b"Unknown".to_vec()),
            ],
            vec![
                Object::Reference(pages[0]),
                Object::Name(b"Fit".to_vec()),
                1.into(),
            ],
            vec![
                Object::Reference(pages[0]),
                Object::Name(b"FitR".to_vec()),
                Object::Null,
                0.into(),
                1.into(),
                1.into(),
            ],
        ]
        .into_iter()
        .rev()
        {
            let mut node = dictionary! {"Title"=>text_string("不可跳转"),"Dest"=>target};
            if let Some(next) = next {
                node.set("Next", Object::Reference(next));
            }
            next = Some(doc.add_object(node));
        }
        first(doc, root, next.unwrap());
    });
    let rows = Session::open(&bytes).unwrap().outline().unwrap();
    assert_eq!(rows.len(), 7);
    assert!(rows.iter().all(|row| row.page_index.is_none()));
}

#[test]
fn name_tree_cycles_odd_pairs_and_duplicate_names_fail_read_only() {
    for case in 0..3 {
        let bytes = fixture(|doc, root, pages| {
            let tree = doc.new_object_id();
            let value = match case {
                0 => dictionary! {"Kids"=>vec![Object::Reference(tree)]},
                1 => dictionary! {"Names"=>vec![text_string("x")]},
                _ => {
                    dictionary! {"Names"=>vec![text_string("x"),fit(pages[0]),text_string("x"),fit(pages[1])]}
                }
            };
            doc.set_object(tree, value);
            doc.catalog_mut()
                .unwrap()
                .set("Names", dictionary! {"Dests"=>tree});
            let node = doc.add_object(dictionary! {"Dest"=>text_string("x")});
            first(doc, root, node);
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.outline().is_err(), "case {case}");
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn exact_depth_count_and_title_boundaries_are_admitted() {
    for (nodes, children, title) in [
        (128, true, "x".into()),
        (4096, false, "x".into()),
        (1, false, "x".repeat(8192)),
    ] {
        let bytes = fixture(|doc, root, _| {
            let mut next = None;
            for _ in 0..nodes {
                let mut node = dictionary! {"Title"=>text_string(&title)};
                if let Some(next) = next {
                    node.set(
                        if children { "First" } else { "Next" },
                        Object::Reference(next),
                    );
                }
                next = Some(doc.add_object(node));
            }
            first(doc, root, next.unwrap());
        });
        let session = Session::open(&bytes).unwrap();
        assert_eq!(session.outline().unwrap().len(), nodes);
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn named_destination_lookup_keys_and_cumulative_hash_work_are_bounded() {
    for (key_bytes, nodes, accepted) in [(8193, 1, false), (8192, 128, true), (8192, 129, false)] {
        let bytes = fixture(|doc, root, pages| {
            let shared_key = doc.add_object(text_string(&"x".repeat(key_bytes)));
            doc.catalog_mut().unwrap().set("Names", dictionary! {"Dests"=>dictionary! {"Names"=>vec![text_string("small"),fit(pages[0])]}});
            let mut next = None;
            for _ in 0..nodes {
                let mut node = dictionary! {"Title"=>text_string("共享目标"),"Dest"=>shared_key};
                if let Some(next) = next {
                    node.set("Next", Object::Reference(next));
                }
                next = Some(doc.add_object(node));
            }
            first(doc, root, next.unwrap());
        });
        let session = Session::open(&bytes).unwrap();
        assert_eq!(
            session.outline().is_ok(),
            accepted,
            "key={key_bytes},nodes={nodes}"
        );
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn cumulative_decoded_title_bytes_have_an_exact_one_mib_budget() {
    for (nodes, accepted) in [(128, true), (129, false)] {
        let bytes = fixture(|doc, root, _| {
            let title = doc.add_object(text_string(&"x".repeat(8192)));
            let mut next = None;
            for _ in 0..nodes {
                let mut node = dictionary! {"Title"=>title};
                if let Some(next) = next {
                    node.set("Next", Object::Reference(next));
                }
                next = Some(doc.add_object(node));
            }
            first(doc, root, next.unwrap());
        });
        let session = Session::open(&bytes).unwrap();
        assert_eq!(session.outline().is_ok(), accepted, "nodes={nodes}");
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}

#[test]
fn name_tree_queue_and_key_bytes_are_bounded_before_projection() {
    for kind in 0..2 {
        let bytes = fixture(|doc, root, pages| {
            let tree = if kind == 0 {
                dictionary! {"Kids"=>vec![Object::Dictionary(dictionary!{});4096]}
            } else {
                let mut pairs = Vec::new();
                for index in 0..129 {
                    let key = format!("{index:03}{}", "x".repeat(8189));
                    pairs.push(text_string(&key));
                    pairs.push(fit(pages[0]));
                }
                dictionary! {"Names"=>pairs}
            };
            doc.catalog_mut()
                .unwrap()
                .set("Names", dictionary! {"Dests"=>tree});
            let node = doc.add_object(dictionary! {"Dest"=>text_string("missing")});
            first(doc, root, node);
        });
        let session = Session::open(&bytes).unwrap();
        assert!(session.outline().is_err(), "kind={kind}");
        assert_eq!(session.bytes().unwrap(), bytes);
    }
}
