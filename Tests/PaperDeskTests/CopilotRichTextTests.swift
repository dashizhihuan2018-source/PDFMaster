import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor final class CopilotRichTextTests: XCTestCase {
    private func attributes(_ text: NSAttributedString, at word: String) -> [NSAttributedString.Key: Any] {
        let range = (text.string as NSString).range(of: word)
        XCTAssertNotEqual(range.location, NSNotFound)
        guard range.location != NSNotFound else { return [:] }
        return text.attributes(at: range.location, effectiveRange: nil)
    }
    func testScreenshotSummaryUsesBoldWithoutLiteralMarkdown() throws {
        let text = PDFCopilotRichText.renderMarkdown("**主题**\nProject Cedar [p.1]。\n\n**主要结论**\n1. **启动**：2026 年 11 月 5 日\n2. **预算**：120 个虚构单位", citedPages: [1])
        XCTAssertFalse(text.string.contains("**")); XCTAssertTrue(text.string.contains("1.")); XCTAssertTrue(text.string.contains("120"))
        let font = try XCTUnwrap(attributes(text, at: "主题")[.font] as? NSFont)
        XCTAssertTrue(NSFontManager.shared.traits(of: font).contains(.boldFontMask))
        XCTAssertEqual(font.pointSize, 15)
        let body = try XCTUnwrap(attributes(text, at: "Project Cedar")[.paragraphStyle] as? NSParagraphStyle)
        XCTAssertEqual(body.paragraphSpacing, 12); XCTAssertEqual(body.lineHeightMultiple, 1.16, accuracy: 0.001)
    }
    func testReferenceHeadingSizesAndListHangingIndent() throws {
        let text = PDFCopilotRichText.renderMarkdown("# 一级\n\n## 二级\n\n### 三级\n\n- 要点一\n- 要点二\n\n> 引用段落", citedPages: [])
        for (word, size) in [("一级", 17.0), ("二级", 16.0), ("三级", 15.0)] {
            XCTAssertEqual(try XCTUnwrap(attributes(text, at: word)[.font] as? NSFont).pointSize, size)
        }
        let list = try XCTUnwrap(attributes(text, at: "要点一")[.paragraphStyle] as? NSParagraphStyle)
        XCTAssertEqual(list.headIndent, 30); XCTAssertLessThan(list.firstLineHeadIndent, list.headIndent)
        XCTAssertEqual(list.paragraphSpacing, 6)
        XCTAssertEqual(try XCTUnwrap(attributes(text, at: "引用段落")[.paragraphStyle] as? NSParagraphStyle).headIndent, 16)
    }
    func testInlineAndFencedCodeStayLiteralAndMonospaced() throws {
        let text = PDFCopilotRichText.renderMarkdown("`**literal**` 与 ***粗斜体***\n\n```swift\nlet x = \"**not bold**\"\n```", citedPages: [])
        XCTAssertTrue(text.string.contains("**literal**")); XCTAssertTrue(text.string.contains("**not bold**"))
        let code = try XCTUnwrap(attributes(text, at: "let x")[.font] as? NSFont)
        XCTAssertEqual(code.pointSize, 14); XCTAssertTrue(code.fontDescriptor.symbolicTraits.contains(.monoSpace))
        let emphasis = try XCTUnwrap(attributes(text, at: "粗斜体")[.font] as? NSFont)
        XCTAssertTrue(NSFontManager.shared.traits(of: emphasis).contains([.boldFontMask, .italicFontMask]))
    }
    func testOnlyValidatedPageCitationsBecomeLocalLinks() {
        let text = PDFCopilotRichText.renderMarkdown("证据 [p.1]，未知 [p.99]，`[p.1]`。", citedPages: [1])
        XCTAssertEqual((attributes(text, at: "[p.1]")[.link] as? URL)?.absoluteString, "pdfmaster-citation://page/1")
        XCTAssertNil(attributes(text, at: "[p.99]")[.link])
        XCTAssertNil(text.attribute(.link, at: (text.string as NSString).range(of: "[p.1]", options: .backwards).location, effectiveRange: nil))
    }
    func testNestedMixedListKeepsItsOwnMarkerAndDepth() throws {
        let text = PDFCopilotRichText.renderMarkdown("1. 外层\n   - 内层甲\n   - 内层乙\n2. 下一项", citedPages: [])
        XCTAssertTrue(text.string.contains("•\t内层甲")); XCTAssertTrue(text.string.contains("•\t内层乙"))
        XCTAssertEqual(try XCTUnwrap(attributes(text, at: "内层甲")[.paragraphStyle] as? NSParagraphStyle).headIndent, 60)
    }
    func testUntrustedMarkdownNeverCreatesAttachmentsOrExternalActions() {
        let text = PDFCopilotRichText.renderMarkdown("[危险](javascript:alert) ![图片](https://example.invalid/pixel) <script>alert(1)</script>\n\n[本机](file:///etc/passwd) [网页](https://example.invalid)", citedPages: [])
        text.enumerateAttributes(in: NSRange(location: 0, length: text.length)) { attrs, _, _ in
            XCTAssertNil(attrs[.attachment]); XCTAssertNil(attrs[.link])
        }
    }
    func testUserQuestionIsLiteralAndRawConversationUnchanged() {
        let messages = [DMXMessage(role: "user", content: "**不要改我的问题**"), DMXMessage(role: "assistant", content: "**答案** [p.1]")]
        let text = PDFCopilotRichText.conversation(messages, citedPages: [1])
        XCTAssertTrue(text.string.contains("**不要改我的问题**")); XCTAssertFalse(text.string.contains("**答案**"))
        XCTAssertEqual(messages[1].content, "**答案** [p.1]")
    }
    func testUnicodeEmptyAndIncompleteMarkdownRetainText() {
        XCTAssertEqual(PDFCopilotRichText.renderMarkdown("", citedPages: []).string, "")
        let text = PDFCopilotRichText.renderMarkdown("未闭合 **中文 👩🏽‍💻，120元\n下一行", citedPages: [])
        XCTAssertTrue(text.string.contains("中文 👩🏽‍💻")); XCTAssertTrue(text.string.contains("120元")); XCTAssertTrue(text.string.contains("下一行"))
    }
}
