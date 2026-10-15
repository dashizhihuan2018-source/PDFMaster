import AppKit
import PDFKit
import Darwin

@MainActor
final class PageOverviewController: NSViewController, NSCollectionViewDataSource, NSCollectionViewDelegateFlowLayout {
    let collectionView = PageOverviewCollectionView()
    private let scrollView = NSScrollView()
    private let flowLayout = NSCollectionViewFlowLayout()
    private let thumbnailRenderer = PageOverviewThumbnailRenderer()
    private var presentationDocument: PDFDocument?
    private var presentationSnapshot: Data?
    private var selectionAnchor: Int?
    private var documentGeneration: UInt64 = 0
    private let dragDiagnosticsEnabled = ProcessInfo.processInfo.environment["PDFMASTER_OVERVIEW_DRAG_DIAGNOSTICS"] == "1"
        || UserDefaults.standard.bool(forKey: "PDFMasterOverviewDragDiagnostics")
    private var lastDragDiagnostic: String?
    private let dragSourceID = UUID()
    private static let pageDragType = NSPasteboard.PasteboardType("cn.nankong.pdfmaster.page-overview-pages")
    private struct DraggedPage: Codable {
        let sourceID: UUID
        let generation: UInt64
        let page: Int
    }
    private(set) var fallbackSnapshotSerializationCount = 0

    private(set) var selectedPages = IndexSet()
    private(set) var thumbnailScale: CGFloat = 1
    private(set) var columns = 1
    private(set) var itemSize = NSSize(width: 180, height: 283)

    var onOpenPage: ((Int) -> Void)?
    var onSelectionChange: ((IndexSet) -> Void)?
    /// External file copies only. The caller performs one atomic workspace merge.
    var onDropPDFs: (([URL], Int) -> Bool)?
    /// Move existing pages atomically in the workspace. The insertion boundary is
    /// expressed in the original document's coordinates (0...pageCount).
    var onMovePages: ((IndexSet, Int) -> Bool)?
    var onDeleteSelection: (() -> Void)? {
        get { collectionView.onDeleteSelection }
        set { collectionView.onDeleteSelection = newValue }
    }
    var thumbnailRendererMaximumConcurrency: Int { thumbnailRenderer.maximumConcurrency }
    var thumbnailCacheCostLimit: Int { thumbnailRenderer.cacheCostLimit }
    var thumbnailDocumentDecodeCount: Int { thumbnailRenderer.documentDecodeCount }
    var thumbnailHasRetainedDocumentForTesting: Bool { thumbnailRenderer.hasRetainedDocument }
    func waitForThumbnailRendererForTesting() { thumbnailRenderer.waitUntilIdle() }

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 800, height: 600))
        root.wantsLayer = true
        root.layer?.backgroundColor = ShellPalette.canvas.cgColor

        flowLayout.sectionInset = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        flowLayout.minimumInteritemSpacing = 16
        flowLayout.minimumLineSpacing = 20
        collectionView.collectionViewLayout = flowLayout
        collectionView.dataSource = self
        collectionView.delegate = self
        collectionView.isSelectable = true
        collectionView.allowsMultipleSelection = true
        collectionView.backgroundColors = [.clear]
        collectionView.register(PageOverviewItem.self, forItemWithIdentifier: PageOverviewItem.identifier)
        collectionView.registerForDraggedTypes([.fileURL, Self.pageDragType])
        collectionView.setDraggingSourceOperationMask(.move, forLocal: true)
        collectionView.setDraggingSourceOperationMask([], forLocal: false)
        let doubleClick = NSClickGestureRecognizer(target: self, action: #selector(openDoubleClickedPage(_:)))
        doubleClick.numberOfClicksRequired = 2
        // NSClickGestureRecognizer otherwise delays the collection's mouse-down
        // tracking until double-click recognition fails, stalling native drags.
        doubleClick.delaysPrimaryMouseButtonEvents = false
        collectionView.addGestureRecognizer(doubleClick)
        collectionView.setAccessibilityLabel("PDF 整页缩略图网格")

        scrollView.documentView = collectionView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: root.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor)
        ])
        view = root
        refreshLayout()
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        refreshLayout()
    }

    /// The caller must pass the read-only PDFDocument currently displayed by InkPDFView,
    /// never PDFWorkspace.document. Page indices are zero based.
    func setDocument(_ document: PDFDocument?, selectedPage: Int, snapshot: Data? = nil) {
        let sameDocument = presentationDocument === document
        let sameKnownBytes = snapshot == nil || snapshot == presentationSnapshot
        if sameDocument, sameKnownBytes {
            updateSelection(selectedPage, in: document)
            return
        }
        documentGeneration &+= 1
        dragDiagnostic("document-changed")
        presentationDocument = document
        // refreshDocument already serializes the workspace once to build this
        // immutable presentation document. Reuse those exact bytes when supplied
        // instead of synchronously serializing the full PDF again on the main actor.
        let rendererSnapshot: Data?
        if let snapshot { rendererSnapshot = snapshot }
        else {
            if document != nil { fallbackSnapshotSerializationCount += 1 }
            rendererSnapshot = document?.dataRepresentation()
        }
        presentationSnapshot = rendererSnapshot
        thumbnailRenderer.advance(to: documentGeneration, snapshot: rendererSnapshot)
        updateSelection(selectedPage, in: document)
        collectionView.reloadData()
    }

    private func updateSelection(_ selectedPage: Int, in document: PDFDocument?) {
        selectionAnchor = nil
        if let document, document.pageCount > 0 {
            let page = min(max(0, selectedPage), document.pageCount - 1)
            selectedPages = IndexSet(integer: page)
            selectionAnchor = page
        } else {
            selectedPages = []
        }
        applySelectionToCollectionView(scroll: false)
        onSelectionChange?(selectedPages)
    }

    func restoreSelection(_ pages: IndexSet) {
        guard let count = presentationDocument?.pageCount, count > 0 else {
            selectedPages = []
            selectionAnchor = nil
            applySelectionToCollectionView(scroll: false)
            onSelectionChange?(selectedPages)
            return
        }
        selectedPages = pages.intersection(IndexSet(integersIn: 0..<count))
        selectionAnchor = selectedPages.last
        applySelectionToCollectionView(scroll: false)
        onSelectionChange?(selectedPages)
    }

    func setThumbnailScale(_ scale: CGFloat) {
        thumbnailScale = min(2, max(0.5, scale.isFinite ? scale : 1))
        refreshLayout()
    }

    func revealSelection() {
        let generation = documentGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self, self.documentGeneration == generation else { return }
            self.view.layoutSubtreeIfNeeded()
            self.applySelectionToCollectionView(scroll: true)
        }
    }

    func refreshLayout() {
        guard isViewLoaded else { return }
        let width = view.bounds.width.isFinite ? max(1, min(20000, view.bounds.width)) : 800
        let horizontalInset = flowLayout.sectionInset.left + flowLayout.sectionInset.right
        let available = max(1, width - horizontalInset)
        let targetWidth = 180 * thumbnailScale
        let proposed = (available + flowLayout.minimumInteritemSpacing) / (targetWidth + flowLayout.minimumInteritemSpacing)
        columns = max(1, Int(proposed.rounded()))
        let gaps = CGFloat(max(0, columns - 1)) * flowLayout.minimumInteritemSpacing
        let actualWidth = max(72, (available - gaps) / CGFloat(columns))
        itemSize = NSSize(width: actualWidth, height: actualWidth * 1.414 + 28)
        if flowLayout.itemSize != itemSize { flowLayout.itemSize = itemSize }
        // Assigning itemSize already invalidates changed geometry. Repeated layout
        // callbacks must not evict pending cells while their thumbnails render.
    }

    /// Testable counterpart of native click/Command-click/Shift-click collection selection.
    func selectPage(_ page: Int, extending: Bool, range: Bool) {
        guard let document = presentationDocument, (0..<document.pageCount).contains(page) else { return }
        if range, let anchor = selectionAnchor {
            let range = min(anchor, page)...max(anchor, page)
            if extending { selectedPages.formUnion(IndexSet(integersIn: range)) }
            else { selectedPages = IndexSet(integersIn: range) }
        } else if extending {
            if selectedPages.contains(page) { selectedPages.remove(page) }
            else { selectedPages.insert(page) }
            selectionAnchor = page
        } else {
            selectedPages = IndexSet(integer: page)
            selectionAnchor = page
        }
        applySelectionToCollectionView(scroll: false)
        onSelectionChange?(selectedPages)
    }

    func openPage(at page: Int) {
        guard let document = presentationDocument, (0..<document.pageCount).contains(page) else { return }
        onOpenPage?(page)
    }

    func numberOfSections(in collectionView: NSCollectionView) -> Int { 1 }

    func collectionView(_ collectionView: NSCollectionView, pasteboardWriterForItemAt indexPath: IndexPath) -> NSPasteboardWriting? {
        guard onMovePages != nil, indexPath.section == 0,
              let count = presentationDocument?.pageCount, (0..<count).contains(indexPath.item),
              let data = try? JSONEncoder().encode(DraggedPage(sourceID: dragSourceID, generation: documentGeneration, page: indexPath.item)) else { return nil }
        let item = NSPasteboardItem()
        item.setData(data, forType: Self.pageDragType)
        dragDiagnostic("writer ready")
        return item
    }

    func collectionView(_ collectionView: NSCollectionView, draggingSession session: NSDraggingSession, willBeginAt screenPoint: NSPoint, forItemsAt indexPaths: Set<IndexPath>) {
        dragDiagnostic("session-begin items=\(indexPaths.count) pasteboardItems=\(session.draggingPasteboard.pasteboardItems?.count ?? 0)")
    }

    func collectionView(_ collectionView: NSCollectionView, draggingSession session: NSDraggingSession, endedAt screenPoint: NSPoint, dragOperation operation: NSDragOperation) {
        dragDiagnostic("session-end operation=\(operation.rawValue)")
    }

    func collectionView(_ collectionView: NSCollectionView, validateDrop draggingInfo: NSDraggingInfo, proposedIndexPath proposedDropIndexPath: AutoreleasingUnsafeMutablePointer<NSIndexPath>, dropOperation proposedDropOperation: UnsafeMutablePointer<NSCollectionView.DropOperation>) -> NSDragOperation {
        dragDiagnostic("validate source=\(draggingInfo.draggingSource.map { String(describing: type(of: $0)) } ?? "nil") sameCollection=\((draggingInfo.draggingSource as? NSCollectionView) === collectionView) mask=\(draggingInfo.draggingSourceOperationMask.rawValue)")
        guard let count = presentationDocument?.pageCount, count > 0 else { dragDiagnostic("validate rejected=no-document"); return [] }
        let internalPages = internalDraggedPages(from: draggingInfo)
        guard internalPages != nil || (canReceivePDFDrop(draggingInfo) && PDFFileDrop.urls(from: draggingInfo.draggingPasteboard) != nil) else { dragDiagnostic("validate rejected=unsupported"); return [] }
        let point = collectionView.convert(draggingInfo.draggingLocation, from: nil)
        let insertion: Int
        if let hit = collectionView.indexPathForItem(at: point),
           let attributes = collectionView.collectionViewLayout?.layoutAttributesForItem(at: hit) {
            // "On" must never replace a page. Choose its left/right insertion edge.
            // Preserve AppKit's existing before-item proposal in the edge zone.
            if proposedDropOperation.pointee == .before,
               proposedDropIndexPath.pointee.section == 0,
               (0...count).contains(proposedDropIndexPath.pointee.item) {
                insertion = proposedDropIndexPath.pointee.item
            } else {
                insertion = min(count, hit.item + (point.x >= attributes.frame.midX ? 1 : 0))
            }
        } else {
            // The free grid background (including below the last row) appends.
            insertion = count
        }
        proposedDropIndexPath.pointee = IndexPath(item: insertion, section: 0) as NSIndexPath
        proposedDropOperation.pointee = .before
        if let internalPages {
            let effective = isEffectiveMove(internalPages, to: insertion)
            dragDiagnostic("validate internal effective=\(effective) insertion=\(insertion) items=\(internalPages.count)")
            return effective ? .move : []
        }
        dragDiagnostic("validate external copy")
        return .copy
    }

    func collectionView(_ collectionView: NSCollectionView, acceptDrop draggingInfo: NSDraggingInfo, indexPath: IndexPath, dropOperation: NSCollectionView.DropOperation) -> Bool {
        dragDiagnostic("accept entered")
        guard let count = presentationDocument?.pageCount, count > 0,
              dropOperation == .before, indexPath.section == 0,
              (0...count).contains(indexPath.item) else { dragDiagnostic("accept rejected=destination"); return false }
        if let pages = internalDraggedPages(from: draggingInfo) {
            guard isEffectiveMove(pages, to: indexPath.item) else { dragDiagnostic("accept rejected=no-op"); return false }
            let accepted = onMovePages?(pages, indexPath.item) ?? false
            dragDiagnostic("accept internal callback=\(accepted)")
            return accepted
        }
        guard canReceivePDFDrop(draggingInfo),
              let urls = PDFFileDrop.urls(from: draggingInfo.draggingPasteboard),
              let onDropPDFs else { dragDiagnostic("accept rejected=unsupported"); return false }
        return onDropPDFs(urls, indexPath.item)
    }

    private func internalDraggedPages(from info: NSDraggingInfo) -> IndexSet? {
        guard onMovePages != nil else { dragDiagnostic("internal rejected=no-receiver"); return nil }
        guard (info.draggingSource as? NSCollectionView) === collectionView else { dragDiagnostic("internal rejected=source"); return nil }
        guard info.draggingSourceOperationMask.contains(.move) else { dragDiagnostic("internal rejected=mask"); return nil }
        guard let count = presentationDocument?.pageCount else { dragDiagnostic("internal rejected=no-document"); return nil }
        guard let items = info.draggingPasteboard.pasteboardItems, !items.isEmpty else { dragDiagnostic("internal rejected=no-items"); return nil }
        var pages = IndexSet()
        for item in items {
            guard let data = item.data(forType: Self.pageDragType) else { dragDiagnostic("internal rejected=missing-type"); return nil }
            guard let payload = try? JSONDecoder().decode(DraggedPage.self, from: data) else { dragDiagnostic("internal rejected=decode"); return nil }
            guard payload.sourceID == dragSourceID else { dragDiagnostic("internal rejected=owner"); return nil }
            guard payload.generation == documentGeneration else { dragDiagnostic("internal rejected=generation"); return nil }
            guard (0..<count).contains(payload.page), !pages.contains(payload.page) else { dragDiagnostic("internal rejected=page"); return nil }
            pages.insert(payload.page)
        }
        return pages
    }

    /// Opt-in local diagnostics only: no file paths, document text, UUIDs or
    /// pasteboard payload bytes are logged. Does not change drop decisions.
    private func dragDiagnostic(_ message: @autoclosure () -> String) {
        guard dragDiagnosticsEnabled else { return }
        let message = message()
        guard message != lastDragDiagnostic else { return }
        lastDragDiagnostic = message
        let data = Data("[PageOverviewDrag] \(message)\n".utf8)
        FileHandle.standardOutput.write(data)
        // Append only, and refuse symlinks at the fixed local diagnostic path.
        // A failed log write must never affect document or drag-session behavior.
        let descriptor = Darwin.open("/tmp/pdfmaster-overview-drag-diagnostics.log", O_WRONLY | O_CREAT | O_APPEND | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard descriptor >= 0 else { return }
        let log = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        try? log.write(contentsOf: data)
    }

    private func isEffectiveMove(_ pages: IndexSet, to insertion: Int) -> Bool {
        guard let first = pages.first, let last = pages.last else { return false }
        return pages.count != last - first + 1 || !(first...(last + 1)).contains(insertion)
    }

    private func canReceivePDFDrop(_ info: NSDraggingInfo) -> Bool {
        guard onDropPDFs != nil, info.draggingSourceOperationMask.contains(.copy) else { return false }
        // Page reordering is a different operation. Do not treat an internal
        // collection drag as an external document import.
        return (info.draggingSource as? NSCollectionView) !== collectionView
    }

    func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int) -> Int {
        presentationDocument?.pageCount ?? 0
    }

    func collectionView(_ collectionView: NSCollectionView, itemForRepresentedObjectAt indexPath: IndexPath) -> NSCollectionViewItem {
        let item = collectionView.makeItem(withIdentifier: PageOverviewItem.identifier, for: indexPath)
        guard let item = item as? PageOverviewItem, presentationDocument?.page(at: indexPath.item) != nil else { return item }
        let token = item.configurePlaceholder(pageNumber: indexPath.item + 1)
        let generation = documentGeneration
        let size = thumbnailRequestSize(page: indexPath.item, itemWidth: itemSize.width)
        thumbnailRenderer.render(page: indexPath.item, size: size, generation: generation) { [weak self, weak item] image, _ in
            DispatchQueue.main.async {
                guard let self, self.documentGeneration == generation else { return }
                item?.applyThumbnail(image, token: token)
            }
        }
        return item
    }

    func thumbnailRequestSize(page: Int, itemWidth: CGFloat) -> NSSize {
        guard let pdfPage = presentationDocument?.page(at: page) else { return NSSize(width: max(80, itemWidth * 2), height: max(80, itemWidth * 2 * 1.414)) }
        let bounds = pdfPage.bounds(for: .cropBox)
        let rotated = abs(pdfPage.rotation) % 180 == 90
        let pageWidth = rotated ? bounds.height : bounds.width
        let pageHeight = rotated ? bounds.width : bounds.height
        let width = max(80, itemWidth * 2)
        let height = pageWidth > 0 && pageHeight > 0 ? width * pageHeight / pageWidth : width * 1.414
        return Self.boundedPixelSize(NSSize(width: width, height: max(80, height)))
    }

    nonisolated static func boundedPixelSize(_ size: NSSize) -> NSSize {
        guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else { return NSSize(width: 360, height: 510) }
        let width = min(1e8, size.width), height = min(1e8, size.height)
        let factor = min(1, 4096 / max(width, height), sqrt(4_000_000 / (width * height)))
        return NSSize(width: max(1, (width * factor).rounded()), height: max(1, (height * factor).rounded()))
    }

    func renderThumbnailForTesting(page: Int, size: NSSize, completion: @escaping (NSImage?, Bool) -> Void) {
        thumbnailRenderer.render(page: page, size: size, generation: documentGeneration, completion: completion)
    }

    func collectionView(_ collectionView: NSCollectionView, didSelectItemsAt indexPaths: Set<IndexPath>) {
        updateSelectionFromCollectionView(preferredAnchor: indexPaths.sorted().last?.item)
    }

    func collectionView(_ collectionView: NSCollectionView, didDeselectItemsAt indexPaths: Set<IndexPath>) {
        updateSelectionFromCollectionView(preferredAnchor: nil)
    }

    func collectionView(_ collectionView: NSCollectionView, layout collectionViewLayout: NSCollectionViewLayout, sizeForItemAt indexPath: IndexPath) -> NSSize {
        guard let page = presentationDocument?.page(at: indexPath.item) else { return itemSize }
        let bounds = page.bounds(for: .cropBox)
        let rotated = abs(page.rotation) % 180 == 90
        let pageWidth = rotated ? bounds.height : bounds.width
        let pageHeight = rotated ? bounds.width : bounds.height
        guard pageWidth.isFinite, pageHeight.isFinite, pageWidth > 0, pageHeight > 0 else { return itemSize }
        return NSSize(width: itemSize.width, height: itemSize.width * min(10, max(0.1, pageHeight / pageWidth)) + 28)
    }

    private func updateSelectionFromCollectionView(preferredAnchor: Int?) {
        let valid = collectionView.selectionIndexPaths.map(\.item).filter { index in
            guard let count = presentationDocument?.pageCount else { return false }
            return (0..<count).contains(index)
        }
        selectedPages = IndexSet(valid)
        if let preferredAnchor { selectionAnchor = preferredAnchor }
        onSelectionChange?(selectedPages)
    }

    private func applySelectionToCollectionView(scroll: Bool) {
        let paths = Set(selectedPages.map { IndexPath(item: $0, section: 0) })
        collectionView.selectionIndexPaths = paths
        if scroll, let page = selectedPages.first { collectionView.scrollToItems(at: [IndexPath(item: page, section: 0)], scrollPosition: .centeredVertically) }
    }

    @objc private func openDoubleClickedPage(_ recognizer: NSClickGestureRecognizer) {
        guard recognizer.state == .ended else { return }
        let point = recognizer.location(in: collectionView)
        if let page = collectionView.indexPathForItem(at: point)?.item { openPage(at: page) }
    }
}

@MainActor
private final class PageOverviewItemView: NSView {
    override var mouseDownCanMoveWindow: Bool { false }
}

@MainActor
private final class PageOverviewPageLabel: NSTextField {
    override var mouseDownCanMoveWindow: Bool { false }
}

@MainActor
private final class PageOverviewItem: NSCollectionViewItem {
    static let identifier = NSUserInterfaceItemIdentifier("PageOverviewItem")
    private let thumbnail = NSImageView()
    private let pageLabel = PageOverviewPageLabel(labelWithString: "")
    private var reuseToken = UUID()

    override func loadView() {
        let root = PageOverviewItemView()
        root.wantsLayer = true
        root.layer?.cornerRadius = 5
        root.layer?.borderWidth = 2
        thumbnail.wantsLayer = true
        thumbnail.layer?.backgroundColor = NSColor.white.cgColor
        thumbnail.imageScaling = .scaleProportionallyUpOrDown
        thumbnail.translatesAutoresizingMaskIntoConstraints = false
        pageLabel.alignment = .center
        pageLabel.font = .monospacedDigitSystemFont(ofSize: 12, weight: .medium)
        pageLabel.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(thumbnail)
        root.addSubview(pageLabel)
        NSLayoutConstraint.activate([
            thumbnail.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 5),
            thumbnail.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -5),
            thumbnail.topAnchor.constraint(equalTo: root.topAnchor, constant: 5),
            thumbnail.bottomAnchor.constraint(equalTo: pageLabel.topAnchor, constant: -5),
            pageLabel.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            pageLabel.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            pageLabel.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -4),
            pageLabel.heightAnchor.constraint(equalToConstant: 18)
        ])
        view = root
        updateSelectionAppearance()
    }

    override var isSelected: Bool {
        didSet { updateSelectionAppearance() }
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        reuseToken = UUID()
        thumbnail.image = nil
        pageLabel.stringValue = ""
    }

    func configurePlaceholder(pageNumber: Int) -> UUID {
        reuseToken = UUID()
        thumbnail.image = nil
        pageLabel.stringValue = "\(pageNumber)"
        view.setAccessibilityLabel("第 \(pageNumber) 页")
        return reuseToken
    }

    func applyThumbnail(_ image: NSImage?, token: UUID) {
        guard token == reuseToken else { return }
        thumbnail.image = image
    }

    private func updateSelectionAppearance() {
        guard isViewLoaded else { return }
        view.layer?.borderColor = (isSelected ? NSColor.controlAccentColor : NSColor.clear).cgColor
        view.layer?.backgroundColor = (isSelected ? NSColor.controlAccentColor.withAlphaComponent(0.12) : NSColor.clear).cgColor
    }
}

private final class PageOverviewThumbnailRenderer {
    let maximumConcurrency = 1
    let cacheCostLimit = 48 * 1_024 * 1_024
    private let queue: OperationQueue
    private let workerQueue: DispatchQueue
    private let cache = NSCache<NSString, NSImage>()
    private let lock = NSLock()
    private var snapshot: Data?
    private var generation: UInt64 = 0
    private var decodeCount = 0
    /// Accessed only by the renderer's serial OperationQueue.
    private var renderDocument: (generation: UInt64, document: PDFDocument)?
    private var resetOperation: Operation?
    var documentDecodeCount: Int {
        lock.lock(); defer { lock.unlock() }
        return decodeCount
    }
    var hasRetainedDocument: Bool {
        lock.lock(); defer { lock.unlock() }
        return renderDocument != nil
    }
    func waitUntilIdle() { queue.waitUntilAllOperationsAreFinished() }

    init() {
        workerQueue = DispatchQueue(label: "PaperDesk.PageOverviewThumbnailRenderer.worker", qos: .userInitiated)
        queue = OperationQueue()
        queue.name = "PaperDesk.PageOverviewThumbnailRenderer"
        queue.qualityOfService = .userInitiated
        queue.maxConcurrentOperationCount = maximumConcurrency
        queue.underlyingQueue = workerQueue
        cache.totalCostLimit = cacheCostLimit
        cache.countLimit = 256
    }

    func advance(to generation: UInt64, snapshot: Data?) {
        queue.cancelAllOperations()
        cache.removeAllObjects()
        lock.lock()
        self.generation = generation
        self.snapshot = snapshot
        lock.unlock()

        // PDFDocument is created, retained and discarded only inside the serial
        // worker context. OperationQueue does not promise a fixed OS thread, so
        // the isolation guarantee here is serial non-concurrent access rather
        // than thread affinity; the object is never exposed to the main actor.
        let reset = BlockOperation { [weak self] in
            guard let self else { return }
            self.lock.lock()
            if self.generation == generation { self.renderDocument = nil }
            self.lock.unlock()
        }
        lock.lock(); resetOperation = reset; lock.unlock()
        queue.addOperation(reset)
    }

    func render(page: Int, size: NSSize, generation: UInt64, completion: @escaping (NSImage?, Bool) -> Void) {
        let bounded = PageOverviewController.boundedPixelSize(size)
        let pixelWidth = Int(bounded.width)
        let pixelHeight = Int(bounded.height)
        let key = "\(generation):\(page):\(pixelWidth)x\(pixelHeight)" as NSString
        if let cached = cache.object(forKey: key) {
            completion(cached, Thread.isMainThread)
            return
        }
        lock.lock()
        let active = self.generation == generation
        let data = active ? snapshot : nil
        lock.unlock()
        guard let data else { completion(nil, Thread.isMainThread); return }

        let operation = BlockOperation { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let stillCurrent = self.generation == generation
            self.lock.unlock()
            guard stillCurrent else { return }
            let document: PDFDocument
            self.lock.lock()
            let retained = self.renderDocument
            self.lock.unlock()
            if let retained, retained.generation == generation {
                document = retained.document
            } else {
                guard let decoded = PDFDocument(data: data) else { return }
                document = decoded
                self.lock.lock()
                if self.generation == generation { self.renderDocument = (generation, decoded) }
                self.decodeCount += 1
                self.lock.unlock()
            }
            guard let pdfPage = document.page(at: page) else { return }
            let renderedOnMainThread = Thread.isMainThread
            let image = pdfPage.thumbnail(of: NSSize(width: pixelWidth, height: pixelHeight), for: .cropBox)
            self.lock.lock()
            let publish = self.generation == generation
            self.lock.unlock()
            guard publish else { return }
            let cost = pixelWidth * pixelHeight * 4
            self.cache.setObject(image, forKey: key, cost: cost)
            completion(image, renderedOnMainThread)
        }
        lock.lock(); let reset = resetOperation; lock.unlock()
        if let reset, !reset.isFinished { operation.addDependency(reset) }
        queue.addOperation(operation)
    }
}
