use super::{Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::recents::{Id, View};
use serde_json::json;

impl Model {
    pub(super) fn refresh_recent(self) {
        if self.recent_mutating.get_untracked() {
            return;
        }
        let mut ticket = 0;
        self.recents.update(|state| ticket = state.begin());
        spawn_local(async move {
            let result = bridge::call::<View>("recent_documents", &json!({})).await;
            self.recents.update(|state| {
                state.finish(ticket, result);
            });
        });
    }
    fn recent_mutation_blocked(self) -> bool {
        self.busy.get()
            || self.recent_mutating.get()
            || self.has_note_draft()
            || self.has_ink()
            || self.has_free_text_draft()
            || self.has_shape_draft()
            || self.image_import.with(|c| c.is_some())
            || self.image_export.with(|c| c.is_some())
    }
    fn recent_mutation(self, command: &'static str, id: Option<u32>) {
        if self.recent_mutation_blocked() {
            return;
        }
        self.recents.update(|state| state.invalidate());
        self.recent_mutating.set(true);
        self.busy.set(true);
        let args = id
            .map(|id| serde_json::to_value(Id { id }).expect("ID wire"))
            .unwrap_or_else(|| json!({}));
        spawn_local(async move {
            let result = if command == "clear_recent_documents" {
                bridge::call::<Option<View>>(command, &args).await
            } else {
                bridge::call::<View>(command, &args).await.map(Some)
            };
            match result {
                Ok(Some(view)) => self.recents.update(|state| state.accept(view)),
                Ok(None) => {}
                Err(error) => self.recents.update(|state| state.error = Some(error)),
            }
            self.recent_mutating.set(false);
            self.busy.set(false);
        });
    }
}

#[component]
pub(super) fn Collection(model: Model, favorite: bool, preview: bool) -> impl IntoView {
    let rows = move || {
        model
            .recents
            .with(|state| state.view.rows(favorite, preview))
    };
    let error = move || {
        model
            .recents
            .with(|state| state.error.clone().or_else(|| state.view.warning.clone()))
    };
    view! {
        <div class="home-collection" data-collection=if favorite {"favorites"} else {"recent"}>
            <Show when=move||error().is_some()><p class="home-collection-error" role="status">{move||error().unwrap_or_default()}</p></Show>
            <Show when=move||model.recents.with(|s|s.loading)&&rows().is_empty()><p class="home-collection-state">"正在读取最近记录…"</p></Show>
            <Show when=move||rows().is_empty()&&error().is_none()&&!model.recents.with(|s|s.loading)>
                {if preview { view! { <super::home::EmptyCollection favorite=favorite/> }.into_any() }
                else { view! { <p class="home-collection-state">{if favorite {"尚无收藏。可在最近文档中标记常用文件。"} else {"尚无最近打开的文档。打开 PDF 后会在这里显示。"}}</p> }.into_any() }}
            </Show>
            <For each=rows key=|entry|(entry.id,entry.favorite,entry.exists,entry.name.clone(),entry.last_opened) children=move|entry| {
                let id = entry.id;
                let label = format!("{}{}",entry.name,if entry.exists {""} else {"（文件缺失）"});
                let star = if entry.favorite {"取消收藏"} else {"收藏"};
                view! {
                    <div class="home-document-row" data-recent-id=id>
                        <button class="home-document-open" title=label.clone() disabled=move||model.busy.get()||model.image_import.with(|c|c.is_some())||model.image_export.with(|c|c.is_some()) on:click=move|_|model.command("open_recent_document",json!({"id":id}))>{label.clone()}</button>
                        <button class="home-document-favorite" title=star aria-label=star aria-pressed=entry.favorite.to_string() disabled=move||model.recent_mutation_blocked() on:click=move|_|model.recent_mutation("toggle_recent_favorite",Some(id))>
                            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9z" fill=if entry.favorite {"currentColor"} else {"none"} stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"/></svg>
                        </button>
                        <Show when=move||!favorite&&!preview><button class="home-document-remove" disabled=move||model.recent_mutation_blocked() on:click=move|_|model.recent_mutation("remove_recent_document",Some(id))>"移除"</button></Show>
                    </div>
                }
            }/>
            <Show when=move||!favorite&&!preview&&!rows().is_empty()><button class="home-recents-clear" disabled=move||model.recent_mutation_blocked() on:click=move|_|model.recent_mutation("clear_recent_documents",None)>"清除最近记录…"</button></Show>
        </div>
    }
}
