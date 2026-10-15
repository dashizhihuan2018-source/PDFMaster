import AppKit
import XCTest
import PaperCore
@testable import PaperDesk

@MainActor final class HoverInteractionTests: XCTestCase {
    private var shells: [ApplicationShellController] = []

    private func dashboard() throws -> DashboardViewController {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMasterHover-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recents.json")))
        shells.append(shell)
        _ = shell.view
        let dashboard = shell.dashboardController
        dashboard.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 780)
        dashboard.refreshResponsiveLayout(); dashboard.view.layoutSubtreeIfNeeded()
        return dashboard
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }

    private func enter(_ view: NSView) throws {
        view.mouseEntered(with: try XCTUnwrap(NSEvent.enterExitEvent(with: .mouseEntered, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)))
    }

    private func exit(_ view: NSView) throws {
        view.mouseExited(with: try XCTUnwrap(NSEvent.enterExitEvent(with: .mouseExited, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)))
    }

    private func drawnColor(_ view: NSView) throws -> NSColor {
        let width = Int(view.bounds.width), height = Int(view.bounds.height)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let context = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = context
        view.draw(view.bounds)
        NSGraphicsContext.restoreGraphicsState()
        return try XCTUnwrap(bitmap.colorAt(x: min(18, width / 2), y: min(18, height - 3))?.usingColorSpace(.sRGB))
    }

    private func assertSame(_ actual: NSColor, _ expected: NSColor, file: StaticString = #filePath, line: UInt = #line) {
        let rhs = expected.usingColorSpace(.sRGB)!
        XCTAssertEqual(actual.redComponent, rhs.redComponent, accuracy: 0.012, file: file, line: line)
        XCTAssertEqual(actual.greenComponent, rhs.greenComponent, accuracy: 0.012, file: file, line: line)
        XCTAssertEqual(actual.blueComponent, rhs.blueComponent, accuracy: 0.012, file: file, line: line)
        XCTAssertEqual(actual.alphaComponent, rhs.alphaComponent, accuracy: 0.012, file: file, line: line)
    }

    func testCardsUseReferenceHoverAndRestoreWithoutChangingGeometry() throws {
        let dashboard = try dashboard()
        let card = try XCTUnwrap(descendants(dashboard.view).compactMap { $0 as? NSButton }.first { $0.frame.height == 120 })
        let frame = card.frame, title = card.title, image = card.image
        assertSame(try drawnColor(card), ShellPalette.card)
        try enter(card)
        assertSame(try drawnColor(card), NSColor(srgbRed: 41.0 / 255, green: 41.0 / 255, blue: 41.0 / 255, alpha: 1))
        XCTAssertEqual(card.frame, frame); XCTAssertEqual(card.title, title); XCTAssertTrue(card.image === image)
        card.highlight(true)
        let pressed = try drawnColor(card)
        XCTAssertLessThan(pressed.redComponent, 41.0 / 255)
        card.highlight(false); try exit(card)
        assertSame(try drawnColor(card), ShellPalette.card)
    }

    func testEveryCustomButtonRebuildsOneVisibleRectTrackingArea() throws {
        let dashboard = try dashboard()
        let buttons = descendants(dashboard.view).compactMap { $0 as? NSButton }.filter { $0.frame.height == 120 || ["打开文件", "自定义", "查看全部", "主界面"].contains($0.title) }
        XCTAssertGreaterThanOrEqual(buttons.count, 9)
        let strip = ToolStripButton(tool: .init(id: "pen", title: "手绘", symbol: "pencil"), target: nil, action: nil)
        for button in buttons + [strip] {
            button.updateTrackingAreas(); button.frame.size.width += 5; button.updateTrackingAreas(); button.updateTrackingAreas()
            // AppKit may also retain its own cell tracking area; only the custom visible-rect area is ours.
            let areas = button.trackingAreas.filter { $0.options.contains(.mouseEnteredAndExited) && $0.options.contains(.inVisibleRect) && $0.options.contains(.activeAlways) }
            XCTAssertEqual(areas.count, 1, "\(button.title) must not leak or omit hover tracking")
            XCTAssertTrue(areas.first?.options.contains(.inVisibleRect) == true)
            XCTAssertTrue(areas.first?.options.contains(.activeAlways) == true)
        }
    }

    func testDisabledCardNeverShowsHoverOrPressedFeedback() throws {
        let dashboard = try dashboard()
        let card = try XCTUnwrap(descendants(dashboard.view).compactMap { $0 as? NSButton }.first { $0.frame.height == 120 })
        try enter(card); card.isEnabled = false
        let disabled = try drawnColor(card)
        try enter(card); card.highlight(true)
        assertSame(try drawnColor(card), disabled)
        card.highlight(false); card.isEnabled = true
        assertSame(try drawnColor(card), ShellPalette.card)
    }

    func testStripHoverPressedAndPersistentSelectionAreDistinct() throws {
        let button = ToolStripButton(tool: .init(id: "pen", title: "手绘", symbol: "pencil"), target: nil, action: nil)
        button.frame = NSRect(x: 0, y: 0, width: 48, height: 28)
        let normal = try drawnColor(button)
        try enter(button); let hover = try drawnColor(button)
        XCTAssertNotEqual(hover, normal)
        button.highlight(true); let pressed = try drawnColor(button)
        XCTAssertNotEqual(pressed, hover)
        button.highlight(false); button.state = .on; try exit(button)
        let selected = try drawnColor(button)
        XCTAssertNotEqual(selected, normal); XCTAssertNotEqual(selected, hover)
        try enter(button); try exit(button)
        assertSame(try drawnColor(button), selected)
        button.isEnabled = false; let disabled = try drawnColor(button)
        try enter(button); button.highlight(true)
        assertSame(try drawnColor(button), disabled)
    }

    func testHomepagePillsAndLinkRespondToHoverAndPressWithoutChangingActions() throws {
        let dashboard = try dashboard()
        for title in ["打开文件", "自定义", "查看全部"] {
            let button = try XCTUnwrap(descendants(dashboard.view).compactMap { $0 as? NSButton }.first { $0.title == title })
            let target = button.target, action = button.action
            let normal = try drawnColor(button)
            try enter(button); let hover = try drawnColor(button)
            XCTAssertNotEqual(hover, normal, title)
            button.highlight(true); XCTAssertNotEqual(try drawnColor(button), hover, title)
            button.highlight(false); try exit(button); assertSame(try drawnColor(button), normal)
            XCTAssertTrue(button.target === target); XCTAssertEqual(button.action, action)
        }
    }

    func testKeyboardFocusAndAccessibleHelpRemainAvailable() throws {
        let dashboard = try dashboard()
        let buttons = descendants(dashboard.view).compactMap { $0 as? NSButton }.filter { $0.frame.height == 120 || $0.title == "打开文件" || $0.title == "主界面" }
        for button in buttons {
            XCTAssertEqual(button.acceptsFirstResponder, button.isEnabled, "Disabled controls must not become keyboard targets")
            XCTAssertEqual(button.focusRingType, .exterior)
            XCTAssertFalse(button.focusRingMaskBounds.isEmpty)
            XCTAssertNotNil(button.toolTip)
            XCTAssertNotNil(button.accessibilityHelp())
        }
    }

    func testHoverDoesNotInvokeOrDuplicateActionAndDisabledStripCannotInvoke() throws {
        let spy = ActionCounter()
        let button = ToolStripButton(tool: .init(id: "pen", title: "手绘", symbol: "pencil"), target: spy, action: #selector(ActionCounter.run))
        try enter(button); try exit(button); XCTAssertEqual(spy.count, 0)
        button.performClick(nil); XCTAssertEqual(spy.count, 1)
        button.isEnabled = false; button.performClick(nil); XCTAssertEqual(spy.count, 1)
        XCTAssertEqual(button.toolID, "pen")
    }

    func testToolStripAccessibleHelpFollowsUpdatedTooltip() {
        let button = ToolStripButton(tool: .init(id: "paste-page", title: "粘贴", symbol: "doc.on.clipboard", available: false), target: nil, action: nil)
        button.toolTip = "粘贴应用内复制的页面"
        XCTAssertEqual(button.accessibilityHelp(), button.toolTip)
    }

    func testDropZoneHoverChangesBorderAndPDFGlyphWithoutChangingFrame() throws {
        let dashboard = try dashboard()
        let drop = try XCTUnwrap(descendants(dashboard.view).first { String(describing: type(of: $0)) == "HomeDropZoneView" })
        let glyph = try XCTUnwrap(drop.subviews.first { String(describing: type(of: $0)) == "HomePDFStackGlyph" })
        let frame = drop.frame, glyphFrame = glyph.frame
        let normal = try drawnColor(drop), normalGlyph = try drawnColor(glyph)
        drop.updateTrackingAreas(); XCTAssertEqual(drop.trackingAreas.count, 1)
        try enter(drop)
        XCTAssertNotEqual(try drawnColor(drop), normal)
        XCTAssertNotEqual(try drawnColor(glyph), normalGlyph)
        XCTAssertEqual(drop.frame, frame); XCTAssertEqual(glyph.frame, glyphFrame)
        try exit(drop); assertSame(try drawnColor(drop), normal); assertSame(try drawnColor(glyph), normalGlyph)
    }

    func testHomepageHasReachableButInitiallyHiddenFileImportCancelControl() throws {
        let dashboard = try dashboard()
        let views = descendants(dashboard.view)
        let status = try XCTUnwrap(views.first { $0.identifier?.rawValue == "home-file-import-status" })
        let cancel = try XCTUnwrap(views.compactMap { $0 as? NSButton }.first { $0.identifier?.rawValue == "home-file-import-cancel" })
        XCTAssertTrue(status.isHidden)
        XCTAssertEqual(cancel.title, "取消")
        XCTAssertNotNil(cancel.target); XCTAssertNotNil(cancel.action)
        XCTAssertNotNil(cancel.accessibilityHelp())
        XCTAssertEqual(dashboard.homeDropZoneFrame.height, 190, accuracy: 1)
    }

    func testHomepageCancelStopsRealImportAndClearsProgress() async throws {
        let dashboard = try dashboard(), shell = try XCTUnwrap(shells.last)
        let directory = shell.recentStore.url.deletingLastPathComponent()
        let text = directory.appendingPathComponent("cancel-sample.txt")
        try Data("Cancellation sample".utf8).write(to: text)
        let views = descendants(dashboard.view)
        let status = try XCTUnwrap(views.first { $0.identifier?.rawValue == "home-file-import-status" })
        let cancel = try XCTUnwrap(views.compactMap { $0 as? NSButton }.first { $0.identifier?.rawValue == "home-file-import-cancel" })
        let label = try XCTUnwrap(descendants(status).compactMap { $0 as? NSTextField }.first)
        let updateUI = shell.onFileImportStateChange
        var transitions: [Bool] = []
        shell.onFileImportStateChange = { running in
            updateUI?(running); transitions.append(running)
            if running {
                XCTAssertFalse(status.isHidden); XCTAssertTrue(cancel.isEnabled)
                cancel.performClick(nil)
                XCTAssertFalse(cancel.isEnabled)
                XCTAssertEqual(label.stringValue, "正在取消导入…")
            } else { XCTAssertTrue(status.isHidden) }
        }
        do {
            _ = try await shell.importDroppedFiles([text])
            XCTFail("Canceled import must not create a tab")
        } catch FilePDFImportError.cancelled {
            // The real converter shares the cancel flag with the native control.
        } catch { XCTFail("Unexpected import error: \(error)") }
        XCTAssertEqual(transitions, [true, false])
        XCTAssertFalse(shell.isFileImportRunning); XCTAssertTrue(shell.documentTabs.isEmpty)
        XCTAssertTrue(status.isHidden); XCTAssertEqual(label.stringValue, "正在本地导入文件…")
        XCTAssertEqual(try String(contentsOf: text, encoding: .utf8), "Cancellation sample")
    }

    func testHomepageProgressClearsAfterSuccessfulRealImport() async throws {
        let dashboard = try dashboard(), shell = try XCTUnwrap(shells.last)
        let text = shell.recentStore.url.deletingLastPathComponent().appendingPathComponent("success-sample.txt")
        try Data("Import sample".utf8).write(to: text)
        let status = try XCTUnwrap(descendants(dashboard.view).first { $0.identifier?.rawValue == "home-file-import-status" })
        let updateUI = shell.onFileImportStateChange
        var transitions: [Bool] = []
        shell.onFileImportStateChange = { running in
            updateUI?(running); transitions.append(running)
            XCTAssertEqual(status.isHidden, !running)
        }
        let tab = try await shell.importDroppedFiles([text])
        XCTAssertEqual(transitions, [true, false])
        XCTAssertFalse(shell.isFileImportRunning); XCTAssertTrue(status.isHidden)
        XCTAssertEqual(tab.controller.workspace.document?.pageCount, 1)
        XCTAssertEqual(try String(contentsOf: text, encoding: .utf8), "Import sample")
    }
}

@MainActor private final class ActionCounter: NSObject {
    var count = 0
    @objc func run() { count += 1 }
}
