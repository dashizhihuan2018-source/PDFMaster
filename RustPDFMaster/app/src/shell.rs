//! Route state is independent of the single native document session.
//! Hiding the reader must never reset, replace or save that session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeSection {
    Home,
    Tools,
    Canvas,
    OnlineCanvas,
    Recent,
    Favorites,
    Account,
    Settings,
    Support,
}

impl HomeSection {
    pub const ALL: [Self; 9] = [
        Self::Home,
        Self::Tools,
        Self::Canvas,
        Self::OnlineCanvas,
        Self::Recent,
        Self::Favorites,
        Self::Account,
        Self::Settings,
        Self::Support,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Home => "主界面",
            Self::Tools => "工具",
            Self::Canvas => "无边记",
            Self::OnlineCanvas => "无限画布",
            Self::Recent => "最近",
            Self::Favorites => "收藏",
            Self::Account => "账号详情",
            Self::Settings => "设置",
            Self::Support => "获取支持",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Home(HomeSection),
    Document,
}

pub fn switch_route(current: Route, target: Route, has_document: bool, blocked: bool) -> Route {
    if blocked || (target == Route::Document && !has_document) {
        current
    } else {
        target
    }
}

/// Call only after a native reply has been successfully decoded. A cancelled
/// dialog, failed command, or export reply must not navigate away from home.
pub fn document_reply_route(current: Route, command: &str, has_document: bool) -> Route {
    if has_document && matches!(command, "open_document" | "new_document" | "document_info") {
        Route::Document
    } else {
        current
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub id: &'static str,
    pub title: &'static str,
    pub group: usize,
    pub color: &'static str,
    pub workbench_target: Option<&'static str>,
}

pub const GROUPS: [&str; 5] = [
    "编辑 PDF",
    "整理和优化",
    "PDF 工具箱",
    "转换 PDF",
    "导出 PDF",
];
pub const RECOMMENDED: [&str; 6] = ["edit", "annotate", "pages", "measure", "sign", "copilot"];
const fn entry(
    id: &'static str,
    title: &'static str,
    group: usize,
    color: &'static str,
    workbench_target: Option<&'static str>,
) -> Tool {
    Tool {
        id,
        title,
        group,
        color,
        workbench_target,
    }
}
// Availability intentionally follows implemented Rust actions, not Swift's
// availability flags. Matching labels must not make missing functionality live.
pub const TOOLS: [Tool; 28] = [
    entry("annotate", "批注", 0, "#ffd60a", Some("mode-annotation")),
    entry("edit", "编辑", 0, "#0a84ff", None),
    entry("sign", "签署 PDF", 0, "#bf5af2", None),
    entry("forms", "填写表单", 0, "#64d2ff", None),
    entry("comments", "评论 PDF", 0, "#ff9f0a", Some("tool-notes")),
    entry("resize", "页面重设大小", 0, "#ff375f", None),
    entry("ocr", "识别文本", 1, "#0a84ff", None),
    entry("pages", "管理页面", 1, "#5e5ce6", Some("tool-pages")),
    entry("compress", "压缩 PDF", 1, "#30d158", None),
    entry("merge", "合并 PDF", 1, "#ff9f0a", Some("tool-batch")),
    entry("compare", "对比 PDF", 1, "#ff375f", None),
    entry("page-numbers", "页码", 1, "#64d2ff", None),
    entry("copilot", "PDF Copilot", 2, "#bf5af2", None),
    entry("blank", "空白 PDF", 2, "#0a84ff", None),
    entry("measure", "度量", 2, "#ff9f0a", None),
    entry("password", "设置密码", 2, "#30d158", None),
    entry("translate", "翻译 PDF", 2, "#5e5ce6", None),
    entry("redact", "隐藏 PDF 内容", 2, "#ff453a", None),
    entry("redline", "红线", 2, "#ff453a", None),
    entry("draw", "绘图", 2, "#bf5af2", Some("tool-ink")),
    entry("image-to-pdf", "图片转 PDF", 3, "#0a84ff", None),
    entry("word-to-pdf", "Word 转 PDF", 3, "#5e5ce6", None),
    entry("sheet-to-pdf", "表转 PDF", 3, "#30d158", None),
    entry("file-to-pdf", "其他文件转 PDF", 3, "#ff9f0a", None),
    entry("pdf-to-word", "PDF 转 Word", 4, "#0a84ff", None),
    entry("pdf-to-image", "PDF 转图片", 4, "#bf5af2", None),
    entry("pdf-to-excel", "PDF 转 Excel", 4, "#30d158", None),
    entry("pdf-to-powerpoint", "PDF 转 PowerPoint", 4, "#ff9f0a", None),
];

pub fn tool(id: &str) -> Option<Tool> {
    TOOLS.iter().find(|tool| tool.id == id).copied()
}

/// At the mobile breakpoint the reader occupies the shell's full content
/// width, just like the always-visible tab strip. That width is usable even
/// before the home -> reader DOM update. Never fit against a hidden zero-size
/// reader, and never use full-shell width for the desktop three-column reader.
pub fn reader_fit_available(
    reader_width: i32,
    shell_width: Option<i32>,
    mobile: bool,
) -> Option<f32> {
    let width = if reader_width > 40 {
        reader_width
    } else if mobile {
        shell_width?
    } else {
        return None;
    };
    (width > 40).then(|| (width - 40) as f32)
}
