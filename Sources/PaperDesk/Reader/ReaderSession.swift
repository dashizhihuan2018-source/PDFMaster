import AppKit
import PDFKit

public enum ReaderLayout: CaseIterable { case single, twoPages, twoPagesWithCover }
public enum ReaderZoom: Equatable { case percent(Double), fitPage, fitWidth }
public enum ReaderError: Error { case invalidZoom, noDocument, invalidPage, copyingDenied }
public struct ReaderHit {
    public let pageIndex: Int
    public let pageLabel: String
    public let excerpt: String
    public let selection: PDFSelection
}
@MainActor public final class ReaderSession {
    public let pdfView: PDFView
    public private(set) var hits: [ReaderHit] = []
    public private(set) var selectedHit: Int?
    public private(set) var query = ""
    public private(set) var searchStatus = "输入关键词搜索"
    public private(set) var isSearching = false
    public private(set) var allowsCopying = false
    private(set) var documentSnapshot: Data?
    /// Nil leaves the initial PDFKit presentation unmanaged until a zoom command.
    public private(set) var currentZoom: ReaderZoom?
    public var onChange: (() -> Void)?
    private var searchTask: Task<Void, Never>?
    private var generation: UInt64 = 0
    private var reachedLimit = false
    private var zoomObservers: [NSObjectProtocol] = []
    private var geometryObservers: [NSObjectProtocol] = []
    private var rotationObservers: [NSKeyValueObservation] = []
    private weak var observedDocumentView: NSView?
    private weak var observedClipView: NSClipView?
    private var observedPages: [ObjectIdentifier] = []
    private var fitScheduled = false
    private var forceNextFit = false
    private var applyingFit = false
    private var lastFit: FitGeometry?

    private struct FitGeometry: Equatable {
        var document: ObjectIdentifier
        var zoom: ReaderZoom
        var viewport: CGSize
        var indices: [Int]
        var sizes: [CGSize]
        var gutter: CGFloat
    }
    public var currentLayout: ReaderLayout {
        switch pdfView.displayMode {
        case .twoUp, .twoUpContinuous: return pdfView.displaysAsBook ? .twoPagesWithCover : .twoPages
        default: return .single
        }
    }
    public init(pdfView: PDFView) {
        self.pdfView = pdfView
        pdfView.postsFrameChangedNotifications = true
        pdfView.postsBoundsChangedNotifications = true
        for name in [NSView.frameDidChangeNotification, NSView.boundsDidChangeNotification,
                     .PDFViewPageChanged, .PDFViewDocumentChanged, .PDFViewDisplayModeChanged,
                     .PDFViewDisplayBoxChanged] {
            zoomObservers.append(observeFitChange(name, object: pdfView))
        }
    }
    deinit {
        searchTask?.cancel()
        for observer in zoomObservers + geometryObservers { NotificationCenter.default.removeObserver(observer) }
    }
    public func setDocument(_ document: PDFDocument?, allowsCopying: Bool, snapshot: Data? = nil) {
        cancelSearch()
        for observer in geometryObservers { NotificationCenter.default.removeObserver(observer) }
        geometryObservers.removeAll()
        rotationObservers.removeAll()
        observedPages.removeAll()
        observedDocumentView = nil
        observedClipView = nil
        pdfView.document = document
        documentSnapshot = snapshot
        self.allowsCopying = allowsCopying
        query = ""; searchStatus = "输入关键词搜索"
        lastFit = nil
        scheduleFit(force: true)
        onChange?()
    }
    public func setLayout(_ layout: ReaderLayout, continuous: Bool) {
        let page = pdfView.currentPage
        pdfView.displaysAsBook = layout == .twoPagesWithCover
        pdfView.displayMode = layout == .single
            ? (continuous ? .singlePageContinuous : .singlePage)
            : (continuous ? .twoUpContinuous : .twoUp)
        if let page { pdfView.go(to: page) }
        refreshFit(force: true)
        onChange?()
    }
    /// Annotation-only updates retain the existing PDFKit page tree and viewport.
    /// The new snapshot invalidates later overview thumbnails without forcing a fit.
    func didUpdateAnnotations(snapshot: Data?) {
        documentSnapshot = snapshot
        onChange?()
    }
    public func setZoom(_ zoom: ReaderZoom) throws {
        guard let document = pdfView.document, document.pageCount > 0 else { throw ReaderError.noDocument }
        switch zoom {
        case .percent(let value):
            guard value.isFinite, (10...6400).contains(value) else { throw ReaderError.invalidZoom }
            pdfView.autoScales = false
            pdfView.minScaleFactor = 0.1; pdfView.maxScaleFactor = 64
            currentZoom = zoom
            lastFit = nil
            pdfView.scaleFactor = value / 100
        case .fitPage, .fitWidth:
            let geometry = try fitGeometry(zoom, prepareLayout: true)
            let scale = try fitScale(geometry)
            currentZoom = zoom
            applyFit(scale, geometry: geometry, revealPage: true)
        }
        onChange?()
    }

    private func observeFitChange(_ name: Notification.Name, object: AnyObject) -> NSObjectProtocol {
        NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.scheduleFit() }
        }
    }

    private func scheduleFit(force: Bool = false) {
        guard currentZoom == .fitPage || currentZoom == .fitWidth else { return }
        forceNextFit = forceNextFit || force
        guard !fitScheduled else { return }
        fitScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.fitScheduled = false
            let force = self.forceNextFit
            self.forceNextFit = false
            self.refreshFit(force: force)
        }
    }

    private func refreshFit(force: Bool = false) {
        guard !applyingFit, let zoom = currentZoom, zoom == .fitPage || zoom == .fitWidth,
              let geometry = try? fitGeometry(zoom, prepareLayout: force), force || geometry != lastFit,
              let scale = try? fitScale(geometry) else { return }
        // Continuous scrolling changes the current page, not the user's zoom
        // intent. Only an explicit fit or a resized viewport should reveal the
        // whole page; passive page notifications must retain the scroll anchor.
        let revealPage = force || lastFit?.viewport != geometry.viewport
        applyFit(scale, geometry: geometry, revealPage: revealPage)
    }

    private func fitGeometry(_ zoom: ReaderZoom, prepareLayout: Bool = false) throws -> FitGeometry {
        guard let document = pdfView.document, document.pageCount > 0 else { throw ReaderError.noDocument }
        if prepareLayout {
            pdfView.layoutSubtreeIfNeeded()
            pdfView.layoutDocumentView()
        }
        updateGeometryObservers()
        let index = pdfView.currentPage.map { document.index(for: $0) } ?? 0
        let current = index == NSNotFound ? 0 : min(max(0, index), document.pageCount - 1)
        let indices: [Int]
        if currentLayout == .single || (currentLayout == .twoPagesWithCover && current == 0) {
            indices = [current]
        } else {
            let base = currentLayout == .twoPagesWithCover ? ((current - 1) / 2) * 2 + 1 : (current / 2) * 2
            indices = [base, base + 1].filter { $0 < document.pageCount }
        }
        let pages = indices.compactMap { document.page(at: $0) }
        let identities = pages.map(ObjectIdentifier.init)
        if identities != observedPages {
            observedPages = identities
            rotationObservers = pages.map { page in
                page.observe(\.rotation, options: [.new]) { [weak self] _, _ in
                    Task { @MainActor [weak self] in self?.scheduleFit() }
                }
            }
        }
        var sizes = pages.map { page -> CGSize in
            let bounds = page.bounds(for: pdfView.displayBox)
            return abs(page.rotation) % 180 == 90 ? CGSize(width: bounds.height, height: bounds.width) : bounds.size
        }
        // Keep PDFKit's empty facing slot for a cover or an unpaired final page;
        // do not turn the native book presentation into a single-page layout.
        if currentLayout != .single, sizes.count == 1, let size = sizes.first { sizes.append(size) }
        let measuredViewport: CGSize
        if let clip = observedClipView {
            // PDFKit scales the clip bounds into document coordinates. Convert
            // them back into the PDFView's viewport coordinates before fitting.
            measuredViewport = pdfView.convert(clip.bounds, from: clip).intersection(pdfView.bounds).size
        } else { measuredViewport = pdfView.bounds.size }
        // Converting a scrolled clip rectangle introduces sub-pixel floating
        // point noise. Compare viewport dimensions at physical-pixel precision.
        let backing = pdfView.window?.backingScaleFactor ?? 2
        let viewport = CGSize(width: (measuredViewport.width * backing).rounded() / backing,
                              height: (measuredViewport.height * backing).rounded() / backing)
        let margins = pdfView.pageBreakMargins
        let gutter = sizes.count > 1 ? max(12, margins.left + margins.right) : 0
        return FitGeometry(document: ObjectIdentifier(document), zoom: zoom, viewport: viewport,
                           indices: indices, sizes: sizes, gutter: gutter)
    }

    private func fitScale(_ geometry: FitGeometry) throws -> CGFloat {
        // A fixed 16-point viewport border on every side is rotation-independent.
        let width = try ZoomInput.fitWidth(viewWidth: geometry.viewport.width - 32,
                                           pageSizes: geometry.sizes, gutter: geometry.gutter)
        if geometry.zoom == .fitWidth { return width }
        let height = geometry.viewport.height - 32
        guard height.isFinite, height > 0, let pageHeight = geometry.sizes.map(\.height).max() else {
            throw ReaderError.invalidZoom
        }
        return min(width, height / pageHeight)
    }

    private func applyFit(_ scale: CGFloat, geometry: FitGeometry, revealPage: Bool) {
        guard scale.isFinite, scale > 0 else { return }
        applyingFit = true
        defer { applyingFit = false }
        lastFit = geometry
        pdfView.autoScales = false
        pdfView.minScaleFactor = min(0.1, scale)
        pdfView.maxScaleFactor = 64
        let next = min(64, scale)
        if abs(pdfView.scaleFactor - next) > 1e-9 { pdfView.scaleFactor = next }
        if revealPage, geometry.zoom == .fitPage, let index = geometry.indices.first,
           let page = pdfView.document?.page(at: index) {
            // Scaling preserves the previous scroll point, which may leave a
            // fitted page partially outside the viewport. Reveal its full box
            // with PDFKit's public navigation primitive (without changing layout).
            pdfView.layoutDocumentView()
            pdfView.go(to: page.bounds(for: pdfView.displayBox), on: page)
        }
    }

    private func updateGeometryObservers() {
        let documentView = pdfView.documentView
        let clipView = documentView?.enclosingScrollView?.contentView
        guard documentView !== observedDocumentView || clipView !== observedClipView else { return }
        for observer in geometryObservers { NotificationCenter.default.removeObserver(observer) }
        geometryObservers.removeAll()
        observedDocumentView = documentView
        observedClipView = clipView
        for view in [documentView, clipView].compactMap({ $0 }) {
            view.postsFrameChangedNotifications = true
            view.postsBoundsChangedNotifications = true
            geometryObservers.append(observeFitChange(NSView.frameDidChangeNotification, object: view))
            geometryObservers.append(observeFitChange(NSView.boundsDidChangeNotification, object: view))
        }
    }
    /// Incremental page-local text search; intentionally does not use or replace the document delegate.
    /// It cannot match a phrase split over pages. PDFKit page text extraction still runs on the main actor.
    public func search(_ text: String) throws {
        cancelSearch()
        query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { searchStatus = "输入关键词搜索"; onChange?(); return }
        guard let document = pdfView.document, !document.isLocked else { throw ReaderError.noDocument }
        guard allowsCopying else { searchStatus = "文档原始权限不允许搜索或选择导出文字"; onChange?(); throw ReaderError.copyingDenied }
        let token = generation, needle = query
        isSearching = true; searchStatus = "正在搜索…"; onChange?()
        searchTask = Task { @MainActor [weak self, weak document] in
            do { try await Task.sleep(nanoseconds: 180_000_000) } catch { return }
            guard let document else { return }
            for pageIndex in 0..<document.pageCount {
                await Task.yield()
                guard !Task.isCancelled, let self, self.generation == token, self.pdfView.document === document else { return }
                guard let page = document.page(at: pageIndex), let string = page.string else { continue }
                let text = string as NSString
                var offset = 0
                while offset < text.length {
                    let range = text.range(of: needle, options: [.caseInsensitive], range: NSRange(location: offset, length: text.length - offset))
                    guard range.location != NSNotFound, range.length > 0 else { break }
                    if let selection = page.selection(for: range) {
                        let start = max(0, range.location - 35), end = min(text.length, NSMaxRange(range) + 50)
                        let excerpt = text.substring(with: NSRange(location: start, length: end - start)).replacingOccurrences(of: "\n", with: " ")
                        self.hits.append(ReaderHit(pageIndex: pageIndex, pageLabel: page.label ?? "\(pageIndex + 1)", excerpt: excerpt, selection: selection))
                        if self.hits.count >= 1000 {
                            self.reachedLimit = true
                            self.isSearching = false
                            self.selectHit(0)
                            return
                        }
                    }
                    offset = NSMaxRange(range)
                    if self.hits.count % 64 == 0 {
                        await Task.yield()
                        guard !Task.isCancelled, self.generation == token, self.pdfView.document === document else { return }
                    }
                }
            }
            guard !Task.isCancelled, let self, self.generation == token else { return }
            self.isSearching = false
            if self.hits.isEmpty { self.searchStatus = "未找到“\(needle)”"; self.onChange?() }
            else { self.selectHit(0) }
        }
    }
    public func cancelSearch() {
        generation &+= 1; searchTask?.cancel(); searchTask = nil
        hits.removeAll(); selectedHit = nil; isSearching = false; reachedLimit = false
        pdfView.setCurrentSelection(nil, animate: false)
        searchStatus = "搜索已取消"
        onChange?()
    }
    public func selectHit(_ index: Int) {
        guard allowsCopying, hits.indices.contains(index), let document = pdfView.document,
              let page = document.page(at: hits[index].pageIndex), hits[index].selection.pages.contains(where: { $0 === page }) else { return }
        selectedHit = index
        pdfView.go(to: page)
        pdfView.setCurrentSelection(hits[index].selection, animate: false)
        pdfView.scrollSelectionToVisible(nil)
        searchStatus = "\(index + 1) / \(hits.count)" + (reachedLimit ? " · 已达1000项上限，请细化关键词" : "")
        onChange?()
    }
    public func nextHit(backward: Bool = false) {
        guard !hits.isEmpty else { return }
        selectHit(((selectedHit ?? (backward ? 0 : -1)) + (backward ? -1 : 1) + hits.count) % hits.count)
    }
    public func goToPage(_ oneBased: Int) throws {
        guard let document = pdfView.document else { throw ReaderError.noDocument }
        guard oneBased > 0, oneBased <= document.pageCount, let page = document.page(at: oneBased - 1) else { throw ReaderError.invalidPage }
        pdfView.go(to: page); refreshFit(); onChange?()
    }
}

public enum ZoomInput {
    public static func parse(_ text: String) throws -> ReaderZoom {
        var value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.hasSuffix("%") { value.removeLast(); value = value.trimmingCharacters(in: .whitespacesAndNewlines) }
        guard let number = Double(value), number.isFinite, (10...6400).contains(number) else { throw ReaderError.invalidZoom }
        return .percent(number)
    }
    public static func fitWidth(viewWidth: CGFloat, pageSizes: [CGSize], gutter: CGFloat) throws -> CGFloat {
        guard viewWidth.isFinite, gutter.isFinite, gutter >= 0, viewWidth > gutter, !pageSizes.isEmpty,
              pageSizes.allSatisfy({ $0.width.isFinite && $0.height.isFinite && $0.width > 0 && $0.height > 0 }) else { throw ReaderError.invalidZoom }
        return (viewWidth - gutter) / pageSizes.reduce(0) { $0 + $1.width }
    }
}

