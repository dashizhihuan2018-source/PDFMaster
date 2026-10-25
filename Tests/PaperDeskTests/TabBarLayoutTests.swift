import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class TabBarLayoutTests: XCTestCase {
    func testTwoPDFTabsStayCompactAndAdjacentWhenSelectionAndWindowWidthChange() throws {
        try withShell { shell, window, _ in
            let first = shell.createBlankDocument()
            let second = shell.createBlankDocument()
            for width: CGFloat in [1280, 3008, 1180, 1600, 3008] {
                window.setContentSize(NSSize(width: width, height: 820))
                var previous: [NSRect]?
                for id in [first.id, second.id, first.id, second.id] {
                    shell.activate(tabID: id)
                    shell.view.layoutSubtreeIfNeeded()
                    let frames = try tabFrames(in: shell)
                    XCTAssertEqual(frames.count, 2)
                    guard frames.count == 2 else { return }
                    XCTAssertEqual(frames[0].minX, 144, accuracy: 1)
                    XCTAssertEqual(frames[1].minX, frames[0].maxX, accuracy: 1,
                                   "Second PDF must remain next to the first, not at the window's right edge")
                    for frame in frames {
                        XCTAssertGreaterThan(frame.width, 60)
                        XCTAssertLessThanOrEqual(frame.width, 360,
                                                 "A document tab must not stretch across unused titlebar space")
                    }
                    let plus = try XCTUnwrap(buttons(in: shell.view).first { $0.accessibilityLabel() == "新标签" })
                    let plusFrame = shell.view.convert(plus.bounds, from: plus)
                    XCTAssertEqual(plusFrame.minX, frames[1].maxX, accuracy: 1)
                    if let previous {
                        for (before, after) in zip(previous, frames) {
                            XCTAssertEqual(after.minX, before.minX, accuracy: 1)
                            XCTAssertEqual(after.width, before.width, accuracy: 1)
                        }
                    }
                    previous = frames
                }
            }
        }
    }

    func testLongPDFNamesDoNotDisplaceSecondTabOrLoseCloseButtons() throws {
        try withShell { shell, window, directory in
            for name in ["北京优锘企业级数字孪生平台V1.0.pdf",
                         "思倍云-专注打造物联空间运营AI运营大脑_20261020.pdf"] {
                let document = PDFDocument()
                document.insert(PDFPage(), at: 0)
                let url = directory.appendingPathComponent(name)
                XCTAssertTrue(document.write(to: url))
                XCTAssertTrue(shell.open(url: url, passwordPrompt: { nil }))
            }
            for width: CGFloat in [1180, 3008] {
                window.setContentSize(NSSize(width: width, height: 820))
                for tab in shell.documentTabs {
                    shell.activate(tabID: tab.id)
                    shell.view.layoutSubtreeIfNeeded()
                    let frames = try tabFrames(in: shell)
                    XCTAssertEqual(frames.count, 2)
                    guard frames.count == 2 else { return }
                    XCTAssertEqual(frames[1].minX, frames[0].maxX, accuracy: 1)
                    for frame in frames { XCTAssertLessThanOrEqual(frame.width, 360) }
                    let closes = buttons(in: shell.view).filter { $0.accessibilityLabel()?.hasPrefix("关闭 ") == true }
                    XCTAssertEqual(closes.count, 2)
                    for close in closes {
                        let frame = shell.view.convert(close.bounds, from: close)
                        XCTAssertGreaterThanOrEqual(frame.width, 16)
                        XCTAssertLessThan(frame.maxX, width - 36)
                    }
                }
            }
        }
    }

    private func tabFrames(in shell: ApplicationShellController) throws -> [NSRect] {
        try buttons(in: shell.view).filter { $0.accessibilityLabel()?.hasPrefix("文档标签 ") == true }.map {
            let box = try XCTUnwrap($0.superview)
            return shell.view.convert(box.bounds, from: box)
        }.sorted { $0.minX < $1.minX }
    }

    private func buttons(in view: NSView) -> [NSButton] {
        (view as? NSButton).map { [$0] } ?? view.subviews.flatMap { buttons(in: $0) }
    }

    private func withShell(_ body: (ApplicationShellController, NSWindow, URL) throws -> Void) throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recent.json")))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1280, height: 820),
                              styleMask: [.titled, .closable, .resizable, .fullSizeContentView],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = shell
        shell.installWindowChrome(in: window)
        defer { window.contentViewController = nil; window.close() }
        try body(shell, window, directory)
    }
}
