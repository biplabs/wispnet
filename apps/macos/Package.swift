// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "WispnetMac",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "Wispnet", targets: ["WispnetMac"])],
    targets: [
        .target(name: "WispnetCore"),
        .executableTarget(name: "WispnetMac", dependencies: ["WispnetCore"]),
        .testTarget(name: "WispnetCoreTests", dependencies: ["WispnetCore"]),
    ],
    swiftLanguageModes: [.v5]
)
