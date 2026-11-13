import SwiftUI

/// The colour a room participant's avatar is drawn in, picked from the Gallop palette by a stable hash of their
/// name, so a participant keeps one colour in every room and across launches.
enum ParticipantColor {
    struct Swatch: Equatable {
        let lightFill: String
        let lightText: String
        let darkFill: String
        let darkText: String

        var fill: Color { HexColor.adaptive(light: lightFill, dark: darkFill) }
        var text: Color { HexColor.adaptive(light: lightText, dark: darkText) }
    }

    /// Each palette family at 100 under 700 in light mode, and 900 under 300 in dark.
    static let swatches: [Swatch] = [
        Swatch(lightFill: "#F4D8D0", lightText: "#93260B", darkFill: "#470E00", darkText: "#EA8864"), // canyon
        Swatch(lightFill: "#DBF0E1", lightText: "#1A5B36", darkFill: "#06321A", darkText: "#7EC89C"), // cactus
        Swatch(lightFill: "#DEEBF2", lightText: "#12496E", darkFill: "#012641", darkText: "#7EAFC8"), // denim
        Swatch(lightFill: "#FBE4CE", lightText: "#A04304", darkFill: "#4D1E00", darkText: "#F7A55F"), // lantern
        Swatch(lightFill: "#FFEECC", lightText: "#A85700", darkFill: "#512900", darkText: "#FFC15C"), // nugget
        Swatch(lightFill: "#EBE5DF", lightText: "#3D3530", darkFill: "#2E2824", darkText: "#CDBFB1"), // hay and bison
    ]

    static func swatch(for name: String) -> Swatch { swatches[index(for: name)] }

    /// FNV-1a over the name's UTF-8, which, unlike a sum of its characters, separates "claude-1" from "claude-2".
    static func index(for name: String) -> Int {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in name.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01B3
        }
        return Int(hash % UInt64(swatches.count))
    }
}
