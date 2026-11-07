import AppKit

@MainActor
final class PDFWindowSession: Identifiable {
    let id = UUID()
    let controller: ApplicationShellController
    let window: NSWindow

    init(controller: ApplicationShellController, window: NSWindow) {
        self.controller = controller
        self.window = window
    }
}

enum TabDragPolicy {
    static let threshold: CGFloat = 6

    static func hasCrossedThreshold(from start: NSPoint, to current: NSPoint) -> Bool {
        hypot(current.x - start.x, current.y - start.y) >= threshold
    }

}

@MainActor
final class TabDragTracker {
    let tabID: UUID
    private let start: NSPoint
    private let isInsideTabBar: (NSPoint) -> Bool
    private let onTransfer: (UUID) -> Bool
    private(set) var isDragging = false
    private var isCancelled = false

    init(tabID: UUID, start: NSPoint, isInsideTabBar: @escaping (NSPoint) -> Bool,
         onTransfer: @escaping (UUID) -> Bool) {
        self.tabID = tabID
        self.start = start
        self.isInsideTabBar = isInsideTabBar
        self.onTransfer = onTransfer
    }

    @discardableResult func update(to point: NSPoint) -> Bool {
        if !isDragging { isDragging = TabDragPolicy.hasCrossedThreshold(from: start, to: point) }
        return isDragging
    }

    func cancel() { isCancelled = true }

    @discardableResult func finish(at point: NSPoint) -> Bool {
        defer { isDragging = false; isCancelled = false }
        guard isDragging, !isCancelled, !isInsideTabBar(point) else { return false }
        return onTransfer(tabID)
    }
}
