import AppKit

/// Presentation is an explicit dependency, not a runtime XCTest heuristic.
/// Interactive app startup retains ordinary macOS window/focus behavior.
enum ApplicationPresentation {
    case interactive
    case background

    @MainActor func present(_ window: NSWindow?, makeKey: Bool = true, activate: Bool = false) {
        guard self == .interactive else { return }
        if makeKey { window?.makeKeyAndOrderFront(nil) }
        else { window?.orderFront(nil) }
        if activate { NSApp.activate(ignoringOtherApps: true) }
    }
}
