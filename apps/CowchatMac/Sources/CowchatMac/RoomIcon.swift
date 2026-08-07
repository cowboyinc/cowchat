import Foundation
import SwiftUI

/// Deterministic parameter source for procedural room icons.
///
/// Swift's `Hasher` is seeded per process, so it would re-roll every icon on
/// relaunch — this is a fixed FNV-1a plus a SplitMix64 finalizer instead. Each
/// parameter draws from its own domain-separated hash (`"hide.palette|<room>"`)
/// rather than successive values off one stream, so adding a parameter later
/// cannot reshuffle the icons that already exist.
struct RoomIconSeed {
    private let name: String

    init(_ name: String) {
        self.name = name
    }

    func bits(_ field: String) -> UInt64 {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        mix(&hash, field.utf8)
        mix(&hash, "|".utf8)
        mix(&hash, name.utf8)
        return avalanche(hash)
    }

    func index(_ field: String, _ count: Int) -> Int {
        Int(bits(field) % UInt64(count))
    }

    func pick<T>(_ field: String, _ options: [T]) -> T {
        options[index(field, options.count)]
    }

    /// Uniform in 0..<1, taken from the top 53 bits after the finalizer.
    func unit(_ field: String) -> Double {
        Double(bits(field) >> 11) * (1.0 / 9_007_199_254_740_992.0)
    }

    func value(_ field: String, _ range: ClosedRange<Double>) -> Double {
        range.lowerBound + unit(field) * (range.upperBound - range.lowerBound)
    }

    private func mix<S: Sequence>(_ hash: inout UInt64, _ bytes: S) where S.Element == UInt8 {
        for byte in bytes {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
    }

    /// FNV-1a alone leaves the high bits weakly mixed, which `unit` reads from.
    private func avalanche(_ value: UInt64) -> UInt64 {
        var z = value
        z = (z ^ (z >> 30)) &* 0xbf58_476d_1ce4_e5b9
        z = (z ^ (z >> 27)) &* 0x94d0_49bb_1331_11eb
        return z ^ (z >> 31)
    }
}

enum RoomIconStyle: String, CaseIterable, Identifiable {
    case initials
    case hide
    case brand
    case bandana
    case brandOnHide
    case cowFace

    static let storageKey = "CowchatMac.roomIconStyle"
    static let fallback = RoomIconStyle.brandOnHide

    var id: String { rawValue }

    var label: String {
        switch self {
        case .initials: return "Initials"
        case .hide: return "Cowhide"
        case .brand: return "Cattle brand"
        case .bandana: return "Bandana"
        case .brandOnHide: return "Brand on hide"
        case .cowFace: return "Cow face"
        }
    }

    var blurb: String {
        switch self {
        case .initials: return "Plain lettering, the way it was."
        case .hide: return "Breed palette and patch field from the name."
        case .brand: return "Room letters plus a hashed brand modifier."
        case .bandana: return "Rotationally symmetric paisley tile."
        case .brandOnHide: return "Hide for the glance, brand for the identity."
        case .cowFace: return "Horns, ears, patches — best above 48pt."
        }
    }
}

struct HidePalette {
    let ground: Color
    let markings: Color
    /// Ink for anything burned on top of this hide.
    let ink: Color
    let muzzle: Color
    let isDarkGround: Bool

    /// A brand is a burn, and a burn is near-black — tinting it to the hide's
    /// own ramp left the pale grounds (roan, sage, hereford) unreadable at 40pt.
    var burnInk: Color {
        isDarkGround ? Palette.hay100 : Palette.bison900
    }
}

struct InkPalette {
    let ground: Color
    let ink: Color
}

enum RoomIconPalette {
    static let hides: [HidePalette] = [
        // Holstein
        HidePalette(
            ground: Palette.hay200, markings: Palette.bison800,
            ink: Palette.bison800, muzzle: Palette.canyon100, isDarkGround: false
        ),
        // Angus
        HidePalette(
            ground: Palette.bison600, markings: Palette.bison900,
            ink: Palette.hay200, muzzle: Palette.bison300, isDarkGround: true
        ),
        // Hereford
        HidePalette(
            ground: Palette.hay400, markings: Palette.canyon700,
            ink: Palette.canyon800, muzzle: Palette.canyon100, isDarkGround: false
        ),
        // Jersey
        HidePalette(
            ground: Palette.nugget200, markings: Palette.nugget800,
            ink: Palette.nugget900, muzzle: Palette.nugget100, isDarkGround: false
        ),
        // Brindle
        HidePalette(
            ground: Palette.nugget300, markings: Palette.bison700,
            ink: Palette.bison900, muzzle: Palette.nugget100, isDarkGround: false
        ),
        // Red roan
        HidePalette(
            ground: Palette.canyon100, markings: Palette.canyon600,
            ink: Palette.canyon900, muzzle: Palette.canyon50, isDarkGround: false
        ),
        // Sage
        HidePalette(
            ground: Palette.cactus100, markings: Palette.cactus700,
            ink: Palette.cactus900, muzzle: Palette.hay100, isDarkGround: false
        ),
        // Blue roan
        HidePalette(
            ground: Palette.denim100, markings: Palette.denim700,
            ink: Palette.denim900, muzzle: Palette.hay100, isDarkGround: false
        ),
    ]

    static let brands: [InkPalette] = [
        InkPalette(ground: Palette.bison700, ink: Palette.hay200),
        InkPalette(ground: Palette.hay300, ink: Palette.bison800),
        InkPalette(ground: Palette.nugget900, ink: Palette.nugget200),
        InkPalette(ground: Palette.hay700, ink: Palette.bison800),
    ]

    static let bandanas: [InkPalette] = [
        InkPalette(ground: Palette.canyon600, ink: Palette.hay100),
        InkPalette(ground: Palette.denim700, ink: Palette.hay100),
        InkPalette(ground: Palette.hay200, ink: Palette.canyon700),
        InkPalette(ground: Palette.bison800, ink: Palette.nugget300),
        InkPalette(ground: Palette.cactus700, ink: Palette.hay200),
        InkPalette(ground: Palette.nugget700, ink: Palette.nugget50),
    ]

    /// Icon fields are art, not chrome: they carry their own light/dark-safe
    /// pairs, so a hide never needs to know the app's appearance.
    static let faceField = Palette.hay200
}

/// Geometry is in unit space (0…1 across the circle) so one trait set renders
/// at any avatar size.
struct HideTraits: Equatable {
    struct Patch: Equatable {
        let x: Double
        let y: Double
        let radius: Double
        let wobble: [Double]
    }

    static let wobblePoints = 9

    let paletteIndex: Int
    let patches: [Patch]

    init(seed: RoomIconSeed, domain: String) {
        paletteIndex = seed.index("\(domain).palette", RoomIconPalette.hides.count)
        let count = 3 + seed.index("\(domain).count", 4)
        patches = (0..<count).map { patch in
            Patch(
                x: seed.value("\(domain).\(patch).x", 0.14...0.86),
                y: seed.value("\(domain).\(patch).y", 0.14...0.86),
                radius: seed.value("\(domain).\(patch).r", 0.15...0.28),
                wobble: (0..<Self.wobblePoints).map { point in
                    seed.value("\(domain).\(patch).w\(point)", 0.62...1.22)
                }
            )
        }
    }
}

enum BrandModifier: String, CaseIterable, Equatable {
    case none
    case barOver
    case barUnder
    case rocking
    case flying
    case walking
    case circled
}

enum BrandStance: String, CaseIterable, Equatable {
    case upright
    /// A brand turned on its side is "lazy"; turned a quarter further, "tumbling".
    case lazy
    case tumbling

    var degrees: Double {
        switch self {
        case .upright: return 0
        case .lazy: return -90
        case .tumbling: return 45
        }
    }
}

struct BrandTraits: Equatable {
    let mark: String
    let modifier: BrandModifier
    let stance: BrandStance
    let paletteIndex: Int

    /// `none` and `upright` are weighted up: every room wearing an ornament
    /// reads as noise, and a wall of rotated letters stops being scannable.
    private static let modifiers: [BrandModifier] = [
        .none, .none, .barOver, .barUnder, .rocking, .flying, .walking, .circled,
    ]
    private static let stances: [BrandStance] = [
        .upright, .upright, .upright, .upright, .upright, .upright, .lazy, .tumbling,
    ]

    init(seed: RoomIconSeed, name: String) {
        mark = Self.mark(for: name)
        let stance = seed.pick("brand.stance", Self.stances)
        self.stance = stance
        // One ornament at a time. A rotated two-letter mark wearing wings is
        // authentic brand grammar and completely unreadable at 40pt.
        modifier = stance == .upright ? seed.pick("brand.modifier", Self.modifiers) : .none
        paletteIndex = seed.index("brand.palette", RoomIconPalette.brands.count)
    }

    /// Two letters, not one: cowboy rooms cluster hard on a single initial
    /// (cbqs-, cbss-, cbfs-, canyon-), and a two-letter mark splits them.
    static func mark(for name: String) -> String {
        let tokens = name.split { !$0.isLetter && !$0.isNumber }
        let heads = tokens.prefix(2).compactMap(\.first)
        guard !heads.isEmpty else { return "#" }
        return String(heads).uppercased()
    }
}

enum BandanaMotif: String, CaseIterable, Equatable {
    case dots
    case diamond
    case paisley
}

struct BandanaTraits: Equatable {
    let paletteIndex: Int
    let motif: BandanaMotif
    let spokes: Int
    let inner: Double
    let outer: Double
    let hub: Double
    let dash: Double

    init(seed: RoomIconSeed) {
        paletteIndex = seed.index("bandana.palette", RoomIconPalette.bandanas.count)
        motif = seed.pick("bandana.motif", BandanaMotif.allCases)
        spokes = 6 + seed.index("bandana.spokes", 4)
        inner = seed.value("bandana.inner", 0.14...0.22)
        outer = seed.value("bandana.outer", 0.30...0.39)
        hub = seed.value("bandana.hub", 0.05...0.10)
        dash = seed.value("bandana.dash", 0.03...0.09)
    }
}

enum CowHorns: String, CaseIterable, Equatable {
    case polled
    case nubs
    case upright
    case longhorn
}

enum CowEyePatch: String, CaseIterable, Equatable {
    case none
    case left
    case right
}

struct CowFaceTraits: Equatable {
    let paletteIndex: Int
    let horns: CowHorns
    let eyePatch: CowEyePatch
    let forelock: Bool
    let earTag: Bool

    init(seed: RoomIconSeed) {
        paletteIndex = seed.index("face.palette", RoomIconPalette.hides.count)
        horns = seed.pick("face.horns", CowHorns.allCases)
        eyePatch = seed.pick("face.patch", CowEyePatch.allCases)
        forelock = seed.unit("face.forelock") < 0.45
        earTag = seed.unit("face.tag") < 0.3
    }
}

struct RoomIconTraits: Equatable {
    let hide: HideTraits
    let combo: HideTraits
    let brand: BrandTraits
    let bandana: BandanaTraits
    let face: CowFaceTraits

    init(name: String) {
        let seed = RoomIconSeed(name)
        hide = HideTraits(seed: seed, domain: "hide")
        // A separate domain so a room's plain hide and its branded hide are not
        // the same patch field with a letter dropped on top.
        combo = HideTraits(seed: seed, domain: "combo")
        brand = BrandTraits(seed: seed, name: name)
        bandana = BandanaTraits(seed: seed)
        face = CowFaceTraits(seed: seed)
    }
}

/// The sidebar rebuilds rows on every activity tick; deriving ~90 hashes per
/// avatar per pass is wasteful when the answer never changes.
@MainActor
enum RoomIconTraitsCache {
    private static var entries: [String: RoomIconTraits] = [:]

    static func traits(for name: String) -> RoomIconTraits {
        if let cached = entries[name] { return cached }
        if entries.count > 256 { entries.removeAll(keepingCapacity: true) }
        let traits = RoomIconTraits(name: name)
        entries[name] = traits
        return traits
    }
}
