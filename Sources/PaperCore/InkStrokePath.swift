import AppKit

/// Midpoint quadratic smoothing adapted from SwiftyDraw (MIT).
/// Copyright (c) 2018 Linus Geffarth <hello@ogli.codes>.
/// Upstream: Awalz/SwiftyDraw dab0f329518762520e39bbd9a05193cbc422f5b5,
/// Source/SwiftyDraw.swift, drawLine(with:). Full license: Resources/Licenses/SwiftyDraw/LICENSE.
/// Unlike the upstream incremental renderer, this completes both endpoints and
/// creates a single path shared by live preview and editable PDF Ink serialization.
/// PDF /InkList is a polyline: PDFKit discards NSBezier curve controls when
/// serializing it. Adaptive quadratic subdivision therefore happens BEFORE
/// preview, with <= 0.15 page-point flatness, so saved and displayed paths match.
public enum InkStrokePath {
    public static let flatness: CGFloat = 0.15
    public static func make(points: [CGPoint]) -> CGPath {
        let path = CGMutablePath()
        guard let first = points.first, points.allSatisfy({ $0.x.isFinite && $0.y.isFinite }) else { return path }
        path.move(to: first)
        guard points.count > 1 else { return path }
        func midpoint(_ a: CGPoint, _ b: CGPoint) -> CGPoint { CGPoint(x: (a.x+b.x)/2, y: (a.y+b.y)/2) }
        var current = first
        func flatten(_ start: CGPoint, _ control: CGPoint, _ end: CGPoint, depth: Int = 0) {
            let chordMidpoint = midpoint(start,end)
            if hypot(control.x-chordMidpoint.x,control.y-chordMidpoint.y) <= flatness || depth >= 16 {
                path.addLine(to:end); return
            }
            let a = midpoint(start,control), b = midpoint(control,end), m = midpoint(a,b)
            flatten(start,a,m,depth:depth+1); flatten(m,b,end,depth:depth+1)
        }
        func quadratic(to end: CGPoint, control: CGPoint) {
            flatten(current,control,end); current = end
        }
        quadratic(to: midpoint(first, points[1]), control: first)
        if points.count > 2 {
            for i in 1..<(points.count-1) {
                quadratic(to: midpoint(points[i], points[i+1]), control: points[i])
            }
        }
        quadratic(to: points[points.count-1], control: points[points.count-1])
        return path
    }

    /// Exact conversion of the shared path into PDFKit's annotation-local path.
    public static func bezier(_ path: CGPath) -> NSBezierPath {
        let result = NSBezierPath()
        var current = CGPoint.zero
        path.applyWithBlock { element in
            let e = element.pointee
            switch e.type {
            case .moveToPoint: result.move(to: e.points[0]); current=e.points[0]
            case .addLineToPoint: result.line(to: e.points[0]); current=e.points[0]
            case .addQuadCurveToPoint:
                let control=e.points[0], end=e.points[1]
                let a=CGPoint(x:current.x+(control.x-current.x)*2/3, y:current.y+(control.y-current.y)*2/3)
                let b=CGPoint(x:end.x+(control.x-end.x)*2/3, y:end.y+(control.y-end.y)*2/3)
                result.curve(to:end, controlPoint1:a, controlPoint2:b); current=end
            case .addCurveToPoint: result.curve(to:e.points[2], controlPoint1:e.points[0], controlPoint2:e.points[1]); current=e.points[2]
            case .closeSubpath: result.close()
            @unknown default: break
            }
        }
        result.lineCapStyle = .round; result.lineJoinStyle = .round
        return result
    }
}
