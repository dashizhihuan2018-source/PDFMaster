use super::*;
use crate::shape_editor::{CanvasShapeTarget, ShapeEditor};
use crate::shape_editor_tests::{begin, rect, target, workspace};
use lopdf::{Object, Stream, dictionary};

fn encoded(doc: &mut lopdf::Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn fixture(
    rotation: i32,
    shared: bool,
    bad_ap: bool,
) -> (Workspace, tempfile::TempDir, std::path::PathBuf) {
    let (mut w, dir, path) = workspace();
    let mut doc = lopdf::Document::load_mem(&w.session.as_ref().unwrap().bytes().unwrap()).unwrap();
    let pages: Vec<_> = doc.get_pages().values().copied().collect();
    for (i, page) in pages.iter().enumerate() {
        let stream = doc.add_object(Stream::new(
            dictionary! {},
            format!(
                "q 0.1 0.6 0.8 rg 0 0 300 200 re f 0.8 0.3 0.1 rg {} 10 120 100 re f Q",
                10 + i * 40
            )
            .into_bytes(),
        ));
        let p = doc.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
        p.set("Contents", stream);
        p.set("Rotate", rotation);
    }
    w.session = Some(Session::open(&encoded(&mut doc)).unwrap());
    w.session
        .as_mut()
        .unwrap()
        .create_canvas_shape(0, &rect())
        .unwrap();
    let mut doc = lopdf::Document::load_mem(&w.session.as_ref().unwrap().bytes().unwrap()).unwrap();
    let annots = doc
        .get_object(pages[0])
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    if shared {
        doc.get_object_mut(pages[1])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", Object::Array(annots.clone()));
    }
    if bad_ap {
        let id = annots[0].as_reference().unwrap();
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("AP", dictionary! { "N" => Object::Reference((999999, 0)) });
    }
    w.session = Some(Session::open(&encoded(&mut doc)).unwrap());
    // Two-sided history proves read-only preview preserves both stacks.
    w.session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    w.session.as_mut().unwrap().undo().unwrap();
    w.session.as_mut().unwrap().mark_saved();
    w.journal
        .as_ref()
        .unwrap()
        .save(
            "shape.pdf",
            &w.session.as_ref().unwrap().bytes().unwrap(),
            false,
        )
        .unwrap();
    (w, dir, path)
}

fn select(w: &mut Workspace) -> ShapeEditor {
    let selected = w
        .session
        .as_ref()
        .unwrap()
        .canvas_shapes(0)
        .unwrap()
        .annotations
        .remove(0);
    begin(w, CanvasShapeTarget::Existing { original: selected })
}

fn rejected(w: &Workspace, path: &Path, id: u32, request: &str, scale: f32) {
    let before = w.session.as_ref().unwrap().bytes().unwrap();
    let info = w.session.as_ref().unwrap().info();
    let owner = w.shape_editor.clone();
    let journal = std::fs::read(path).unwrap();
    assert!(w.shape_editor_background(id, request, scale).is_err());
    assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(w.session.as_ref().unwrap().info(), info);
    assert_eq!(w.shape_editor, owner);
    assert_eq!(std::fs::read(path).unwrap(), journal);
}

#[test]
fn background_matches_independent_omitted_pdf_all_rotations_and_shared_occurrence() {
    for rotation in [0, 90, 180, 270] {
        let (mut w, _dir, path) = fixture(rotation, true, false);
        let before = w.session.as_ref().unwrap().bytes().unwrap();
        let mut omitted = lopdf::Document::load_mem(&before).unwrap();
        let page = *omitted.get_pages().values().next().unwrap();
        omitted
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", Object::Array(Vec::new()));
        let expected = Session::open(&encoded(&mut omitted))
            .unwrap()
            .render_png(0, 1.)
            .unwrap();
        let ordinary = w.session.as_ref().unwrap().render_png(0, 1.).unwrap();
        assert_ne!(expected, ordinary);
        let other = w.session.as_ref().unwrap().render_png(1, 1.).unwrap();
        let o = select(&mut w);
        let info = w.session.as_ref().unwrap().info();
        let journal = std::fs::read(&path).unwrap();
        assert_eq!(
            w.shape_editor_background(o.id, &o.request, 1.).unwrap(),
            expected
        );
        assert_eq!(
            w.session.as_ref().unwrap().render_png(1, 1.).unwrap(),
            other
        );
        assert_eq!(w.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(w.session.as_ref().unwrap().info(), info);
        assert_eq!(w.shape_editor.as_ref().unwrap(), &o);
        assert_eq!(std::fs::read(&path).unwrap(), journal);
    }
}

#[test]
fn new_background_is_original_page_and_invalid_scale_is_read_only() {
    let (mut w, _dir, path) = fixture(0, false, false);
    let t = target(&w);
    let o = begin(&mut w, t);
    let expected = w.session.as_ref().unwrap().render_png(0, 1.).unwrap();
    assert_eq!(
        w.shape_editor_background(o.id, &o.request, 1.).unwrap(),
        expected
    );
    for scale in [f32::NAN, f32::INFINITY, 0., -1., 1000.] {
        rejected(&w, &path, o.id, &o.request, scale);
    }
}

#[test]
fn strict_original_bad_appearance_rejects_without_fallback() {
    let (mut w, _dir, path) = fixture(0, false, true);
    assert!(w.session.as_ref().unwrap().render_png(0, 1.).is_err());
    let o = select(&mut w);
    rejected(&w, &path, o.id, &o.request, 1.);
}

#[test]
fn preview_identity_rejects_wrong_owner_request_document_epoch_and_exact_bytes() {
    for mismatch in 0..5 {
        let (mut w, _dir, path) = fixture(0, false, false);
        let o = select(&mut w);
        match mismatch {
            2 => w.active_document_id += 1,
            3 => w.document_epoch += 1,
            4 => {
                w.session
                    .as_mut()
                    .unwrap()
                    .apply(Edit::Rotate {
                        page: 1,
                        degrees: 90,
                    })
                    .unwrap();
            }
            _ => {}
        }
        rejected(
            &w,
            &path,
            if mismatch == 0 { o.id + 1 } else { o.id },
            if mismatch == 1 {
                "shape-other-0002"
            } else {
                &o.request
            },
            1.,
        );
    }
}
