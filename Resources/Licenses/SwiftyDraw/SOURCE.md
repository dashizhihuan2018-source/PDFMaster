# SwiftyDraw attribution

- Upstream: https://github.com/Awalz/SwiftyDraw
- Pinned commit: `dab0f329518762520e39bbd9a05193cbc422f5b5`
- License: MIT; full upstream text in `LICENSE` alongside this file.
- Reused source: `Source/SwiftyDraw.swift`, lines 347–361 (midpoint calculation and quadratic subpath).
- Immutable source URL: https://github.com/Awalz/SwiftyDraw/blob/dab0f329518762520e39bbd9a05193cbc422f5b5/Source/SwiftyDraw.swift#L347-L361
- Copyright: Copyright (c) 2018 Linus Geffarth <hello@ogli.codes>
- Adaptation: native PDF Master path construction joins midpoint quadratic segments, preserves sampled endpoints, and converts quadratic segments to equivalent cubic AppKit paths where needed. UIKit view, touches, Pencil event handling, brushes, assets, history and archiving are not copied.
- This is not Procreate source code, a Procreate brush preset or a claim of Procreate equivalence.
