use super::Model;
use leptos::prelude::*;
use pdfmaster_ui::shell::{GROUPS, HomeSection, RECOMMENDED, Route, TOOLS, Tool, tool};
use serde_json::json;

#[component]
fn ToolCard(model: Model, tool: Tool) -> impl IntoView {
    let available = tool.id == "blank" || tool.workbench_target.is_some();
    view! {
        <button class="home-tool-card" aria-label=tool.title
            title=move||if !available { "此功能尚未迁移" } else if tool.id=="blank" { "新建空白 PDF" } else if !model.loaded() { "请先打开 PDF" } else { "进入当前文档的已迁移工具；原版模式工具栏仍待迁移" }
            disabled=move||!available||model.busy.get()||(tool.id!="blank"&&!model.loaded())
            on:click=move|_|model.show_tool(tool.id)>
            <svg viewBox="0 0 32 32" class="home-tool-icon" style:color=tool.color aria-hidden="true">
                <path d=match tool.id {
                    "pages"=>"M4 4h9v9H4z M19 4h9v9h-9z M4 19h9v9H4z M19 19h9v9h-9z",
                    "annotate"|"draw"|"sign"=>"M7 23 22 5l5 5L12 28H5z M19 9l5 5 M5 28h22",
                    "copilot"=>"M16 3l3 10 10 3-10 3-3 10-3-10-10-3 10-3z",
                    "measure"=>"M4 22 22 4l6 6-18 18z M10 16l3 3 M15 11l3 3 M20 6l3 3",
                    "password"=>"M8 14h16v14H8z M11 14V9a5 5 0 0 1 10 0v5 M16 20v3",
                    "comments"=>"M5 5h22v17H14l-7 6v-6H5z M10 11h12 M10 16h9",
                    "image-to-pdf"|"pdf-to-image"=>"M4 6h24v21H4z M5 24l8-10 5 5 4-4 6 8 M23 11h.1",
                    "redline"=>"M5 27 27 5",
                    "merge"=>"M6 11 16 5l10 6-10 6z M6 17l10 6 10-6 M6 23l10 6 10-6",
                    _=>"M8 3h11l6 6v20H8z M19 3v7h6 M12 16h9 M12 21h9",
                } fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
            <span>{tool.title}</span>
        </button>
    }
}

#[component]
fn EmptyCollection(favorite: bool) -> impl IntoView {
    view! {
        <div class="home-empty-collection">
            <svg viewBox="0 0 78 54" aria-hidden="true"><g transform="translate(0 54) scale(1 -1)" fill="none" stroke="#303030" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"><path d="M10 5 7 44Q7 49 12 49H56Q61 49 61 44V38"/><path d=if favorite {"M61 38V5 M10 5"} else {"M10 5 20 37Q21 40 25 40H72L61 5"}/></g><Show when=move||favorite><path d="m50 14 2 5 6 .5-4 4 1 6-5-3-5 3 1-6-4-4 6-.5z" fill="#303030"/></Show></svg>
            <p>{if favorite {"收藏记录尚未迁移；不会读取或覆盖原版收藏。"}else{"最近记录尚未迁移；不会读取或覆盖原版记录。"}}</p>
        </div>
    }
}

#[component]
pub(super) fn Dashboard(model: Model) -> impl IntoView {
    let section = move || match model.route.get() {
        Route::Home(s) => s,
        Route::Document => HomeSection::Home,
    };
    view! {
        <div class="dashboard">
            <nav class="home-sidebar" aria-label="主界面导航">
                {HomeSection::ALL.into_iter().enumerate().map(|(index,item)|view!{
                    <button class="home-nav" class:bottom-start=index==6 class:selected=move||section()==item aria-current=move||if section()==item {Some("page")}else{None} on:click=move|_|model.switch_to(Route::Home(item))>
                        {if item==HomeSection::Home { view!{<img class="home-nav-brand" src="32x32.png" alt=""/>}.into_any() } else {view!{<span class="home-nav-symbol" aria-hidden="true">{match item {HomeSection::Tools=>"▱",HomeSection::Canvas=>"∞",HomeSection::OnlineCanvas=>"◎",HomeSection::Recent=>"◷",HomeSection::Favorites=>"★",HomeSection::Account=>"♙",HomeSection::Settings=>"⚙",_=>"⊙"}}</span>}.into_any()}}
                        <span>{item.title()}</span>
                    </button>
                }).collect_view()}
            </nav>
            <section class="home-scroll" id="home-scroll" aria-label="首页内容">
                <div class="home-column" class:tools-column=move||section()!=HomeSection::Home>
                    <div class="home-heading-row"><h1>{move||section().title()}</h1><Show when=move||section()==HomeSection::Home><button class="home-customize" disabled title="工具自定义尚未迁移">"自定义"</button></Show></div>
                    <Show when=move||section()==HomeSection::Home>
                        <div class="home-banner">"PDF Master · 阅读、批注、整理，让文档与灵感井然有序"</div>
                        <div class="home-drop-zone" class:drop-active=move||model.home_drop_active.get() aria-label="拖入 PDF 或支持的图片，新建独立合并文档" on:dragover=move|e|e.prevent_default() on:drop=move|e|e.prevent_default()>
                            <svg class="home-pdf-stack" viewBox="0 0 85 64" aria-hidden="true"><g transform="translate(0 64) scale(1 -1)" stroke="#454545" stroke-width="0.8" fill="#303030"><g transform="translate(30.5 29) rotate(-15)"><rect x="-21" y="-27" width="42" height="48" rx="1"/><path d="M-17 12h34 M-17 8h34 M-17 4h34 M-17 0h34 M-17 -4h34"/></g><g transform="translate(54.5 29) rotate(15)"><rect x="-21" y="-27" width="42" height="48" rx="1"/><path d="M-17 12h34 M-17 8h34 M-17 4h34 M-17 0h34 M-17 -4h34"/></g><rect x="20.5" y="3" width="44" height="60" rx="2" fill="#333" stroke-width="1"/></g><text x="42.5" y="50" text-anchor="middle" fill="#666" font-size="14" font-weight="700">"PDF"</text></svg>
                            <button class="home-open" disabled=move||model.busy.get() on:click=move|_|model.command("open_document",json!({}))>"打开文件"</button>
                            <p>"拖入 PDF 或图片，按顺序创建独立 PDF"</p>
                            <Show when=move||model.file_drop_request.get().is_some()><div class="home-import-progress"><span class="home-import-spinner" aria-hidden="true"></span><span>"正在本地导入文件…"</span><button aria-label="取消文件导入" title="取消当前文件导入，不修改原文件" on:click=move|_|model.cancel_file_import()>"取消"</button></div></Show>
                        </div>
                        <div class="home-heading-row"><h2>"工具专为您"</h2><button class="home-all-tools" on:click=move|_|model.switch_to(Route::Home(HomeSection::Tools))>"查看全部"</button></div>
                        <div class="home-tool-grid">{RECOMMENDED.into_iter().filter_map(tool).map(|tool|view!{<ToolCard model=model tool=tool/>}).collect_view()}</div>
                        <h2>"最近"</h2><EmptyCollection favorite=false/>
                        <h2>"收藏"</h2><EmptyCollection favorite=true/>
                    </Show>
                    <Show when=move||section()==HomeSection::Tools>
                        {GROUPS.into_iter().enumerate().map(|(group,title)|view!{<h2>{title}</h2><div class="home-tool-grid">{TOOLS.into_iter().filter(move|t|t.group==group).map(|tool|view!{<ToolCard model=model tool=tool/>}).collect_view()}</div>}).collect_view()}
                        <p class="home-disclosure">"灰色入口尚未迁移，或需要先打开 PDF。可用入口进入当前文档；工具图标和文档模式工具栏仍待原版对齐。"</p>
                    </Show>
                    <Show when=move||section()==HomeSection::Recent><EmptyCollection favorite=false/></Show>
                    <Show when=move||section()==HomeSection::Favorites><EmptyCollection favorite=true/></Show>
                    <Show when=move||!matches!(section(),HomeSection::Home|HomeSection::Tools|HomeSection::Recent|HomeSection::Favorites)><p class="home-disclosure">"此页面仍在迁移中。原版数据保持不变，本候选不会显示虚构的账号、设置或画布。"</p></Show>
                    <p class="home-disclosure">"Rust 迁移候选 · 非正式版 · 多 PDF 标签保留独立修改，画布标签与完整三端 UI 对等仍在迁移。"</p>
                </div>
            </section>
            <footer class="home-status"><span role="status" aria-live="polite">{move||model.document.with(|d|pdfmaster_ui::visible_status(d.as_ref().and_then(|d|d.warning.as_deref()),&model.message.get(),model.busy.get()))}</span><Show when=move||model.file_drop_request.get().is_some()&&!matches!(model.route.get(),Route::Home(HomeSection::Home))><button aria-label="取消文件导入" on:click=move|_|model.cancel_file_import()>"取消文件导入"</button></Show></footer>
        </div>
    }
}
