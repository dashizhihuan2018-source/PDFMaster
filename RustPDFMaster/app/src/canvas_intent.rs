//! Shared guarded canvas actions, executed exactly once after handoff.
use super::*;
#[derive(Clone, Debug)]
pub(super) enum Intent {
    Mode(Mode),
    Tool(ActiveTool),
    Panel(Option<Panel>),
    Strip(Action),
    PageAction(pdfmaster_ui::workbench::PageAction),
    Command(&'static str, Value, Option<usize>),
    Close(u32),
    Navigate(usize, Option<pdfmaster_ui::annotation_navigation::Target>),
    Route(Route),
    Zoom(f32),
    ChooseZoom(String),
    FitPage,
    FitWidth,
    Sidebar,
    Overview(bool),
    OutlinePage(usize, usize),
    AnnotationPage(
        usize,
        pdfmaster_ui::annotations::Identity,
        Option<pdfmaster_ui::annotation_navigation::Target>,
    ),
}
impl Model {
    pub(super) fn canvas_intent_disabled(self) -> bool {
        self.busy.get()
            || !self.loaded()
            || self.has_note_draft()
            || self.has_ink()
            || self.image_import.with(|i| i.is_some())
            || self.free_text.with(|s| {
                matches!(
                    s.phase,
                    pdfmaster_ui::canvas_text::Phase::Uncertain
                        | pdfmaster_ui::canvas_text::Phase::Cancelling
                        | pdfmaster_ui::canvas_text::Phase::Handoff
                )
            })
            || self.shape_ui.with(|u| {
                matches!(
                    u.session.phase,
                    pdfmaster_ui::canvas_shapes_state::Phase::Uncertain
                        | pdfmaster_ui::canvas_shapes_state::Phase::Cancelling
                        | pdfmaster_ui::canvas_shapes_state::Phase::Handoff
                )
            })
    }
    pub(super) fn defer_canvas_edit(self, intent: Option<Intent>) -> bool {
        if self.has_free_text_draft() {
            return self.defer_free_text(intent);
        }
        self.defer_shape_edit(intent)
    }
    pub(super) fn execute_canvas_intent(self, intent: Intent) {
        match intent {
            Intent::Mode(v) => self.select_mode(v),
            Intent::Tool(v) => self.activate_tool(v),
            Intent::Panel(v) => self.select_panel(v),
            Intent::Strip(v) => self.run_strip_action(v),
            Intent::PageAction(v) => self.prompt_page_action(v),
            Intent::Command(c, a, p) => self.command_at_page(c, a, p),
            Intent::Close(id) => self.close_tab(id),
            Intent::Navigate(p, t) => self.navigate_with_annotation(p, t),
            Intent::Route(v) => self.switch_to(v),
            Intent::Zoom(v) => self.zoom_by(v),
            Intent::ChooseZoom(v) => self.choose_zoom(&v),
            Intent::FitPage => self.fit_page(),
            Intent::FitWidth => self.fit_width(),
            Intent::Sidebar => self.toggle_sidebar(),
            Intent::Overview(v) => self.set_overview(v),
            Intent::OutlinePage(page, index) => {
                self.outline_selection.set(Some(index));
                self.navigate(page);
            }
            Intent::AnnotationPage(page, identity, target) => {
                self.annotation_selection.set(Some(identity));
                let target = target.and_then(|mut t| {
                    t.generation = self.annotation_cache.with_untracked(|c| c.generation())?;
                    Some(t)
                });
                self.navigate_with_annotation(page, target);
            }
        }
    }
}
