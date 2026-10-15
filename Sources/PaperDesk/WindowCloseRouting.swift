import AppKit

@MainActor enum WindowCloseRouting {
    static func perform(keyWindow: NSWindow?, mainWindow: NSWindow?, closeTab: () -> Void) {
        if let keyWindow, keyWindow !== mainWindow {
            if keyWindow.styleMask.contains(.closable) { keyWindow.performClose(nil) }
            return
        }
        closeTab()
    }
}
