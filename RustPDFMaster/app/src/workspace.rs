#[path = "bridge.rs"]
mod bridge;
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    RasterPlan, adjacent_page, command_status_message, fit_width_zoom, raster_plan, step_zoom,
    visible_status,
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
        self.busy.get() || !self.loaded()
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
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let result: Result<Option<Document>, _> = bridge::call(command, &args).await;
            match result {
                Ok(Some(doc)) => {
                    self.page
                        .update(|p| *p = (*p).min(doc.info.page_count.saturating_sub(1)));
                    let status =
                        command_status_message(command, doc.restored, doc.warning.as_deref());
                    self.document.set(Some(doc));
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
    }
    fn navigate(self, page: usize) {
        if self.busy.get_untracked() || !self.loaded() {
            return;
        }
        self.page.set(page.min(self.count().saturating_sub(1)));
        self.refresh();
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
                <h2>"文档工具"</h2><div class="tool-group"><h3>"页面"</h3>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"rotate","page":m.page.get_untracked(),"degrees":90}))>"顺时针旋转 90°"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"duplicate_page","page":m.page.get_untracked(),"index":m.page.get_untracked()+1}))>"复制当前页"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.edit(json!({"kind":"insert_blank","index":m.page.get_untracked()+1,"width":595.,"height":842.}))>"在后方插入空白页"</button>
                    <button class="danger" disabled={move || m.blocked()||m.count()<=1} on:click=move|_|m.edit(json!({"kind":"delete_page","page":m.page.get_untracked()}))>"删除当前页"</button>
                </div>
                <div class="tool-group"><h3>"文字批注"</h3><textarea aria-label="文字批注内容" placeholder="输入要添加的文字" prop:value=move||m.text.get() on:input=move|e|m.text.set(event_target_value(&e))></textarea><button disabled=move||m.blocked() on:click=move|_|m.add_text()>"添加到页面中部"</button><p class="muted">"字体与复杂 PDF 兼容性仍在迁移验证中。"</p></div>
                <div class="migration-note"><h3>"完整迁移进行中"</h3><p>"本候选用于验证 Rust 文档核心。画布、AI、OCR 和完整编辑工具尚未迁移，不会显示为已完成。"</p></div>
            </aside>
            <footer class="statusbar"><span role="status" aria-live="polite">{move||m.document.with(|d|visible_status(d.as_ref().and_then(|d|d.warning.as_deref()), &m.message.get(), m.busy.get()))}</span><span class="flex"></span><Show when=move||m.loaded()><button aria-label="上一页" disabled=move||m.blocked()||m.page.get()==0 on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),-1,m.count()))>"‹"</button><span>{move||format!("{} / {}",m.page.get()+1,m.count())}</span><button aria-label="下一页" disabled={move || m.blocked()||m.page.get()+1>=m.count()} on:click=move|_|m.navigate(adjacent_page(m.page.get_untracked(),1,m.count()))>"›"</button><span class="page-dimensions">{move||m.document.with(|d|d.as_ref().and_then(|d|d.info.pages.get(m.page.get())).map(|p|format!("{:.0} × {:.0} pt · {}°",p.width,p.height,p.rotation)).unwrap_or_default())}</span></Show></footer>
        </main>
    }
}
