// swift-tools-version: 6.0
import PackageDescription
let package = Package(
    name: "PaperDesk",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "PaperCore", targets: ["PaperCore"]),
        .executable(name: "PaperDesk", targets: ["PaperDesk"])
    ],
    targets: [
        .target(name: "PaperCore"),
        .executableTarget(name: "PaperDesk", dependencies: ["PaperCore"]),
        .testTarget(name: "PaperCoreTests", dependencies: ["PaperCore"]),
        .testTarget(name: "PaperDeskTests", dependencies: ["PaperDesk"])
    ],
    swiftLanguageModes: [.v5]
)
