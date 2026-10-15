import AppKit
import PDFKit

/// Keyboard owner for the selected pages in the overview grid.
final class PageOverviewCollectionView: NSCollectionView {
    var onDeleteSelection: (() -> Void)?

    override var acceptsFirstResponder: Bool { true }
    override var mouseDownCanMoveWindow: Bool { false }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        super.mouseDown(with: event)
    }

    override func keyDown(with event: NSEvent) {
        if PageSelectionDeleteKey.matches(event, in: self), let onDeleteSelection {
            onDeleteSelection()
            return
        }
        super.keyDown(with: event)
    }
}

/// Keeps page deletion local to the thumbnail sidebar's responder chain.
public final class ReaderPageThumbnailView: PDFThumbnailView {
    public var onDeleteSelection: (() -> Void)?
    private var deleteKeyMonitor: Any?

    deinit {
        if let deleteKeyMonitor { NSEvent.removeMonitor(deleteKeyMonitor) }
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let deleteKeyMonitor { NSEvent.removeMonitor(deleteKeyMonitor) }
        deleteKeyMonitor = nil
        guard window != nil else { return }
        // PDFKit puts a private NSCollectionView inside PDFThumbnailView. That
        // collection consumes Delete instead of forwarding it up the responder
        // chain, so inspect just this surface before its collection receives it.
        deleteKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            self?.handlePageDeletionKey(event) == true ? nil : event
        }
    }

    public override var acceptsFirstResponder: Bool { true }
    public override var mouseDownCanMoveWindow: Bool { false }

    public override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        window?.makeFirstResponder(self)
    }

    public override func keyDown(with event: NSEvent) {
        if handlePageDeletionKey(event) { return }
        super.keyDown(with: event)
    }

    /// Shared by the scoped pre-dispatch hook and the ordinary responder path.
    @discardableResult
    func handlePageDeletionKey(_ event: NSEvent) -> Bool {
        guard let window, event.window === window,
              PageSelectionDeleteKey.matches(event, in: self), let onDeleteSelection else { return false }
        onDeleteSelection()
        return true
    }
}

private enum PageSelectionDeleteKey {
    static func matches(_ event: NSEvent, in view: NSView) -> Bool {
        guard !view.isHiddenOrHasHiddenAncestor,
              event.type == .keyDown, event.keyCode == 51 || event.keyCode == 117,
              event.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty else { return false }
        // Only the active page-selection surface may consume the event. In
        // particular a field editor nested inside it retains normal text deletion.
        guard let window = view.window else { return true }
        guard let responder = window.firstResponder, !(responder is NSTextView), !(responder is NSTextField) else { return false }
        return responder === view || (responder as? NSView)?.isDescendant(of: view) == true
    }
}
