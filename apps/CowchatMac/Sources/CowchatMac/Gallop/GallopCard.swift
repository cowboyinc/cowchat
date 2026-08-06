// Vendored from cowboyinc/macos SharedUI/GallopCard.swift — `import Gallop` dropped
// (SemanticColor is vendored locally); recipe otherwise verbatim. Diff against
// upstream before re-copying.

import SwiftUI

extension View {
    /// Gallop-styled raised card: `surface600` fill with a default border.
    func gallopCard(cornerRadius: CGFloat = 8) -> some View {
        padding(16)
            .background(
                SemanticColor.surface600,
                in: RoundedRectangle(cornerRadius: cornerRadius)
            )
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }
    }
}
