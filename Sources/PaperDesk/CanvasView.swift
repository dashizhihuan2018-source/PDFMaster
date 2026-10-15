import AppKit
import PDFKit
import ImageIO
import PaperCore

enum CanvasTool: Int, CaseIterable {
    case selection, hand, sticky, text, rectangle, ellipse, connector, ink
    var title: String { ["选择","手掌","便笺","文本","矩形","圆形","连线","手绘"][rawValue] }
    var symbol: String { ["cursorarrow","hand.draw","note.text","textformat","rectangle","circle","line.diagonal","pencil.tip"][rawValue] }
    var kind: CanvasElementKind? {
        switch self { case .sticky:return .sticky; case .text:return .text; case .rectangle:return .rectangle
        case .ellipse:return .ellipse; case .connector:return .connector; case .ink:return .ink
        default:return nil }
    }
}

enum CanvasBackground: String { case light, dark }

/// A finite viewport over an unbounded logical plane. Draft gestures never mutate the board.
@MainActor final class CanvasView: NSView, NSTextFieldDelegate {
    let session: CanvasSession
    var tool: CanvasTool = .selection { didSet { cancelGesture(); finishTextEditing(); needsDisplay = true } }
    private(set) var viewport = CanvasViewport()
    private(set) var selectedID: UUID?
    private(set) var previewElement: CanvasElement?
    var strokeHex = "#26334D"
    var fillHex = "#FFF2AB"
    var lineWidth: Double = 2
    var showsDotGrid = true { didSet { needsDisplay = true } }
    var background: CanvasBackground = .light { didSet { needsDisplay = true } }
    var backgroundColor: NSColor { background == .dark ? .black : .white }
    var onSelectionChange: (() -> Void)?
    var onViewportChange: (() -> Void)?
    var onOpenPDF: ((Data,String) -> Void)?
    var onError: ((Error) -> Void)?
    var onDropFiles: (([URL],CGPoint) -> Bool)?
    private var acceptsDrop = false { didSet { needsDisplay = true } }
    private enum Gesture { case pan, create, move(CanvasElement), resize(CanvasElement) }
    private var gesture: Gesture?
    private var startView = CGPoint.zero
    private var startWorld = CGPoint.zero
    private var initialOffset = CGPoint.zero
    private var gestureTravel: CGFloat = 0
    private var textEditor: NSTextField?
    private var editedID: UUID?
    private var imageCache: [UUID:NSImage] = [:]
    private var lastRefreshedElements: [CanvasElement]
    private var spaceHeld = false

    init(session: CanvasSession) {
        self.session = session; lastRefreshedElements = session.board.elements; super.init(frame:.zero)
        wantsLayer = true
        // macOS 14+ leaves NSView drawing unclipped by default. The logical plane
        // is unbounded, but its native viewport must never cover sibling chrome.
        clipsToBounds = true
        registerForDraggedTypes([.fileURL])
    }
    required init?(coder:NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NotificationCenter.default.removeObserver(self) }
    override func viewWillMove(toWindow newWindow:NSWindow?) {
        NotificationCenter.default.removeObserver(self,name:NSWindow.didResignKeyNotification,object:window)
        if window !== newWindow { suspendPointerInput() }
        if let newWindow {
            NotificationCenter.default.addObserver(self,selector:#selector(suspendPointerInput),
                name:NSWindow.didResignKeyNotification,object:newWindow)
        }
        super.viewWillMove(toWindow:newWindow)
    }
    @objc private func suspendPointerInput() { spaceHeld = false; cancelGesture() }
    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned { spaceHeld = false; cancelGesture() }
        return resigned
    }
    // The app permits dragging its custom tab/title-bar background. Canvas
    // gestures are content input and must never enter NSWindow's drag loop.
    override var mouseDownCanMoveWindow: Bool { false }

    static func acceptsLocalAssetURL(_ url:URL) -> Bool {
        url.isFileURL && (url.host == nil || url.host == "" || url.host == "localhost") &&
            ["pdf","png","jpg","jpeg","tiff","tif","heic"].contains(url.pathExtension.lowercased())
    }
    static func localDropURLs(_ pasteboard:NSPasteboard) -> [URL]? {
        // Read without the file-only filter: that filter silently drops remote
        // members from a mixed batch. We reject the entire batch instead.
        guard let urls = pasteboard.readObjects(forClasses:[NSURL.self],options:nil) as? [URL],
              !urls.isEmpty,urls.count <= 500,urls.allSatisfy({ acceptsLocalAssetURL($0) }) else { return nil }
        return urls
    }
    override func draggingEntered(_ sender:NSDraggingInfo) -> NSDragOperation { draggingUpdated(sender) }
    override func draggingUpdated(_ sender:NSDraggingInfo) -> NSDragOperation {
        acceptsDrop = sender.draggingSourceOperationMask.contains(.copy) && Self.localDropURLs(sender.draggingPasteboard) != nil && onDropFiles != nil
        return acceptsDrop ? .copy : []
    }
    override func draggingExited(_ sender:NSDraggingInfo?) { acceptsDrop = false }
    override func prepareForDragOperation(_ sender:NSDraggingInfo) -> Bool {
        sender.draggingSourceOperationMask.contains(.copy) && Self.localDropURLs(sender.draggingPasteboard) != nil && onDropFiles != nil
    }
    override func performDragOperation(_ sender:NSDraggingInfo) -> Bool {
        acceptsDrop = false
        guard sender.draggingSourceOperationMask.contains(.copy),let urls = Self.localDropURLs(sender.draggingPasteboard) else { return false }
        let point = viewport.worldPoint(from:convert(sender.draggingLocation,from:nil))
        return onDropFiles?(urls,point) ?? false
    }
    override func concludeDragOperation(_ sender:NSDraggingInfo?) { acceptsDrop = false }

    func refresh() {
        let current = session.board.elements
        let before = Dictionary(uniqueKeysWithValues:lastRefreshedElements.map { ($0.id,$0) })
        let after = Dictionary(uniqueKeysWithValues:current.map { ($0.id,$0) })
        if let id = selectedID, after[id] == nil { selectedID = nil }
        // Undoing a vector or renaming/saving the board must not re-decode every
        // embedded PDF/image. Only changed or removed asset bytes invalidate it.
        imageCache = imageCache.filter { id,_ in
            guard let old = before[id],let new = after[id] else { return false }
            return old.kind == new.kind && old.assetData == new.assetData
        }
        var damage = CGRect.null
        for old in lastRefreshedElements where after[old.id] != old { damage = damage.union(paintBounds(of:old)) }
        for new in current where before[new.id] != new { damage = damage.union(paintBounds(of:new)) }
        // A pure order change alters overlap without changing any element value.
        let previousOrder = lastRefreshedElements.filter { after[$0.id] != nil }.map(\.id)
        let currentOrder = current.filter { before[$0.id] != nil }.map(\.id)
        if previousOrder != currentOrder {
            for item in current { damage = damage.union(paintBounds(of:item)) }
        }
        lastRefreshedElements = current
        let visibleDamage = damage.intersection(bounds)
        if !visibleDamage.isNull && !visibleDamage.isEmpty { setNeedsDisplay(visibleDamage) }
    }
    /// Include thick strokes, connector arrowheads and selection handles. Loaded
    /// boards may have points outside their nominal frame; never clip those away.
    private func paintBounds(of item:CanvasElement) -> CGRect {
        var world = item.frame
        if !item.points.isEmpty { world = world.union(Self.pointBounds(item.points)) }
        let extent = item.kind == .connector ? max(12,item.lineWidth*4) : item.lineWidth/2
        let padding = max(CGFloat(extent)*viewport.scale+2,6)
        return CGRect(origin:viewport.viewPoint(from:world.origin),
            size:CGSize(width:world.width*viewport.scale,height:world.height*viewport.scale)).insetBy(dx:-padding,dy:-padding)
    }
    func select(_ id:UUID?) { selectedID = id; onSelectionChange?(); needsDisplay = true }
    func activate(_ id:UUID) {
        guard let item = session.board.elements.first(where:{$0.id == id}) else { return }
        select(id)
        if item.kind == .pdf, let data = item.assetData { onOpenPDF?(data,item.assetName ?? "画布文档.pdf") }
        else if item.kind == .text || item.kind == .sticky { beginTextEditing(item) }
    }
    func zoom(to scale:CGFloat,anchor:CGPoint? = nil) {
        cancelGesture(); finishTextEditing(); viewport.zoom(to:scale,anchor:anchor ?? CGPoint(x:bounds.midX,y:bounds.midY))
        onViewportChange?(); needsDisplay = true
    }
    func pan(by delta:CGSize) {
        cancelGesture(); finishTextEditing(); viewport.pan(by:delta); onViewportChange?(); needsDisplay = true
    }
    func fitContent() {
        cancelGesture(); finishTextEditing()
        guard let content = contentBounds(), bounds.width > 0, bounds.height > 0 else { zoom(to:1); return }
        let scale = min((bounds.width-100)/max(content.width,1),(bounds.height-100)/max(content.height,1))
        viewport = CanvasViewport(scale:max(0.05,min(8,scale)),offset:.zero)
        viewport.pan(by:CGSize(width:bounds.midX-content.midX*viewport.scale,height:bounds.midY-content.midY*viewport.scale))
        needsDisplay = true; onViewportChange?()
    }
    func contentBounds() -> CGRect? {
        let elements = session.board.elements
        guard let first = elements.first else { return nil }
        return elements.dropFirst().reduce(first.frame) { $0.union($1.frame) }.insetBy(dx:-20,dy:-20)
    }
    func beginGesture(at point:CGPoint) {
        finishTextEditing(); cancelGesture()
        guard point.x.isFinite, point.y.isFinite else { return }
        startView = point; startWorld = viewport.worldPoint(from:point); initialOffset = viewport.offset
        gestureTravel = 0
        if tool == .hand || spaceHeld { gesture = .pan; return }
        if tool == .selection {
            if let selected = session.board.elements.first(where:{$0.id == selectedID}),
               hypot(point.x-viewport.viewPoint(from:CGPoint(x:selected.frame.maxX,y:selected.frame.maxY)).x,
                     point.y-viewport.viewPoint(from:CGPoint(x:selected.frame.maxX,y:selected.frame.maxY)).y) <= 10 {
                gesture = .resize(selected); previewElement = selected; return
            }
            let found = session.board.elements.reversed().first { item in
                item.frame.insetBy(dx:-5/viewport.scale,dy:-5/viewport.scale).contains(startWorld)
            }
            select(found?.id)
            if let found { gesture = .move(found); previewElement = found }
            return
        }
        guard let kind = tool.kind else { return }
        let size:CGSize = kind == .sticky ? CGSize(width:200,height:180) : kind == .text ? CGSize(width:240,height:70) : CGSize(width:1,height:1)
        previewElement = CanvasElement(kind:kind,frame:CGRect(origin:startWorld,size:size),
            text:kind == .sticky ? "双击编辑便笺" : kind == .text ? "双击编辑文本" : "",
            strokeHex:strokeHex,fillHex:fillHex,lineWidth:lineWidth,
            points:kind == .ink || kind == .connector ? [startWorld] : [])
        gesture = .create; needsDisplay = true
    }
    func updateGesture(to point:CGPoint) {
        guard let gesture, point.x.isFinite,point.y.isFinite else { return }
        gestureTravel = max(gestureTravel, hypot(point.x-startView.x, point.y-startView.y))
        let world = viewport.worldPoint(from:point)
        switch gesture {
        case .pan:
            viewport = CanvasViewport(scale:viewport.scale,offset:CGPoint(x:initialOffset.x+point.x-startView.x,y:initialOffset.y+point.y-startView.y))
            onViewportChange?()
        case .move(let original):
            var draft = original
            let dx = world.x-startWorld.x, dy = world.y-startWorld.y
            draft.frame = original.frame.offsetBy(dx:dx,dy:dy)
            draft.points = original.points.map { CGPoint(x:$0.x+dx,y:$0.y+dy) }
            previewElement = draft
        case .resize(let original):
            var draft = original
            let width = max(12,original.frame.width+world.x-startWorld.x), height = max(12,original.frame.height+world.y-startWorld.y)
            draft.frame.size = CGSize(width:width,height:height)
            draft.points = original.points.map { CGPoint(x:original.frame.minX+($0.x-original.frame.minX)*width/original.frame.width,
                y:original.frame.minY+($0.y-original.frame.minY)*height/original.frame.height) }
            previewElement = draft
        case .create:
            guard var draft = previewElement else { return }
            if draft.kind == .ink {
                if draft.points.count < 200_000, let last = draft.points.last, hypot(world.x-last.x,world.y-last.y) > 0.3/viewport.scale {
                    draft.points.append(world)
                    let minX = min(draft.frame.minX,world.x), minY = min(draft.frame.minY,world.y)
                    draft.frame = CGRect(x:minX,y:minY,width:max(0.1,max(draft.frame.maxX,world.x)-minX),height:max(0.1,max(draft.frame.maxY,world.y)-minY))
                }
            } else if draft.kind == .connector {
                draft.points = [startWorld,world]; draft.frame = Self.pointBounds(draft.points)
            } else if draft.kind != .text && draft.kind != .sticky {
                draft.frame = CGRect(x:min(startWorld.x,world.x),y:min(startWorld.y,world.y),
                    width:max(0.1,abs(world.x-startWorld.x)),height:max(0.1,abs(world.y-startWorld.y)))
            }
            previewElement = draft
        }
        needsDisplay = true
    }
    func finishGesture() {
        defer { gesture = nil; previewElement = nil; needsDisplay = true }
        guard let gesture, let draft = previewElement else { return }
        switch gesture {
        case .pan:return
        case .create:
            // Measure intent in viewport points, not world units: a one-point
            // mouse jitter must not become a large object when zoomed out.
            if [.rectangle,.ellipse,.connector,.ink].contains(draft.kind), gestureTravel < 3 { return }
            if [.rectangle,.ellipse,.connector,.ink].contains(draft.kind) &&
                (draft.kind == .ink ? draft.points.count < 2 : max(draft.frame.width,draft.frame.height) <= 1) { return }
            do { try session.mutate { $0.elements.append(draft) }; select(draft.id) } catch { onError?(error) }
        case .move(let original), .resize(let original):
            guard draft != original else { return }
            do { try session.mutate { board in
                guard let index = board.elements.firstIndex(where:{$0.id == draft.id}) else { return }
                board.elements[index] = draft
            } } catch { onError?(error) }
        }
    }
    func cancelGesture() {
        guard gesture != nil || previewElement != nil else { return }
        gesture = nil; previewElement = nil; needsDisplay = true
    }
    func commitText(_ text:String,for id:UUID) {
        do { try session.mutate { board in
            if let index = board.elements.firstIndex(where:{$0.id == id}) { board.elements[index].text = text }
        } } catch { onError?(error) }
    }
    func applyStyle(stroke:String,fill:String,width:Double) {
        guard width.isFinite,width >= 0.1,width <= 128 else { return }
        strokeHex = stroke; fillHex = fill; lineWidth = width
        if var draft = previewElement { draft.strokeHex = stroke; draft.fillHex = fill; draft.lineWidth = width; previewElement = draft }
        if let id = selectedID, gesture == nil {
            do { try session.mutate { board in
                guard let i = board.elements.firstIndex(where:{$0.id == id}) else { return }
                board.elements[i].strokeHex = stroke; board.elements[i].fillHex = fill; board.elements[i].lineWidth = width
            } } catch { onError?(error) }
        }
        needsDisplay = true
    }
    func deleteSelection() {
        finishTextEditing(); guard let id = selectedID else { return }
        do { try session.mutate { $0.elements.removeAll(where:{$0.id == id}) }; select(nil) } catch { onError?(error) }
    }
    private static func pointBounds(_ points:[CGPoint]) -> CGRect {
        guard let first = points.first else { return CGRect(x:0,y:0,width:1,height:1) }
        let minX = points.map(\.x).min() ?? first.x, minY = points.map(\.y).min() ?? first.y
        return CGRect(x:minX,y:minY,width:max(0.1,(points.map(\.x).max() ?? first.x)-minX),height:max(0.1,(points.map(\.y).max() ?? first.y)-minY))
    }
    override func mouseDown(with event:NSEvent) {
        window?.makeFirstResponder(self)
        let point = convert(event.locationInWindow,from:nil)
        if let item = session.board.elements.reversed().first(where:{$0.frame.contains(viewport.worldPoint(from:point))}) {
            let editsText = (tool == .text || tool == .sticky) && (item.kind == .text || item.kind == .sticky)
            // A double-click includes a first single click. Creation tools
            // must edit an existing text object on that first click, otherwise
            // they insert a duplicate before clickCount ever reaches two.
            if event.clickCount == 2 || (editsText && !spaceHeld) { activate(item.id); return }
        }
        beginGesture(at:point)
    }
    override func mouseDragged(with event:NSEvent) { updateGesture(to:convert(event.locationInWindow,from:nil)) }
    override func mouseUp(with event:NSEvent) {
        updateGesture(to:convert(event.locationInWindow,from:nil))
        var textDraftID: UUID?
        if case .create? = gesture, let draft = previewElement, draft.kind == .text || draft.kind == .sticky {
            textDraftID = draft.id
        }
        finishGesture()
        // Start editing only after a successful creation, outside the gesture's
        // cleanup; the first keystroke should replace the selected prompt.
        if let id = textDraftID, session.board.elements.contains(where:{$0.id == id}) { activate(id) }
    }
    override func scrollWheel(with event:NSEvent) {
        let dx = event.scrollingDeltaX, dy = event.scrollingDeltaY
        guard dx.isFinite, dy.isFinite else { return }
        if event.modifierFlags.contains(.shift) || tool == .hand || spaceHeld {
            pan(by:CGSize(width:dx,height:dy)); return
        }
        guard dy != 0 else { return }
        // Discrete wheel notches need more gain than trackpad pixel deltas.
        // Cap a single event's exponent, then CanvasViewport applies 5%–800%.
        let gain:CGFloat = event.hasPreciseScrollingDeltas ? 1/120 : 1/12
        let exponent = min(4,max(-4,dy*gain))
        zoom(to:viewport.scale*exp(exponent),anchor:convert(event.locationInWindow,from:nil))
    }
    override func magnify(with event:NSEvent) { zoom(to:viewport.scale*(1+event.magnification),anchor:convert(event.locationInWindow,from:nil)) }
    override func keyDown(with event:NSEvent) {
        if event.keyCode == 49 { spaceHeld = true; return }
        if event.keyCode == 53 { cancelGesture(); finishTextEditing(commit:false); return }
        if event.keyCode == 51 || event.keyCode == 117 { deleteSelection(); return }
        if event.modifierFlags.contains(.command), event.charactersIgnoringModifiers?.lowercased() == "z" {
            finishTextEditing(); cancelGesture()
            if event.modifierFlags.contains(.shift) { session.redo() } else { session.undo() }
            return
        }
        super.keyDown(with:event)
    }
    override func keyUp(with event:NSEvent) { if event.keyCode == 49 { spaceHeld = false } else { super.keyUp(with:event) } }
    private func beginTextEditing(_ item:CanvasElement) {
        finishTextEditing(); let origin = viewport.viewPoint(from:item.frame.origin)
        let editor = NSTextField(frame:CGRect(x:origin.x+8,y:origin.y+8,width:max(50,item.frame.width*viewport.scale-16),height:max(24,item.frame.height*viewport.scale-16)))
        editor.stringValue = item.text; editor.font = .systemFont(ofSize:18*viewport.scale)
        editor.isBordered = true; editor.backgroundColor = Self.color(item.fillHex); editor.textColor = Self.color(item.strokeHex)
        editor.isEditable = true; editor.isSelectable = true; editor.delegate = self
        addSubview(editor); textEditor = editor; editedID = item.id; window?.makeFirstResponder(editor)
    }
    func finishTextEditing(commit:Bool = true) {
        guard let editor = textEditor else { return }
        let fieldEditor = editor.currentEditor() as? NSTextView
        let id = editedID, text = fieldEditor?.string ?? editor.stringValue
        let ownsKeyboardFocus = window?.firstResponder === editor ||
            (fieldEditor != nil && window?.firstResponder === fieldEditor)
        textEditor = nil; editedID = nil
        // Removing the field alone leaves its shared field editor as first responder.
        // End that editing session before disposal, but do not steal another control's focus.
        if ownsKeyboardFocus { window?.makeFirstResponder(self) }
        editor.removeFromSuperview()
        if commit, let id { commitText(text,for:id) }
    }
    func controlTextDidEndEditing(_ obj:Notification) { finishTextEditing() }

    static func color(_ hex:String) -> NSColor {
        let raw = String(hex.dropFirst()); guard let value = UInt64(raw,radix:16),raw.count == 6 || raw.count == 8 else { return .black }
        let rgba = raw.count == 6 ? (value << 8)|255 : value
        return NSColor(srgbRed:CGFloat((rgba >> 24)&255)/255,green:CGFloat((rgba >> 16)&255)/255,blue:CGFloat((rgba >> 8)&255)/255,alpha:CGFloat(rgba&255)/255)
    }
    override func draw(_ dirtyRect:NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let damage = dirtyRect.intersection(bounds)
        guard !damage.isNull && !damage.isEmpty else { return }
        context.saveGState(); defer { context.restoreGState() }
        context.clip(to:damage)
        backgroundColor.setFill(); damage.fill()
        if showsDotGrid { drawGrid(context) }
        context.saveGState(); context.translateBy(x:viewport.offset.x,y:viewport.offset.y); context.scaleBy(x:viewport.scale,y:viewport.scale)
        for item in session.board.elements where item.id != previewElement?.id && paintBounds(of:item).intersects(damage) { draw(item,in:context) }
        if let previewElement,paintBounds(of:previewElement).intersects(damage) { draw(previewElement,in:context) }
        if let item = previewElement ?? session.board.elements.first(where:{$0.id == selectedID}) {
            context.setStrokeColor(NSColor.systemBlue.cgColor); context.setLineWidth(1.5/viewport.scale)
            context.setLineDash(phase:0,lengths:[4/viewport.scale,3/viewport.scale]); context.stroke(item.frame.insetBy(dx:-3/viewport.scale,dy:-3/viewport.scale)); context.setLineDash(phase:0,lengths:[])
            let size = 8/viewport.scale; context.setFillColor(NSColor.systemBlue.cgColor)
            context.fill(CGRect(x:item.frame.maxX-size/2,y:item.frame.maxY-size/2,width:size,height:size))
        }
        context.restoreGState()
        if acceptsDrop {
            context.setStrokeColor(NSColor.controlAccentColor.cgColor); context.setLineWidth(3)
            context.stroke(bounds.insetBy(dx:3,dy:3))
        }
    }
    private func drawGrid(_ context:CGContext) {
        let spacing = max(12,24*viewport.scale)
        let x0 = viewport.offset.x.truncatingRemainder(dividingBy:spacing), y0 = viewport.offset.y.truncatingRemainder(dividingBy:spacing)
        context.setFillColor(NSColor.tertiaryLabelColor.cgColor)
        for x in stride(from:x0,to:bounds.width,by:spacing) {
            for y in stride(from:y0,to:bounds.height,by:spacing) { context.fillEllipse(in:CGRect(x:x,y:y,width:1.3,height:1.3)) }
        }
    }
    private func draw(_ item:CanvasElement,in context:CGContext) {
        context.saveGState(); defer { context.restoreGState() }
        let frame = item.frame
        context.setStrokeColor(Self.color(item.strokeHex).cgColor); context.setFillColor(Self.color(item.fillHex).cgColor)
        context.setLineWidth(item.lineWidth); context.setLineCap(.round); context.setLineJoin(.round)
        switch item.kind {
        case .rectangle,.sticky:
            let path = CGPath(roundedRect:frame,cornerWidth:item.kind == .sticky ? 8 : 0,cornerHeight:item.kind == .sticky ? 8 : 0,transform:nil)
            context.addPath(path); context.drawPath(using:.fillStroke)
            if item.kind == .sticky { drawText(item.text,frame:frame.insetBy(dx:14,dy:14),color:Self.color(item.strokeHex)) }
        case .ellipse: context.fillEllipse(in:frame); context.strokeEllipse(in:frame)
        case .text: drawText(item.text,frame:frame.insetBy(dx:8,dy:8),color:Self.color(item.strokeHex))
        case .ink:
            context.addPath(InkStrokePath.make(points:item.points)); context.strokePath()
        case .connector:
            if let a = item.points.first,let b = item.points.last {
                context.move(to:a); context.addLine(to:b); context.strokePath()
                let angle = atan2(b.y-a.y,b.x-a.x), length = max(12,item.lineWidth*4)
                context.move(to:CGPoint(x:b.x-length*cos(angle-0.45),y:b.y-length*sin(angle-0.45))); context.addLine(to:b)
                context.addLine(to:CGPoint(x:b.x-length*cos(angle+0.45),y:b.y-length*sin(angle+0.45))); context.strokePath()
            }
        case .image,.pdf:
            NSColor.white.setFill(); frame.fill(); context.stroke(frame)
            if let image = image(for:item) {
                let area = frame.insetBy(dx:8,dy:8); let ratio = min(area.width/image.size.width,area.height/image.size.height)
                let size = CGSize(width:image.size.width*ratio,height:image.size.height*ratio)
                image.draw(in:CGRect(x:area.midX-size.width/2,y:area.midY-size.height/2,width:size.width,height:size.height),from:.zero,operation:.sourceOver,fraction:1,respectFlipped:true,hints:nil)
            }
            if item.kind == .pdf {
                let label = CGRect(x:frame.minX,y:frame.maxY-30,width:frame.width,height:30)
                NSColor(calibratedWhite:0.94,alpha:1).setFill(); label.fill()
                drawText("PDF · "+(item.assetName ?? "文档"),frame:label.insetBy(dx:8,dy:5),color:.darkGray,size:12)
            }
        }
    }
    private func image(for item:CanvasElement) -> NSImage? {
        if let cached = imageCache[item.id] { return cached }
        guard let data = item.assetData else { return nil }
        let image: NSImage?
        if item.kind == .pdf,let document = PDFDocument(data:data) {
            image = withExtendedLifetime(document) {
                document.page(at:0)?.thumbnail(of:CGSize(width:480,height:640),for:.cropBox)
            }
        }
        else if let source = CGImageSourceCreateWithData(data as CFData,[kCGImageSourceShouldCache:false] as CFDictionary),
                let thumbnail = CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,
                    kCGImageSourceThumbnailMaxPixelSize:1024,kCGImageSourceCreateThumbnailWithTransform:true] as CFDictionary) {
            image = NSImage(cgImage:thumbnail,size:CGSize(width:thumbnail.width,height:thumbnail.height))
        } else { image = nil }
        if let image {
            // Decoded thumbnails are larger than compressed board assets. Keep
            // a bounded cache rather than retaining every item in a large board.
            if imageCache.count >= 32 { imageCache.removeAll(keepingCapacity:true) }
            imageCache[item.id] = image
        }; return image
    }
    private func drawText(_ text:String,frame:CGRect,color:NSColor,size:CGFloat = 18) {
        let p = NSMutableParagraphStyle(); p.lineBreakMode = .byWordWrapping
        (text as NSString).draw(in:frame,withAttributes:[.font:NSFont.systemFont(ofSize:size),.foregroundColor:color,.paragraphStyle:p])
    }
    func renderPNG() -> Data? { renderPNGData() }
    func renderPNGData(allContent:Bool = false) -> Data? {
        let content = allContent ? contentBounds() ?? CGRect(x:0,y:0,width:800,height:600) : bounds
        guard content.width.isFinite,content.height.isFinite,content.width > 0,content.height > 0 else { return nil }
        let scale:CGFloat = min(1,4096/max(content.width,content.height))
        let width = max(1,Int(ceil(content.width*scale))),height = max(1,Int(ceil(content.height*scale)))
        guard let bitmap = NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:width,pixelsHigh:height,bitsPerSample:8,samplesPerPixel:4,
                hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:width*4,bitsPerPixel:32),
            let graphics = NSGraphicsContext(bitmapImageRep:bitmap) else { return nil }
        let previous = NSGraphicsContext.current
        defer { NSGraphicsContext.current = previous }
        let context = graphics.cgContext; context.translateBy(x:0,y:CGFloat(height)); context.scaleBy(x:scale,y:-scale)
        NSGraphicsContext.current = NSGraphicsContext(cgContext:context,flipped:true)
        if allContent {
            context.setFillColor(NSColor.white.cgColor); context.fill(CGRect(x:0,y:0,width:content.width,height:content.height))
            context.translateBy(x:-content.minX,y:-content.minY)
            for item in session.board.elements { draw(item,in:context) }
        } else { draw(bounds) }
        return bitmap.representation(using:.png,properties:[:])
    }
    func exportPDFData() -> Data? {
        finishTextEditing(); cancelGesture()
        let content = contentBounds() ?? CGRect(x:0,y:0,width:800,height:600)
        guard content.width.isFinite,content.height.isFinite else { return nil }
        let scale = min(1,14400/max(content.width,content.height))
        var rect = CGRect(x:0,y:0,width:max(1,content.width*scale),height:max(1,content.height*scale))
        let data = NSMutableData(); guard let consumer = CGDataConsumer(data:data as CFMutableData),
            let context = CGContext(consumer:consumer,mediaBox:&rect,nil) else { return nil }
        context.beginPDFPage(nil)
        context.setFillColor(NSColor.white.cgColor); context.fill(rect)
        context.translateBy(x:0,y:rect.height); context.scaleBy(x:scale,y:-scale); context.translateBy(x:-content.minX,y:-content.minY)
        let previous = NSGraphicsContext.current
        NSGraphicsContext.current = NSGraphicsContext(cgContext:context,flipped:true)
        for item in session.board.elements { draw(item,in:context) }
        NSGraphicsContext.current = previous
        context.endPDFPage(); context.closePDF(); return data as Data
    }
}
