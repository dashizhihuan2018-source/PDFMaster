import AppKit

/// Only the unused titlebar background participates in this window gesture.
final class WindowTitlebarStackView: NSStackView {
    weak var blankSpace: NSView?
    var visibleScreenFrame: (NSWindow) -> NSRect? = { $0.screen?.visibleFrame }
    private var restoreFrame: NSRect?
    private var maximizedFrame: NSRect?
    // Receive the click before handing a single click to native window dragging.
    override var mouseDownCanMoveWindow: Bool { false }

    @discardableResult func handleBackgroundMouseDown(_ event: NSEvent) -> Bool {
        guard event.type == .leftMouseDown, [1, 2].contains(event.clickCount), let window,
              event.window === window else { return false }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return false }
        // Tab groups include labels, close buttons and the spaces between them.
        guard !arrangedSubviews.contains(where: { $0 !== blankSpace && $0.frame.contains(point) }) else { return false }
        for type in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            if let button = window.standardWindowButton(type),
               button.bounds.contains(button.convert(event.locationInWindow, from: nil)) { return false }
        }
        if event.clickCount == 2 {
            guard !window.styleMask.contains(.fullScreen),
                  let visibleFrame = visibleScreenFrame(window), visibleFrame.width > 0, visibleFrame.height > 0 else { return false }
            if let restoreFrame, let maximizedFrame, Self.sameFrame(window.frame, maximizedFrame) {
                window.setFrame(restoreFrame, display: true)
                self.restoreFrame = nil; self.maximizedFrame = nil
            } else {
                restoreFrame = window.frame; maximizedFrame = visibleFrame
                window.setFrame(visibleFrame, display: true)
            }
        } else { window.performDrag(with: event) }
        return true
    }

    private static func sameFrame(_ a: NSRect, _ b: NSRect) -> Bool {
        abs(a.minX - b.minX) < 1 && abs(a.minY - b.minY) < 1 && abs(a.width - b.width) < 1 && abs(a.height - b.height) < 1
    }

    override func mouseDown(with event: NSEvent) {
        if !handleBackgroundMouseDown(event) { super.mouseDown(with: event) }
    }
}

final class WindowTitlebarBlankSpace: NSView {
    override var mouseDownCanMoveWindow: Bool { false }
    override func mouseDown(with event: NSEvent) {
        if (superview as? WindowTitlebarStackView)?.handleBackgroundMouseDown(event) != true {
            super.mouseDown(with: event)
        }
    }
}

enum ShellPalette {
    static let canvas = NSColor(srgbRed: 0.071, green: 0.071, blue: 0.071, alpha: 1)
    static let sidebar = NSColor(srgbRed: 0.125, green: 0.129, blue: 0.129, alpha: 1)
    static let card = NSColor(srgbRed: 0.118, green: 0.118, blue: 0.118, alpha: 1)
    static let top = NSColor(srgbRed: 0.09, green: 0.09, blue: 0.102, alpha: 1)
    static let primary = NSColor(calibratedWhite: 0.933, alpha: 1)
    static let secondary = NSColor(calibratedWhite: 0.6, alpha: 1)
    static let accent = NSColor(srgbRed: 0.039, green: 0.471, blue: 1, alpha: 1)
}
