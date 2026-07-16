// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Genaryx",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        .executableTarget(
            name: "Genaryx"
        )
    ]
)
