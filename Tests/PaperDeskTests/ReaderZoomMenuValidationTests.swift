import AppKit
import PDFKit
import XCTest
@testable import PaperDesk

@MainActor
final class ReaderZoomMenuValidationTests: XCTestCase {
    func testNativeZoomPopupMenuUpdateEnablesPresetsWithDocument() throws {
        let (commands, popup, window) = try fixture(hasDocument: true)
        defer { window.orderOut(nil); _ = commands }
        let menu = try XCTUnwrap(popup.menu)
        XCTAssertTrue(menu.autoenablesItems)

        menu.update()

        XCTAssertEqual(menu.items.count, 10)
        XCTAssertTrue(menu.items.allSatisfy(\.isEnabled), "Native menu.update must not disable popup presets: \(menu.items.map { "\($0.title)=\($0.isEnabled)" })")
    }

    func testNativeZoomPopupMenuUpdateEnablesDynamicallyInserted85Percent() throws {
        let (commands, popup, window) = try fixture(hasDocument: true)
        defer { window.orderOut(nil); _ = commands }
        popup.addItem(withTitle: "85%")

        popup.menu?.update()

        XCTAssertTrue(try XCTUnwrap(popup.item(withTitle: "85%")).isEnabled)
    }

    func testNativeZoomPopupMenuUpdateDisablesPresetsAndDynamicPercentWithoutDocument() throws {
        let (commands, popup, window) = try fixture(hasDocument: false)
        defer { window.orderOut(nil); _ = commands }
        popup.addItem(withTitle: "85%")

        popup.menu?.update()

        XCTAssertTrue(popup.itemArray.allSatisfy { !$0.isEnabled })
    }

    func testPopupSelectionsDispatchRealSessionZoom() throws {
        let (commands, popup, window) = try fixture(hasDocument: true)
        defer { window.orderOut(nil); _ = commands }
        popup.addItem(withTitle: "85%")
        for (title, zoom) in [("适合宽度", ReaderZoom.fitWidth), ("适合页面", .fitPage), ("125%", .percent(125)), ("85%", .percent(85))] {
            popup.menu?.update()
            XCTAssertTrue(try XCTUnwrap(popup.item(withTitle: title)).isEnabled)
            popup.selectItem(withTitle: title)
            XCTAssertTrue(popup.sendAction(try XCTUnwrap(popup.action), to: popup.target))
            XCTAssertEqual(commands.session.currentZoom, zoom)
            if case .percent(let percent) = zoom {
                XCTAssertEqual(commands.session.pdfView.scaleFactor, percent / 100, accuracy: 1e-9)
            }
        }
    }

    func testExistingPopupRevalidatesWhenDocumentIsRemovedAndRestored() throws {
        let (commands, popup, window) = try fixture(hasDocument: true)
        defer { window.orderOut(nil) }
        popup.addItem(withTitle: "85%")
        let document = try XCTUnwrap(commands.session.pdfView.document)
        popup.menu?.update()
        XCTAssertTrue(popup.itemArray.allSatisfy(\.isEnabled))

        commands.session.setDocument(nil, allowsCopying: false)
        popup.menu?.update()
        XCTAssertTrue(popup.itemArray.allSatisfy { !$0.isEnabled })

        commands.session.setDocument(document, allowsCopying: true)
        popup.menu?.update()
        XCTAssertTrue(popup.itemArray.allSatisfy(\.isEnabled))
    }

    func testUnavailablePopupActionCannotBypassDocumentValidation() throws {
        let (commands, popup, window) = try fixture(hasDocument: false)
        defer { window.orderOut(nil) }
        var errors = 0
        commands.onError = { _ in errors += 1 }
        popup.selectItem(withTitle: "适合宽度")

        XCTAssertTrue(popup.sendAction(try XCTUnwrap(popup.action), to: popup.target))

        XCTAssertNil(commands.session.currentZoom)
        XCTAssertEqual(errors, 0, "Disabled popup actions should be ignored, not attempt a missing-document fit")
    }

    func testZoomPopupValidationDoesNotRelaxCommandOrForeignMenuPermissions() throws {
        let (commands, popup, window) = try fixture(hasDocument: true, allowsCopying: false)
        defer { window.orderOut(nil) }
        popup.menu?.update()
        let menu = commands.makeMenu(commands: [.find, .findSelection, .actualSize], title: "Reader")
        menu.update()
        XCTAssertFalse(menu.items[0].isEnabled)
        XCTAssertFalse(menu.items[1].isEnabled)
        XCTAssertTrue(menu.items[2].isEnabled)
        let foreign = NSMenuItem(title: "85%", action: nil, keyEquivalent: "")
        XCTAssertFalse(commands.validateMenuItem(foreign), "A percentage title outside the owned zoom popup is not authorization")
        popup.addItem(withTitle: "0%")
        popup.addItem(withTitle: "6401%")
        popup.addItem(withTitle: "not zoom")
        popup.menu?.update()
        for title in ["0%", "6401%", "not zoom"] {
            XCTAssertFalse(try XCTUnwrap(popup.item(withTitle: title)).isEnabled)
        }
    }

    private func fixture(hasDocument: Bool, allowsCopying: Bool = true) throws -> (ReaderCommandController, NSPopUpButton, NSWindow) {
        _ = NSApplication.shared
        let view = PDFView(frame: NSRect(x: 0, y: 0, width: 900, height: 650))
        let session = ReaderSession(pdfView: view)
        if hasDocument {
            let document = PDFDocument()
            let page = PDFPage()
            page.setBounds(CGRect(x: 0, y: 0, width: 600, height: 840), for: .mediaBox)
            document.insert(page, at: 0)
            session.setDocument(document, allowsCopying: allowsCopying)
        }
        let sidebar = ReaderSidebarController(session: session)
        let commands = ReaderCommandController(session: session, sidebar: sidebar)
        let popup = commands.makeZoomPopup()
        popup.frame = NSRect(x: 20, y: 20, width: 180, height: 28)
        let host = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        host.addSubview(popup)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        return (commands, popup, window)
    }
}
