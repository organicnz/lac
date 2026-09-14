// swift-tools-version: 6.0
// The swift-tools-version determines the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "LoopLACStudio",
    platforms: [.macOS(.v13)],
    dependencies: [
        // Dependencies can be added here; we use standard library only for minimal footprint
    ],
    targets: [
        // Targets consist of a name and a set of source files. Targets can depend on
        // other targets in this package, and dependencies.
        .executableTarget(
            name: "LoopLACStudio",
            dependencies: []),
        .testTarget(
            name: "LoopLACStudioTests",
            dependencies: ["LoopLACStudio"]),
    ]
)