import Foundation

/// Generates `animal-color-noun` room names (llama-orange-soviet) so the
/// create sheet can offer one ready-made: leave the field empty and you get
/// the suggestion, type to override.
enum RoomNameSuggestion {
    static let animals = [
        "llama", "bison", "heron", "otter", "badger", "coyote", "falcon",
        "gecko", "ibex", "jackal", "kestrel", "lemur", "marmot", "narwhal",
        "ocelot", "pelican", "quokka", "raven", "stallion", "toucan",
        "urchin", "viper", "walrus", "yak", "zebra", "armadillo", "bobcat",
        "condor", "dingo", "egret", "ferret", "gopher", "hedgehog",
        "iguana", "jaguar", "kiwi", "lynx", "mustang", "osprey", "puffin",
    ]

    static let colors = [
        "amber", "azure", "bronze", "carmine", "cobalt", "copper", "coral",
        "crimson", "emerald", "fuchsia", "gold", "indigo", "ivory", "jade",
        "lavender", "magenta", "maroon", "ochre", "olive", "orange",
        "russet", "saffron", "scarlet", "teal", "umber", "violet",
    ]

    static let nouns = [
        "soviet", "abacus", "bagpipe", "cosmonaut", "dynamo", "eclipse",
        "fresco", "gondola", "harpoon", "isotope", "javelin", "kazoo",
        "lantern", "monolith", "nomad", "obelisk", "pylon", "quasar",
        "rickshaw", "sextant", "tundra", "ultimatum", "vortex", "waltz",
        "xylophone", "yonder", "zeppelin", "almanac", "ballast", "caravan",
        "derrick", "epoch", "flotilla", "gauntlet", "hangar", "ingot",
        "jubilee", "keel", "lodestar", "meridian",
    ]

    static func generate() -> String {
        var generator = SystemRandomNumberGenerator()
        return generate(using: &generator)
    }

    static func generate(using generator: inout some RandomNumberGenerator) -> String {
        let animal = animals.randomElement(using: &generator) ?? "llama"
        let color = colors.randomElement(using: &generator) ?? "orange"
        let noun = nouns.randomElement(using: &generator) ?? "soviet"
        return "\(animal)-\(color)-\(noun)"
    }
}
