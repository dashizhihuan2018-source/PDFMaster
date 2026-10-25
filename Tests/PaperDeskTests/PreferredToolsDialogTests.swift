import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class PreferredToolsDialogTests: XCTestCase {
    func testSixRowsDoNotOverlapEachOtherOrResponseButtonsInDarkAppearance() throws {
        try assertDialogGeometry(appearance: .darkAqua)
    }

    func testSixRowsDoNotOverlapEachOtherOrResponseButtonsInLightAppearance() throws {
        try assertDialogGeometry(appearance: .aqua)
    }

    func testCancelLeavesSavedRecommendedToolsUnchanged() throws {
        try withDashboard { shell, dashboard in
            let original = shell.preferredTools.toolIDs
            try shell.preferredTools.replace(with: original)
            let bytes = try Data(contentsOf: shell.preferredTools.url)
            try runDialog(dashboard, response: "取消") { _, popups in
                popups[0].selectItem(at: 0)
            }
            XCTAssertEqual(shell.preferredTools.toolIDs, original)
            XCTAssertEqual(try Data(contentsOf: shell.preferredTools.url), bytes)
        }
    }

    func testSavePersistsSixIndependentlySelectedToolsInOrder() throws {
        try withDashboard { shell, dashboard in
            let selected = ["pdf-to-powerpoint", "redact", "compress", "blank", "annotate", "draw"]
            try runDialog(dashboard, response: "保存") { _, popups in
                for (popup, id) in zip(popups, selected) {
                    popup.selectItem(at: try XCTUnwrap(ToolCatalog.all.firstIndex { $0.id == id }))
                }
            }
            XCTAssertEqual(shell.preferredTools.toolIDs, selected)
            XCTAssertEqual(PreferredToolStore(url: shell.preferredTools.url).toolIDs, selected)
        }
    }

    private func assertDialogGeometry(appearance: NSAppearance.Name) throws {
        _ = NSApplication.shared
        let previousAppearance = NSApp.appearance
        NSApp.appearance = NSAppearance(named: appearance)
        defer { NSApp.appearance = previousAppearance }
        try withDashboard { _, dashboard in
            try runDialog(dashboard, response: "取消") { content, popups in
                let accessory = try XCTUnwrap(popups.first?.superview?.superview)
                XCTAssertGreaterThanOrEqual(accessory.frame.height, 190,
                    "NSAlert must reserve space for all six rows before laying out its response buttons")
                let frames = popups.map { $0.convert($0.bounds, to: content) }
                let responses = Self.descendants(content).compactMap { $0 as? NSButton }
                    .filter { ["保存", "取消"].contains($0.title) }
                XCTAssertEqual(responses.count, 2)
                let responseFrames = responses.map { $0.convert($0.bounds, to: content) }
                for (index, frame) in frames.enumerated() {
                    XCTAssertGreaterThanOrEqual(frame.width, 240)
                    XCTAssertGreaterThanOrEqual(frame.height, 24)
                    XCTAssertTrue(content.bounds.insetBy(dx: -1, dy: -1).contains(frame))
                    for other in frames.dropFirst(index + 1) {
                        XCTAssertFalse(frame.intersects(other), "Every tool needs its own visible row")
                        XCTAssertGreaterThanOrEqual(abs(frame.midY - other.midY), 30)
                    }
                    for responseFrame in responseFrames {
                        XCTAssertFalse(frame.intersects(responseFrame), "Tool choices must not overlap Save or Cancel")
                    }
                }
            }
        }
    }

    private func withDashboard(_ body: (ApplicationShellController, DashboardViewController) throws -> Void) throws {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("PreferredToolsDialog-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: root.appendingPathComponent("recents.json")))
        let dashboard = shell.dashboardController
        _ = dashboard.view
        try body(shell, dashboard)
    }

    private func runDialog(_ dashboard: DashboardViewController, response: String,
                           inspect: @escaping (NSView, [NSPopUpButton]) throws -> Void) throws {
        var inspected = false
        var inspectionError: Error?
        // Inspect the actual alert opened by the production action, not a
        // separately recreated layout that could hide its sizing regression.
        let timer = Timer(timeInterval: 0.04, repeats: true) { _ in
            MainActor.assumeIsolated {
                guard !inspected, let modal = NSApp.modalWindow, let content = modal.contentView else { return }
                inspected = true
                content.layoutSubtreeIfNeeded()
                let popups = Self.descendants(content).compactMap { $0 as? NSPopUpButton }
                XCTAssertEqual(popups.count, 6)
                do {
                    guard popups.count == 6 else { throw CocoaError(.validationMissingMandatoryProperty) }
                    try inspect(content, popups)
                } catch { inspectionError = error }
                if let button = Self.descendants(content).compactMap({ $0 as? NSButton }).first(where: { $0.title == response }) {
                    button.performClick(nil)
                } else {
                    XCTFail("Missing dialog response button")
                    NSApp.abortModal()
                }
            }
        }
        RunLoop.main.add(timer, forMode: .modalPanel)
        RunLoop.main.add(timer, forMode: .common)
        defer { timer.invalidate() }
        let action = NSSelectorFromString("customizeTools")
        XCTAssertTrue(dashboard.responds(to: action))
        _ = dashboard.perform(action)
        XCTAssertTrue(inspected)
        if let inspectionError { throw inspectionError }
    }

    private static func descendants(_ view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }
}
