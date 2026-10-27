use pdfmaster_ui::annotations::{Row, Sort, sort_rows, summary};
use pdfmaster_ui::outline::ProjectionCache as Cache;
use serde_json::json;

#[test]
fn delayed_copy_status_belongs_only_to_its_unchanged_document_and_message_even_during_render() {
    use pdfmaster_ui::annotations::copy_status_is_current;
    let mut cache = Cache::<Row>::default();
    let generation = cache.generation();
    assert!(copy_status_is_current(
        generation,
        cache.generation(),
        "copying",
        "copying",
    ));
    assert!(!copy_status_is_current(
        generation,
        cache.generation(),
        "copying",
        "new error",
    ));
    cache.invalidate();
    assert!(!copy_status_is_current(
        generation,
        cache.generation(),
        "copying",
        "copying",
    ));
}

fn row(page: usize, index: usize, kind: &str, text: &str) -> Row {
    serde_json::from_value(json!({"page_index":page,"annotation_index":index,"page_id":[page+1,0],"object_id":[index+100,0],"subtype":kind,"contents":text,"modified":null,"color":null,"bounds":null})).unwrap()
}

#[test]
fn original_annotation_titles_detail_heights_and_summary_keep_real_content() {
    let a = row(1, 2, "Text", "第一行\n 第二行");
    assert_eq!(a.heading(), "第 2 页 · 便笺");
    assert_eq!(a.detail(), "第一行 第二行");
    assert_eq!(a.height(), 60);
    assert_eq!(row(0, 0, "Ink", "  \n ").height(), 44);
    assert_eq!(row(0, 0, "Stamp", "").heading(), "第 1 页 · Stamp");
    assert_eq!(
        summary(&[a, row(0, 0, "Line", "")]),
        "第 2 页 · 便笺\n第一行\n 第二行\n\n第 1 页 · 直线 / 箭头"
    );
}

#[test]
fn date_sort_has_page_index_ties_and_does_not_change_selected_object_identity() {
    let mut a = row(1, 3, "Text", "A");
    a.modified_millis = Some(200);
    let mut b = row(0, 5, "Ink", "B");
    b.modified_millis = Some(200);
    let mut c = row(0, 1, "Stamp", "C");
    c.modified_millis = Some(100);
    let selected = a.identity();
    let mut rows = vec![a, b, c, row(0, 0, "Text", "")];
    sort_rows(&mut rows, Sort::Newest);
    assert_eq!(
        rows.iter().map(|r| r.contents.as_str()).collect::<Vec<_>>(),
        vec!["B", "A", "C", ""]
    );
    assert_eq!(rows[1].identity(), selected);
    sort_rows(&mut rows, Sort::Page);
    assert_eq!(
        rows.iter().map(|r| r.annotation_index).collect::<Vec<_>>(),
        vec![0, 1, 5, 3]
    );
    assert_eq!(rows[3].identity(), selected);
}

#[test]
fn annotation_payload_reuses_the_single_native_flight_generation_guard() {
    let mut cache = Cache::<Row>::default();
    let old = cache.begin().unwrap();
    cache.invalidate();
    assert!(cache.begin().is_none());
    assert!(!cache.finish(old, Ok(vec![row(0, 0, "Text", "stale")])));
    assert!(cache.result().is_none());
    let new = cache.begin().unwrap();
    assert!(cache.finish(new, Ok(vec![row(1, 0, "Text", "current")])));
    assert!(!cache.finish(old, Ok(vec![])));
    assert_eq!(
        cache.result().unwrap().as_ref().unwrap()[0].contents,
        "current"
    );
    cache.invalidate();
    let timed = cache.begin().unwrap();
    cache.suspend(timed);
    cache.invalidate();
    assert!(cache.begin().is_none());
    assert!(!cache.finish(timed, Ok(vec![])));
}

#[test]
fn real_sidebar_invalidates_rows_and_wires_bounded_reads_and_original_geometry() {
    let workspace = include_str!("../src/workspace.rs").replace("\r\n", "\n");
    assert!(
        workspace.contains("annotation_cache:"),
        "annotation data must belong to the current document model"
    );
    let replace = workspace
        .split("fn replace_document_view")
        .nth(1)
        .unwrap()
        .split("fn close_chrome_popups")
        .next()
        .unwrap();
    assert!(replace.contains("self.annotation_cache.update(|cache| cache.invalidate())"));
    assert!(replace.contains("self.annotation_selection.set(None)"));
    let view = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/annotation_view.rs"
    ))
    .expect("real annotation component")
    .replace("\r\n", "\n");
    assert!(view.contains("bridge::call_bounded::<Vec<Row>>"));
    assert!(view.contains("\"annotations_document\""));
    assert!(view.contains("cache.suspend(ticket)"));
    assert!(view.contains("m.blocked()"));
    assert!(view.contains("m.annotation_rows()"));
    assert!(view.contains("Promise::race"));
    assert!(view.contains("timer.clear()"));
    let completion = view
        .split("let result = copy_text(&text).await;")
        .nth(1)
        .unwrap()
        .split("fn annotation_tool")
        .next()
        .unwrap();
    assert!(completion.contains("copy_status_is_current("));
    assert!(
        !completion.contains("self.blocked()"),
        "a page render must not leave the owned pending copy status forever"
    );
    assert!(view.contains("self.navigate(row.page_index)"));
    assert!(view.contains("使用批注和绘画工具标记PDF"));
    let sidebar = include_str!("../src/thumbnail_view.rs");
    assert!(sidebar.contains("Mode::Annotations"));
    assert!(sidebar.contains("<AnnotationSidebar model=m/>"));
    let css = include_str!("../workbench.css");
    assert!(css.contains(".annotation-heading{font-size:11px"));
    assert!(css.contains(".annotation-scroll .annotation-row.selected:hover:enabled"));
    assert!(css.contains(".annotation-time{font-size:10.5px"));
    let menu = css
        .split(".annotation-menu{")
        .nth(1)
        .unwrap()
        .split('}')
        .next()
        .unwrap();
    assert!(menu.contains("box-sizing:border-box"));
    assert!(menu.contains("max-width:calc(var(--reader-sidebar-width,176px) - 16px)"));
}
