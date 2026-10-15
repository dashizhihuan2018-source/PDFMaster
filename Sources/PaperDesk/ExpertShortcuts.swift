import AppKit

/// Independent bindings from PDF Expert 3.13.4 (1177) compiled MainMenu.nib.
/// Standard menu bindings only; no upstream application resources are bundled.
@MainActor enum ExpertShortcuts {
    static func apply(to item: NSMenuItem, action: Selector) {
        let tools: [Selector: String] = [
            #selector(WorkbenchController.highlight): "h", #selector(WorkbenchController.underline): "u",
            #selector(WorkbenchController.strikeout): "s", #selector(WorkbenchController.addFreeText): "t",
            #selector(WorkbenchController.addPopupNote): "n", #selector(WorkbenchController.addRectangle): "r",
            #selector(WorkbenchController.addEllipse): "o", #selector(WorkbenchController.addLineAnnotation): "l",
            #selector(WorkbenchController.addArrow): "a", #selector(WorkbenchController.addDoubleArrow): "d",
            #selector(WorkbenchController.toggleInk(_:)): "p", #selector(WorkbenchController.selectEraser): "e"
        ]
        if let key = tools[action] {
            item.keyEquivalent = key; item.keyEquivalentModifierMask = [.command, .control]
        } else {
            let standard: [Selector: String] = [
                #selector(WorkbenchController.rotatePage): "r", #selector(WorkbenchController.rotatePageLeft): "l",
                #selector(WorkbenchController.cropCurrentPage): "k", #selector(WorkbenchController.addCurrentPageBookmark): "d"
            ]
            if let key = standard[action] { item.keyEquivalent = key; item.keyEquivalentModifierMask = .command }
        }
    }
}
