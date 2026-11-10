import AppKit

/// Only the unused titlebar background participates in this window gesture.
final class WindowTitlebarStackView: NSStackView {
    weak var blankSpace: NSView?
    override var mouseDownCanMoveWindow: Bool { true }

    @discardableResult func handleBackgroundDoubleClick(_ event: NSEvent) -> Bool {
        guard event.type == .leftMouseDown, event.clickCount == 2, let window,
              event.window === window else { return false }
        let point = convert(event.locationInWindow, from: nil)
        guard bounds.contains(point) else { return false }
        // Tab groups include labels, close buttons and the spaces between them.
        guard !arrangedSubviews.contains(where: { $0 !== blankSpace && $0.frame.contains(point) }) else { return false }
        for type in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            if let button = window.standardWindowButton(type),
               button.bounds.contains(button.convert(event.locationInWindow, from: nil)) { return false }
        }
        window.toggleFullScreen(nil)
        return true
    }

    override func mouseDown(with event: NSEvent) {
        if !handleBackgroundDoubleClick(event) { super.mouseDown(with: event) }
    }
}

final class WindowTitlebarBlankSpace: NSView {
    override var mouseDownCanMoveWindow: Bool { true }
    override func mouseDown(with event: NSEvent) {
        if (superview as? WindowTitlebarStackView)?.handleBackgroundDoubleClick(event) != true {
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
