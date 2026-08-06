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
            resources: [
                .process("Resources"),
                // Directory-preserving copies — GallopIcon/SeasonFonts look up
                // "Icons/svg/<name>.svg" and "Fonts/<name>.ttf" by subpath.
                // NOTE: SwiftPM forbids overlapping resource declarations; if
                // .process("Resources") conflicts with nested .copy dirs, move the
                // font/icon dirs OUT of Resources/ to Sources/CowchatMac/Bundled/
                // and declare .copy("Bundled/Fonts"), .copy("Bundled/Icons") instead
                // (lookup subpaths stay "Fonts/…" and "Icons/…").
                .copy("Bundled/Fonts"),
                .copy("Bundled/Icons"),
            ]
        ),
        .testTarget(name: "CowchatMacTests", dependencies: ["CowchatMac"]),
    ],
    swiftLanguageVersions: [.v5]
)
