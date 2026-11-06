//! Original-informed horizontal controls and true analytic page overlay.
use super::canvas_shapes_controller::color;
use super::*;
use pdfmaster_ui::canvas_shapes::{self as shapes, Draft, Kind};
use pdfmaster_ui::canvas_shapes_state::Phase;

fn transform(page: shapes::Page) -> String {
    let p = |x, y| {
        pdfmaster_drawing::display_point(
            pdfmaster_drawing::Point { x, y },
            page.width,
            page.height,
            page.rotation,
        )
    };
    let o = p(0., 0.);
    let x = p(1., 0.);
    let y = p(0., 1.);
    format!(
        "matrix({} {} {} {} {} {})",
        x.x - o.x,
        x.y - o.y,
        y.x - o.x,
        y.y - o.y,
        o.x,
        o.y
    )
}
fn paths(draft: Draft, page: shapes::Page) -> impl IntoView {
    pdfmaster_ui::canvas_shape_paths::paths(&draft,page).unwrap_or_default().into_iter().map(|p|{
        let fill=if p.fill{draft.fill.map(color).unwrap_or_else(||"none".into())}else{"none".into()};
        let dash=p.dash_pattern.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
        view!{<path d=p.d fill=fill stroke=color(draft.stroke) stroke-width=draft.line_width fill-opacity=draft.opacity stroke-opacity=draft.opacity stroke-linejoin=if p.round_join{"round"}else{"miter"} stroke-linecap="butt" stroke-dasharray=dash/>}
    }).collect_view()
}
fn sample(m: Model) -> Option<Draft> {
    let page = shapes::Page {
        index: 0,
        width: 480.,
        height: 280.,
        rotation: 0,
    };
    let (draft, kind) = m.shape_ui.with(|u| {
        (
            u.session
                .candidate
                .clone()
                .or_else(|| u.selected.as_ref().and_then(|s| s.draft().ok())),
            u.settings.kind,
        )
    });
    if let Some(d) = draft {
        let source = m.shape_page()?;
        shapes::transform(
            &d,
            shapes::Rect {
                x: 30.,
                y: 30.,
                width: 420.,
                height: 220.,
            },
            shapes::Page {
                width: source.width.max(480.),
                height: source.height.max(280.),
                ..source
            },
        )
        .ok()
    } else {
        shapes::new_draft(
            kind,
            pdfmaster_drawing::Point { x: 40., y: 60. },
            pdfmaster_drawing::Point { x: 440., y: 220. },
            m.shape_style(),
            page,
        )
        .ok()
    }
}
#[component]
pub(super) fn Editor(model: Model) -> impl IntoView {
    let m = model;
    let resize =
        leptos::leptos_dom::helpers::window_event_listener(leptos::ev::resize, move |_| {
            m.measure_shape_frame()
        });
    let key = leptos::leptos_dom::helpers::window_event_listener(leptos::ev::keydown, move |e| {
        m.shape_key(e)
    });
    on_cleanup(move || {
        resize.remove();
        key.remove();
    });
    view! {
        {move||m.shape_ui.with(|u|u.preview.clone()).filter(|p|!p.installed).map(|p|{
            let t=p.ticket;let fail=t.clone();let source=p.source.clone();let failed_source=source.clone();let attempt=p.attempt;
            view!{<img class="canvas-shape-staged" alt="" aria-hidden="true" src=p.source on:load=move|_|m.shape_stage_decoded(&t,attempt,&source) on:error=move|_|m.shape_stage_failed(&fail,attempt,&failed_source)/>}
        })}
        <div class="canvas-shapes-overlay" aria-label="页内形状选择" hidden=move||m.chrome.get().active_tool!=Some(ActiveTool::Shapes)>
            {move||{
                let page=m.shape_page();
                let state=m.shape_ui.get();
                let Some(page)=page else{return ().into_any();};
                let frame=state.frame.map(|f|shapes::Frame{left:0.,top:0.,..f});
                let live=matches!(state.session.phase,Phase::Editing|Phase::Committing|Phase::Uncertain)&&state.preview.as_ref().is_some_and(|p|p.installed);
                let draft=if live{state.session.candidate.clone()}else if state.session.active(){None}else{state.selected.as_ref().and_then(|s|s.draft().ok())};
                let Some(draft)=draft else{return ().into_any();};
                let (w,h)=if page.rotation.rem_euclid(180)==0{(page.width,page.height)}else{(page.height,page.width)};
                let bounds=draft.bounds(page).ok();let corners=bounds.and_then(|b|frame.and_then(|f|shapes::display_corners(b,f,page).ok()));
                let painted=draft.clone();let clip=bounds.unwrap_or(shapes::Rect{x:0.,y:0.,width:page.width,height:page.height});
                view!{
                    <svg class="canvas-shape-preview" viewBox=format!("0 0 {w} {h}") preserveAspectRatio="none" aria-hidden="true">
                        <defs><clipPath id="shape-frame-clip"><rect x=clip.x y=clip.y width=clip.width height=clip.height/></clipPath></defs>
                        <g transform=transform(page)><g clip-path="url(#shape-frame-clip)">{live.then(||paths(painted,page))}</g></g>
                    </svg>
                    {corners.map(|corners|{
                        let minx=corners.iter().map(|(_,p)|p.x).fold(f32::INFINITY,f32::min);let miny=corners.iter().map(|(_,p)|p.y).fold(f32::INFINITY,f32::min);
                        let maxx=corners.iter().map(|(_,p)|p.x).fold(f32::NEG_INFINITY,f32::max);let maxy=corners.iter().map(|(_,p)|p.y).fold(f32::NEG_INFINITY,f32::max);
                        view!{<div class="canvas-shape-bounds" style=format!("left:{minx}px;top:{miny}px;width:{}px;height:{}px",maxx-minx,maxy-miny)></div>
                            {corners.into_iter().map(|(handle,p)|view!{<span class="canvas-shape-handle" data-handle=format!("{handle:?}") style=format!("left:{}px;top:{}px",p.x,p.y)></span>}).collect_view()}}
                    })}
                }.into_any()
            }}
        </div>
    }
}
#[component]
pub(super) fn Formatting(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <div class="canvas-shapes-formatting" role="group" aria-label="形状样式" hidden=move||m.chrome.get().active_tool!=Some(ActiveTool::Shapes)||m.overview_visible.get()>
            <div class="canvas-shape-kinds">
                {[(Kind::Rectangle,"矩形","M4 5h20v18H4z"),(Kind::Ellipse,"椭圆","M24 14a10 9 0 1 1-20 0 10 9 0 1 1 20 0"),(Kind::Line,"直线","M4 23 24 5"),(Kind::Arrow,"单箭头","M4 23 24 5 M14 5h10v10"),(Kind::DoubleArrow,"双箭头","M4 23 24 5 M14 5h10v10 M4 13v10h10")].into_iter().map(|(kind,label,path)|view!{
                    <button aria-label=label title=label aria-pressed=move||(m.shape_ui.with(|u|u.settings.kind)==kind).to_string() disabled=move||m.shape_style_disabled()||m.has_shape_draft() on:click=move|_|m.select_shape_kind(kind)><svg viewBox="0 0 28 28" aria-hidden="true"><path d=path fill="none" stroke="currentColor" stroke-width="1.4"/></svg></button>
                }).collect_view()}
            </div>
            <input type="color" class="shape-color" aria-label="形状描边颜色" disabled=move||m.shape_style_disabled() prop:value=move||color(m.shape_style().stroke) on:change=move|e|m.shape_format("stroke",&event_target_value(&e),true)/>
            <label class="shape-fill-toggle"><input type="checkbox" aria-label="启用形状填充" disabled=move||m.shape_style_disabled()||m.shape_ui.with(|u|u.selected.is_none()&&!u.settings.kind.supports_fill()) prop:checked=move||m.shape_style().fill.is_some() on:change=move|e|m.shape_format("enabled",if event_target_checked(&e){"true"}else{"false"},true)/>"填充"</label>
            <input type="color" class="shape-color" aria-label="形状填充颜色" disabled=move||m.shape_style_disabled()||m.shape_style().fill.is_none() prop:value=move||color(m.shape_style().fill.unwrap_or(m.shape_ui.with(|u|u.settings.fill))) on:change=move|e|m.shape_format("fill",&event_target_value(&e),true)/>
            <span>"线宽"</span><input type="range" class="shape-width-slider" aria-label="形状线宽滑块" min="0.25" max="12" step="0.01" disabled=move||m.shape_style_disabled() prop:value=move||m.shape_style().line_width.min(12.).to_string() on:input=move|e|m.shape_format("width",&event_target_value(&e),false) on:change=move|e|m.shape_format("width",&event_target_value(&e),true)/>
            <input type="number" class="shape-width-field" aria-label="形状线宽" min="0.25" max="32" step="any" disabled=move||m.shape_style_disabled() prop:value=move||m.shape_ui.with(|u|u.settings.width.clone()) on:input=move|e|m.shape_number("width",event_target_value(&e),false) on:change=move|e|m.shape_number("width",event_target_value(&e),true) on:keydown=move|e|if e.key()=="Enter"{m.shape_format("width",&m.shape_ui.with_untracked(|u|u.settings.width.clone()),true);}/>
            <select aria-label="形状线宽预设" class="shape-presets" disabled=move||m.shape_style_disabled() prop:value=move||m.shape_style().line_width.to_string() on:change=move|e|m.shape_format("width",&event_target_value(&e),true)>
                <option value="" disabled>"▾"</option>{shapes::WIDTH_PRESETS.into_iter().map(|v|view!{<option value=v.to_string()>{v.to_string()}</option>}).collect_view()}
            </select>
            <span>"透明度"</span><input type="range" class="shape-alpha-slider" aria-label="形状透明度滑块" min="0" max="100" step="1" disabled=move||m.shape_style_disabled() prop:value=move||((1.-m.shape_style().opacity)*100.).to_string() on:input=move|e|m.shape_format("transparency",&event_target_value(&e),false) on:change=move|e|m.shape_format("transparency",&event_target_value(&e),true)/>
            <input type="number" class="shape-alpha-field" aria-label="形状透明度" min="0" max="100" step="any" disabled=move||m.shape_style_disabled() prop:value=move||m.shape_ui.with(|u|u.settings.transparency.clone()) on:input=move|e|m.shape_number("transparency",event_target_value(&e),false) on:change=move|e|m.shape_number("transparency",event_target_value(&e),true)/><span>"%"</span>
            <span class="shape-mode">{move||if m.shape_ui.with(|u|u.selected.is_some()){"已选择"}else{"拖动创建"}}</span>
            <svg class="shape-live-sample" viewBox="0 0 480 280" width="48" height="28" aria-label="形状样式预览">
                {move||sample(m).map(|d|paths(d,shapes::Page{index:0,width:480.,height:280.,rotation:0}))}
            </svg>
        </div>
    }
}
#[component]
pub(super) fn Recovery(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <Show when=move||m.shape_ui.with(|u|u.session.active()||u.error.is_some())>
            <div class="canvas-shapes-recovery" role="group" aria-label="形状事务恢复">
                <span role="status">{move||m.shape_ui.with(|u|u.error.clone().or_else(||u.session.error.clone()).unwrap_or_else(||match u.session.phase{Phase::BeginPending=>"正在建立形状编辑…",Phase::Preparing=>"正在解码省略背景…",Phase::Committing=>"正在提交形状…",Phase::Uncertain=>"结果未知；仅精确重试／同步",Phase::Cancelling=>"正在确认取消…",Phase::Handoff=>"正在刷新页面与完整投影…",_=>"拖动修改；松开自动完成"}.into()))}</span>
                <button hidden=move||!m.shape_ui.with(|u|u.session.phase==Phase::Uncertain) on:click=move|_|m.submit_canvas_shape(true)>"精确重试形状"</button>
                <button hidden=move||!m.shape_ui.with(|u|matches!(u.session.phase,Phase::Preparing|Phase::Handoff)) on:click=move|_|m.retry_shape_background()>"重试形状背景"</button>
                <button hidden=move||!m.shape_ui.with(|u|u.completion.is_some()) on:click=move|_|m.complete_canvas_shape()>"重试形状投影"</button>
                <button disabled=move||m.shape_ui.with(|u|u.syncing||!u.session.active()) on:click=move|_|m.sync_canvas_shape()>"同步形状状态"</button>
                <button disabled=move||m.shape_ui.with(|u|!u.session.active()||matches!(u.session.phase,Phase::Committing|Phase::Handoff)) on:click=move|_|m.cancel_canvas_shape()>"取消／同步形状"</button>
            </div>
        </Show>
    }
}
