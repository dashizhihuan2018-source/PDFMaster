use serde_json::Value;

fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
    let tail = source.split_once(&format!("fn {name}(")).unwrap().1;
    let start = tail.find('{').unwrap();
    let mut depth = 0;
    for (offset, ch) in tail[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &tail[start..=start + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed function {name}");
}

#[test]
fn canvas_shapes_exact_commands_keep_native_owner_only_wire() {
    let source = include_str!("../src/lib.rs");
    let module = include_str!("../src/shape_editor.rs");
    let handlers = source
        .rsplit_once(".invoke_handler(tauri::generate_handler![")
        .unwrap()
        .1;
    let commands = [
        (
            "canvas_shapes_document",
            "document_id:u32,page:usize,state:State<'_,Store>,",
        ),
        (
            "begin_canvas_shape_editor",
            "request:String,document_id:u32,page:usize,target:CanvasShapeTarget,state:State<'_,Store>,",
        ),
        (
            "cancel_canvas_shape_request",
            "request:String,document_id:u32,state:State<'_,Store>,",
        ),
        (
            "cancel_shape_editor",
            "id:u32,request:String,state:State<'_,Store>,",
        ),
        (
            "commit_shape_editor",
            "id:u32,request:String,draft:CanvasShapeDraft,state:State<'_,Store>,",
        ),
        (
            "render_shape_editor_background",
            "id:u32,request:String,scale:f32,state:State<'_,Store>,",
        ),
        (
            "delete_shape_editor",
            "app:tauri::AppHandle,id:u32,request:String,state:State<'_,Store>,",
        ),
    ];
    for (command, expected) in commands {
        assert!(
            handlers.contains(&format!("shape_editor::{command}")),
            "{command}"
        );
        let marker = format!("pub(crate) async fn {command}(");
        let (prefix, rest) = module.split_once(&marker).unwrap();
        assert!(
            prefix
                .trim_end()
                .ends_with("#[tauri::command(rename_all = \"snake_case\")]"),
            "{command}"
        );
        let args = rest.split(") ->").next().unwrap();
        let compact: String = args.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(compact, expected, "{command}");
        for forbidden in ["Path", "bytes", "confirmed", "annotation_index"] {
            assert!(!args.contains(forbidden), "{command}: {forbidden}");
        }
    }
    assert!(module.contains("format!(\"data:image/png;base64,{}\", STANDARD.encode(png))"));
}

#[test]
fn canvas_shape_delete_prepares_before_parented_native_dialog_and_applies_after_await() {
    let module = include_str!("../src/shape_editor.rs");
    let body = module
        .split("pub(crate) async fn delete_shape_editor(")
        .nth(1)
        .expect("delete command");
    let prepare = body.find("prepare_shape_editor_delete(").unwrap();
    let finished = body
        .find("ShapeDeletePreparation::Finished(view) => return Ok(Some(view))")
        .unwrap();
    let dialog = body.find(".dialog()").unwrap();
    let show = body.find("dialog.show(").unwrap();
    let wait = body.find(".await").unwrap();
    let apply = body.find("apply_confirmed_shape_editor_delete(").unwrap();
    assert!(
        prepare < finished && finished < dialog && dialog < show && show < wait && wait < apply
    );
    assert!(body.contains("OkCancelCustom("));
    assert!(body.contains("let dialog = dialog.parent("));
    assert!(body.contains("get_webview_window(\"main\")"));
    assert!(body.contains("tokio::sync::oneshot::channel()"));
    assert!(body.contains("return Ok(None)"));
    assert!(body.contains("let prepared = {"));
    assert_eq!(body.matches("state.lock()").count(), 2);
    assert!(body[wait..apply].contains("state.lock()"));
}

#[test]
fn shape_owner_wires_shared_guard_to_private_provider_routes_without_visibility_changes() {
    // Wiring evidence only; these are not OS clipboard/image provider executions.
    let overview = include_str!("../src/overview.rs");
    let clipboard = include_str!("../src/clipboard.rs");
    let image = include_str!("../src/image_import.rs");
    for (source, name, required) in [
        (overview, "prepare_overview", "self.ensure_editor_idle()?;"),
        (overview, "validate_overview", "self.ensure_editor_idle()?;"),
        (overview, "move_overview", "self.validate_overview(token)?;"),
        (overview, "prepare_page_overview", ".prepare_overview()"),
        (overview, "move_overview_pages", ".move_overview("),
        (
            clipboard,
            "validate_clipboard_document",
            "self.ensure_editor_idle()?;",
        ),
        (
            clipboard,
            "begin_clipboard_copy",
            "self.validate_clipboard_document(document, token)?;",
        ),
        (clipboard, "copy_pages_document", ".begin_clipboard_copy("),
        (
            clipboard,
            "paste_pages_document",
            "w.begin_insertion(index)?",
        ),
        (image, "start_image_picker", "self.ensure_editor_idle()?;"),
        (
            image,
            "begin_image_conversion",
            "self.ensure_editor_idle()?;",
        ),
        (image, "prepare_image_import", ".start_image_picker()"),
        (image, "convert_image_import", ".begin_image_conversion("),
    ] {
        let body = function_body(source, name);
        assert!(body.contains(required), "{name}");
    }
    let source = include_str!("../src/lib.rs");
    let idle = function_body(source, "ensure_editor_idle");
    assert!(idle.contains("self.shape_editor.is_some()"));
    let checkpoint = function_body(source, "checkpoint");
    assert!(checkpoint.contains("self.finished_shape = None;"));
}

#[test]
fn image_tool_selects_native_paths_and_conversion_accepts_only_ticket_indices_and_layout() {
    let source = include_str!("../src/lib.rs");
    for command in [
        "prepare_image_import",
        "convert_image_import",
        "cancel_image_import",
    ] {
        assert!(source.contains(&format!("image_import::{command}")));
    }
    let module = include_str!("../src/image_import.rs");
    assert!(module.contains(".pick_files("));
    let args = module
        .split("async fn convert_image_import(")
        .nth(1)
        .unwrap()
        .split(") ->")
        .next()
        .unwrap();
    assert!(args.contains("order") && args.contains("layout"));
    assert!(!args.contains("Path") && !args.contains("paths") && !args.contains("bytes"));
    assert!(module.contains("with_read_slot("));
}

#[test]
fn home_file_drop_is_registered_without_accepting_frontend_paths_or_pdf_bytes() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("file_drop::open_file_drop,"));
    let drop = include_str!("../src/file_drop.rs");
    let args = drop
        .split("async fn open_file_drop(")
        .nth(1)
        .unwrap()
        .split(") ->")
        .next()
        .unwrap();
    assert!(!args.contains("Path") && !args.contains("paths") && !args.contains("bytes"));
}

#[test]
fn native_file_drop_routes_the_main_window_and_denies_path_bearing_event_listeners() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("tauri::WindowEvent::DragDrop(event)"));
    assert!(source.contains("file_drop::event(window, event)"));
    assert!(!source.contains(".on_webview_event(file_drop::event)"));
    let capability: Value =
        serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
    assert!(
        capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "core:event:deny-listen")
    );
    let drop = include_str!("../src/file_drop.rs");
    assert!(drop.contains("tauri::ipc::Channel<Notice>"));
    let args = drop
        .split("async fn import_file_drop(")
        .nth(1)
        .unwrap()
        .split(") ->")
        .next()
        .unwrap();
    assert!(!args.contains("Path") && !args.contains("paths") && !args.contains("bytes"));
}

#[test]
fn unified_annotation_deletion_uses_the_native_confirmation_and_snapshot_path() {
    let source = include_str!("../src/lib.rs");
    assert!(source.contains("async fn delete_annotation_document("));
    let body = source
        .split("async fn delete_annotation_document(")
        .nth(1)
        .unwrap()
        .split("#[tauri::command")
        .next()
        .unwrap();
    assert!(body.contains("prepare_annotation_delete("));
    assert!(body.contains("dialog.show("));
    assert!(body.contains("receiver"));
    assert!(body.contains("return Ok(None)"));
    assert!(body.contains("apply_confirmed_annotation_delete("));
    let handlers = source
        .rsplit_once(".invoke_handler(tauri::generate_handler![")
        .unwrap()
        .1;
    assert!(handlers.contains("delete_annotation_document"));
}

#[test]
fn wasm_can_load_only_from_the_packaged_origin() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let csp = config["app"]["security"]["csp"].as_str().unwrap();
    let connect = csp
        .split(';')
        .map(str::trim)
        .find(|s| s.starts_with("connect-src "))
        .unwrap();
    assert!(
        connect.split_whitespace().any(|s| s == "'self'"),
        "WASM fetch needs its packaged origin"
    );
    assert!(!connect.contains('*'));
    assert!(!connect.contains("https:"));
    assert!(!csp.contains("'unsafe-eval'"));
    assert!(csp.contains("object-src 'none'"));
}

#[test]
fn windows_publication_uses_write_through_move_with_explicit_replace_policy() {
    let storage = include_str!("../src/storage.rs");
    let recovery = include_str!("../src/recovery.rs");
    assert!(storage.contains("MoveFileExW"));
    assert!(storage.contains("MOVEFILE_WRITE_THROUGH"));
    assert!(storage.contains("move_file_write_through(&temp, &target, false)"));
    assert!(recovery.contains("move_file_write_through(&staged_path, &self.path, true)"));
}

#[test]
fn ios_readback_requires_scope_and_coordinates_only_the_staged_move_on_a_worker() {
    let source = include_str!("../src/ios_export.rs");
    assert!(source.contains("PreparedExport::new(&cache, name, bytes)"));
    let constructor = source
        .split("let picker = UIDocumentPickerViewController::initForExportingURLs_asCopy(")
        .nth(1)
        .unwrap()
        .split(");")
        .next()
        .unwrap();
    assert!(constructor.contains("false,"));
    assert!(!constructor.contains("true,"));
    assert!(source.contains("if !scoped"));
    assert!(source.contains("stopAccessingSecurityScopedResource"));
    assert!(source.contains("coordinateReadingItemAtURL_options_error_byAccessor"));
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("coordinated_url.path()"));
    assert!(source.contains("std::thread::Builder::new()"));
    assert!(source.contains("verify_completion(true, Some(&bytes), &expected)"));
    assert!(!source.contains("File::open"));
}

#[test]
fn all_native_exit_paths_include_editor_state_and_history_export_require_idle_editor() {
    // Source-wiring regression, complementary to real Workspace state tests.
    let source = include_str!("../src/lib.rs");
    let cocoa = include_str!("../src/macos_termination.rs");
    assert!(cocoa.contains("workspace.has_unsaved_work()"));
    assert!(source.matches("|w| w.has_unsaved_work()").count() >= 1);
    assert!(source.contains("|w|w.has_unsaved_work()"));
    for command in ["undo_document", "redo_document", "export_document"] {
        let body = source
            .split(&format!("async fn {command}("))
            .nth(1)
            .unwrap()
            .split("#[tauri::command")
            .next()
            .unwrap();
        assert!(body.contains("ensure_editor_idle()?"), "{command}");
    }
}

#[test]
fn canvas_free_text_commands_are_module_registered_and_accept_no_paths_or_pdf_bytes() {
    let source = include_str!("../src/lib.rs");
    let module = include_str!("../src/free_text_editor.rs");
    let handlers = source
        .rsplit_once(".invoke_handler(tauri::generate_handler![")
        .unwrap()
        .1;
    for command in [
        "free_texts_document",
        "begin_canvas_free_text_editor",
        "cancel_canvas_free_text_request",
        "cancel_free_text_editor",
        "commit_free_text_editor",
    ] {
        assert!(
            handlers.contains(&format!("free_text_editor::{command}")),
            "{command}"
        );
        let args = module
            .split(&format!("async fn {command}("))
            .nth(1)
            .unwrap()
            .split(") ->")
            .next()
            .unwrap();
        assert!(!args.contains("Path"), "{command}");
        assert!(!args.contains("bytes"), "{command}");
    }
    assert!(module.contains("document_id != workspace.active_document_id"));
    assert!(module.contains("document_id != self.active_document_id"));
}

#[test]
fn free_text_background_command_is_registered_with_only_owner_identity_and_scale() {
    let source = include_str!("../src/lib.rs");
    let module = include_str!("../src/free_text_editor.rs");
    let handlers = source
        .rsplit_once(".invoke_handler(tauri::generate_handler![")
        .unwrap()
        .1;
    assert!(handlers.contains("free_text_editor::render_free_text_editor_background"));
    let args = module
        .split("async fn render_free_text_editor_background(")
        .nth(1)
        .unwrap()
        .split(") ->")
        .next()
        .unwrap();
    for required in ["id: u32", "request: String", "scale: f32", "state: State"] {
        assert!(args.contains(required), "{required}");
    }
    for forbidden in [
        "Path",
        "bytes",
        "document_id",
        "page:",
        "target",
        "annotation",
        "style",
    ] {
        assert!(!args.contains(forbidden), "{forbidden}");
    }
    assert!(module.contains("data:image/png;base64,"));
}
