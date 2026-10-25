import AppKit
import PDFKit
import PaperCore

/// Previews live in a view overlay, never in the document or its undo history.
@MainActor final class InkPDFView: PDFView, NSMenuItemValidation {
    var inkEnabled = false { didSet { if oldValue != inkEnabled { cancelGesture() } } }
    var placementEnabled = false { didSet { if oldValue != placementEnabled { cancelGesture() } } }
    var contentEditingEnabled = false { didSet { if oldValue != contentEditingEnabled { cancelGesture() } } }
    var freeTextInputEnabled = false
    var inlineTextSession: PDFInlineTextSession?
    weak var textStyleToolbarHost: NSView?
    var onTextStyleToolbarVisibilityChange: ((Bool) -> Void)?
    var onTextAnnotationEditing: ((Int, Int) -> Void)?
    @discardableResult func finishInlineTextEditing(commit: Bool = true) -> Bool {
        inlineTextSession?.finish(commit: commit) ?? true
    }
    var markupSelectionEnabled = false { didSet { if oldValue != markupSelectionEnabled { markupStart = nil; setCurrentSelection(nil, animate: false) } } }
    var onTextSelectionCompleted: ((PDFSelection) -> Void)?
    private var markupStart: (page: PDFPage, point: CGPoint, document: PDFDocument)?
    override var document: PDFDocument? { didSet { if oldValue !== document { _ = finishInlineTextEditing(commit: false); markupStart = nil; restoreNativeInkRendering(); clearShapeSelection() } } }
    var allowsCopying = true
    var shapeTool: ShapeToolKind?
    var shapeStyle = AnnotationStyle(strokeColor: .systemRed, fillColor: nil, lineWidth: 2)
    var inkStyle = AnnotationStyle(strokeColor: .systemIndigo, fillColor: nil, lineWidth: 2)
    var onInk: (([CGPoint], Int, AnnotationStyle) -> Void)?
    var onDropFiles: (([URL]) -> Bool)?
    var onPlacement: ((CGPoint, CGPoint, Int) -> Void)?
    var onContentSelection: ((CGPoint, CGPoint, Int) -> Void)?
    var onAnnotationSelection: ((Int, Int) -> Void)?
    var onShapeTransform: ((Int, Int, CGRect) -> Void)?
    var onShapeDeselection: (() -> Void)?
    var annotationManipulationEnabled = true
    var onCancelPlacement: (() -> Void)?
    var onBlockedWidget: (() -> Void)?
    var onCopyDenied: (() -> Void)?
    var onDeleteShape: (() -> Void)?
    var canDeleteAnnotation: (() -> Bool)?
    var onClipboard: ((ClipboardCommand) -> Bool)?
    var canClipboard: ((ClipboardCommand) -> Bool)?
    private var gesturePage: PDFPage?
    private var points: [CGPoint] = []
    private var receivedMouseDrag = false
    private var transform: (page: Int, index: Int, bounds: CGRect, handle: Int?)?
    private var selectedShape: (page: Int, index: Int)?
    private var selectedInk: (page: Int, index: Int)?
    private let previewLayer = CAShapeLayer()
    private let handlesLayer = CAShapeLayer()
    private let overlay = ShapeOverlayView()
    private(set) var hasShapePreview = false
    private(set) var hasInkPreview = false
    private(set) var hasInkStylePreview = false
    private(set) var inkPreviewPath: CGPath?
    private var gestureInkStyle: AnnotationStyle?
    private let inkClipLayer = CAShapeLayer()
    private var shapePreviewBounds: CGRect?
    private var previewLine: (CGPoint, CGPoint)?
    private var previewStyle: AnnotationStyle?
    private var isCommittingInk = false
    // PDFKit's native Ink display uses an annotation bitmap that can remain at
    // page resolution while the page is zoomed. Keep Ink on the same vector
    // presentation surface as its live preview; the actual PDF remains native Ink.
    private let committedInkLayer = CALayer()
    private final class InkPresentation {
        weak var annotation: PDFAnnotation?
        let layer = CAShapeLayer()
        var geometry: [CGFloat]?
        init(_ annotation: PDFAnnotation) { self.annotation = annotation }
    }
    private var committedInk: [ObjectIdentifier: InkPresentation] = [:]
    // FreeText and imported Stamp appearances share PDFKit's low-resolution
    // annotation cache. Draw the original /AP at viewport zoom and device scale.
    // All presented annotations share one parent so PDF stacking order survives
    // (e.g. a CAD Stamp, then Ink, then arrows drawn above it).
    private final class TextPresentation {
        weak var annotation: PDFAnnotation?
        let layer = FreeTextAppearanceLayer()
        var geometry: [CGFloat]?
        init(_ annotation: PDFAnnotation) { self.annotation = annotation; layer.annotation = annotation }
    }
    private var committedText: [ObjectIdentifier: TextPresentation] = [:]

    override init(frame: NSRect) {
        super.init(frame: frame); wantsLayer = true
        registerForDraggedTypes([.fileURL])
        overlay.frame = bounds; overlay.autoresizingMask = [.width, .height]; overlay.wantsLayer = true
        overlay.layer?.zPosition = 1000
        addSubview(overlay, positioned: .above, relativeTo: nil)
        previewLayer.fillColor = nil
        handlesLayer.fillColor = nil; handlesLayer.strokeColor = NSColor.systemBlue.cgColor
        handlesLayer.lineWidth = 1
        overlay.layer?.addSublayer(previewLayer); overlay.layer?.addSublayer(handlesLayer)
        committedInkLayer.zPosition = -1
        overlay.layer?.addSublayer(committedInkLayer)
        for name in [Notification.Name.PDFViewScaleChanged, .PDFViewPageChanged, NSView.boundsDidChangeNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(redrawOverlay), name: name, object: nil)
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard onDropFiles != nil, sender.draggingSourceOperationMask.contains(.copy), PDFFileDrop.urls(from: sender.draggingPasteboard) != nil else { return [] }
        return .copy
    }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { draggingEntered(sender) }
    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { draggingEntered(sender) == .copy }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard draggingEntered(sender) == .copy, let urls = PDFFileDrop.urls(from: sender.draggingPasteboard) else { return false }
        return onDropFiles?(urls) ?? false
    }
    deinit { NotificationCenter.default.removeObserver(self) }
    override func layout() { super.layout(); redrawOverlay() }
    override func viewDidChangeBackingProperties() { super.viewDidChangeBackingProperties(); redrawOverlay() }

    /// Call after page-local presentation annotations are changed, without
    /// replacing PDFView.document or discarding PDFKit's page tiles.
    func refreshCommittedInkRendering() {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        for entry in committedInk.values { entry.geometry = nil }
        for entry in committedText.values { entry.geometry = nil }
        updateCommittedInkRendering()
        CATransaction.commit()
        if !isCommittingInk { CATransaction.flush() }
    }

    /// Display suppression belongs only to this view. Never serialize that flag
    /// into a lazy thumbnail snapshot or an exported presentation document.
    func presentationDataRepresentation() -> Data? {
        CATransaction.begin(); CATransaction.setDisableActions(true)
        let annotations = committedInk.values.compactMap(\.annotation) + committedText.values.compactMap(\.annotation)
        for annotation in annotations { annotation.shouldDisplay = true }
        defer {
            for annotation in annotations { annotation.shouldDisplay = false }
            CATransaction.commit()
        }
        return document?.dataRepresentation()
    }

    private func restoreNativeInkRendering() {
        for entry in committedInk.values {
            entry.annotation?.shouldDisplay = true
            entry.layer.removeFromSuperlayer()
        }
        committedInk.removeAll()
        for entry in committedText.values {
            entry.annotation?.shouldDisplay = true
            entry.layer.removeFromSuperlayer()
        }
        committedText.removeAll()
    }

    private func updateCommittedInkRendering() {
        updateCommittedTextRendering()
        committedInkLayer.frame = bounds
        let backingScale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        var retained = Set<ObjectIdentifier>()
        // Only visible pages are visited. A 300-page PDF must not be rescanned
        // for every mouse sample while a single stroke is being drawn.
        for page in visiblePages {
            let crop = convert(page.bounds(for: .cropBox), from: page)
            guard crop.intersects(bounds) else { continue }
            let origin = convert(CGPoint.zero, from: page)
            let x = convert(CGPoint(x: 1, y: 0), from: page)
            let y = convert(CGPoint(x: 0, y: 1), from: page)
            let mapping = CGAffineTransform(a: x.x-origin.x, b: x.y-origin.y,
                                            c: y.x-origin.x, d: y.y-origin.y,
                                            tx: origin.x, ty: origin.y)
            for (index, annotation) in page.annotations.enumerated() where annotation.type == "Ink" {
                let key = ObjectIdentifier(annotation)
                guard (annotation.shouldDisplay || committedInk[key] != nil),
                      let paths = annotation.paths, !paths.isEmpty else { continue }
                let entry = committedInk[key] ?? InkPresentation(annotation)
                if committedInk[key] == nil {
                    committedInk[key] = entry
                    annotation.shouldDisplay = false
                    committedInkLayer.addSublayer(entry.layer)
                }
                retained.insert(key)
                let layer = entry.layer
                layer.zPosition = CGFloat(index)
                layer.isHidden = hasInkStylePreview && selectedInkAnnotation === annotation
                let geometry = [mapping.a, mapping.b, mapping.c, mapping.d, mapping.tx, mapping.ty,
                                bounds.width, bounds.height, backingScale]
                guard entry.geometry != geometry else { continue }
                entry.geometry = geometry
                let absolute = CGMutablePath()
                var offset = CGAffineTransform(translationX: annotation.bounds.minX,
                                              y: annotation.bounds.minY).concatenating(mapping)
                for path in paths {
                    if let converted = path.cgPath.copy(using: &offset) { absolute.addPath(converted) }
                }
                layer.frame = bounds; layer.contentsScale = backingScale
                layer.path = absolute; layer.strokeColor = AnnotationStyle(annotation: annotation).strokeColor.cgColor
                layer.fillColor = nil; layer.lineCap = .round; layer.lineJoin = .round
                layer.lineWidth = (annotation.border?.lineWidth ?? 1) * scaleFactor
                let clip = CAShapeLayer(); clip.frame = bounds
                clip.path = CGPath(rect: crop, transform: nil); layer.mask = clip
            }
        }
        for key in Set(committedInk.keys).subtracting(retained) {
            guard let entry = committedInk.removeValue(forKey: key) else { continue }
            entry.annotation?.shouldDisplay = true
            entry.layer.removeFromSuperlayer()
        }
    }

    private func updateCommittedTextRendering() {
        let backingScale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        var retained = Set<ObjectIdentifier>()
        for page in visiblePages {
            let crop = convert(page.bounds(for: .cropBox), from: page)
            guard crop.intersects(bounds) else { continue }
            let origin = convert(CGPoint.zero, from: page)
            let x = convert(CGPoint(x: 1, y: 0), from: page)
            let y = convert(CGPoint(x: 0, y: 1), from: page)
            let mapping = CGAffineTransform(a: x.x-origin.x, b: x.y-origin.y,
                                            c: y.x-origin.x, d: y.y-origin.y,
                                            tx: origin.x, ty: origin.y)
            // A Stamp can contain a whole drawing, not just a small rubber stamp.
            // Render its surrounding graphic annotations too, in source order,
            // so a sharp opaque Stamp cannot cover later native arrows/markup.
            // Interactive links, widgets and popup windows remain PDFKit-owned.
            let hasStamp = page.annotations.contains { $0.type == "Stamp" }
            for (index, annotation) in page.annotations.enumerated() where
                annotation.type == "FreeText" || (hasStamp && !["Ink", "Link", "Widget", "Popup"].contains(annotation.type ?? "")) {
                let key = ObjectIdentifier(annotation)
                guard annotation.shouldDisplay || committedText[key] != nil else { continue }
                // Move the actual appearance while dragging, without changing
                // authoritative bounds or creating repeated undo transactions.
                let liveBounds: CGRect
                if let t = transform, t.handle == nil, let preview = shapePreviewBounds,
                   t.page == document?.index(for: page),
                   page.annotations.indices.contains(t.index), page.annotations[t.index] === annotation {
                    liveBounds = preview
                } else { liveBounds = annotation.bounds }
                let translation = CGAffineTransform(translationX: liveBounds.minX - annotation.bounds.minX,
                                                    y: liveBounds.minY - annotation.bounds.minY)
                let liveMapping = translation.concatenating(mapping)
                // Allocate only the visible annotation rectangle, not a full
                // viewport bitmap for every text block in a long document.
                let frame = convert(liveBounds, from: page).insetBy(dx: -2, dy: -2)
                    .intersection(crop).intersection(bounds).integral
                guard !frame.isEmpty, !frame.isNull else { continue }
                let entry = committedText[key] ?? TextPresentation(annotation)
                if committedText[key] == nil {
                    committedText[key] = entry
                    annotation.shouldDisplay = false
                    committedInkLayer.addSublayer(entry.layer)
                }
                retained.insert(key)
                let layer = entry.layer
                layer.zPosition = CGFloat(index)
                layer.isHidden = inlineTextSession?.editedAnnotation === annotation
                let geometry = [liveMapping.a, liveMapping.b, liveMapping.c, liveMapping.d, liveMapping.tx, liveMapping.ty,
                                frame.minX, frame.minY, frame.width, frame.height, backingScale]
                guard entry.geometry != geometry else { continue }
                entry.geometry = geometry
                layer.frame = frame; layer.contentsScale = backingScale
                layer.pageToLayer = liveMapping.concatenating(CGAffineTransform(translationX: -frame.minX, y: -frame.minY))
                layer.setNeedsDisplay(); layer.displayIfNeeded()
            }
        }
        for key in Set(committedText.keys).subtracting(retained) {
            guard let entry = committedText.removeValue(forKey: key) else { continue }
            entry.annotation?.shouldDisplay = true
            entry.layer.removeFromSuperlayer()
        }
    }

    func selectShape(page: Int, index: Int) {
        selectedInk = nil
        selectedShape = (page, index); redrawOverlay()
    }
    func selectInk(page: Int, index: Int) {
        selectedShape = nil; selectedInk = (page,index); cancelGesture()
    }
    func clearShapeSelection() { selectedShape = nil; selectedInk = nil; cancelGesture() }
    func previewSelectedInkStyle(_ style: AnnotationStyle) {
        guard let selected = selectedInk, let page = document?.page(at:selected.page),
              let annotation = selectedInkAnnotation, let paths = annotation.paths else { return }
        let absolute = CGMutablePath()
        var offset = CGAffineTransform(translationX:annotation.bounds.minX,y:annotation.bounds.minY)
        for path in paths { if let translated = path.cgPath.copy(using:&offset) { absolute.addPath(translated) } }
        gesturePage = page; gestureInkStyle = style; inkPreviewPath = absolute
        hasInkPreview = true; hasInkStylePreview = true; redrawOverlay()
    }
    func previewSelectedStyle(_ style: AnnotationStyle) {
        guard let s = selectedShape, let a = selectedAnnotation, let page = document?.page(at: s.page) else { return }
        gesturePage = page; shapePreviewBounds = a.bounds; previewStyle = style; hasShapePreview = true; redrawOverlay()
    }
    func clearStylePreview() {
        if hasInkStylePreview { cancelGesture(); return }
        previewStyle = nil; shapePreviewBounds = nil; hasShapePreview = false; gesturePage = nil; redrawOverlay()
    }
    private var selectedInkAnnotation: PDFAnnotation? {
        guard let s = selectedInk, let p = document?.page(at:s.page), p.annotations.indices.contains(s.index), p.annotations[s.index].type == "Ink" else { return nil }
        return p.annotations[s.index]
    }
    private var selectedAnnotation: PDFAnnotation? {
        guard let s = selectedShape, let p = document?.page(at: s.page), p.annotations.indices.contains(s.index) else { return nil }
        let a = p.annotations[s.index]
        return ["Square", "Circle", "Line", "FreeText"].contains(a.type ?? "") ? a : nil
    }
    private func clamp(_ point: CGPoint, on page: PDFPage) -> CGPoint {
        var b = page.bounds(for: .cropBox)
        if let style = gestureInkStyle {
            let inset = min(style.lineWidth/2, min(b.width,b.height)/2)
            b = b.insetBy(dx:inset,dy:inset)
        }
        return CGPoint(x: min(max(point.x, b.minX), b.maxX), y: min(max(point.y, b.minY), b.maxY))
    }
    func beginGesture(on page: PDFPage, at point: CGPoint) {
        cancelGesture()
        guard point.x.isFinite, point.y.isFinite else { return }
        if inkEnabled { gestureInkStyle = inkStyle }
        gesturePage = page; points = [clamp(point, on: page)]
    }
    func updateGesture(to point: CGPoint) {
        guard let page = gesturePage, let first = points.first else { return }
        guard point.x.isFinite, point.y.isFinite else { return }
        let end = clamp(point, on: page)
        if points.last != end { points.append(end) }
        if let t = transform {
            var b = t.bounds
            if let h = t.handle {
                let opposite = [CGPoint(x: b.maxX,y: b.maxY), CGPoint(x:b.minX,y:b.maxY), CGPoint(x:b.minX,y:b.minY), CGPoint(x:b.maxX,y:b.minY)][h]
                b = CGRect(x: min(opposite.x,end.x), y: min(opposite.y,end.y), width: abs(end.x-opposite.x), height: abs(end.y-opposite.y))
            } else {
                let crop = page.bounds(for: .cropBox)
                b.origin.x = min(max(b.minX + end.x-first.x,crop.minX),crop.maxX-b.width)
                b.origin.y = min(max(b.minY + end.y-first.y,crop.minY),crop.maxY-b.height)
            }
            shapePreviewBounds = b; hasShapePreview = true
        } else if placementEnabled, shapeTool != nil {
            shapePreviewBounds = CGRect(x:min(first.x,end.x),y:min(first.y,end.y),width:abs(end.x-first.x),height:abs(end.y-first.y))
            previewLine = (first,end); hasShapePreview = true
        } else if gestureInkStyle != nil, points.count > 1 {
            inkPreviewPath = InkStrokePath.make(points: points); hasInkPreview = true
        }
        redrawOverlay()
    }
    func finishGesture() {
        guard !isCommittingInk else { return }
        guard let page = gesturePage, let first = points.first, let last = points.last, let document else { cancelGesture(); return }
        let pageIndex = document.index(for: page)
        let transaction = transform; let b = shapePreviewBounds; let captured = points; let ink = gestureInkStyle
        if pageIndex != NSNotFound, transaction == nil, !contentEditingEnabled,
           inkEnabled, let ink, captured.count > 1 {
            // Keep the last live stroke on screen throughout the synchronous commit.
            // Flush only after PDFKit's persistent annotation and the cleared overlay
            // have joined the same transaction; never deliver an empty handoff frame.
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            isCommittingInk = true
            defer {
                cancelGesture()
                isCommittingInk = false
                CATransaction.commit()
                CATransaction.flush()
            }
            onInk?(captured, pageIndex, ink)
            displayIfNeeded()
            return
        }
        cancelGesture()
        guard pageIndex != NSNotFound else { return }
        if let t = transaction, let b, b.width > 0, b.height > 0, b != t.bounds { onShapeTransform?(t.page,t.index,b) }
        else if transaction == nil {
            if contentEditingEnabled { onContentSelection?(first,last,pageIndex) }
            else if placementEnabled {
                // Content tools accept a click with a default size. Shape tools
                // still require a drag; tiny pointer jitter is not a tiny box.
                if shapeTool == nil {
                    let a = convert(first, from: page), z = convert(last, from: page)
                    let isClick = hypot(z.x-a.x, z.y-a.y) < 4
                    onPlacement?(first, isClick ? first : last, pageIndex)
                } else if first != last { onPlacement?(first,last,pageIndex) }
            }
        }
    }
    private func cancelGesture() {
        receivedMouseDrag = false
        gesturePage = nil; points = []; transform = nil; shapePreviewBounds = nil; previewLine = nil; previewStyle = nil; hasShapePreview = false
        gestureInkStyle = nil; inkPreviewPath = nil; hasInkPreview = false
        hasInkStylePreview = false
        redrawOverlay()
    }
    override func mouseDown(with event: NSEvent) {
        guard finishInlineTextEditing() else { return }
        window?.makeFirstResponder(self)
        let v = convert(event.locationInWindow, from:nil)
        if (freeTextInputEnabled || contentEditingEnabled || event.clickCount == 2 || (!inkEnabled && !placementEnabled && !markupSelectionEnabled)),
           let page = page(for: v, nearest: false), let document,
           let annotation = page.annotations.reversed().first(where: { $0.type == "FreeText" && convert($0.bounds, from: page).contains(v) }),
           let index = page.annotations.firstIndex(of: annotation) {
            if event.clickCount >= 2 {
                clearShapeSelection()
                onTextAnnotationEditing?(document.index(for: page), index)
            } else {
                let pageIndex = document.index(for: page)
                onAnnotationSelection?(pageIndex, index)
                if annotationManipulationEnabled {
                    window?.contentView?.layoutSubtreeIfNeeded()
                    let anchor = convert(convert(event.locationInWindow, from: nil), to: page)
                    selectedShape = (pageIndex, index)
                    beginGesture(on: page, at: anchor)
                    transform = (pageIndex, index, annotation.bounds, nil)
                    redrawOverlay()
                }
            }
            return
        }
        if markupSelectionEnabled {
            markupStart = nil; setCurrentSelection(nil, animate: false)
            guard let document, let page = page(for: v, nearest: false) else { return }
            let point = convert(v, to: page)
            // Starting in empty space must not snap to an unrelated text block.
            let character = page.characterIndex(at: point)
            guard character != NSNotFound, character >= 0, character < page.numberOfCharacters,
                  page.characterBounds(at: character).insetBy(dx: -3, dy: -3).contains(point),
                  page.selection(for: NSRange(location: character, length: 1))?.string?.contains(where: { !$0.isWhitespace }) == true else { return }
            markupStart = (page, point, document)
            return
        }
        // Selected handles win over drawing, making the just-created shape immediately editable.
        let isPlacingContent = placementEnabled && shapeTool == nil
        if annotationManipulationEnabled, !isPlacingContent, !inkEnabled, !contentEditingEnabled, let s = selectedShape, let a = selectedAnnotation, let page = document?.page(at:s.page) {
            let b = convert(a.bounds, from:page)
            let corners = [CGPoint(x:b.minX,y:b.minY),CGPoint(x:b.maxX,y:b.minY),CGPoint(x:b.maxX,y:b.maxY),CGPoint(x:b.minX,y:b.maxY)]
            if let hit = corners.firstIndex(where: { abs($0.x-v.x)<=7 && abs($0.y-v.y)<=7 }) {
                let pagePoint = convert(v,to:page)
                let pageCorners = [CGPoint(x:a.bounds.minX,y:a.bounds.minY),CGPoint(x:a.bounds.maxX,y:a.bounds.minY),CGPoint(x:a.bounds.maxX,y:a.bounds.maxY),CGPoint(x:a.bounds.minX,y:a.bounds.maxY)]
                let nearest = pageCorners.indices.min { hypot(pageCorners[$0].x-pagePoint.x,pageCorners[$0].y-pagePoint.y)<hypot(pageCorners[$1].x-pagePoint.x,pageCorners[$1].y-pagePoint.y) } ?? hit
                beginGesture(on:page,at:pagePoint); transform=(s.page,s.index,a.bounds,nearest); return
            }
        }
        guard let page = page(for:v,nearest:false), let document else { clearShapeSelection(); super.mouseDown(with:event); return }
        let p = convert(v,to:page); let i = document.index(for:page)
        if !isPlacingContent, !inkEnabled, !contentEditingEnabled {
            let a = page.annotations.reversed().first { annotation in
                let b = convert(annotation.bounds, from:page)
                return b.insetBy(dx:-4,dy:-4).contains(v) && annotation.type != "Link"
            }
            if let a, let ai = page.annotations.firstIndex(of:a) {
                if a.type == "Widget" { onBlockedWidget?(); return }
                onAnnotationSelection?(i,ai)
                if annotationManipulationEnabled, ["Square","Circle","Line","FreeText"].contains(a.type ?? "") {
                    // Revealing the style bar can reflow PDFKit's page. Anchor the
                    // gesture after that layout, so selection is not a translation.
                    window?.contentView?.layoutSubtreeIfNeeded()
                    let anchor = convert(convert(event.locationInWindow, from: nil), to: page)
                    selectedShape=(i,ai); beginGesture(on:page,at:anchor); transform=(i,ai,a.bounds,nil); redrawOverlay()
                }
                return
            }
        }
        clearShapeSelection()
        onShapeDeselection?()
        guard inkEnabled || placementEnabled || contentEditingEnabled else { super.mouseDown(with:event); return }
        beginGesture(on:page,at:p)
    }
    override func mouseDragged(with event: NSEvent) {
        if markupSelectionEnabled { updateMarkupSelection(with: event); return }
        guard let page = gesturePage else { super.mouseDragged(with:event); return }
        receivedMouseDrag = true
        updateGesture(to: convert(convert(event.locationInWindow,from:nil),to:page))
    }
    override func mouseUp(with event: NSEvent) {
        if markupSelectionEnabled {
            updateMarkupSelection(with: event)
            guard let start = markupStart else { return }
            markupStart = nil
            let a = convert(start.point, from: start.page), z = convert(event.locationInWindow, from: nil)
            guard hypot(z.x - a.x, z.y - a.y) >= 3,
                  let selection = currentSelection,
                  selection.string?.contains(where: { !$0.isWhitespace }) == true else {
                setCurrentSelection(nil, animate: false); return
            }
            onTextSelectionCompleted?(selection)
            return
        }
        guard let page = gesturePage else { super.mouseUp(with:event); return }
        if transform != nil, !receivedMouseDrag { cancelGesture(); return }
        updateGesture(to: convert(convert(event.locationInWindow,from:nil),to:page)); finishGesture()
    }
    private func updateMarkupSelection(with event: NSEvent) {
        guard let start = markupStart, let document, document === start.document,
              let endPage = page(for: convert(event.locationInWindow, from: nil), nearest: true) else { return }
        let end = convert(convert(event.locationInWindow, from: nil), to: endPage)
        let selection = document.selection(from: start.page, at: start.point, to: endPage, at: end)
        setCurrentSelection(selection, animate: false)
    }
    @objc private func redrawOverlay() {
        inlineTextSession?.reposition()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer {
            CATransaction.commit()
            // Mouse drags run in AppKit's event-tracking loop. Committing updates the
            // model layer, but that loop can defer delivery to the render server until
            // mouseUp unless we flush the transaction explicitly.
            if !isCommittingInk { CATransaction.flush() }
        }
        // PDFKit inserts/rebuilds its own scroll/document views after setDocument.
        // Keep the transparent preview above those surfaces, not under the page.
        if subviews.last !== overlay { addSubview(overlay, positioned: .above, relativeTo: nil) }
        overlay.frame = bounds
        updateCommittedInkRendering()
        previewLayer.frame = bounds; handlesLayer.frame = bounds
        var annotation: PDFAnnotation? = selectedAnnotation
        var page = selectedShape.flatMap { document?.page(at:$0.page) }
        var rect = annotation?.bounds
        var tool = annotation.flatMap { ShapeToolKind.annotationKind($0) }
        var style = annotation.map { AnnotationStyle(annotation: $0) } ?? shapeStyle
        if annotation?.type == "FreeText" { tool = .rectangle; style = AnnotationStyle(strokeColor: .clear, fillColor: nil, lineWidth: 1) }
        if hasShapePreview { page=gesturePage; rect=shapePreviewBounds; if let previewStyle { style=previewStyle } else if transform == nil { tool=shapeTool; style=shapeStyle; annotation=nil } }
        let path = CGMutablePath(); let handles = CGMutablePath()
        if let page, let rect, let tool {
            let b=convert(rect,from:page)
            if tool == .rectangle { path.addRect(b) }
            else if tool == .ellipse { path.addEllipse(in:b) }
            else {
                var a:CGPoint; var z:CGPoint
                if let annotation {
                    let old=annotation.bounds
                    let sx=rect.width/old.width, sy=rect.height/old.height
                    a=CGPoint(x:rect.minX+annotation.startPoint.x*sx,y:rect.minY+annotation.startPoint.y*sy)
                    z=CGPoint(x:rect.minX+annotation.endPoint.x*sx,y:rect.minY+annotation.endPoint.y*sy)
                } else { a=previewLine?.0 ?? rect.origin; z=previewLine?.1 ?? CGPoint(x:rect.maxX,y:rect.maxY) }
                a=convert(a,from:page); z=convert(z,from:page); path.move(to:a); path.addLine(to:z)
                func arrow(_ tip:CGPoint,_ from:CGPoint) { let angle=atan2(tip.y-from.y,tip.x-from.x); let size=max(8,style.lineWidth*scaleFactor*3); for offset:CGFloat in [-0.45,0.45] { path.move(to:tip); path.addLine(to:CGPoint(x:tip.x-size*cos(angle+offset),y:tip.y-size*sin(angle+offset))) } }
                if tool == .arrow || tool == .doubleArrow { arrow(z,a) }; if tool == .doubleArrow { arrow(a,z) }
            }
            if selectedShape != nil, transform != nil || !hasShapePreview {
                handles.addRect(b)
                for p in [CGPoint(x:b.minX,y:b.minY),CGPoint(x:b.maxX,y:b.minY),CGPoint(x:b.maxX,y:b.maxY),CGPoint(x:b.minX,y:b.maxY)] { handles.addRect(CGRect(x:p.x-3,y:p.y-3,width:6,height:6)) }
            }
        }
        var renderedPath: CGPath = path
        if let selected = selectedInk, let page = document?.page(at:selected.page), let annotation = selectedInkAnnotation {
            handles.addRect(convert(annotation.bounds,from:page))
        }
        previewLayer.mask = nil
        if hasInkPreview, let page = gesturePage, let inkPath = inkPreviewPath, let inkStyle = gestureInkStyle {
            // PDFKit's conversion includes crop-origin, page rotation, scroll and zoom.
            let origin = convert(CGPoint.zero, from:page)
            let x = convert(CGPoint(x:1,y:0), from:page), y = convert(CGPoint(x:0,y:1), from:page)
            var mapping = CGAffineTransform(a:x.x-origin.x,b:x.y-origin.y,c:y.x-origin.x,d:y.y-origin.y,tx:origin.x,ty:origin.y)
            renderedPath = inkPath.copy(using:&mapping) ?? path
            style = inkStyle; tool = nil
            inkClipLayer.frame = bounds
            inkClipLayer.path = CGPath(rect:convert(page.bounds(for:.cropBox),from:page),transform:nil)
            previewLayer.mask = inkClipLayer
        }
        previewLayer.path=renderedPath; previewLayer.strokeColor=style.strokeColor.cgColor
        previewLayer.fillColor=(tool == .rectangle || tool == .ellipse) ? style.fillColor?.cgColor : nil
        previewLayer.lineWidth=style.lineWidth*scaleFactor; handlesLayer.path=handles
        previewLayer.lineCap = hasInkPreview ? .round : .butt
        previewLayer.lineJoin = hasInkPreview ? .round : .miter
        // For a static selected object PDFKit already paints it; only handles need drawing.
        previewLayer.isHidden = !hasShapePreview && !hasInkPreview
    }
    /// Native overlay bitmap used to verify mid-gesture pixels without modifying the PDF.
    func renderShapeOverlay(backingScale: CGFloat = 1) -> Data? {
        guard bounds.width > 0, bounds.height > 0, backingScale.isFinite, (1...4).contains(backingScale),
              let bitmap = NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:Int(ceil(bounds.width * backingScale)),pixelsHigh:Int(ceil(bounds.height * backingScale)),bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:0,bitsPerPixel:0),
              let context = NSGraphicsContext(bitmapImageRep:bitmap)?.cgContext else { return nil }
        context.scaleBy(x: backingScale, y: backingScale)
        overlay.layer?.render(in:context)
        return bitmap.representation(using:.png,properties:[:])
    }
    override func copy(_ sender:Any?) {
        if onClipboard?(.copy) == true { return }
        guard isPDFActionAllowed(DocumentMenuSelectors.copy,hasSelection:currentSelection != nil) else { onCopyDenied?(); return }
        super.copy(sender)
    }
    @objc func cut(_ sender:Any?) { if onClipboard?(.cut) != true { onCopyDenied?() } }
    @objc func paste(_ sender:Any?) { _ = onClipboard?(.paste) }
    override func keyDown(with event:NSEvent) {
        if event.keyCode == 53, cancelPlacementForEscape() { return }
        if PageSelectionDeleteKey.matches(event, in: self),
           canDeleteAnnotation?() ?? (selectedAnnotation != nil || selectedInkAnnotation != nil),
           let onDeleteShape { onDeleteShape(); return }
        let commandModifiers = event.modifierFlags.intersection([.command, .control, .option, .shift])
        if commandModifiers.isEmpty, ReaderArrowNavigation.pageDelta(for: event.keyCode) != nil {
            // A selected custom annotation owns arrows before reader navigation.
            // Shapes support precise one-point nudging; ink remains with PDFKit's
            // editing path and must never accidentally turn the page.
            if let selected = selectedShape, let annotation = selectedAnnotation,
               let page = document?.page(at: selected.page) {
                let crop = page.bounds(for: .cropBox)
                var bounds = annotation.bounds
                switch event.keyCode {
                case 123: bounds.origin.x -= 1
                case 124: bounds.origin.x += 1
                case 125: bounds.origin.y -= 1
                case 126: bounds.origin.y += 1
                default: break
                }
                bounds.origin.x = min(max(bounds.minX, crop.minX), crop.maxX - bounds.width)
                bounds.origin.y = min(max(bounds.minY, crop.minY), crop.maxY - bounds.height)
                if bounds != annotation.bounds { onShapeTransform?(selected.page, selected.index, bounds) }
                return
            }
            if selectedInkAnnotation != nil { return }
            guard let document, let currentPage else { super.keyDown(with:event); return }
            let current = document.index(for: currentPage)
            guard current != NSNotFound,
                  let destination = ReaderArrowNavigation.destination(
                    keyCode: event.keyCode, current: current, pageCount: document.pageCount
                  ), let page = document.page(at: destination) else {
                super.keyDown(with:event); return
            }
            if destination != current { go(to: page) }
            return
        }
        super.keyDown(with:event)
    }
    @discardableResult func cancelPlacementForEscape()->Bool {
        if inlineTextSession != nil { return finishInlineTextEditing(commit: false) }
        guard markupSelectionEnabled || inkEnabled || placementEnabled || contentEditingEnabled || gesturePage != nil || selectedShape != nil || selectedInk != nil else { return false }
        markupSelectionEnabled=false; inkEnabled=false; placementEnabled=false; contentEditingEnabled=false; shapeTool=nil; cancelGesture(); clearShapeSelection(); onCancelPlacement?(); return true
    }
    func isPDFActionAllowed(_ action:Selector,hasSelection:Bool)->Bool { if action == DocumentMenuSelectors.copy { return allowsCopying && hasSelection }; return action != DocumentMenuSelectors.cut }
    func validateMenuItem(_ item:NSMenuItem)->Bool {
        if let command = ClipboardCommand(item.action), command != .selectAll {
            if canClipboard?(command) == true { return true }
            if command != .copy { return false }
        }
        return item.action.map { isPDFActionAllowed($0,hasSelection:currentSelection != nil) } ?? true
    }
}

final class FreeTextAppearanceLayer: CALayer {
    weak var annotation: PDFAnnotation?
    var pageToLayer = CGAffineTransform.identity
    override func draw(in context: CGContext) {
        guard let annotation else { return }
        context.saveGState()
        context.setShouldAntialias(true)
        context.interpolationQuality = .high
        context.concatenate(pageToLayer)
        // Native annotation.draw applies the page's box/rotation transform itself.
        // PDFView.convert already includes it; remove that second transform or
        // rotated imported stamps are turned twice and clipped to the wrong area.
        if let page = annotation.page {
            context.concatenate(page.transform(for: .cropBox).inverted())
        }
        annotation.draw(with: .cropBox, in: context)
        context.restoreGState()
    }
}

@MainActor private final class ShapeOverlayView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override var isOpaque: Bool { false }
}

extension ShapeToolKind {
    static func annotationKind(_ a:PDFAnnotation)->ShapeToolKind? {
        switch a.type { case "Square": return .rectangle; case "Circle": return .ellipse
        case "Line": return a.startLineStyle != .none ? .doubleArrow : (a.endLineStyle != .none ? .arrow : .line)
        default: return nil }
    }
}
