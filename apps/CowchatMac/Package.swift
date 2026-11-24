// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "CowchatMac",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "CowchatMac", targets: ["CowchatMac"]),
    ],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.10.0"),
        .package(path: "../CowchatKit"),
    ],
    targets: [
        .executableTarget(
            name: "CowchatMac",
            dependencies: [
                .product(name: "Sparkle", package: "Sparkle"),
                .product(name: "CowchatKit", package: "CowchatKit"),
            ],
            resources: [
                .process("Resources"),
                .copy("Bundled/Charts"),
            ],
            linkerSettings: [
                .unsafeFlags(["-Xlinker", "-rpath", "-Xlinker", "@loader_path/../Frameworks"]),
            ]
        ),
        .testTarget(name: "CowchatMacTests", dependencies: ["CowchatMac", .product(name: "CowchatKit", package: "CowchatKit")]),
    ],
    swiftLanguageVersions: [.v5]
)
