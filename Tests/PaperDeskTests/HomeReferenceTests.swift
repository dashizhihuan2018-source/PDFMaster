import AppKit
import XCTest
@testable import PaperDesk

@MainActor final class HomeReferenceTests: XCTestCase {
    private func dashboard() throws -> DashboardViewController {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("PDFMasterHome-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        let shell = ApplicationShellController(recentStore: RecentDocumentStore(url: directory.appendingPathComponent("recents.json")))
        let dashboard = shell.dashboardController
        // The returned view controller references its parent unowned, as in production.
        dashboard.view.setAccessibilityHelp("test retained parent")
        retainedShells.append(shell)
        dashboard.view.frame = NSRect(x: 0, y: 0, width: 1280, height: 780)
        dashboard.refreshResponsiveLayout(); dashboard.view.layoutSubtreeIfNeeded()
        return dashboard
    }
    private var retainedShells: [ApplicationShellController] = []

    func testSwitchingToCanvasResetsOldSectionScrollAndShowsCreateButton() throws {
        let dashboard = try dashboard()
        dashboard.select(.tools)
        dashboard.view.layoutSubtreeIfNeeded()
        let scroll = try XCTUnwrap(dashboard.view.subviews.compactMap { $0 as? NSScrollView }.first)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: 900))
        scroll.reflectScrolledClipView(scroll.contentView)
        XCTAssertGreaterThan(scroll.contentView.bounds.minY, 0)
        dashboard.select(.canvas)
        XCTAssertEqual(scroll.contentView.bounds.minY, 0, accuracy: 1)
    }

    func testDropZoneMatchesReferenceGeometryAndKeepsRealOpenAction() throws {
        let dashboard = try dashboard()
        XCTAssertEqual(dashboard.homeDropZoneFrame.height, 190, accuracy: 1)
        XCTAssertEqual(dashboard.homeDropZoneFrame.midX, dashboard.contentColumnFrame.midX, accuracy: 1)
        XCTAssertTrue(dashboard.homeDropZoneHasDashedBorder)
        XCTAssertTrue(dashboard.homeDropZoneHasPDFGlyph)
        XCTAssertEqual(dashboard.homeOpenButtonStyle, "blue-pill")
        XCTAssertTrue(dashboard.homeOpenButtonHasAction)
        XCTAssertTrue(dashboard.acceptsLocalPDFDrop)
    }

    func testAllToolsIsBorderlessBlueLinkAndRoutesToActualTools() throws {
        let dashboard = try dashboard()
        XCTAssertEqual(dashboard.homeAllToolsButtonStyle, "blue-link")
        XCTAssertTrue(dashboard.homeAllToolsButtonIsBorderless)
        dashboard.activateAllToolsForTesting()
        XCTAssertEqual(dashboard.selectedSection, .tools)
        XCTAssertEqual(dashboard.toolCardFrames.count, 28)
    }

    func testHomeNavigationUsesOwnPDFBrandAndCustomizesWithPill() throws {
        let dashboard = try dashboard()
        XCTAssertTrue(dashboard.homeNavigationUsesBrandIcon)
        XCTAssertEqual(dashboard.homeCustomizeButtonStyle, "neutral-pill")
        XCTAssertFalse(dashboard.homeHasSubscriptionTrialButton)
    }

    func testEmptyDocumentCollectionsHaveCenteredIllustrations() throws {
        let dashboard = try dashboard()
        XCTAssertEqual(dashboard.homeEmptyIllustrationKinds, ["recent-folder", "favorite-folder"])
        for frame in dashboard.homeEmptyIllustrationFrames {
            XCTAssertGreaterThanOrEqual(frame.height, 44)
            XCTAssertEqual(frame.midX, dashboard.contentColumnFrame.midX, accuracy: 1)
        }
    }

    func testHomeRecommendedCardsFillReferenceColumnAndAlignWithDropZone() throws {
        let dashboard = try dashboard()
        XCTAssertEqual(dashboard.toolCardFrames.count, 6)
        let first = try XCTUnwrap(dashboard.toolCardFrames.first)
        let last = try XCTUnwrap(dashboard.toolCardFrames.last)
        XCTAssertEqual(first.minX, dashboard.homeDropZoneFrame.minX, accuracy: 1)
        XCTAssertEqual(last.maxX, dashboard.homeDropZoneFrame.maxX, accuracy: 1)
        for frame in dashboard.toolCardFrames {
            XCTAssertEqual(frame.width, 140, accuracy: 0.5)
            XCTAssertEqual(frame.height, 120, accuracy: 0.5)
        }
    }
}
