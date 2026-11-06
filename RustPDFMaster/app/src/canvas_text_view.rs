use super::canvas_text_controller::font_stack;
use super::*;
use pdfmaster_ui::canvas_text::{Alignment, Font, Handle, Phase};

fn editor_style(m: Model) -> String {
    m.free_text.with(|s|{
        let Some(d)=&s.draft else{return String::new();};let Some(frame)=m.free_text_frame.get() else{return String::new();};
        let Some(page)=m.document.with(|doc|doc.as_ref()?.info.pages.get(d.owner.page).cloned()) else{return String::new();};
        let frame=pdfmaster_ui::canvas_notes::Frame{left:0.,top:0.,..frame};
        let Ok([a,b,c,e,x,y])=pdfmaster_ui::canvas_text::text_matrix(d.bounds,frame,page.width,page.height,page.rotation) else{return String::new();};
        let style=d.style.clone().unwrap_or_else(||{
            let mut style=pdfmaster_ui::canvas_text::Appearance::default();
            if let pdfmaster_ui::canvas_text::Target::Existing{original}= &d.owner.target{style.font_size=original.style.font_size;style.color=original.style.color;style.alignment=original.style.alignment;}style
        });
        format!("width:{}px;height:{}px;transform:matrix({a},{b},{c},{e},{x},{y});font-family:{};font-size:{}px;line-height:{};color:rgb({},{},{});text-align:{};",d.bounds.width,d.bounds.height,font_stack(style.font),style.font_size,style.line_height,style.color.r*255.,style.color.g*255.,style.color.b*255.,match style.alignment{Alignment::Left=>"left",Alignment::Center=>"center",Alignment::Right=>"right"})
    })
}
#[component]
pub(super) fn Editor(model: Model) -> impl IntoView {
    let m = model;
    let resize =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::resize, move |_| {
            m.measure_free_text_frame()
        });
    on_cleanup(move || resize.remove());
    view! {
        {move||m.free_text_preview.get().filter(|p|!p.installed&&!p.source.is_empty()).map(|p|{
            let ticket=p.ticket;let failed=ticket.clone();let source=p.source.clone();let failed_source=source.clone();let attempt=p.attempt;
            view!{<img class="canvas-text-staged" src=p.source alt="" aria-hidden="true" on:load=move|_|m.free_text_stage_decoded(&ticket,attempt,&source) on:error=move|_|m.free_text_stage_failed(&failed,attempt,&failed_source)/>}
        })}
        <Show when=move||m.free_text.with(|s|s.draft.is_some()&&!matches!(s.phase,Phase::Idle|Phase::BeginPending|Phase::Preparing))&&m.free_text_frame.get().is_some()>
            <div class="canvas-text-editor" style=move||editor_style(m) aria-label="页内文字编辑框">
                <textarea id="canvas-text-body" aria-label="页内文字正文" spellcheck="false"
                    prop:value=move||m.free_text.with(|s|s.draft.as_ref().map(|d|d.contents.clone()).unwrap_or_default())
                    readonly=move||m.free_text.with(|s|s.phase!=Phase::Editing||s.draft.as_ref().is_none_or(|d|d.body_locked()||d.style.is_none()))
                    on:input=move|e|m.free_text_input(event_target_value(&e))
                    on:compositionstart=move|_|m.free_text_composition(true)
                    on:compositionend=move|_|m.free_text_composition(false)
                    on:blur=move|_|m.free_text_blur()
                    on:keydown=move|e|{
                        if e.is_composing()||m.free_text.with_untracked(|s|s.composing){return;}
                        if e.key()=="Escape"{e.prevent_default();e.stop_propagation();m.cancel_free_text();}
                        else if e.key()=="Enter"&&(e.meta_key()||e.ctrl_key()){e.prevent_default();e.stop_propagation();m.submit_free_text(false);}
                    }></textarea>
                <div class="canvas-text-move" role="button" tabindex="-1" aria-label="移动文字框" title="拖动边框移动" on:pointerdown=move|e|m.free_text_drag(&e,Handle::Move)></div>
                {[ (Handle::Nw,"nw"),(Handle::N,"n"),(Handle::Ne,"ne"),(Handle::E,"e"),(Handle::Se,"se"),(Handle::S,"s"),(Handle::Sw,"sw"),(Handle::W,"w") ].into_iter().map(|(handle,name)|view!{
                    <span class=format!("canvas-text-handle handle-{name}") role="button" tabindex="-1" aria-label=format!("调整文字框 {name}") on:pointerdown=move|e|m.free_text_drag(&e,handle)></span>
                }).collect_view()}
            </div>
        </Show>
    }
}
#[component]
pub(super) fn Formatting(model: Model) -> impl IntoView {
    let m = model;
    let locked = move || {
        m.free_text.with(|s| {
            s.phase != Phase::Editing || s.draft.as_ref().is_none_or(|d| d.geometry_locked())
        })
    };
    view! {
        <div class="canvas-text-formatting" role="group" aria-label="页内文字格式" hidden=move||m.chrome.get().active_tool!=Some(ActiveTool::Text) on:focusout=move|_|m.free_text_blur()>
            <select aria-label="文字字体" disabled=locked prop:value=move||m.free_text.with(|s|s.draft.as_ref().and_then(|d|d.style.as_ref()).map(|s|match s.font{Font::Sans=>"sans",Font::CjkSc=>"cjk_sc",Font::Arabic=>"arabic",Font::Devanagari=>"devanagari"}).unwrap_or("unknown")) on:change=move|e|m.free_text_format("font",&event_target_value(&e))>
                <option value="unknown" disabled>"选择打包字体"</option><option value="sans">"Noto Sans"</option><option value="cjk_sc">"Noto 简体中文"</option><option value="arabic">"Noto العربية"</option><option value="devanagari">"Noto देवनागरी"</option>
            </select>
            <input aria-label="文字字号" type="number" min="1" max="512" step="1" disabled=locked prop:value=move||m.free_text.with(|s|s.draft.as_ref().and_then(|d|d.style.as_ref()).map(|s|s.font_size.to_string()).unwrap_or_default()) on:change=move|e|m.free_text_format("size",&event_target_value(&e))/>
            <input aria-label="文字颜色" type="color" disabled=locked prop:value=move||m.free_text.with(|s|s.draft.as_ref().and_then(|d|d.style.as_ref()).map(|s|format!("#{:02x}{:02x}{:02x}",(s.color.r*255.).round() as u8,(s.color.g*255.).round() as u8,(s.color.b*255.).round() as u8)).unwrap_or_else(||"#000000".into())) on:input=move|e|m.free_text_format("color",&event_target_value(&e))/>
            <select aria-label="文字对齐" disabled=locked prop:value=move||m.free_text.with(|s|s.draft.as_ref().and_then(|d|d.style.as_ref()).map(|s|match s.alignment{Alignment::Left=>"left",Alignment::Center=>"center",Alignment::Right=>"right"}).unwrap_or("left")) on:change=move|e|m.free_text_format("alignment",&event_target_value(&e))>
                <option value="left">"左对齐"</option><option value="center">"居中"</option><option value="right">"右对齐"</option>
            </select>
        </div>
    }
}
#[component]
pub(super) fn Recovery(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <Show when=move||m.has_free_text_draft()>
            <div class="canvas-text-recovery" role="group" aria-label="文字事务恢复">
                <span role="status">{move||m.free_text.with(|s|s.error.clone().unwrap_or_else(||match s.phase{Phase::BeginPending=>"正在建立文字编辑…",Phase::Preparing=>"正在解码透明编辑背景和打包字体…",Phase::Committing=>"正在提交文字…",Phase::Uncertain=>"提交结果未知；仅精确重试或取消／同步。",Phase::Cancelling=>"正在确认取消／同步…",Phase::Handoff=>"正在解码已刷新页面…",_=>"Enter 换行 · ⌘/Ctrl+Enter 完成 · Esc 取消"}.into()))}</span>
                <button disabled=move||m.free_text.with(|s|s.composing||!matches!(s.phase,Phase::Editing|Phase::Uncertain)||s.phase==Phase::Uncertain&&s.frozen.is_none()) on:click=move|_|m.submit_free_text(m.free_text.with_untracked(|s|s.phase==Phase::Uncertain))>{move||if m.free_text.with(|s|s.phase==Phase::Uncertain){"精确重试文字"}else{"完成文字"}}</button>
                <button disabled=move||m.free_text.with(|s|s.composing||matches!(s.phase,Phase::Cancelling|Phase::Handoff)) on:click=move|_|m.cancel_free_text()>"取消／同步文字"</button>
                <button aria-label="同步文字状态" disabled=move||m.free_text_sync.with(|s|s.is_some())||m.free_text.with(|s|s.composing||!matches!(s.phase,Phase::Editing|Phase::Uncertain)) on:click=move|_|m.command("document_info",json!({}))>"同步文字状态"</button>
                <button hidden=move||!m.free_text.with(|s|matches!(s.phase,Phase::Preparing|Phase::Handoff)) on:click=move|_|m.retry_free_text_preview()>"重试文字背景"</button>
            </div>
        </Show>
    }
}
