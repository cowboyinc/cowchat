// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "CowchatMac",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "CowchatMac", targets: ["CowchatMac"]),
    ],
    targets: [
        .executableTarget(
            name: "CowchatMac",
            resources: [.process("Resources")]
        ),
        .testTarget(name: "CowchatMacTests", dependencies: ["CowchatMac"]),
    ],
    swiftLanguageVersions: [.v5]
)
