//! Source-derived reader chrome, independent from document/history ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Annotation,
    Editing,
    Drawing,
    Signature,
    Pages,
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Search,
    Pages,
    Batch,
    Notes,
    Ink,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTool {
    Ink,
    Eraser,
    Note,
    Text,
    Shapes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Activate(ActiveTool),
    Panel(Panel),
    InsertBlank,
    InsertPdf,
    Rotate,
    CopyPages,
    PastePages,
    Extract,
    DeleteCurrent,
    Export,
    ExportImages(bool),
    ExportText(crate::text_export::TextExportFormat),
    Prompt(PageAction),
    Duplicate,
    MoveBy(i32),
    InsertPdfs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageAction {
    Extract,
    RotateRange,
    DeleteRange,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub mode: Option<Mode>,
    pub panel: Option<Panel>,
    pub active_tool: Option<ActiveTool>,
    pub sidebar: bool,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            mode: None,
            panel: None,
            active_tool: None,
            sidebar: true,
        }
    }
}

impl Chrome {
    pub fn document_opened(self, note: bool, ink: bool, replacement: bool) -> Self {
        if note {
            Self {
                mode: Some(Mode::Annotation),
                panel: None,
                active_tool: Some(ActiveTool::Note),
                ..self
            }
        } else if ink {
            Self {
                mode: Some(Mode::Drawing),
                panel: None,
                active_tool: Some(ActiveTool::Ink),
                ..self
            }
        } else if replacement {
            Self {
                sidebar: self.sidebar,
                ..Self::default()
            }
        } else {
            self
        }
    }
    pub fn select_mode(self, mode: Mode, blocked: bool) -> Self {
        if blocked {
            self
        } else {
            Self {
                mode: Some(mode),
                panel: None,
                active_tool: None,
                ..self
            }
        }
    }
    pub fn select_panel(self, panel: Option<Panel>, blocked: bool) -> Self {
        if panel == Some(Panel::Ink) {
            return self.activate_tool(ActiveTool::Ink, blocked);
        }
        if panel == Some(Panel::Notes) {
            return self.activate_tool(ActiveTool::Note, blocked);
        }
        if blocked {
            self
        } else {
            Self {
                panel,
                active_tool: None,
                ..self
            }
        }
    }
    pub fn activate_tool(self, tool: ActiveTool, blocked: bool) -> Self {
        if blocked {
            return self;
        }
        Self {
            mode: Some(match tool {
                ActiveTool::Note => Mode::Annotation,
                ActiveTool::Text | ActiveTool::Shapes => match self.mode {
                    Some(Mode::Signature) => Mode::Signature,
                    Some(Mode::Drawing) => Mode::Drawing,
                    _ => Mode::Annotation,
                },
                ActiveTool::Ink | ActiveTool::Eraser => match self.mode {
                    Some(Mode::Annotation) => Mode::Annotation,
                    _ => Mode::Drawing,
                },
            }),
            panel: None,
            active_tool: (self.active_tool != Some(tool)).then_some(tool),
            ..self
        }
    }
    pub fn toggle_sidebar(self, blocked: bool) -> Self {
        if blocked {
            self
        } else {
            Self {
                sidebar: !self.sidebar,
                ..self
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: &'static str,
    pub title: &'static str,
    pub shows_title: bool,
    pub action: Option<Action>,
}

pub const PRIMARY_MODES: [(&str, Option<Mode>); 6] = [
    ("批注", Some(Mode::Annotation)),
    ("编辑", Some(Mode::Editing)),
    ("导出", Some(Mode::Export)),
    ("填写和签名", Some(Mode::Signature)),
    ("扫描和 OCR", None),
    ("度量", None),
];

const fn tool(
    id: &'static str,
    title: &'static str,
    shows_title: bool,
    action: Option<Action>,
) -> Tool {
    Tool {
        id,
        title,
        shows_title,
        action,
    }
}

pub fn tools(mode: Mode) -> &'static [Tool] {
    use Action::*;
    match mode {
        Mode::Annotation | Mode::Drawing => {
            const {
                &[
                    tool("highlight", "高亮", false, None),
                    tool("underline", "下划线", false, None),
                    tool("strikeout", "删除线", false, None),
                    tool(
                        "pen",
                        "手绘",
                        false,
                        Some(Activate(crate::workbench::ActiveTool::Ink)),
                    ),
                    tool(
                        "eraser",
                        "橡皮（整条笔迹）",
                        false,
                        Some(Activate(crate::workbench::ActiveTool::Eraser)),
                    ),
                    tool(
                        "text",
                        "文字批注",
                        false,
                        Some(Activate(crate::workbench::ActiveTool::Text)),
                    ),
                    tool(
                        "shapes",
                        "形状",
                        false,
                        Some(Activate(crate::workbench::ActiveTool::Shapes)),
                    ),
                    tool(
                        "note",
                        "便笺",
                        false,
                        Some(Activate(crate::workbench::ActiveTool::Note)),
                    ),
                    tool("stamp", "印章", false, None),
                    tool("signature", "签名", false, None),
                    tool("selection", "选择批注", false, None),
                ]
            }
        }
        Mode::Editing => {
            const {
                &[
                    tool("content-text", "文本", true, None),
                    tool("content-image", "图像", true, None),
                    tool("content-link", "链接", true, None),
                    tool("content-redact", "隐藏", true, None),
                ]
            }
        }
        Mode::Pages => {
            const {
                &[
                    tool("insert", "插入页", true, Some(InsertBlank)),
                    tool("attach", "附加文件", true, Some(InsertPdf)),
                    tool("rotate", "旋转", true, Some(Rotate)),
                    tool("copy-page", "复制", true, Some(CopyPages)),
                    tool("paste-page", "粘贴", true, Some(PastePages)),
                    tool("extract", "摘录", true, Some(Extract)),
                    tool("delete-page", "删除", true, Some(DeleteCurrent)),
                ]
            }
        }
        Mode::Signature => {
            const {
                &[
                    tool("signature", "签名", true, None),
                    tool(
                        "text",
                        "文字",
                        true,
                        Some(Activate(crate::workbench::ActiveTool::Text)),
                    ),
                ]
            }
        }
        Mode::Export => {
            const {
                &[
                    tool("save", "存储副本", true, Some(Export)),
                    tool("export-image", "PNG 图片", true, Some(ExportImages(false))),
                    tool("password", "密码保护", true, None),
                    tool(
                        "export-text",
                        "纯文本",
                        true,
                        Some(ExportText(crate::text_export::TextExportFormat::PlainText)),
                    ),
                ]
            }
        }
    }
}

/// Uses measured available reader content, never hidden DOM or guessed window
/// dimensions. Zoom follows the same limits as the actual raster plan.
pub fn fit_page_zoom(
    width: f32,
    height: f32,
    rotation: i32,
    available_width: f32,
    available_height: f32,
) -> Option<f32> {
    if [width, height, available_width, available_height]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.)
        || rotation.rem_euclid(90) != 0
    {
        return None;
    }
    let (w, h) = if rotation.rem_euclid(180) == 0 {
        (width, height)
    } else {
        (height, width)
    };
    Some(crate::render_zoom(
        (available_width / w).min(available_height / h),
    ))
}
