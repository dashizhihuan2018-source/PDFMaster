use super::Model;
use leptos::prelude::*;
use pdfmaster_ui::workbench::{Action, Mode, PRIMARY_MODES, Panel, tools};
use serde_json::json;

// Independent vectors preserve the original control roles. Exact SF Symbol
// skin matching is still an explicit parity gap, not claimed by these paths.
#[component]
fn Icon(name: &'static str) -> impl IntoView {
    view! { <svg viewBox="0 0 24 24" class="workbench-icon" aria-hidden="true"><path d=match name {
        "sidebar"=>"M3 4h18v16H3z M9 4v16 M5 8h2 M5 12h2",
        "overview"=>"M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
        "layout"=>"M12 5C8 3 5 3 2 5v15c4-2 7-2 10 0 3-2 6-2 10 0V5c-3-2-6-2-10 0z M12 5v15",
        "minus"=>"M5 12h14", "plus"=>"M5 12h14 M12 5v14",
        "fit"=>"M3 9V3h6 M15 3h6v6 M21 15v6h-6 M9 21H3v-6 M3 3l6 6 M21 3l-6 6 M21 21l-6-6 M3 21l6-6",
        "more"=>"M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18 M7 12h10 M12 7v10",
        "copilot"=>"m12 2 2.5 7.5L22 12l-7.5 2.5L12 22l-2.5-7.5L2 12l7.5-2.5z",
        "translate"=>"M2 5h12 M8 2v3 M5 8c1 5 4 7 8 9 M12 5c-1 6-4 10-9 13 M14 22l4-12 4 12 M16 18h4",
        "share"=>"M5 10v11h14V10 M12 15V2 M7 7l5-5 5 5",
        "previous"=>"m14 5-7 7 7 7", "next"=>"m10 5 7 7-7 7",
        "highlight"=>"m6 15 9-12 5 5-9 12H5z M3 22h13",
        "underline"=>"M6 3v9a6 6 0 0 0 12 0V3 M4 22h16",
        "strikeout"=>"M18 5C10-2 3 7 10 10 M14 14c8 6-1 12-8 6 M3 12h18",
        "pen"=>"M7 18 18 3l4 4-11 15H4z M14 8l5 4",
        "eraser"=>"m3 14 11-11 8 8-11 11H9z M9 8l8 8 M11 22h11",
        "text"|"content-text"=>"M3 4h18v16H3z M7 8h10 M12 8v9 M9 17h6",
        "shapes"=>"M3 3h12v12H3z M13 9a6 6 0 1 1-4 4",
        "note"=>"M3 4h18v13H10l-5 5v-5H3z M7 9h10 M7 13h7",
        "stamp"=>"M9 3h6v8l4 4v4H5v-4l4-4z M4 22h16",
        "signature"=>"M2 18c4-1 10-16 13-15 3 1-8 19-10 17-2-3 15-5 17-2",
        "selection"=>"M3 8V3h5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5",
        "rotate"=>"M4 12a8 8 0 1 0 3-6 M3 2v6h6",
        "delete-page"=>"M4 6h16 M9 3h6 M6 6l1 16h10l1-16 M10 10v8 M14 10v8",
        "save"=>"M4 14v7h16v-7 M12 2v14 M7 11l5 5 5-5",
        "password"=>"M5 11h14v11H5z M8 11V6a4 4 0 0 1 8 0v5",
        "content-image"=>"M3 3h18v18H3z M4 18l6-7 5 5 3-3 3 4 M16 7h.1",
        "content-link"=>"m10 7 3-3a5 5 0 0 1 7 7l-3 3 M14 17l-3 3a5 5 0 0 1-7-7l3-3 M8 16l8-8",
        _=>"M5 2h10l4 4v16H5z M15 2v5h4 M8 12h8 M8 16h8",
    } fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg> }
}

#[component]
pub(super) fn Toolbar(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <nav class="workbench-toolbar" aria-label="阅读工具栏">
            <div class="workbench-group workbench-left">
                <button class="chrome-icon-button" aria-label="侧栏" title="侧栏" aria-pressed=move||m.chrome.get().sidebar.to_string() disabled=move||m.blocked() on:click=move|_|m.toggle_sidebar()><Icon name="sidebar"/></button>
                <button class="chrome-icon-button" aria-label="页面预览" title="缩略图总览尚未迁移" disabled><Icon name="overview"/></button>
                <details class="chrome-popup"><summary aria-label="布局选项" title="布局选项"><Icon name="layout"/></summary><div class="chrome-popup-content">
                    <strong>"布局选项"</strong><button disabled=move||m.blocked() on:click=move|_|m.fit_page()>"适合页面"</button><button disabled=move||m.blocked() on:click=move|_|m.fit_width()>"适合宽度"</button>
                    <button disabled aria-pressed="true">"单页（当前）"</button><button disabled title="连续页布局尚未迁移">"连续滚动"</button><button disabled title="双页布局尚未迁移">"双页"</button><button disabled>"封面"</button>
                </div></details>
                <select class="chrome-zoom" aria-label="缩放" disabled=move||m.blocked() prop:value=move||{let _=m.zoom.get();"current"} on:change=move|e|m.choose_zoom(&event_target_value(&e))>
                    <option value="current">{move||format!("{:.0}%",m.zoom.get()*100.)}</option><option value="page">"适合页面"</option><option value="width">"适合宽度"</option><option value="1">"100%"</option><option value="1.25">"125%"</option><option value="1.5">"150%"</option><option value="2">"200%"</option><option value="3">"300%"</option>
                </select>
                <button class="chrome-icon-button" aria-label="缩小" title="缩小" disabled=move||m.blocked() on:click=move|_|m.zoom_by(0.8)><Icon name="minus"/></button>
                <button class="chrome-icon-button" aria-label="放大" title="放大" disabled=move||m.blocked() on:click=move|_|m.zoom_by(1.25)><Icon name="plus"/></button>
                <button class="chrome-icon-button" aria-label="适合页面" title="适合页面" disabled=move||m.blocked() on:click=move|_|m.fit_page()><Icon name="fit"/></button>
            </div>
            <div class="workbench-group workbench-modes" aria-label="文档模式">
                {PRIMARY_MODES.into_iter().map(|(title,mode)|view!{
                    <button class="chrome-mode" class:active=move||mode.is_some()&&m.chrome.get().mode==mode aria-pressed=move||(mode.is_some()&&m.chrome.get().mode==mode).to_string()
                        disabled=move||mode.is_none()||m.blocked() title=if mode.is_none(){"此功能尚未迁移"}else{"选择模式；仅可用的二级工具会执行操作"} on:click=move|_|{if let Some(mode)=mode{m.select_mode(mode);}}>{title}</button>
                }).collect_view()}
                <details class="chrome-popup"><summary aria-label="更多工具" title="更多工具"><Icon name="more"/></summary><div class="chrome-popup-content">
                    <button disabled=move||m.blocked() on:click=move|_|m.select_mode(Mode::Pages)>"页面管理"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.select_panel(Some(Panel::Batch))>"批量页面"</button>
                    <button disabled=move||m.blocked() on:click=move|_|m.select_panel(Some(Panel::File))>"文档操作"</button>
                </div></details>
            </div>
            <div class="workbench-group workbench-right">
                <button class="chrome-icon-button" aria-label="PDF Copilot" title="PDF Copilot 尚未迁移" disabled><Icon name="copilot"/></button>
                <button class="chrome-icon-button" aria-label="翻译" title="翻译尚未迁移" disabled><Icon name="translate"/></button>
                <button class="chrome-icon-button" aria-label="分享" title="分享尚未迁移" disabled><Icon name="share"/></button>
                <input class="chrome-search" aria-label="搜索文档文字" placeholder="搜索" disabled=move||m.blocked() prop:value=move||m.search_query.get()
                    on:focus=move|_|m.select_panel(Some(Panel::Search)) on:input=move|e|{m.search_query.set(event_target_value(&e));m.invalidate_search();}
                    on:keydown=move|e|{if e.key()=="Enter"{m.search();}}/>
                <button class="chrome-icon-button" aria-label="上个结果" title="上一处" disabled=move||m.blocked()||!m.search_result.with(|r|r.as_ref().is_some_and(|r|!r.hits.is_empty())) on:click=move|_|m.step_search_hit(true)><Icon name="previous"/></button>
                <button class="chrome-icon-button" aria-label="下个结果" title="下一处" disabled=move||m.blocked()||!m.search_result.with(|r|r.as_ref().is_some_and(|r|!r.hits.is_empty())) on:click=move|_|m.step_search_hit(false)><Icon name="next"/></button>
            </div>
        </nav>
    }
}

#[component]
pub(super) fn SecondaryTools(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <nav class="workbench-secondary" aria-label="模式工具" hidden=move||m.chrome.get().mode.is_none()>
            {move||m.chrome.get().mode.map(|mode|tools(mode).iter().copied().map(|tool|view!{
                <button class="chrome-strip-tool" class:icon-only=!tool.shows_title class:danger=tool.id=="delete-page"
                    class:active=move||matches!(tool.action,Some(Action::Panel(p))if m.chrome.get().panel==Some(p))
                    aria-label=format!("工具 · {}",tool.title) title=if tool.action.is_none(){format!("{}：尚未迁移",tool.title)}else{format!("{}：有限迁移工具，页面内完整交互仍待",tool.title)}
                    disabled=move||m.blocked()||tool.action.is_none() on:click=move|_|{if let Some(action)=tool.action{m.run_strip_action(action);}}>
                    <Icon name=tool.id/>{tool.shows_title.then_some(tool.title)}
                </button>
            }).collect_view())}
        </nav>
    }
}

#[component]
pub(super) fn FileActions(model: Model) -> impl IntoView {
    let m = model;
    view! { <div class="tool-group" hidden=move||m.chrome.get().panel!=Some(Panel::File)><h3>"文档操作"</h3>
        <button disabled=move||m.blocked() on:click=move|_|m.command("open_document",json!({}))>"打开 PDF"</button>
        <button disabled=move||m.blocked() on:click=move|_|m.command("new_document",json!({}))>"新建"</button>
        <button disabled=move||m.blocked() on:click=move|_|m.command("export_document",json!({}))>"导出副本"</button>
        <button disabled=move||m.blocked()||!m.document.with(|d|d.as_ref().is_some_and(|d|d.info.can_undo)) on:click=move|_|m.command("undo_document",json!({}))>"撤销"</button>
        <button disabled=move||m.blocked()||!m.document.with(|d|d.as_ref().is_some_and(|d|d.info.can_redo)) on:click=move|_|m.command("redo_document",json!({}))>"重做"</button>
        <p class="muted">"当前仅有一份文档会话；替换前须保存已有修改。完整原生菜单和快捷键仍在迁移。"</p>
    </div> }
}
