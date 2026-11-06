use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub last_opened: Option<u64>,
    pub favorite: bool,
    pub exists: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub entries: Vec<Entry>,
    pub warning: Option<String>,
}
#[derive(Serialize)]
pub struct Id {
    pub id: u32,
}
impl View {
    pub fn rows(&self, favorites: bool, preview: bool) -> Vec<Entry> {
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|e| {
                if favorites {
                    e.favorite
                } else {
                    e.last_opened.is_some()
                }
            })
            .cloned()
            .collect();
        rows.sort_by_key(|e| (std::cmp::Reverse(e.last_opened), e.id));
        if preview {
            rows.truncate(5);
        }
        rows
    }
}
#[derive(Clone, Debug, Default)]
pub struct History {
    generation: u64,
    pub loading: bool,
    pub view: View,
    pub error: Option<String>,
}
impl History {
    pub fn begin(&mut self) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.loading = true;
        self.generation
    }
    pub fn finish(&mut self, ticket: u64, result: Result<View, String>) -> bool {
        if ticket != self.generation || !self.loading {
            return false;
        }
        self.loading = false;
        match result {
            Ok(view) => {
                self.view = view;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
    pub fn accept(&mut self, view: View) {
        self.generation = self.generation.saturating_add(1);
        self.loading = false;
        self.view = view;
        self.error = None;
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.loading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{HomeSection, Route, document_reply_route};
    #[test]
    fn recents_wire_is_typed_id_only_and_filters_first_five_or_full() {
        let entries: Vec<Entry> = (1..=8)
            .map(|id| Entry {
                id,
                name: format!("{id}.pdf"),
                last_opened: Some(u64::from(id)),
                favorite: id % 2 == 0,
                exists: true,
            })
            .collect();
        let view = View {
            entries,
            warning: None,
        };
        assert_eq!(
            view.rows(false, true)
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![8, 7, 6, 5, 4]
        );
        assert_eq!(view.rows(false, false).len(), 8);
        assert_eq!(view.rows(true, false).len(), 4);
        assert_eq!(
            serde_json::to_value(Id { id: 8 }).unwrap(),
            serde_json::json!({"id":8})
        );
        assert!(serde_json::from_value::<Entry>(serde_json::json!({"id":1,"name":"a","last_opened":null,"favorite":true,"exists":true,"path":"private"})).is_err());
        let home = Route::Home(HomeSection::Recent);
        assert_eq!(
            document_reply_route(home, "open_recent_document", true),
            Route::Document
        );
        assert_eq!(
            document_reply_route(home, "open_recent_document", false),
            home
        );
    }
    #[test]
    fn recents_read_generation_rejects_stale_load_after_mutation_and_newer_read() {
        let mut state = History::default();
        let first = state.begin();
        let latest = state.begin();
        assert!(!state.finish(first, Ok(View::default())));
        assert!(state.loading);
        state.accept(View {
            entries: vec![],
            warning: Some("mutated".into()),
        });
        assert!(!state.finish(latest, Ok(View::default())));
        assert_eq!(state.view.warning.as_deref(), Some("mutated"));
        assert!(!state.loading);
    }
    #[test]
    fn recents_controller_uses_shared_command_and_isolated_loader_not_global_busy() {
        let rows = include_str!("recents_view.rs");
        let loader = rows.split("fn recent_mutation_blocked").next().unwrap();
        assert!(!loader.contains("busy.set("));
        assert!(!loader.contains("message.set("));
        assert!(rows.contains("model.command(\"open_recent_document\",json!({\"id\":id}))"));
        let drop = include_str!("file_drop_view.rs")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(
            drop.contains(
                "if!home{self.page.set(index.min(doc.info.page_count.saturating_sub(1)));}"
            )
        );
        assert!(drop.contains("ifhome{self.refresh_recent();}"));
    }
    #[test]
    fn recents_existing_open_keeps_tab_chrome_and_mobile_zoom() {
        let source = include_str!("workspace.rs")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        assert!(source.contains(
            "letknown_tab=self.tab_presentations.with_untracked(|tabs|tabs.contains_key(&doc.id));"
        ));
        assert!(source.contains("!known_tab&&matches!(command,\"open_document\"|\"open_recent_document\"|\"new_document\""));
    }
}
