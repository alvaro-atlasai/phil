// swift-tools-version: 6.3
import PackageDescription

let package = Package(
    name: "phil-apple",
    platforms: [.macOS(.v26)],
    targets: [
        .executableTarget(
            name: "phil-apple",
            path: "Sources"
        )
    ]
)
