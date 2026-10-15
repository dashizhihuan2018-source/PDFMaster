import AppKit
import PaperCore
import XCTest
@testable import PaperDesk

@MainActor
final class ShapeStyleBarTests: XCTestCase {
    func testProgrammaticConfigurationIsSilentAndReflectsActualStyle() {
        let bar = ShapeStyleBarView()
        var callbacks = 0
        bar.onToolChange = { _ in callbacks += 1 }
        bar.onStyleChange = { _ in callbacks += 1 }
        bar.configure(tool: .ellipse,
            style: AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 21.5), mode: .selection)
        XCTAssertEqual(callbacks, 0)
        XCTAssertEqual(bar.tool, .ellipse)
        XCTAssertEqual(bar.style.lineWidth, 21.5)
        XCTAssertEqual(bar.widthField.doubleValue, 21.5)
        XCTAssertEqual(bar.widthSlider.doubleValue, 12)
        XCTAssertEqual(bar.strokeColorWell.color, .red)
        XCTAssertEqual(bar.fillColorWell.color, .yellow)
        XCTAssertEqual(bar.fillEnabledButton.state, .on)
        XCTAssertFalse(bar.toolChooser.isEnabled)
        XCTAssertTrue(bar.widthField.isEnabled)
        let labels = bar.subviews.flatMap(\.subviews).compactMap { ($0 as? NSTextField)?.stringValue }
        XCTAssertTrue(labels.contains("所选形状"),"Default selection label must remain unchanged for shapes")
    }

    func testToolChooserEmitsOneCallbackAndDisablesFillForLineFamily() {
        let bar = ShapeStyleBarView()
        bar.configure(tool: .rectangle,
            style: AnnotationStyle(strokeColor: .blue, fillColor: .yellow, lineWidth: 2))
        var tools: [ShapeToolKind] = []
        var styles = 0
        bar.onToolChange = { tools.append($0) }
        bar.onStyleChange = { _ in styles += 1 }
        for tool in [ShapeToolKind.line, .arrow, .doubleArrow] {
            bar.toolChooser.selectedSegment = ShapeToolKind.allCases.firstIndex(of: tool)!
            fire(bar.toolChooser)
            XCTAssertEqual(bar.tool, tool)
            XCTAssertNil(bar.style.fillColor)
            XCTAssertFalse(bar.fillEnabledButton.isEnabled)
            XCTAssertFalse(bar.fillColorWell.isEnabled)
        }
        XCTAssertEqual(tools, [.line, .arrow, .doubleArrow])
        XCTAssertEqual(styles, 0)
        bar.toolChooser.selectedSegment = 1
        fire(bar.toolChooser)
        XCTAssertEqual(bar.tool, .ellipse)
        XCTAssertEqual(bar.style.fillColor, .yellow)
        XCTAssertTrue(bar.fillColorWell.isEnabled)
    }

    func testSliderAndPresetImmediatelyEmitRealWidths() {
        let bar = ShapeStyleBarView()
        var widths: [CGFloat] = []
        bar.onStyleChange = { widths.append($0.lineWidth) }
        XCTAssertTrue(bar.widthSlider.isContinuous)
        XCTAssertEqual(bar.widthSlider.minValue, 0.25)
        XCTAssertEqual(bar.widthSlider.maxValue, 12)
        bar.widthSlider.doubleValue = 3.75
        fire(bar.widthSlider)
        XCTAssertEqual(bar.widthField.doubleValue, 3.75)
        XCTAssertEqual(bar.style.lineWidth, 3.75)
        bar.widthPresets.selectItem(at: 5)
        fire(bar.widthPresets)
        XCTAssertEqual(bar.style.lineWidth, 8)
        XCTAssertEqual(widths, [3.75, 8])
    }

    func testSliderTrackingPreviewsEveryStepAndCommitsOnceAtRelease() {
        let bar = ShapeStyleBarView()
        var previews:[CGFloat]=[], commits:[CGFloat]=[]
        bar.onStylePreview = { previews.append($0.lineWidth) }
        bar.onStyleChange = { commits.append($0.lineWidth) }
        bar.widthSlider.beginShapeTracking()
        for w in [3.0,5.0,8.0] { bar.widthSlider.doubleValue=w; fire(bar.widthSlider) }
        XCTAssertEqual(previews,[3,5,8]); XCTAssertTrue(commits.isEmpty)
        bar.widthSlider.endShapeTracking()
        XCTAssertEqual(commits,[8])
    }

    func testRealMouseEventsKeepTrackingNonblockingAndCommitOnlyAtRelease() throws {
        let bar = try mouseEventBar(width: 8)
        let slider = bar.widthSlider
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let knob = cell.knobRect(flipped: slider.isFlipped)
        let start = CGPoint(x: knob.midX + 2, y: knob.midY)
        var previews: [CGFloat] = [], commits: [CGFloat] = []
        bar.onStylePreview = { previews.append($0.lineWidth) }
        bar.onStyleChange = { commits.append($0.lineWidth) }
        try beginMouseGesture(slider, at: start)
        XCTAssertTrue(slider.isShapeTracking, "mouseDown must return before release")
        XCTAssertEqual(bar.style.lineWidth, 8, "Grabbing off-center must not jump")
        XCTAssertTrue(previews.isEmpty)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged, at: start, in: slider))
        XCTAssertEqual(bar.style.lineWidth, 8, accuracy: 1e-10)
        for x in [start.x - 12, start.x - 24, -50] {
            slider.mouseDragged(with: mouseEvent(.leftMouseDragged,
                                                 at: CGPoint(x: x, y: start.y), in: slider))
            XCTAssertTrue(commits.isEmpty, "Drag changes must be previews only")
        }
        XCTAssertEqual(previews.count, 3)
        if previews.count >= 2 { XCTAssertGreaterThan(previews[0], previews[1]) }
        XCTAssertEqual(previews.last, 0.25)
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: CGPoint(x: -50, y: start.y), in: slider))
        XCTAssertFalse(slider.isShapeTracking)
        XCTAssertEqual(commits, [0.25])
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: start, in: slider))
        XCTAssertEqual(commits.count, 1)
    }

    func testRealTrackClickPreviewsAndReleaseCommitsWithoutManualAction() throws {
        let bar = try mouseEventBar(width: 1)
        let slider = bar.widthSlider
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let track = cell.barRect(flipped: slider.isFlipped)
        let point = CGPoint(x: track.maxX - 1, y: track.midY)
        var previews: [CGFloat] = [], commits: [CGFloat] = []
        bar.onStylePreview = { previews.append($0.lineWidth) }
        bar.onStyleChange = { commits.append($0.lineWidth) }
        try beginMouseGesture(slider, at: point)
        XCTAssertTrue(slider.isShapeTracking)
        XCTAssertGreaterThan(bar.style.lineWidth, 10)
        XCTAssertEqual(previews.count, 1)
        XCTAssertTrue(commits.isEmpty)
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: point, in: slider))
        XCTAssertEqual(commits.count, 1)
        XCTAssertEqual(commits.first, previews.last)
    }

    func testDisabledRealMouseEventsDoNotBeginPreviewOrCommit() throws {
        let bar = try mouseEventBar(width: 8)
        bar.configure(tool: .rectangle, style: bar.style, enabled: false)
        var callbacks = 0
        bar.onStylePreview = { _ in callbacks += 1 }
        bar.onStyleChange = { _ in callbacks += 1 }
        let slider = bar.widthSlider
        let point = CGPoint(x: 10, y: slider.bounds.midY)
        try beginMouseGesture(slider, at: point)
        XCTAssertFalse(slider.isShapeTracking)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged, at: .zero, in: slider))
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: .zero, in: slider))
        XCTAssertEqual(bar.style.lineWidth, 8)
        XCTAssertEqual(callbacks, 0)
    }

    func testReconfigurationDuringRealDragCannotCommitToNewSelection() throws {
        let bar = try mouseEventBar(width: 8)
        let slider = bar.widthSlider
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let knob = cell.knobRect(flipped: slider.isFlipped)
        let start = CGPoint(x: knob.midX, y: knob.midY)
        var commits: [CGFloat] = []
        bar.onStyleChange = { commits.append($0.lineWidth) }
        try beginMouseGesture(slider, at: start)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged,
            at: CGPoint(x: start.x - 15, y: start.y), in: slider))
        bar.configure(tool: .ellipse,
            style: AnnotationStyle(strokeColor: .red, fillColor: nil, lineWidth: 3), mode: .selection)
        XCTAssertFalse(slider.isShapeTracking)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged, at: .zero, in: slider))
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: .zero, in: slider))
        XCTAssertEqual(bar.style.lineWidth, 3)
        XCTAssertTrue(commits.isEmpty)
    }

    func testNativeAccessibilityIncrementStillCommitsWithoutMouseTracking() throws {
        let bar = try mouseEventBar(width: 5)
        let slider = bar.widthSlider
        var previews: [CGFloat] = [], commits: [CGFloat] = []
        bar.onStylePreview = { previews.append($0.lineWidth) }
        bar.onStyleChange = { commits.append($0.lineWidth) }
        _ = slider.accessibilityPerformIncrement()
        XCTAssertGreaterThan(bar.style.lineWidth, 5)
        XCTAssertFalse(slider.isShapeTracking)
        XCTAssertEqual(commits.count, 1)
        XCTAssertTrue(previews.isEmpty)
        XCTAssertEqual(slider.accessibilityLabel(), "线宽滑块")
    }

    func testDisableDuringRealDragDropsTheGestureWithoutCommit() throws {
        let bar = try mouseEventBar(width: 8)
        let slider = bar.widthSlider
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let knob = cell.knobRect(flipped: slider.isFlipped)
        let start = CGPoint(x: knob.midX, y: knob.midY)
        var commits = 0
        bar.onStyleChange = { _ in commits += 1 }
        try beginMouseGesture(slider, at: start)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged,
            at: CGPoint(x: start.x - 15, y: start.y), in: slider))
        slider.isEnabled = false
        XCTAssertFalse(slider.isShapeTracking)
        let lastPreview = bar.style.lineWidth
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged, at: .zero, in: slider))
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: .zero, in: slider))
        XCTAssertEqual(bar.style.lineWidth, lastPreview)
        XCTAssertEqual(commits, 0)
    }

    func testNativeBarAndKnobGeometryMapClicksToBothExactLimits() throws {
        let bar = try mouseEventBar(width: 5)
        let slider = bar.widthSlider
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        let track = cell.barRect(flipped: slider.isFlipped)
        let knob = cell.knobRect(flipped: slider.isFlipped)
        var commits: [CGFloat] = []
        bar.onStyleChange = { commits.append($0.lineWidth) }
        for (point, expected) in [
            (CGPoint(x: track.minX + knob.width / 2, y: track.midY), CGFloat(slider.minValue)),
            (CGPoint(x: track.maxX - knob.width / 2, y: track.midY), CGFloat(slider.maxValue))
        ] {
            try beginMouseGesture(slider, at: point)
            XCTAssertEqual(bar.style.lineWidth, expected, accuracy: 1e-10)
            slider.mouseUp(with: mouseEvent(.leftMouseUp, at: point, in: slider))
        }
        XCTAssertEqual(commits, [0.25, 12])
    }

    func testResigningFocusCancelsRealGestureWithoutLateCommit() throws {
        let bar = try mouseEventBar(width: 8)
        let slider = bar.widthSlider
        let knob = try XCTUnwrap(slider.cell as? NSSliderCell).knobRect(flipped: slider.isFlipped)
        let start = CGPoint(x: knob.midX, y: knob.midY)
        var commits = 0
        bar.onStyleChange = { _ in commits += 1 }
        try beginMouseGesture(slider, at: start)
        slider.mouseDragged(with: mouseEvent(.leftMouseDragged,
            at: CGPoint(x: start.x - 15, y: start.y), in: slider))
        XCTAssertTrue(slider.resignFirstResponder())
        XCTAssertFalse(slider.isShapeTracking)
        slider.mouseUp(with: mouseEvent(.leftMouseUp, at: .zero, in: slider))
        XCTAssertEqual(commits, 0)
    }

    func testWidthFieldAcceptsFullRangeAndRejectsInvalidValuesWithoutChangingStyle() {
        let bar = ShapeStyleBarView()
        var widths: [CGFloat] = []
        bar.onStyleChange = { widths.append($0.lineWidth) }
        for valid in ["0.25", "17.5", "32"] {
            bar.widthField.stringValue = valid
            fire(bar.widthField)
            XCTAssertEqual(bar.style.lineWidth, CGFloat(Double(valid)!))
        }
        for invalid in ["", "abc", "nan", "inf", "-2", "0.249", "32.01", "2 pt", "1e999"] {
            bar.widthField.stringValue = invalid
            fire(bar.widthField)
            XCTAssertEqual(bar.style.lineWidth, 32, invalid)
            XCTAssertEqual(bar.widthField.doubleValue, 32, invalid)
        }
        XCTAssertEqual(widths, [0.25, 17.5, 32])
        // AppKit may send both the field action and end-edit notification.
        bar.controlTextDidEndEditing(Notification(name: NSControl.textDidEndEditingNotification, object: bar.widthField))
        XCTAssertEqual(widths.count, 3)
    }

    func testColorAndFillChangesEmitActualStyleWithoutModalConfirmation() {
        let bar = ShapeStyleBarView()
        var styles: [AnnotationStyle] = []
        bar.onStyleChange = { styles.append($0) }
        bar.strokeColorWell.color = .purple
        fire(bar.strokeColorWell)
        bar.fillEnabledButton.state = .on
        fire(bar.fillEnabledButton)
        bar.fillColorWell.color = .green
        fire(bar.fillColorWell)
        XCTAssertEqual(styles.count, 3)
        XCTAssertEqual(styles[0].strokeColor, .purple)
        XCTAssertNil(styles[0].fillColor)
        XCTAssertNotNil(styles[1].fillColor)
        XCTAssertEqual(styles[2].fillColor, .green)
        bar.fillEnabledButton.state = .off
        fire(bar.fillEnabledButton)
        XCTAssertNil(bar.style.fillColor)
        XCTAssertFalse(bar.fillColorWell.isEnabled)
        XCTAssertEqual(styles.count, 4)
    }

    func testDisabledAndSelectionModesCannotAccidentallySwitchTools() {
        let bar = ShapeStyleBarView()
        var callbacks = 0
        bar.onToolChange = { _ in callbacks += 1 }
        bar.onStyleChange = { _ in callbacks += 1 }
        bar.configure(tool: .rectangle, style: .default, mode: .selection)
        bar.toolChooser.selectedSegment = 3
        fire(bar.toolChooser)
        XCTAssertEqual(bar.tool, .rectangle)
        XCTAssertEqual(bar.toolChooser.selectedSegment, 0)
        bar.configure(tool: .rectangle, style: .default, enabled: false)
        bar.widthField.stringValue = "4"
        fire(bar.widthField)
        XCTAssertEqual(bar.style.lineWidth, 1)
        XCTAssertEqual(callbacks, 0)
        XCTAssertFalse(bar.strokeColorWell.isEnabled)
    }

    func testNativeControlsHaveChineseAccessibilityLabelsAndPreview() {
        let bar = ShapeStyleBarView()
        XCTAssertEqual(bar.toolChooser.accessibilityLabel(), "形状工具")
        XCTAssertEqual(bar.strokeColorWell.accessibilityLabel(), "描边颜色")
        XCTAssertEqual(bar.fillEnabledButton.accessibilityLabel(), "启用填充")
        XCTAssertEqual(bar.fillColorWell.accessibilityLabel(), "填充颜色")
        XCTAssertEqual(bar.widthSlider.accessibilityLabel(), "线宽滑块")
        XCTAssertEqual(bar.widthField.accessibilityLabel(), "线宽，单位点")
        XCTAssertEqual(bar.widthPresets.accessibilityLabel(), "预设线宽")
        XCTAssertTrue(bar.subviews.contains { containsPreview($0) })
    }

    func testPreciseWidthSurvivesConfigurationAndEndEditing() {
        let bar = ShapeStyleBarView()
        let width: CGFloat = 3.123456789
        bar.configure(tool: .rectangle,
            style: AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: width))
        var callbacks = 0
        bar.onStyleChange = { _ in callbacks += 1 }
        XCTAssertEqual(bar.widthField.doubleValue, Double(width), accuracy: 1e-10)
        fire(bar.widthField)
        XCTAssertEqual(bar.style.lineWidth, width, accuracy: 1e-10)
        XCTAssertEqual(callbacks, 0)
    }

    func testInlineLayoutFitsAllControlsAndPreviewAtAdvertisedSize() {
        let bar = ShapeStyleBarView()
        bar.frame = NSRect(origin: .zero, size: bar.intrinsicContentSize)
        bar.layoutSubtreeIfNeeded()
        for control in [bar.toolChooser, bar.strokeColorWell, bar.fillColorWell,
                        bar.widthField, bar.widthPresets] as [NSView] {
            let frame = bar.convert(control.bounds, from: control)
            XCTAssertGreaterThan(frame.width, 0)
            XCTAssertGreaterThanOrEqual(frame.minX, 0)
            XCTAssertLessThanOrEqual(frame.maxX, bar.bounds.maxX)
            XCTAssertGreaterThanOrEqual(frame.minY, 0)
            XCTAssertLessThanOrEqual(frame.maxY, bar.bounds.maxY)
        }
        let preview = findPreview(in: bar)
        XCTAssertNotNil(preview)
        if let preview {
            let frame = bar.convert(preview.bounds, from: preview)
            XCTAssertEqual(frame.width, 48)
            XCTAssertLessThanOrEqual(frame.maxX, bar.bounds.maxX)
        }
    }

    func testNativeExampleRedrawsWhenToolOrActualStyleChanges() throws {
        let bar = ShapeStyleBarView()
        bar.frame = NSRect(origin: .zero, size: bar.intrinsicContentSize)
        bar.layoutSubtreeIfNeeded()
        let preview = try XCTUnwrap(findPreview(in: bar))
        func renderedPixels() throws -> Data {
            let bitmap = try XCTUnwrap(preview.bitmapImageRepForCachingDisplay(in: preview.bounds))
            preview.cacheDisplay(in: preview.bounds, to: bitmap)
            return try XCTUnwrap(bitmap.tiffRepresentation)
        }
        let rectangle = try renderedPixels()
        bar.configure(tool: .ellipse, style: .default)
        let ellipse = try renderedPixels()
        XCTAssertNotEqual(rectangle, ellipse)
        bar.configure(tool: .ellipse,
            style: AnnotationStyle(strokeColor: .red, fillColor: .yellow, lineWidth: 4))
        let coloredEllipse = try renderedPixels()
        XCTAssertNotEqual(ellipse, coloredEllipse)
        bar.configure(tool: .doubleArrow,
            style: AnnotationStyle(strokeColor: .red, fillColor: nil, lineWidth: 4))
        XCTAssertNotEqual(coloredEllipse, try renderedPixels())
    }

    private func findPreview(in view: NSView) -> NSView? {
        if view.accessibilityLabel() == "形状样式预览" { return view }
        return view.subviews.lazy.compactMap { self.findPreview(in: $0) }.first
    }

    private func containsPreview(_ view: NSView) -> Bool {
        view.accessibilityLabel() == "形状样式预览" || view.subviews.contains { containsPreview($0) }
    }

    private func fire(_ control: NSControl) {
        XCTAssertNotNil(control.action)
        XCTAssertTrue(control.sendAction(control.action, to: control.target))
    }

    private func mouseEventBar(width: CGFloat) throws -> ShapeStyleBarView {
        let bar = ShapeStyleBarView()
        bar.frame = NSRect(origin: .zero, size: bar.intrinsicContentSize)
        bar.configure(tool: .rectangle,
            style: AnnotationStyle(strokeColor: .black, fillColor: nil, lineWidth: width))
        bar.layoutSubtreeIfNeeded()
        let slider = bar.widthSlider
        slider.cell?.calcDrawInfo(slider.bounds)
        let bitmap = try XCTUnwrap(slider.bitmapImageRepForCachingDisplay(in: slider.bounds))
        slider.cacheDisplay(in: slider.bounds, to: bitmap)
        let cell = try XCTUnwrap(slider.cell as? NSSliderCell)
        XCTAssertGreaterThan(cell.barRect(flipped: slider.isFlipped).width, 0)
        return bar
    }

    private func mouseEvent(_ kind: NSEvent.EventType, at point: CGPoint, in view: NSView) -> NSEvent {
        NSEvent.mouseEvent(with: kind, location: view.convert(point, to: nil), modifierFlags: [],
            timestamp: 0, windowNumber: view.window?.windowNumber ?? 0, context: nil,
            eventNumber: 0, clickCount: 1, pressure: kind == .leftMouseUp ? 0 : 1)!
    }

    private func beginMouseGesture(_ slider: ShapeWidthSlider, at point: CGPoint) throws {
        // Bound the old NSSlider blocking loop during RED. A nonblocking
        // implementation leaves this fail-safe release queued and we remove it.
        NSApplication.shared.postEvent(mouseEvent(.leftMouseUp, at: point, in: slider), atStart: true)
        slider.mouseDown(with: mouseEvent(.leftMouseDown, at: point, in: slider))
        _ = NSApplication.shared.nextEvent(matching: .leftMouseUp, until: .distantPast,
                                          inMode: .default, dequeue: true)
    }
}
