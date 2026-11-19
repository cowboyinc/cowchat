// Vendored from cowboyinc/macos Gallop/Generated/GallopTokens.swift — do not edit; re-copy to sync.
// Regenerate: pnpm --filter @cowboyinc/gallop-foundations tokens:swift
// Source of truth: gallop/packages/foundations/src/{colors,typography}.ts
//
// WARNING: this file is NOT reproducible from gallop's main. Its colour tokens
// were reconciled against the Dash Figma library, which outranks Gallop, and
// the matching gallop-repo edits were reverted at the owner's instruction.
// Regenerating against gallop's main today reverts 61 semantic tokens, the
// `lantern` ramp, nine palette hexes, and the glass alpha — and breaks
// SemanticColorTests and the gallop-tokens.json fixture.
// Apply docs/gallop-token-reconciliation.patch to the gallop checkout first.
// See docs/dash-design-system.md.

import CoreGraphics
import SwiftUI

/// Raw Gallop palette hues. Theme-independent.
public enum Palette {
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay50 = HexColor.color("#FCFAF8")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay100 = HexColor.color("#F9F7F5")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay200 = HexColor.color("#F4F0EB")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay300 = HexColor.color("#F1EBE5")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay400 = HexColor.color("#EBE5DF")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay500 = HexColor.color("#E6DED6")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay600 = HexColor.color("#DFD6CD")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay700 = HexColor.color("#D9CFC4")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay800 = HexColor.color("#D3C7BB")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay900 = HexColor.color("#CDBFB1")
    /// Warm off-white to mid taupe. Primary neutral surface colour.
    public static let hay950 = HexColor.color("#C5B6A5")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison50 = HexColor.color("#AEA399")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison100 = HexColor.color("#988E86")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison200 = HexColor.color("#8A7F76")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison300 = HexColor.color("#72675F")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison400 = HexColor.color("#61574F")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison500 = HexColor.color("#4C433C")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison600 = HexColor.color("#3D3530")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison700 = HexColor.color("#2E2824")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison800 = HexColor.color("#26211D")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison900 = HexColor.color("#1D1916")
    /// Light warm taupe to near-black. Primary text and dark surface colour.
    public static let bison950 = HexColor.color("#0F0E0C")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget50 = HexColor.color("#FFF7EB")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget100 = HexColor.color("#FFEECC")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget200 = HexColor.color("#FFDA99")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget300 = HexColor.color("#FFC15C")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget400 = HexColor.color("#FFAD33")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget500 = HexColor.color("#FF9D14")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget600 = HexColor.color("#E58200")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget700 = HexColor.color("#A85700")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget800 = HexColor.color("#783E00")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget900 = HexColor.color("#512900")
    /// Warm amber. Primary brand accent and interactive colour.
    public static let nugget950 = HexColor.color("#291400")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon50 = HexColor.color("#FDF3F1")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon100 = HexColor.color("#F4D8D0")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon200 = HexColor.color("#EEAF9C")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon300 = HexColor.color("#EA8864")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon400 = HexColor.color("#E35531")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon500 = HexColor.color("#CD3914")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon600 = HexColor.color("#B23010")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon700 = HexColor.color("#93260B")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon800 = HexColor.color("#77210B")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon900 = HexColor.color("#470E00")
    /// Deep terracotta red. Used for error states and destructive actions.
    public static let canyon950 = HexColor.color("#290800")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus50 = HexColor.color("#EEF7F0")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus100 = HexColor.color("#DBF0E1")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus200 = HexColor.color("#ACD8BC")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus300 = HexColor.color("#7EC89C")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus400 = HexColor.color("#4BAA6E")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus500 = HexColor.color("#328F58")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus600 = HexColor.color("#29754A")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus700 = HexColor.color("#1A5B36")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus800 = HexColor.color("#0F4327")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus900 = HexColor.color("#06321A")
    /// Fresh sage green. Used for success states and positive feedback.
    public static let cactus950 = HexColor.color("#001F0E")
    /// Cool steel blue. Used for informational states and links.
    public static let denim50 = HexColor.color("#F0F6FA")
    /// Cool steel blue. Used for informational states and links.
    public static let denim100 = HexColor.color("#DEEBF2")
    /// Cool steel blue. Used for informational states and links.
    public static let denim200 = HexColor.color("#AACADA")
    /// Cool steel blue. Used for informational states and links.
    public static let denim300 = HexColor.color("#7EAFC8")
    /// Cool steel blue. Used for informational states and links.
    public static let denim400 = HexColor.color("#5997BB")
    /// Cool steel blue. Used for informational states and links.
    public static let denim500 = HexColor.color("#3982AC")
    /// Cool steel blue. Used for informational states and links.
    public static let denim600 = HexColor.color("#1D5E86")
    /// Cool steel blue. Used for informational states and links.
    public static let denim700 = HexColor.color("#12496E")
    /// Cool steel blue. Used for informational states and links.
    public static let denim800 = HexColor.color("#07304B")
    /// Cool steel blue. Used for informational states and links.
    public static let denim900 = HexColor.color("#012641")
    /// Cool steel blue. Used for informational states and links.
    public static let denim950 = HexColor.color("#001829")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern50 = HexColor.color("#FEF5ED")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern100 = HexColor.color("#FBE4CE")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern200 = HexColor.color("#F8C69A")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern300 = HexColor.color("#F7A55F")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern400 = HexColor.color("#F48732")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern500 = HexColor.color("#EB7214")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern600 = HexColor.color("#BA5208")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern700 = HexColor.color("#A04304")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern800 = HexColor.color("#5F2702")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern900 = HexColor.color("#4D1E00")
    /// Burnt orange. Used for warning states and code syntax keywords.
    public static let lantern950 = HexColor.color("#290F00")
    /// Pure white and black for backgrounds, overlays, and high-contrast text.
    public static let white = HexColor.color("#FFFFFF")
    /// Pure white and black for backgrounds, overlays, and high-contrast text.
    public static let black = HexColor.color("#000000")

    public struct Token: Sendable, Equatable {
        public let name: String
        public let hex: String
    }

    /// Every palette token, in generation order. Used by parity tests.
    public static let allTokens: [Token] = [
        Token(name: "hay50", hex: "#FCFAF8"),
        Token(name: "hay100", hex: "#F9F7F5"),
        Token(name: "hay200", hex: "#F4F0EB"),
        Token(name: "hay300", hex: "#F1EBE5"),
        Token(name: "hay400", hex: "#EBE5DF"),
        Token(name: "hay500", hex: "#E6DED6"),
        Token(name: "hay600", hex: "#DFD6CD"),
        Token(name: "hay700", hex: "#D9CFC4"),
        Token(name: "hay800", hex: "#D3C7BB"),
        Token(name: "hay900", hex: "#CDBFB1"),
        Token(name: "hay950", hex: "#C5B6A5"),
        Token(name: "bison50", hex: "#AEA399"),
        Token(name: "bison100", hex: "#988E86"),
        Token(name: "bison200", hex: "#8A7F76"),
        Token(name: "bison300", hex: "#72675F"),
        Token(name: "bison400", hex: "#61574F"),
        Token(name: "bison500", hex: "#4C433C"),
        Token(name: "bison600", hex: "#3D3530"),
        Token(name: "bison700", hex: "#2E2824"),
        Token(name: "bison800", hex: "#26211D"),
        Token(name: "bison900", hex: "#1D1916"),
        Token(name: "bison950", hex: "#0F0E0C"),
        Token(name: "nugget50", hex: "#FFF7EB"),
        Token(name: "nugget100", hex: "#FFEECC"),
        Token(name: "nugget200", hex: "#FFDA99"),
        Token(name: "nugget300", hex: "#FFC15C"),
        Token(name: "nugget400", hex: "#FFAD33"),
        Token(name: "nugget500", hex: "#FF9D14"),
        Token(name: "nugget600", hex: "#E58200"),
        Token(name: "nugget700", hex: "#A85700"),
        Token(name: "nugget800", hex: "#783E00"),
        Token(name: "nugget900", hex: "#512900"),
        Token(name: "nugget950", hex: "#291400"),
        Token(name: "canyon50", hex: "#FDF3F1"),
        Token(name: "canyon100", hex: "#F4D8D0"),
        Token(name: "canyon200", hex: "#EEAF9C"),
        Token(name: "canyon300", hex: "#EA8864"),
        Token(name: "canyon400", hex: "#E35531"),
        Token(name: "canyon500", hex: "#CD3914"),
        Token(name: "canyon600", hex: "#B23010"),
        Token(name: "canyon700", hex: "#93260B"),
        Token(name: "canyon800", hex: "#77210B"),
        Token(name: "canyon900", hex: "#470E00"),
        Token(name: "canyon950", hex: "#290800"),
        Token(name: "cactus50", hex: "#EEF7F0"),
        Token(name: "cactus100", hex: "#DBF0E1"),
        Token(name: "cactus200", hex: "#ACD8BC"),
        Token(name: "cactus300", hex: "#7EC89C"),
        Token(name: "cactus400", hex: "#4BAA6E"),
        Token(name: "cactus500", hex: "#328F58"),
        Token(name: "cactus600", hex: "#29754A"),
        Token(name: "cactus700", hex: "#1A5B36"),
        Token(name: "cactus800", hex: "#0F4327"),
        Token(name: "cactus900", hex: "#06321A"),
        Token(name: "cactus950", hex: "#001F0E"),
        Token(name: "denim50", hex: "#F0F6FA"),
        Token(name: "denim100", hex: "#DEEBF2"),
        Token(name: "denim200", hex: "#AACADA"),
        Token(name: "denim300", hex: "#7EAFC8"),
        Token(name: "denim400", hex: "#5997BB"),
        Token(name: "denim500", hex: "#3982AC"),
        Token(name: "denim600", hex: "#1D5E86"),
        Token(name: "denim700", hex: "#12496E"),
        Token(name: "denim800", hex: "#07304B"),
        Token(name: "denim900", hex: "#012641"),
        Token(name: "denim950", hex: "#001829"),
        Token(name: "lantern50", hex: "#FEF5ED"),
        Token(name: "lantern100", hex: "#FBE4CE"),
        Token(name: "lantern200", hex: "#F8C69A"),
        Token(name: "lantern300", hex: "#F7A55F"),
        Token(name: "lantern400", hex: "#F48732"),
        Token(name: "lantern500", hex: "#EB7214"),
        Token(name: "lantern600", hex: "#BA5208"),
        Token(name: "lantern700", hex: "#A04304"),
        Token(name: "lantern800", hex: "#5F2702"),
        Token(name: "lantern900", hex: "#4D1E00"),
        Token(name: "lantern950", hex: "#290F00"),
        Token(name: "white", hex: "#FFFFFF"),
        Token(name: "black", hex: "#000000"),
    ]
}

/// Semantic Gallop colours. Each resolves per system appearance (light/dark).
public enum SemanticColor {
    /// Deepest recessed surface. Used for inset wells and deeply nested containers.
    public static let surface300 = HexColor.adaptive(light: "#F1EBE5", dark: "#000000")
    /// Sunken surface. Used for input fields, code blocks, and recessed areas.
    public static let surface400 = HexColor.adaptive(light: "#F4F0EB", dark: "#0F0E0C")
    /// Base surface. The default page/app background.
    public static let surface500 = HexColor.adaptive(light: "#F9F7F5", dark: "#1D1916")
    /// Raised surface. Cards, panels, and other elevated containers.
    public static let surface600 = HexColor.adaptive(light: "#FCFAF8", dark: "#26211D")
    /// Highest surface. Modals, popovers, and floating overlays.
    public static let surface700 = HexColor.adaptive(light: "#FFFFFF", dark: "#2E2824")
    /// Raised glass surface. One step lighter than the base glass layer — use for hovered or elevated frosted panels.
    public static let surfaceGlass600 = HexColor.adaptive(light: "#FFFFFFB8", dark: "#3D3530B8")
    /// Primary glass surface. Warm off-white frosted layer (light) or dark-tinted (dark). Use for blurred panel backgrounds.
    public static let surfaceGlass500 = HexColor.adaptive(light: "#FCFAF8B8", dark: "#2E2824B8")
    /// Sunken glass surface. One step darker than the base glass layer — use for pressed or recessed frosted panels.
    public static let surfaceGlass400 = HexColor.adaptive(light: "#F9F7F5B8", dark: "#26211DB8")
    /// Bright inner border for glass containers — simulates a light catching the top edge.
    public static let surfaceGlassBorderHighlight = HexColor.adaptive(light: "#FFFFFFCC", dark: "#FFFFFF14")
    /// Dark inner border for glass containers — simulates a shadow on the bottom edge.
    public static let surfaceGlassBorderShadow = HexColor.adaptive(light: "#2E282414", dark: "#0F0E0CA3")
    /// Subtle tinted overlay applied to a glass surface on hover (off/unselected state).
    public static let surfaceGlassOffHover = HexColor.adaptive(light: "#C5B6A51F", dark: "#0F0E0C29")
    /// Stronger tinted overlay applied to a glass surface on press (off/unselected state).
    public static let surfaceGlassOffPressed = HexColor.adaptive(light: "#C5B6A53D", dark: "#0F0E0C3D")
    /// Glass surface tint in the selected/on state at rest. Warm amber highlight.
    public static let surfaceGlassOnDefault = HexColor.adaptive(light: "#FFF7EB", dark: "#291400")
    /// Glass surface tint in the selected/on state on hover. Matches default — no additional feedback.
    public static let surfaceGlassOnHover = HexColor.adaptive(light: "#FFF7EB", dark: "#291400")
    /// Glass surface overlay in the selected/on state on press. Re-uses the off.pressed overlay regardless of selection state.
    public static let surfaceGlassOnPressed = HexColor.adaptive(light: "#C5B6A53D", dark: "#0F0E0C3D")
    /// Text colour on an unselected glass surface at rest.
    public static let surfaceGlassOffTextDefault = HexColor.adaptive(light: "#3D3530", dark: "#D3C7BB")
    /// Text colour on an unselected glass surface on hover.
    public static let surfaceGlassOffTextHover = HexColor.adaptive(light: "#1D1916", dark: "#F4F0EB")
    /// Text colour on an unselected glass surface on press.
    public static let surfaceGlassOffTextPressed = HexColor.adaptive(light: "#1D1916", dark: "#D3C7BB")
    /// Icon colour on an unselected glass surface at rest.
    public static let surfaceGlassOffIconDefault = HexColor.adaptive(light: "#4C433C", dark: "#CDBFB1")
    /// Icon colour on an unselected glass surface on hover.
    public static let surfaceGlassOffIconHover = HexColor.adaptive(light: "#26211D", dark: "#F1EBE5")
    /// Icon colour on an unselected glass surface on press.
    public static let surfaceGlassOffIconPressed = HexColor.adaptive(light: "#26211D", dark: "#F1EBE5")
    /// Text colour on a selected glass surface at rest. Warm amber to match the on-state fill.
    public static let surfaceGlassOnTextDefault = HexColor.adaptive(light: "#A85700", dark: "#FFAD33")
    /// Text colour on a selected glass surface on hover. Matches default — no additional feedback.
    public static let surfaceGlassOnTextHover = HexColor.adaptive(light: "#A85700", dark: "#FFAD33")
    /// Icon colour on a selected glass surface at rest. Warm amber to match the on-state fill.
    public static let surfaceGlassOnIconDefault = HexColor.adaptive(light: "#E58200", dark: "#FF9D14")
    /// Icon colour on a selected glass surface on hover. Matches default — no additional feedback.
    public static let surfaceGlassOnIconHover = HexColor.adaptive(light: "#E58200", dark: "#FF9D14")
    /// Primary text. Headings, body copy, and high-emphasis labels.
    public static let textPrimary = HexColor.adaptive(light: "#1D1916", dark: "#F4F0EB")
    /// Secondary text. Supporting copy and medium-emphasis labels.
    public static let textSecondary = HexColor.adaptive(light: "#3D3530", dark: "#D3C7BB")
    /// Tertiary text. Placeholders, captions, and low-emphasis metadata.
    public static let textTertiary = HexColor.adaptive(light: "#72675F", dark: "#AEA399")
    /// Error message text and inline validation feedback.
    public static let textError = HexColor.adaptive(light: "#B23010", dark: "#E35531")
    /// Label text for destructive actions — delete, remove, revoke. Resolves to the same hue as text.error today but carries a distinct intent.
    public static let textDestructive = HexColor.adaptive(light: "#B23010", dark: "#E35531")
    /// Text colour for disabled controls and non-interactive content.
    public static let textDisabled = HexColor.adaptive(light: "#988E86", dark: "#4C433C")
    /// Primary icon colour. High-emphasis icons and decorative marks.
    public static let iconPrimary = HexColor.adaptive(light: "#26211D", dark: "#F1EBE5")
    /// Secondary icon colour. Supporting and medium-emphasis icons.
    public static let iconSecondary = HexColor.adaptive(light: "#4C433C", dark: "#CDBFB1")
    /// Tertiary icon colour. Low-emphasis and decorative icons.
    public static let iconTertiary = HexColor.adaptive(light: "#72675F", dark: "#988E86")
    /// Error icon colour. Validation icons and destructive action indicators.
    public static let iconError = HexColor.adaptive(light: "#CD3914", dark: "#CD3914")
    /// Icon colour for destructive actions — delete, remove, revoke. Slightly lighter than icon.error in light mode so it holds up beside destructive labels.
    public static let iconDestructive = HexColor.adaptive(light: "#E35531", dark: "#CD3914")
    /// Icon colour for disabled controls.
    public static let iconDisabled = HexColor.adaptive(light: "#AEA399", dark: "#3D3530")
    /// Subtle icon colour. Use sparingly — primarily for right-facing chevrons that indicate a cell acts as a link or expands folder-like.
    public static let iconSubtle = HexColor.adaptive(light: "#AEA399", dark: "#3D3530")
    /// Default border. Used on cards, dividers, and container outlines at rest.
    public static let borderDefault = HexColor.adaptive(light: "#D9CFC4", dark: "#3D3530")
    /// Border on hover.
    public static let borderHover = HexColor.adaptive(light: "#CDBFB1", dark: "#4C433C")
    /// Border on press.
    public static let borderPressed = HexColor.adaptive(light: "#988E86", dark: "#26211D")
    /// Focus border. Used on focused inputs and keyboard-navigated controls.
    public static let borderFocus = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Accessible border. Meets WCAG 3:1 contrast ratio against the base background.
    public static let borderAccessibleDefault = HexColor.adaptive(light: "#988E86", dark: "#72675F")
    /// Accessible border on hover.
    public static let borderAccessibleHover = HexColor.adaptive(light: "#8A7F76", dark: "#8A7F76")
    /// Accessible border on press.
    public static let borderAccessiblePressed = HexColor.adaptive(light: "#72675F", dark: "#61574F")
    /// Error border. Used on invalid inputs and form fields with validation errors.
    public static let borderError = HexColor.adaptive(light: "#CD3914", dark: "#CD3914")
    /// Border colour for disabled controls.
    public static let borderDisabled = HexColor.adaptive(light: "#E6DED6", dark: "#2E2824")
    /// Primary button fill at rest.
    public static let buttonPrimaryDefault = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Primary button fill on hover.
    public static let buttonPrimaryHover = HexColor.adaptive(light: "#FFAD33", dark: "#FFAD33")
    /// Primary button fill when pressed.
    public static let buttonPrimaryPressed = HexColor.adaptive(light: "#E58200", dark: "#E58200")
    /// Primary button fill when disabled.
    public static let buttonPrimaryDisabled = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Secondary button fill at rest.
    public static let buttonSecondaryDefault = HexColor.adaptive(light: "#EBE5DF", dark: "#2E2824")
    /// Secondary button fill on hover.
    public static let buttonSecondaryHover = HexColor.adaptive(light: "#F1EBE5", dark: "#3D3530")
    /// Secondary button fill when pressed.
    public static let buttonSecondaryPressed = HexColor.adaptive(light: "#E6DED6", dark: "#26211D")
    /// Secondary button fill when disabled.
    public static let buttonSecondaryDisabled = HexColor.adaptive(light: "#EBE5DF", dark: "#2E2824")
    /// Ghost button fill on hover.
    public static let buttonGhostHover = HexColor.adaptive(light: "#F1EBE5", dark: "#3D3530")
    /// Ghost button fill when pressed.
    public static let buttonGhostPressed = HexColor.adaptive(light: "#EBE5DF", dark: "#2E2824")
    /// Label colour on a primary button at rest.
    public static let buttonPrimaryTextDefault = HexColor.adaptive(light: "#512900", dark: "#512900")
    /// Label colour on a primary button on hover.
    public static let buttonPrimaryTextHover = HexColor.adaptive(light: "#783E00", dark: "#783E00")
    /// Label colour on a primary button when pressed.
    public static let buttonPrimaryTextPressed = HexColor.adaptive(light: "#291400", dark: "#291400")
    /// Label colour on a disabled primary button.
    public static let buttonPrimaryTextDisabled = HexColor.adaptive(light: "#512900", dark: "#512900")
    /// Icon colour on a primary button at rest.
    public static let buttonPrimaryIconDefault = HexColor.adaptive(light: "#783E00", dark: "#783E00")
    /// Icon colour on a primary button on hover.
    public static let buttonPrimaryIconHover = HexColor.adaptive(light: "#A85700", dark: "#A85700")
    /// Icon colour on a primary button when pressed.
    public static let buttonPrimaryIconPressed = HexColor.adaptive(light: "#512900", dark: "#512900")
    /// Icon colour on a disabled primary button.
    public static let buttonPrimaryIconDisabled = HexColor.adaptive(light: "#783E00", dark: "#783E00")
    /// Label colour on a secondary button at rest.
    public static let buttonSecondaryTextDefault = HexColor.adaptive(light: "#3D3530", dark: "#D3C7BB")
    /// Label colour on a secondary button on hover.
    public static let buttonSecondaryTextHover = HexColor.adaptive(light: "#4C433C", dark: "#D9CFC4")
    /// Label colour on a secondary button when pressed.
    public static let buttonSecondaryTextPressed = HexColor.adaptive(light: "#1D1916", dark: "#AEA399")
    /// Label colour on a disabled secondary button.
    public static let buttonSecondaryTextDisabled = HexColor.adaptive(light: "#72675F", dark: "#AEA399")
    /// Icon colour on a secondary button at rest.
    public static let buttonSecondaryIconDefault = HexColor.adaptive(light: "#4C433C", dark: "#CDBFB1")
    /// Icon colour on a secondary button on hover.
    public static let buttonSecondaryIconHover = HexColor.adaptive(light: "#61574F", dark: "#D3C7BB")
    /// Icon colour on a secondary button when pressed.
    public static let buttonSecondaryIconPressed = HexColor.adaptive(light: "#26211D", dark: "#988E86")
    /// Icon colour on a disabled secondary button.
    public static let buttonSecondaryIconDisabled = HexColor.adaptive(light: "#72675F", dark: "#988E86")
    /// Icon colour on a ghost button at rest.
    public static let buttonGhostIconDefault = HexColor.adaptive(light: "#72675F", dark: "#988E86")
    /// Icon colour on a ghost button on hover.
    public static let buttonGhostIconHover = HexColor.adaptive(light: "#8A7F76", dark: "#AEA399")
    /// Icon colour on a ghost button when pressed.
    public static let buttonGhostIconPressed = HexColor.adaptive(light: "#26211D", dark: "#988E86")
    /// Icon colour on a disabled ghost button.
    public static let buttonGhostIconDisabled = HexColor.adaptive(light: "#72675F", dark: "#988E86")
    /// Floating button fill at rest. Frosted glass so underlying content shows through.
    public static let buttonFloatingDefault = HexColor.adaptive(light: "#FCFAF8B8", dark: "#2E2824B8")
    /// Floating button fill on hover.
    public static let buttonFloatingHover = HexColor.adaptive(light: "#FFFFFFB8", dark: "#3D3530B8")
    /// Floating button fill when pressed.
    public static let buttonFloatingPressed = HexColor.adaptive(light: "#F9F7F5B8", dark: "#26211DB8")
    /// Floating button fill when disabled.
    public static let buttonFloatingDisabled = HexColor.adaptive(light: "#F9F7F5B8", dark: "#26211DB8")
    /// Label colour on a floating button at rest.
    public static let buttonFloatingTextDefault = HexColor.adaptive(light: "#3D3530", dark: "#D3C7BB")
    /// Label colour on a floating button on hover.
    public static let buttonFloatingTextHover = HexColor.adaptive(light: "#4C433C", dark: "#D9CFC4")
    /// Label colour on a floating button when pressed.
    public static let buttonFloatingTextPressed = HexColor.adaptive(light: "#1D1916", dark: "#AEA399")
    /// Label colour on a disabled floating button.
    public static let buttonFloatingTextDisabled = HexColor.adaptive(light: "#72675F", dark: "#AEA399")
    /// Icon colour on a floating button at rest.
    public static let buttonFloatingIconDefault = HexColor.adaptive(light: "#72675F", dark: "#988E86")
    /// Icon colour on a floating button on hover.
    public static let buttonFloatingIconHover = HexColor.adaptive(light: "#8A7F76", dark: "#AEA399")
    /// Icon colour on a floating button when pressed.
    public static let buttonFloatingIconPressed = HexColor.adaptive(light: "#4C433C", dark: "#CDBFB1")
    /// Icon colour on a disabled floating button.
    public static let buttonFloatingIconDisabled = HexColor.adaptive(light: "#AEA399", dark: "#3D3530")
    /// Destructive primary button fill at rest. Used for delete, remove, and revoke confirmations.
    public static let buttonPrimaryDestructiveDefault = HexColor.adaptive(light: "#CD3914", dark: "#CD3914")
    /// Destructive primary button fill on hover.
    public static let buttonPrimaryDestructiveHover = HexColor.adaptive(light: "#E35531", dark: "#E35531")
    /// Destructive primary button fill when pressed.
    public static let buttonPrimaryDestructivePressed = HexColor.adaptive(light: "#B23010", dark: "#B23010")
    /// Destructive primary button fill when disabled.
    public static let buttonPrimaryDestructiveDisabled = HexColor.adaptive(light: "#CD3914", dark: "#CD3914")
    /// Label colour on a destructive primary button at rest.
    public static let buttonPrimaryDestructiveTextDefault = HexColor.adaptive(light: "#FDF3F1", dark: "#FDF3F1")
    /// Label colour on a destructive primary button on hover.
    public static let buttonPrimaryDestructiveTextHover = HexColor.adaptive(light: "#FFFFFF", dark: "#FFFFFF")
    /// Label colour on a destructive primary button when pressed.
    public static let buttonPrimaryDestructiveTextPressed = HexColor.adaptive(light: "#F4D8D0", dark: "#F4D8D0")
    /// Label colour on a disabled destructive primary button.
    public static let buttonPrimaryDestructiveTextDisabled = HexColor.adaptive(light: "#FDF3F1", dark: "#FDF3F1")
    /// Icon colour on a destructive primary button at rest.
    public static let buttonPrimaryDestructiveIconDefault = HexColor.adaptive(light: "#F4D8D0", dark: "#F4D8D0")
    /// Icon colour on a destructive primary button on hover.
    public static let buttonPrimaryDestructiveIconHover = HexColor.adaptive(light: "#FDF3F1", dark: "#FDF3F1")
    /// Icon colour on a destructive primary button when pressed.
    public static let buttonPrimaryDestructiveIconPressed = HexColor.adaptive(light: "#EEAF9C", dark: "#EEAF9C")
    /// Icon colour on a disabled destructive primary button.
    public static let buttonPrimaryDestructiveIconDisabled = HexColor.adaptive(light: "#FDF3F1", dark: "#FDF3F1")
    /// Unselected control fill at rest.
    public static let controlOffDefault = HexColor.adaptive(light: "#EBE5DF", dark: "#2E2824")
    /// Unselected control fill on hover.
    public static let controlOffHover = HexColor.adaptive(light: "#F1EBE5", dark: "#3D3530")
    /// Unselected control fill when pressed.
    public static let controlOffPressed = HexColor.adaptive(light: "#E6DED6", dark: "#26211D")
    /// Selected control fill at rest.
    public static let controlOnDefault = HexColor.adaptive(light: "#FFDA99", dark: "#512900")
    /// Selected control fill on hover.
    public static let controlOnHover = HexColor.adaptive(light: "#FFEECC", dark: "#783E00")
    /// Selected control fill when pressed.
    public static let controlOnPressed = HexColor.adaptive(light: "#FFDA99", dark: "#512900")
    /// Text field background at rest.
    public static let textfieldDefault = HexColor.adaptive(light: "#F9F7F5", dark: "#1D1916")
    /// Text field background on hover.
    public static let textfieldHover = HexColor.adaptive(light: "#FCFAF8", dark: "#26211D")
    /// Text field background when focused.
    public static let textfieldFocus = HexColor.adaptive(light: "#FCFAF8", dark: "#26211D")
    /// Text field background when disabled.
    public static let textfieldDisabled = HexColor.adaptive(light: "#F1EBE5", dark: "#0F0E0C")
    /// Checkbox fill when disabled (both on and off states).
    public static let checkboxDisabled = HexColor.adaptive(light: "#F1EBE5", dark: "#0F0E0C")
    /// Checked checkbox fill at rest.
    public static let checkboxOnDefault = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Checked checkbox fill on hover.
    public static let checkboxOnHover = HexColor.adaptive(light: "#FFAD33", dark: "#FFAD33")
    /// Checked checkbox fill when pressed.
    public static let checkboxOnPressed = HexColor.adaptive(light: "#E58200", dark: "#E58200")
    /// Unchecked checkbox fill at rest.
    public static let checkboxOffDefault = HexColor.adaptive(light: "#F9F7F5", dark: "#1D1916")
    /// Unchecked checkbox fill on hover.
    public static let checkboxOffHover = HexColor.adaptive(light: "#FCFAF8", dark: "#26211D")
    /// Unchecked checkbox fill when pressed.
    public static let checkboxOffPressed = HexColor.adaptive(light: "#F4F0EB", dark: "#0F0E0C")
    /// Radio button fill when disabled.
    public static let radioDisabled = HexColor.adaptive(light: "#F1EBE5", dark: "#0F0E0C")
    /// Selected radio button fill at rest.
    public static let radioOnDefault = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Selected radio button fill on hover.
    public static let radioOnHover = HexColor.adaptive(light: "#FFAD33", dark: "#FFAD33")
    /// Selected radio button fill when pressed.
    public static let radioOnPressed = HexColor.adaptive(light: "#E58200", dark: "#E58200")
    /// Unselected radio button fill at rest.
    public static let radioOffDefault = HexColor.adaptive(light: "#F9F7F5", dark: "#1D1916")
    /// Unselected radio button fill on hover.
    public static let radioOffHover = HexColor.adaptive(light: "#FCFAF8", dark: "#26211D")
    /// Unselected radio button fill when pressed.
    public static let radioOffPressed = HexColor.adaptive(light: "#F4F0EB", dark: "#0F0E0C")
    /// Toggle track fill when disabled.
    public static let toggleDisabled = HexColor.adaptive(light: "#DFD6CD", dark: "#2E2824")
    /// Toggle track fill in the off state at rest.
    public static let toggleOffDefault = HexColor.adaptive(light: "#AEA399", dark: "#4C433C")
    /// Toggle track fill in the off state on hover.
    public static let toggleOffHover = HexColor.adaptive(light: "#C5B6A5", dark: "#61574F")
    /// Toggle track fill in the off state when pressed.
    public static let toggleOffPressed = HexColor.adaptive(light: "#988E86", dark: "#3D3530")
    /// Toggle track fill in the on state at rest.
    public static let toggleOnDefault = HexColor.adaptive(light: "#FF9D14", dark: "#FF9D14")
    /// Toggle track fill in the on state on hover.
    public static let toggleOnHover = HexColor.adaptive(light: "#FFAD33", dark: "#FFAD33")
    /// Toggle track fill in the on state when pressed.
    public static let toggleOnPressed = HexColor.adaptive(light: "#E58200", dark: "#E58200")
    /// Success banner fill.
    public static let notificationSuccessDefault = HexColor.adaptive(light: "#EEF7F0", dark: "#001F0E")
    /// Success banner border.
    public static let notificationSuccessBorderDefault = HexColor.adaptive(light: "#ACD8BC", dark: "#0F4327")
    /// Success banner text.
    public static let notificationSuccessTextDefault = HexColor.adaptive(light: "#29754A", dark: "#7EC89C")
    /// Success banner status icon. Held constant across themes so the hue reads the same in both.
    public static let notificationSuccessIconPrimary = HexColor.adaptive(light: "#328F58", dark: "#328F58")
    /// Supporting icon on a success banner — dismiss controls and secondary affordances.
    public static let notificationSuccessIconSecondary = HexColor.adaptive(light: "#4BAA6E", dark: "#29754A")
    /// Warning banner fill.
    public static let notificationWarningDefault = HexColor.adaptive(light: "#FEF5ED", dark: "#290F00")
    /// Warning banner border.
    public static let notificationWarningBorderDefault = HexColor.adaptive(light: "#F8C69A", dark: "#5F2702")
    /// Warning banner text.
    public static let notificationWarningTextDefault = HexColor.adaptive(light: "#BA5208", dark: "#F48732")
    /// Warning banner status icon. Held constant across themes so the hue reads the same in both.
    public static let notificationWarningIconPrimary = HexColor.adaptive(light: "#EB7214", dark: "#EB7214")
    /// Supporting icon on a warning banner — dismiss controls and secondary affordances.
    public static let notificationWarningIconSecondary = HexColor.adaptive(light: "#F48732", dark: "#BA5208")
    /// Error banner fill.
    public static let notificationErrorDefault = HexColor.adaptive(light: "#FDF3F1", dark: "#290800")
    /// Error banner border.
    public static let notificationErrorBorderDefault = HexColor.adaptive(light: "#EEAF9C", dark: "#77210B")
    /// Error banner text.
    public static let notificationErrorTextDefault = HexColor.adaptive(light: "#B23010", dark: "#EA8864")
    /// Error banner status icon. Held constant across themes so the hue reads the same in both.
    public static let notificationErrorIconPrimary = HexColor.adaptive(light: "#CD3914", dark: "#CD3914")
    /// Supporting icon on an error banner — dismiss controls and secondary affordances.
    public static let notificationErrorIconSecondary = HexColor.adaptive(light: "#E35531", dark: "#B23010")
    /// Informational banner fill.
    public static let notificationInfoDefault = HexColor.adaptive(light: "#F0F6FA", dark: "#001829")
    /// Informational banner border.
    public static let notificationInfoBorderDefault = HexColor.adaptive(light: "#AACADA", dark: "#07304B")
    /// Informational banner text.
    public static let notificationInfoTextDefault = HexColor.adaptive(light: "#1D5E86", dark: "#7EAFC8")
    /// Informational banner status icon. Held constant across themes so the hue reads the same in both.
    public static let notificationInfoIconPrimary = HexColor.adaptive(light: "#3982AC", dark: "#3982AC")
    /// Supporting icon on an informational banner — dismiss controls and secondary affordances.
    public static let notificationInfoIconSecondary = HexColor.adaptive(light: "#5997BB", dark: "#1D5E86")
    /// Code block background. Always dark so syntax colours stay legible in either theme.
    public static let codeSurface = HexColor.adaptive(light: "#2E2824", dark: "#2E2824")
    /// Unhighlighted code text — identifiers and anything without a more specific token.
    public static let codePlainText = HexColor.adaptive(light: "#AEA399", dark: "#AEA399")
    /// Language keywords — if, return, const, function.
    public static let codeKeyword = HexColor.adaptive(light: "#EB7214", dark: "#EB7214")
    /// String and template literals.
    public static let codeString = HexColor.adaptive(light: "#FFAD33", dark: "#FFAD33")
    /// Function and method names at declaration and call sites.
    public static let codeFunction = HexColor.adaptive(light: "#5997BB", dark: "#5997BB")
    /// Type names, interfaces, and class names.
    public static let codeType = HexColor.adaptive(light: "#4BAA6E", dark: "#4BAA6E")
    /// Numeric and boolean literals.
    public static let codeNumber = HexColor.adaptive(light: "#4BAA6E", dark: "#4BAA6E")
    /// Decorators, annotations, and attribute macros.
    public static let codeDecorator = HexColor.adaptive(light: "#E6DED6", dark: "#E6DED6")
    /// Operators — assignment, arithmetic, comparison.
    public static let codeOperator = HexColor.adaptive(light: "#D3C7BB", dark: "#D3C7BB")
    /// Brackets, braces, commas, and semicolons.
    public static let codePunctuation = HexColor.adaptive(light: "#D3C7BB", dark: "#D3C7BB")
    /// Line and block comments.
    public static let codeComment = HexColor.adaptive(light: "#72675F", dark: "#72675F")

    public struct Token: Sendable, Equatable {
        public let name: String
        public let light: String
        public let dark: String
    }

    /// Every semantic token, in generation order. Used by parity tests.
    public static let allTokens: [Token] = [
        Token(name: "surface300", light: "#F1EBE5", dark: "#000000"),
        Token(name: "surface400", light: "#F4F0EB", dark: "#0F0E0C"),
        Token(name: "surface500", light: "#F9F7F5", dark: "#1D1916"),
        Token(name: "surface600", light: "#FCFAF8", dark: "#26211D"),
        Token(name: "surface700", light: "#FFFFFF", dark: "#2E2824"),
        Token(name: "surfaceGlass600", light: "#FFFFFFB8", dark: "#3D3530B8"),
        Token(name: "surfaceGlass500", light: "#FCFAF8B8", dark: "#2E2824B8"),
        Token(name: "surfaceGlass400", light: "#F9F7F5B8", dark: "#26211DB8"),
        Token(name: "surfaceGlassBorderHighlight", light: "#FFFFFFCC", dark: "#FFFFFF14"),
        Token(name: "surfaceGlassBorderShadow", light: "#2E282414", dark: "#0F0E0CA3"),
        Token(name: "surfaceGlassOffHover", light: "#C5B6A51F", dark: "#0F0E0C29"),
        Token(name: "surfaceGlassOffPressed", light: "#C5B6A53D", dark: "#0F0E0C3D"),
        Token(name: "surfaceGlassOnDefault", light: "#FFF7EB", dark: "#291400"),
        Token(name: "surfaceGlassOnHover", light: "#FFF7EB", dark: "#291400"),
        Token(name: "surfaceGlassOnPressed", light: "#C5B6A53D", dark: "#0F0E0C3D"),
        Token(name: "surfaceGlassOffTextDefault", light: "#3D3530", dark: "#D3C7BB"),
        Token(name: "surfaceGlassOffTextHover", light: "#1D1916", dark: "#F4F0EB"),
        Token(name: "surfaceGlassOffTextPressed", light: "#1D1916", dark: "#D3C7BB"),
        Token(name: "surfaceGlassOffIconDefault", light: "#4C433C", dark: "#CDBFB1"),
        Token(name: "surfaceGlassOffIconHover", light: "#26211D", dark: "#F1EBE5"),
        Token(name: "surfaceGlassOffIconPressed", light: "#26211D", dark: "#F1EBE5"),
        Token(name: "surfaceGlassOnTextDefault", light: "#A85700", dark: "#FFAD33"),
        Token(name: "surfaceGlassOnTextHover", light: "#A85700", dark: "#FFAD33"),
        Token(name: "surfaceGlassOnIconDefault", light: "#E58200", dark: "#FF9D14"),
        Token(name: "surfaceGlassOnIconHover", light: "#E58200", dark: "#FF9D14"),
        Token(name: "textPrimary", light: "#1D1916", dark: "#F4F0EB"),
        Token(name: "textSecondary", light: "#3D3530", dark: "#D3C7BB"),
        Token(name: "textTertiary", light: "#72675F", dark: "#AEA399"),
        Token(name: "textError", light: "#B23010", dark: "#E35531"),
        Token(name: "textDestructive", light: "#B23010", dark: "#E35531"),
        Token(name: "textDisabled", light: "#988E86", dark: "#4C433C"),
        Token(name: "iconPrimary", light: "#26211D", dark: "#F1EBE5"),
        Token(name: "iconSecondary", light: "#4C433C", dark: "#CDBFB1"),
        Token(name: "iconTertiary", light: "#72675F", dark: "#988E86"),
        Token(name: "iconError", light: "#CD3914", dark: "#CD3914"),
        Token(name: "iconDestructive", light: "#E35531", dark: "#CD3914"),
        Token(name: "iconDisabled", light: "#AEA399", dark: "#3D3530"),
        Token(name: "iconSubtle", light: "#AEA399", dark: "#3D3530"),
        Token(name: "borderDefault", light: "#D9CFC4", dark: "#3D3530"),
        Token(name: "borderHover", light: "#CDBFB1", dark: "#4C433C"),
        Token(name: "borderPressed", light: "#988E86", dark: "#26211D"),
        Token(name: "borderFocus", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "borderAccessibleDefault", light: "#988E86", dark: "#72675F"),
        Token(name: "borderAccessibleHover", light: "#8A7F76", dark: "#8A7F76"),
        Token(name: "borderAccessiblePressed", light: "#72675F", dark: "#61574F"),
        Token(name: "borderError", light: "#CD3914", dark: "#CD3914"),
        Token(name: "borderDisabled", light: "#E6DED6", dark: "#2E2824"),
        Token(name: "buttonPrimaryDefault", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "buttonPrimaryHover", light: "#FFAD33", dark: "#FFAD33"),
        Token(name: "buttonPrimaryPressed", light: "#E58200", dark: "#E58200"),
        Token(name: "buttonPrimaryDisabled", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "buttonSecondaryDefault", light: "#EBE5DF", dark: "#2E2824"),
        Token(name: "buttonSecondaryHover", light: "#F1EBE5", dark: "#3D3530"),
        Token(name: "buttonSecondaryPressed", light: "#E6DED6", dark: "#26211D"),
        Token(name: "buttonSecondaryDisabled", light: "#EBE5DF", dark: "#2E2824"),
        Token(name: "buttonGhostHover", light: "#F1EBE5", dark: "#3D3530"),
        Token(name: "buttonGhostPressed", light: "#EBE5DF", dark: "#2E2824"),
        Token(name: "buttonPrimaryTextDefault", light: "#512900", dark: "#512900"),
        Token(name: "buttonPrimaryTextHover", light: "#783E00", dark: "#783E00"),
        Token(name: "buttonPrimaryTextPressed", light: "#291400", dark: "#291400"),
        Token(name: "buttonPrimaryTextDisabled", light: "#512900", dark: "#512900"),
        Token(name: "buttonPrimaryIconDefault", light: "#783E00", dark: "#783E00"),
        Token(name: "buttonPrimaryIconHover", light: "#A85700", dark: "#A85700"),
        Token(name: "buttonPrimaryIconPressed", light: "#512900", dark: "#512900"),
        Token(name: "buttonPrimaryIconDisabled", light: "#783E00", dark: "#783E00"),
        Token(name: "buttonSecondaryTextDefault", light: "#3D3530", dark: "#D3C7BB"),
        Token(name: "buttonSecondaryTextHover", light: "#4C433C", dark: "#D9CFC4"),
        Token(name: "buttonSecondaryTextPressed", light: "#1D1916", dark: "#AEA399"),
        Token(name: "buttonSecondaryTextDisabled", light: "#72675F", dark: "#AEA399"),
        Token(name: "buttonSecondaryIconDefault", light: "#4C433C", dark: "#CDBFB1"),
        Token(name: "buttonSecondaryIconHover", light: "#61574F", dark: "#D3C7BB"),
        Token(name: "buttonSecondaryIconPressed", light: "#26211D", dark: "#988E86"),
        Token(name: "buttonSecondaryIconDisabled", light: "#72675F", dark: "#988E86"),
        Token(name: "buttonGhostIconDefault", light: "#72675F", dark: "#988E86"),
        Token(name: "buttonGhostIconHover", light: "#8A7F76", dark: "#AEA399"),
        Token(name: "buttonGhostIconPressed", light: "#26211D", dark: "#988E86"),
        Token(name: "buttonGhostIconDisabled", light: "#72675F", dark: "#988E86"),
        Token(name: "buttonFloatingDefault", light: "#FCFAF8B8", dark: "#2E2824B8"),
        Token(name: "buttonFloatingHover", light: "#FFFFFFB8", dark: "#3D3530B8"),
        Token(name: "buttonFloatingPressed", light: "#F9F7F5B8", dark: "#26211DB8"),
        Token(name: "buttonFloatingDisabled", light: "#F9F7F5B8", dark: "#26211DB8"),
        Token(name: "buttonFloatingTextDefault", light: "#3D3530", dark: "#D3C7BB"),
        Token(name: "buttonFloatingTextHover", light: "#4C433C", dark: "#D9CFC4"),
        Token(name: "buttonFloatingTextPressed", light: "#1D1916", dark: "#AEA399"),
        Token(name: "buttonFloatingTextDisabled", light: "#72675F", dark: "#AEA399"),
        Token(name: "buttonFloatingIconDefault", light: "#72675F", dark: "#988E86"),
        Token(name: "buttonFloatingIconHover", light: "#8A7F76", dark: "#AEA399"),
        Token(name: "buttonFloatingIconPressed", light: "#4C433C", dark: "#CDBFB1"),
        Token(name: "buttonFloatingIconDisabled", light: "#AEA399", dark: "#3D3530"),
        Token(name: "buttonPrimaryDestructiveDefault", light: "#CD3914", dark: "#CD3914"),
        Token(name: "buttonPrimaryDestructiveHover", light: "#E35531", dark: "#E35531"),
        Token(name: "buttonPrimaryDestructivePressed", light: "#B23010", dark: "#B23010"),
        Token(name: "buttonPrimaryDestructiveDisabled", light: "#CD3914", dark: "#CD3914"),
        Token(name: "buttonPrimaryDestructiveTextDefault", light: "#FDF3F1", dark: "#FDF3F1"),
        Token(name: "buttonPrimaryDestructiveTextHover", light: "#FFFFFF", dark: "#FFFFFF"),
        Token(name: "buttonPrimaryDestructiveTextPressed", light: "#F4D8D0", dark: "#F4D8D0"),
        Token(name: "buttonPrimaryDestructiveTextDisabled", light: "#FDF3F1", dark: "#FDF3F1"),
        Token(name: "buttonPrimaryDestructiveIconDefault", light: "#F4D8D0", dark: "#F4D8D0"),
        Token(name: "buttonPrimaryDestructiveIconHover", light: "#FDF3F1", dark: "#FDF3F1"),
        Token(name: "buttonPrimaryDestructiveIconPressed", light: "#EEAF9C", dark: "#EEAF9C"),
        Token(name: "buttonPrimaryDestructiveIconDisabled", light: "#FDF3F1", dark: "#FDF3F1"),
        Token(name: "controlOffDefault", light: "#EBE5DF", dark: "#2E2824"),
        Token(name: "controlOffHover", light: "#F1EBE5", dark: "#3D3530"),
        Token(name: "controlOffPressed", light: "#E6DED6", dark: "#26211D"),
        Token(name: "controlOnDefault", light: "#FFDA99", dark: "#512900"),
        Token(name: "controlOnHover", light: "#FFEECC", dark: "#783E00"),
        Token(name: "controlOnPressed", light: "#FFDA99", dark: "#512900"),
        Token(name: "textfieldDefault", light: "#F9F7F5", dark: "#1D1916"),
        Token(name: "textfieldHover", light: "#FCFAF8", dark: "#26211D"),
        Token(name: "textfieldFocus", light: "#FCFAF8", dark: "#26211D"),
        Token(name: "textfieldDisabled", light: "#F1EBE5", dark: "#0F0E0C"),
        Token(name: "checkboxDisabled", light: "#F1EBE5", dark: "#0F0E0C"),
        Token(name: "checkboxOnDefault", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "checkboxOnHover", light: "#FFAD33", dark: "#FFAD33"),
        Token(name: "checkboxOnPressed", light: "#E58200", dark: "#E58200"),
        Token(name: "checkboxOffDefault", light: "#F9F7F5", dark: "#1D1916"),
        Token(name: "checkboxOffHover", light: "#FCFAF8", dark: "#26211D"),
        Token(name: "checkboxOffPressed", light: "#F4F0EB", dark: "#0F0E0C"),
        Token(name: "radioDisabled", light: "#F1EBE5", dark: "#0F0E0C"),
        Token(name: "radioOnDefault", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "radioOnHover", light: "#FFAD33", dark: "#FFAD33"),
        Token(name: "radioOnPressed", light: "#E58200", dark: "#E58200"),
        Token(name: "radioOffDefault", light: "#F9F7F5", dark: "#1D1916"),
        Token(name: "radioOffHover", light: "#FCFAF8", dark: "#26211D"),
        Token(name: "radioOffPressed", light: "#F4F0EB", dark: "#0F0E0C"),
        Token(name: "toggleDisabled", light: "#DFD6CD", dark: "#2E2824"),
        Token(name: "toggleOffDefault", light: "#AEA399", dark: "#4C433C"),
        Token(name: "toggleOffHover", light: "#C5B6A5", dark: "#61574F"),
        Token(name: "toggleOffPressed", light: "#988E86", dark: "#3D3530"),
        Token(name: "toggleOnDefault", light: "#FF9D14", dark: "#FF9D14"),
        Token(name: "toggleOnHover", light: "#FFAD33", dark: "#FFAD33"),
        Token(name: "toggleOnPressed", light: "#E58200", dark: "#E58200"),
        Token(name: "notificationSuccessDefault", light: "#EEF7F0", dark: "#001F0E"),
        Token(name: "notificationSuccessBorderDefault", light: "#ACD8BC", dark: "#0F4327"),
        Token(name: "notificationSuccessTextDefault", light: "#29754A", dark: "#7EC89C"),
        Token(name: "notificationSuccessIconPrimary", light: "#328F58", dark: "#328F58"),
        Token(name: "notificationSuccessIconSecondary", light: "#4BAA6E", dark: "#29754A"),
        Token(name: "notificationWarningDefault", light: "#FEF5ED", dark: "#290F00"),
        Token(name: "notificationWarningBorderDefault", light: "#F8C69A", dark: "#5F2702"),
        Token(name: "notificationWarningTextDefault", light: "#BA5208", dark: "#F48732"),
        Token(name: "notificationWarningIconPrimary", light: "#EB7214", dark: "#EB7214"),
        Token(name: "notificationWarningIconSecondary", light: "#F48732", dark: "#BA5208"),
        Token(name: "notificationErrorDefault", light: "#FDF3F1", dark: "#290800"),
        Token(name: "notificationErrorBorderDefault", light: "#EEAF9C", dark: "#77210B"),
        Token(name: "notificationErrorTextDefault", light: "#B23010", dark: "#EA8864"),
        Token(name: "notificationErrorIconPrimary", light: "#CD3914", dark: "#CD3914"),
        Token(name: "notificationErrorIconSecondary", light: "#E35531", dark: "#B23010"),
        Token(name: "notificationInfoDefault", light: "#F0F6FA", dark: "#001829"),
        Token(name: "notificationInfoBorderDefault", light: "#AACADA", dark: "#07304B"),
        Token(name: "notificationInfoTextDefault", light: "#1D5E86", dark: "#7EAFC8"),
        Token(name: "notificationInfoIconPrimary", light: "#3982AC", dark: "#3982AC"),
        Token(name: "notificationInfoIconSecondary", light: "#5997BB", dark: "#1D5E86"),
        Token(name: "codeSurface", light: "#2E2824", dark: "#2E2824"),
        Token(name: "codePlainText", light: "#AEA399", dark: "#AEA399"),
        Token(name: "codeKeyword", light: "#EB7214", dark: "#EB7214"),
        Token(name: "codeString", light: "#FFAD33", dark: "#FFAD33"),
        Token(name: "codeFunction", light: "#5997BB", dark: "#5997BB"),
        Token(name: "codeType", light: "#4BAA6E", dark: "#4BAA6E"),
        Token(name: "codeNumber", light: "#4BAA6E", dark: "#4BAA6E"),
        Token(name: "codeDecorator", light: "#E6DED6", dark: "#E6DED6"),
        Token(name: "codeOperator", light: "#D3C7BB", dark: "#D3C7BB"),
        Token(name: "codePunctuation", light: "#D3C7BB", dark: "#D3C7BB"),
        Token(name: "codeComment", light: "#72675F", dark: "#72675F"),
    ]
}

/// Primitive typography scales (desktop values, points), keyed by step.
public enum GallopTypeScale {
    public static let sizes: [Int: CGFloat] = [0: 10, 1: 12, 2: 13, 3: 14, 4: 16, 5: 17, 6: 20, 7: 24, 8: 36, 9: 44, 10: 56]
    public static let lineHeights: [Int: CGFloat] = [0: 16, 1: 20, 2: 24, 3: 28, 4: 32, 5: 44, 6: 52, 7: 64]
}

/// The Gallop semantic type roles.
extension GallopTextStyle {
    /// Largest display heading. Hero sections and marketing callouts.
    public static let title = GallopTextStyle(name: "title", family: .display, weight: 720, size: 56, lineHeight: 64, letterSpacing: -0.01)
    /// Page-level heading. Primary content titles.
    public static let h1 = GallopTextStyle(name: "h1", family: .display, weight: 780, size: 44, lineHeight: 52, letterSpacing: -0.005)
    /// Section heading. Major content groupings.
    public static let h2 = GallopTextStyle(name: "h2", family: .display, weight: 780, size: 36, lineHeight: 44, letterSpacing: 0.005)
    /// Sub-section heading.
    public static let h3 = GallopTextStyle(name: "h3", family: .display, weight: 780, size: 24, lineHeight: 32, letterSpacing: 0.01)
    /// Minor heading. Card titles and grouped labels.
    public static let h4 = GallopTextStyle(name: "h4", family: .display, weight: 780, size: 20, lineHeight: 28, letterSpacing: 0.01)
    /// Smallest heading. Tight section labels.
    public static let h5 = GallopTextStyle(name: "h5", family: .sans, weight: 750, size: 17, lineHeight: 24, letterSpacing: 0.02)
    /// Large body. Default prose and descriptions.
    public static let bodyL = GallopTextStyle(name: "body-l", family: .sans, weight: 550, size: 16, lineHeight: 24, letterSpacing: 0.025)
    /// Large body, bold weight. Emphasis within body copy.
    public static let bodyLStrong = GallopTextStyle(name: "body-l-strong", family: .sans, weight: 750, size: 16, lineHeight: 24, letterSpacing: 0.025)
    /// Medium body. Secondary content and supporting text.
    public static let bodyM = GallopTextStyle(name: "body-m", family: .sans, weight: 550, size: 14, lineHeight: 20, letterSpacing: 0.03)
    /// Medium body, bold weight.
    public static let bodyMStrong = GallopTextStyle(name: "body-m-strong", family: .sans, weight: 750, size: 14, lineHeight: 20, letterSpacing: 0.03)
    /// Small body. Captions, helper text, and metadata.
    public static let bodyS = GallopTextStyle(name: "body-s", family: .sans, weight: 550, size: 13, lineHeight: 20, letterSpacing: 0.035)
    /// Small body, bold weight.
    public static let bodySStrong = GallopTextStyle(name: "body-s-strong", family: .sans, weight: 750, size: 13, lineHeight: 20, letterSpacing: 0.035)
    /// Default code. Inline code and code block content.
    public static let code = GallopTextStyle(name: "code", family: .mono, weight: 500, size: 14, lineHeight: 20, letterSpacing: -0.02)
    /// Small code. Syntax-highlighted labels and annotations.
    public static let codeSm = GallopTextStyle(name: "code-sm", family: .mono, weight: 500, size: 12, lineHeight: 20, letterSpacing: -0.015)
    /// Legal and terms text. Fine print.
    public static let terms = GallopTextStyle(name: "terms", family: .sans, weight: 550, size: 12, lineHeight: 16, letterSpacing: 0.02)
    /// Image captions and contextual labels.
    public static let caption = GallopTextStyle(name: "caption", family: .sans, weight: 550, size: 12, lineHeight: 16, letterSpacing: 0.04)
    /// Data visualisation labels and micro UI text.
    public static let dataLabel = GallopTextStyle(name: "data-label", family: .sans, weight: 550, size: 10, lineHeight: 16, letterSpacing: 0.05)

    /// Every type role, in generation order. Used by parity tests.
    public static let allRoles: [GallopTextStyle] = [
        .title,
        .h1,
        .h2,
        .h3,
        .h4,
        .h5,
        .bodyL,
        .bodyLStrong,
        .bodyM,
        .bodyMStrong,
        .bodyS,
        .bodySStrong,
        .code,
        .codeSm,
        .terms,
        .caption,
        .dataLabel,
    ]
}
