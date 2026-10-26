#[path = "bridge.rs"]
mod bridge;
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    RasterPlan, adjacent_page, command_status_message, fit_width_zoom, note_edit, page_batch_args,
    page_jump_index, page_jump_success_message, page_move_edit, raster_plan,
    search_conditions_message, search_response_is_current, search_status_message, step_zoom,
    update_note_edit, visible_status,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize)]
struct Page {
    width: f32,
    height: f32,
    rotation: i32,
}
#[derive(Clone, Debug, Deserialize)]
struct Info {
    page_count: usize,
    pages: Vec<Page>,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
}
#[derive(Clone, Debug, Deserialize)]
struct Document {
    name: String,
    info: Info,
    restored: bool,
    #[serde(default)]
    warning: Option<String>,
    #[serde(default)]
    note_editor: Option<NoteDraft>,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Debug, Deserialize)]
struct TextNote {
    annotation_index: usize,
    bounds: NoteBounds,
    contents: String,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteList {
    notes: Vec<TextNote>,
    unsupported_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct NoteDraft {
    id: u32,
    page: usize,
    original: TextNote,
}

#[derive(Clone, Debug, Deserialize)]
struct SearchHit {
    page_index: usize,
    match_start: usize,
    match_end: usize,
    snippet: String,
    snippet_match_start: usize,
    snippet_match_end: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct SearchResponse {
    query: String,
    case_sensitive: bool,
    hits: Vec<SearchHit>,
    pages_scanned: usize,
    searchable_pages: usize,
    truncated: bool,
    limitation: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PageBatchReply {
    Cancelled,
    Applied {
        document: Document,
        affected_count: usize,
    },
}

#[derive(Clone, Copy)]
struct Model {
    document: RwSignal<Option<Document>>,
    page: RwSignal<usize>,
    zoom: RwSignal<f32>,
    image: RwSignal<String>,
    message: RwSignal<String>,
    busy: RwSignal<bool>,
    text: RwSignal<String>,
    note_text: RwSignal<String>,
    notes: RwSignal<Option<Result<NoteList, String>>>,
    note_draft: RwSignal<Option<NoteDraft>>,
    note_edit_text: RwSignal<String>,
    search_query: RwSignal<String>,
    search_case_sensitive: RwSignal<bool>,
    search_result: RwSignal<Option<SearchResponse>>,
    page_jump: RwSignal<String>,
    page_range: RwSignal<String>,
}

impl Model {
    fn loaded(self) -> bool {
        self.document.with(|d| d.is_some())
    }
    fn count(self) -> usize {
        self.document
            .with(|d| d.as_ref().map_or(0, |d| d.info.page_count))
    }
    fn blocked(self) -> bool {
        self.busy.get() || !self.loaded() || self.has_note_draft()
    }
    fn has_note_draft(self) -> bool {
        self.note_draft.with(|draft| draft.is_some())
    }
    fn raster(self) -> Option<RasterPlan> {
        let density = web_sys::window().map_or(1., |w| w.device_pixel_ratio() as f32);
        self.document.with(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get()))
                .map(|p| raster_plan(p.width, p.height, p.rotation, self.zoom.get(), density))
        })
    }
    fn command(self, command: &'static str, args: Value) {
        self.command_at_page(command, args, None);
    }
    fn command_at_page(self, command: &'static str, args: Value, selected_page: Option<usize>) {
        if self.busy.get_untracked() {
            return;
        }
        if self.has_note_draft() {
            self.message.set("请先保存或取消便笺编辑。".into());
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let result: Result<Option<Document>, _> = bridge::call(command, &args).await;
            match result {
                Ok(Some(doc)) => {
                    self.page.update(|p| {
                        *p = selected_page
                            .unwrap_or(*p)
                            .min(doc.info.page_count.saturating_sub(1))
                    });
                    let mut status =
                        command_status_message(command, doc.restored, doc.warning.as_deref());
                    if let Some(editor) = &doc.note_editor {
                        self.page.set(editor.page);
                        self.note_edit_text.set(editor.original.contents.clone());
                        if doc.warning.is_none() {
                            status = "便笺编辑尚未结束；当前显示原始内容。编辑框输入未持久化，请核对后保存或取消。".into();
                        }
                    }
                    self.note_draft.set(doc.note_editor.clone());
                    let page_count = doc.info.page_count;
                    self.document.set(Some(doc));
                    if matches!(
                        command,
                        "open_document"
                            | "new_document"
                            | "edit_document"
                            | "delete_note_document"
                            | "insert_pdf_document"
                            | "undo_document"
                            | "redo_document"
                    ) {
                        self.search_result.set(None);
                    }
                    if page_count > 0 {
                        self.page_jump
                            .set((self.page.get_untracked() + 1).to_string());
                    }
                    if matches!(command, "open_document" | "new_document" | "document_info")
                        && web_sys::window()
                            .and_then(|w| w.inner_width().ok())
                            .and_then(|v| v.as_f64())
                            .is_some_and(|w| w <= 720.)
                    {
                        self.set_fit_width();
                    }
                    self.message.set(status);
                    self.render().await;
                }
                Ok(None) => self.message.set(if command == "document_info" {
                    String::new()
                } else {
                    "已取消，文档保持不变。".into()
                }),
                Err(e) => self.message.set(e),
            }
            self.busy.set(false);
        });
    }
    fn edit(self, edit: Value) {
        self.command("edit_document", json!({"edit":edit}));
    }
    async fn render(self) {
        self.notes.set(None);
        if !self.loaded() {
            return;
        }
        let Some(plan) = self.raster() else {
            return;
        };
        let args = json!({"page":self.page.get_untracked(),"scale":plan.scale});
        match bridge::call::<String>("render_page", &args).await {
            Ok(image) => self.image.set(image),
            Err(e) => {
                self.image.set(String::new());
                self.message.set(e);
            }
        }
        self.refresh_notes().await;
    }
    async fn refresh_notes(self) {
        self.notes.set(Some(
            bridge::call::<NoteList>("notes_document", &json!({"page":self.page.get_untracked()}))
                .await,
        ));
    }
    fn navigate(self, page: usize) {
        if self.busy.get_untracked() || !self.loaded() {
            return;
        }
        if self.has_note_draft() {
            self.message.set("请先保存或取消便笺编辑。".into());
            return;
        }
        if page >= self.count() {
            self.message.set("页码无效，请输入现有页面。".into());
            return;
        }
        self.page.set(page);
        self.page_jump.set((page + 1).to_string());
        self.refresh();
    }
    fn jump_to_page(self) {
        if self.blocked() {
            return;
        }
        match page_jump_index(&self.page_jump.get_untracked(), self.count()) {
            Ok(page) => {
                self.message
                    .update(|message| *message = page_jump_success_message(message));
                self.navigate(page);
            }
            Err(error) => self.message.set(error),
        }
    }
    fn batch_pages(self, operation: &'static str) {
        self.batch_range(self.page_range.get_untracked(), operation);
    }
    fn delete_current_page(self) {
        self.batch_range((self.page.get_untracked() + 1).to_string(), "delete");
    }
    fn move_page(self, delta: i32) {
        if self.blocked() {
            return;
        }
        if let Some((edit, target)) = page_move_edit(self.page.get_untracked(), delta, self.count())
        {
            self.command_at_page("edit_document", json!({"edit":edit}), Some(target));
        }
    }
    fn batch_range(self, range: String, operation: &'static str) {
        if self.blocked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let args = page_batch_args(&range, operation);
            let result = bridge::call::<PageBatchReply>("batch_pages", &args).await;
            match result {
                Ok(PageBatchReply::Applied {
                    document,
                    affected_count,
                }) => {
                    self.page.update(|page| {
                        *page = (*page).min(document.info.page_count.saturating_sub(1))
                    });
                    self.page_jump
                        .set((self.page.get_untracked() + 1).to_string());
                    self.search_result.set(None);
                    self.message.set(if operation == "delete" {
                        format!("已删除 {affected_count} 页，可使用撤销恢复。")
                    } else {
                        format!("已顺时针旋转 {affected_count} 页。")
                    });
                    self.document.set(Some(document));
                    self.render().await;
                }
                Ok(PageBatchReply::Cancelled) => {
                    self.message.set("已取消批量删除，文档保持不变。".into());
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn invalidate_search(self) {
        let summary = self.search_result.with_untracked(|result| {
            result.as_ref().map(|result| {
                search_status_message(&result.query, result.hits.len(), result.truncated)
            })
        });
        self.message
            .update(|message| *message = search_conditions_message(message, summary.as_deref()));
        self.search_result.set(None);
    }
    fn search(self) {
        if self.blocked() {
            return;
        }
        let query = self.search_query.get_untracked();
        if query.trim().is_empty() {
            self.search_result.set(None);
            self.message.set("请输入搜索关键词。".into());
            return;
        }
        let case_sensitive = self.search_case_sensitive.get_untracked();
        let requested_query = query.trim().to_owned();
        self.busy.set(true);
        spawn_local(async move {
            let args = json!({"query":query,"case_sensitive":case_sensitive});
            match bridge::call::<SearchResponse>("search_document", &args).await {
                Ok(result)
                    if search_response_is_current(
                        &requested_query,
                        case_sensitive,
                        &result.query,
                        result.case_sensitive,
                    ) =>
                {
                    self.message.set(search_status_message(
                        &result.query,
                        result.hits.len(),
                        result.truncated,
                    ));
                    self.search_result.set(Some(result));
                }
                Ok(_) => {
                    self.search_result.set(None);
                    self.message.set("搜索条件已改变，请重新搜索。".into());
                }
                Err(error) => {
                    self.search_result.set(None);
                    self.message.set(error);
                }
            }
            self.busy.set(false);
        });
    }
    fn refresh(self) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            self.render().await;
            self.busy.set(false);
        });
    }
    fn zoom_by(self, factor: f32) {
        if self.busy.get_untracked() {
            return;
        }
        self.zoom.update(|z| *z = step_zoom(*z, factor));
        self.refresh();
    }
    fn set_fit_width(self) {
        let Some(reader) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("pdf-reader"))
        else {
            return;
        };
        let available = (reader.client_width() - 40).max(1) as f32;
        if let Some(zoom) = self.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                .map(|p| fit_width_zoom(p.width, p.height, p.rotation, available))
        }) {
            self.zoom.set(zoom);
        }
    }
    fn fit_width(self) {
        if self.blocked() {
            return;
        }
        self.set_fit_width();
        self.refresh();
    }
    fn add_text(self) {
        let text = self.text.get_untracked();
        if text.trim().is_empty() {
            self.message.set("请先输入文字。".into());
            return;
        }
        let (width, height) = self
            .document
            .with_untracked(|d| {
                d.as_ref()
                    .and_then(|d| d.info.pages.get(self.page.get_untracked()))
                    .map(|p| (p.width, p.height))
            })
            .unwrap_or((595., 842.));
        self.edit(json!({"kind":"add_text","page":self.page.get_untracked(),"bounds":{"x":36.,"y":height*0.5,"width":(width-72.).max(20.),"height":60.},"text":text,"font_size":18.,"color":{"r":0.1,"g":0.1,"b":0.1}}));
    }
    fn add_note(self) {
        if self.blocked() {
            return;
        }
        let page = self.page.get_untracked();
        let edit = self.document.with_untracked(|document| {
            document
                .as_ref()
                .and_then(|document| document.info.pages.get(page))
                .map(|info| {
                    note_edit(
                        page,
                        info.width,
                        info.height,
                        info.rotation,
                        &self.note_text.get_untracked(),
                    )
                })
        });
        match edit {
            Some(Ok(edit)) => self.edit(edit),
            Some(Err(error)) => self.message.set(error),
            None => self.message.set("请先打开 PDF。".into()),
        }
    }
    fn delete_note(self, note: TextNote) {
        if self.blocked() {
            return;
        }
        self.command("delete_note_document", json!({"page":self.page.get_untracked(),"annotation_index":note.annotation_index,"expected_contents":note.contents}));
    }
    fn begin_note_edit(self, note: TextNote) {
        if self.blocked() {
            return;
        }
        self.busy.set(true);
        let args = json!({"page":self.page.get_untracked(),"annotation_index":note.annotation_index,"expected_contents":note.contents});
        spawn_local(async move {
            // Expose input only after the native guard acknowledges this snapshot.
            match bridge::call::<NoteDraft>("begin_note_editor", &args).await {
                Ok(editor) => {
                    self.note_edit_text.set(editor.original.contents.clone());
                    self.note_draft.set(Some(editor));
                    self.message
                        .set("编辑便笺中；保存修改后仍需导出 PDF。".into());
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn cancel_note_edit(self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Document>("cancel_note_editor", &json!({"id":draft.id})).await {
                Ok(document) => {
                    self.document.set(Some(document));
                    self.note_draft.set(None);
                    self.note_edit_text.set(String::new());
                    self.message
                        .set("已取消便笺编辑；当前 PDF 会话已刷新。".into());
                    self.render().await;
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
    fn save_note(self) {
        if self.busy.get_untracked() || !self.loaded() {
            return;
        }
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        if draft.page != self.page.get_untracked() {
            self.message
                .set("页面已改变；请取消后重新选择便笺。".into());
            return;
        }
        let contents = self.note_edit_text.get_untracked();
        if let Err(error) = update_note_edit(
            draft.page,
            draft.original.annotation_index,
            &draft.original.contents,
            &contents,
        ) {
            self.message.set(error);
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Document>(
                "commit_note_editor",
                &json!({"id":draft.id,"contents":contents}),
            )
            .await
            {
                Ok(document) => {
                    self.message.set(
                        document
                            .warning
                            .clone()
                            .unwrap_or_else(|| "便笺已修改；请导出副本保存。".into()),
                    );
                    self.document.set(Some(document));
                    self.note_draft.set(None);
                    self.note_edit_text.set(String::new());
                    self.search_result.set(None);
                    self.render().await;
                }
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
}

#[component]
pub fn Workspace() -> impl IntoView {
    let m = Model {
        document: RwSignal::new(None),
        page: RwSignal::new(0),
        zoom: RwSignal::new(1.),
        image: RwSignal::new(String::new()),
        message: RwSignal::new(String::new()),
        busy: RwSignal::new(false),
        text: RwSignal::new(String::new()),
        note_text: RwSignal::new(String::new()),
        notes: RwSignal::new(None),
        note_draft: RwSignal::new(None),
        note_edit_text: RwSignal::new(String::new()),
        search_query: RwSignal::new(String::new()),
        search_case_sensitive: RwSignal::new(false),
        search_result: RwSignal::new(None),
        page_jump: RwSignal::new(String::new()),
        page_range: RwSignal::new(String::new()),
    };
    m.command("document_info", json!({}));
    view! {
        <main class="workspace">
            <header class="titlebar">
                <div class="identity"><span class="app-mark">"P"</span><span>"PDF Master"</span><small>"RUST"</small></div>
                <span class="document-title">{move||m.document.with(|d|d.as_ref().map_or("文档工作台".into(),|d|format!("{}{}",d.name,if d.info.dirty{" · 未保存"}else{""})))}</span>
                <span class="build-state">"迁移候选 · 非正式版"</span>
            </header>
            <nav class="commandbar" aria-label="文档操作">
                <button class="primary" disabled=move||m.busy.get() on:click=move|_|m.command("open_document",json!({}))>"打开 PDF"</button>
                <button disabled=move||m.busy.get() on:click=move|_|m.command("new_document",json!({}))>"新建"</button>
                <span class="separator"></span>
                <button disabled=move||m.blocked() on:click=move|_|m.command("export_document",json!({}))>"导出副本"</button>
                <button disabled=move||m.blocked()||!m.document.with(|d|d.as_ref().is_some_and(|d|d.info.can_undo)) on:click=move|_|m.command("undo_document",json!({}))>"↶ 撤销"</button>
                <button disabled=move||m.blocked()||!m.document.with(|d|d.as_ref().is_some_and(|d|d.info.can_redo)) on:click=move|_|m.command("redo_document",json!({}))>"↷ 重做"</button>
                <span class="flex"></span>
                <button aria-label="缩小" disabled=move||m.blocked() on:click=move|_|m.zoom_by(0.8)>"−"</button>
                <output class="zoom">{move||format!("{:.0}%",m.zoom.get()*100.)}</output>
                <button aria-label="放大" disabled=move||m.blocked() on:click=move|_|m.zoom_by(1.25)>"+"</button>
                <button disabled=move||m.blocked() on:click=move|_|m.fit_width()>"适合宽度"</button>
            </nav>
            <aside class="pages" aria-label="页面列表">
                <div class="section-label">"页面"<span>{move||m.count()}</span></div>
                <For each={move || (0..m.count()).collect::<Vec<_>>()} key=|p|*p children=move|p|view!{
                    <button class="page-entry" class:selected=move||m.page.get()==p disabled=move||m.busy.get() on:click=move|_|m.navigate(p)>
                        <span class="page-symbol">{p+1}</span><span>{format!("第 {} 页",p+1)}</span>
                    </button>
                }/>
                <Show when=move||!m.loaded()><p class="muted sidebar-hint">"打开文档后，在这里切换页面。"</p></Show>
            </aside>
            <section id="pdf-reader" class="reader" aria-label="PDF 阅读区域" aria-busy=move||m.busy.get().to_string()>
                <Show when=move||m.loaded() fallback=move||view!{
                    <div class="empty-state"><div class="paper-emblem">"PDF"</div><h1>"让文档回到工作中心。"</h1><p>"打开一份 PDF，或从空白页开始。"</p><button class="primary" disabled=move||m.busy.get() on:click=move|_|m.command("open_document",json!({}))>"选择 PDF 文件"</button><div class="safety-note">"仅在会话副本中编辑，原文件不会自动覆盖。"</div></div>
                }>
                    <div class="paper-frame"><Show when=move||!m.image.get().is_empty() fallback=move||view!{<p class="render-placeholder">"等待页面渲染"</p>}><img class="pdf-page" src=move||m.image.get() style=move||m.raster().map(|p|p.image_style()).unwrap_or_default() alt=move||format!("PDF 第 {} 页",m.page.get()+1) draggable="false"/></Show></div>
                </Show>
            </section>
            <aside class="inspector">
                <h2>"文档工具"</h2>
                <div class="tool-group search-panel"><h3>"文档搜索"</h3>
                    <input aria-label="搜索文档文字" disabled=move||m.busy.get() placeholder="输入关键词" prop:value=move||m.search_query.get() on:input=move|e|{m.search_query.set(event_target_value(&e));m.invalidate_search();}/>
                    <label class="search-case"><input type="checkbox" disabled=move||m.busy.get() prop:checked=move||m.search_case_sensitive.get() on:change=move|e|{m.search_case_sensitive.set(event_target_checked(&e));m.invalidate_search();}/><span>"区分大小写"</span></label>
                    <button disabled=move||m.blocked() on:click=move|_|m.search()>"搜索当前文档"</button>
                    <Show when=move||m.search_result.with(|r|r.is_some())>
                        <div class="search-meta">{move||m.search_result.with(|r|r.as_ref().map(|r|format!("已扫描 {} 页 · {} 页含文字{}",r.pages_scanned,r.searchable_pages,if r.case_sensitive{" · 区分大小写"}else{""})).unwrap_or_default())}</div>
                        <div class="search-results">
                            <For each=move||m.search_result.with(|r|r.as_ref().map(|r|r.hits.clone().into_iter().enumerate().collect::<Vec<_>>()).unwrap_or_default()) key=|(index,hit)|(*index,hit.page_index,hit.match_start) children=move|(index,hit)|{
                                let target=hit.page_index;
                                view!{<button class="search-hit" disabled=move||m.busy.get() on:click=move|_|m.navigate(target)><strong>{format!("第 {} 页 · 命中 {}",hit.page_index+1,index+1)}</strong><span>{hit.snippet}</span><small>{format!("字符 {}–{} · 片段内 {}–{}",hit.match_start,hit.match_end,hit.snippet_match_start,hit.snippet_match_end)}</small></button>}
                            }/>
                        </div>
                        <p class="muted search-limit">{move||m.search_result.with(|r|r.as_ref().map(|r|r.limitation.clone()).unwrap_or_default())}</p>
                    </Show>
                </div>
                <div class="tool-group"><h3>"页面"</h3>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"rotate","page":m.page.get_untracked(),"degrees":90}))>"顺时针旋转 90°"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"duplicate_page","page":m.page.get_untracked(),"index":m.page.get_untracked()+1}))>"复制当前页"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"insert_blank","index":m.page.get_untracked()+1,"width":595.,"height":842.}))>"在后方插入空白页"</button>
                    <button disabled=move||m.blocked()||m.page.get()==0 on:click=move|_|m.move_page(-1)>"当前页前移"</button>
                    <button disabled={move||m.blocked()||m.page.get()+1>=m.count()} on:click=move|_|m.move_page(1)>"当前页后移"</button>
                    <button class="danger" disabled={move || m.blocked()||m.count()<=1} on:click=move|_|m.delete_current_page()>"删除当前页"</button>
                </div>
                <div class="tool-group"><h3>"批量页面"</h3>
                    <input aria-label="批量页码范围" disabled=move||m.busy.get() placeholder="例如 1-3,5" prop:value=move||m.page_range.get() on:input=move|e|m.page_range.set(event_target_value(&e))/>
                    <button disabled=move||m.blocked() on:click=move|_|m.batch_pages("rotate_90")>"批量顺时针旋转 90°"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.command("extract_pages_document",json!({"range":m.page_range.get_untracked()}))>"提取页面到新 PDF"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.command("insert_pdf_document",json!({"index":m.page.get_untracked()+1}))>"在当前页后插入 PDF"</button>
                    <button class="danger" disabled=move||m.blocked()||m.count()<=1 on:click=move|_|m.batch_pages("delete")>"批量删除"</button>
                    <p class="muted">"页码从 1 开始；删除会先显示选中页数并要求确认。"</p>
                </div>
                <div class="tool-group"><h3>"文字批注"</h3><textarea aria-label="文字批注内容" placeholder="输入要添加的文字" prop:value=move||m.text.get() on:input=move|e|m.text.set(event_target_value(&e))></textarea><button disabled=move||m.blocked() on:click=move|_|m.add_text()>"添加到页面中部"</button><p class="muted">"字体与复杂 PDF 兼容性仍在迁移验证中。"</p></div>
                <div class="tool-group notes-panel"><h3>"便笺与评论"</h3>
                    <textarea aria-label="便笺内容" disabled=move||m.busy.get() placeholder="支持中文和 emoji；保存为 PDF 便笺" prop:value=move||m.note_text.get() on:input=move|event|m.note_text.set(event_target_value(&event))></textarea>
                    <button disabled=move||m.blocked() on:click=move|_|m.add_note()>"添加便笺到左上方"</button>
                    <div class="note-list" aria-label="当前页便笺">
                        {move||match m.notes.get() {
                            None => view!{<p class="note-meta">{if m.loaded(){"正在读取当前页便笺…"}else{"打开 PDF 后显示当前页便笺。"}}</p>}.into_any(),
                            Some(Err(error)) => view!{<p class="note-error">{format!("无法读取批注：{error}")}</p>}.into_any(),
                            Some(Ok(list)) => view!{
                                <p class="note-meta">{format!("当前页 {} 条便笺",list.notes.len())}</p>
                                <p class="note-meta">{if list.unsupported_count>0{format!("另有 {} 条其他类型批注尚不支持显示。",list.unsupported_count)}else{String::new()}}</p>
                                {list.notes.into_iter().map(|note| {let edit_note=note.clone();view!{
                                    <article class="note-item"><strong>{format!("便笺 {}",note.annotation_index+1)}</strong><p>{note.contents.clone()}</p><small>{format!("PDF 坐标 {:.0}, {:.0} · {:.0} × {:.0} pt",note.bounds.x,note.bounds.y,note.bounds.width,note.bounds.height)}</small><button disabled=move||m.blocked() on:click=move|_|m.begin_note_edit(edit_note.clone())>"编辑便笺"</button><button class="danger" disabled=move||m.blocked() on:click=move|_|m.delete_note(note.clone())>"删除便笺…"</button></article>
                                }}).collect_view()}
                            }.into_any(),
                        }}
                    </div>
                    <Show when=move||m.has_note_draft()>
                        <div class="note-editor" aria-label="便笺编辑">
                            <textarea aria-label="编辑便笺内容" disabled=move||m.busy.get() prop:value=move||m.note_edit_text.get() on:input=move|event|m.note_edit_text.set(event_target_value(&event))></textarea>
                            <button disabled=move||m.busy.get() on:click=move|_|m.save_note()>"保存便笺修改"</button>
                            <button disabled=move||m.busy.get() on:click=move|_|m.cancel_note_edit()>"取消便笺编辑"</button>
                            <p class="note-meta">"编辑期间阻止正常关闭；输入未自动持久化，崩溃或强制退出仍可能丢失。保存后需导出 PDF。"</p>
                        </div>
                    </Show>
                    <Show when=move||m.loaded()&&!m.has_note_draft()>
                        <button disabled=move||m.busy.get() on:click=move|_|m.command("document_info",json!({}))>"同步便笺编辑状态"</button>
                        <p class="note-meta">"编辑确认未返回时，可恢复当前进程的编辑入口；不会恢复未持久化的输入。"</p>
                    </Show>
                    <p class="muted">"便笺文字保存为 Unicode 元数据；图标不是正文文字。仅支持受限 Text 便笺；标签、回复线程及歧义弹窗关系暂不编辑或删除。关系检查最多 16384 条。定位和弹窗显示仍在迁移；修改和删除均不是安全擦除。"</p>
                </div>
                <div class="migration-note"><h3>"完整迁移进行中"</h3><p>"本候选用于验证 Rust 文档核心。画布、AI、OCR 和完整编辑工具尚未迁移，不会显示为已完成。"</p></div>
            </aside>
            <footer class="statusbar"><span role="status" aria-live="polite">{move||m.document.with(|d|visible_status(d.as_ref().and_then(|d|d.warning.as_deref()), &m.message.get(), m.busy.get()))}</span><span class="flex"></span><Show when=move||m.loaded()><button aria-label="上一页" disabled=move||m.blocked()||m.page.get()==0 on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),-1,m.count()))>"‹"</button><span>{move||format!("{} / {}",m.page.get()+1,m.count())}</span><button aria-label="下一页" disabled={move || m.blocked()||m.page.get()+1>=m.count()} on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),1,m.count()))>"›"</button><label class="page-jump-label"><span>"页码"</span><input aria-label="跳转页码" inputmode="numeric" prop:value=move||m.page_jump.get() on:input=move|e|m.page_jump.set(event_target_value(&e))/></label><button class="page-jump-button" disabled=move||m.blocked() on:click=move|_|m.jump_to_page()>"转到"</button><span class="page-dimensions">{move||m.document.with(|d|d.as_ref().and_then(|d|d.info.pages.get(m.page.get())).map(|p|format!("{:.0} × {:.0} pt · {}°",p.width,p.height,p.rotation)).unwrap_or_default())}</span></Show></footer>
        </main>
    }
}
