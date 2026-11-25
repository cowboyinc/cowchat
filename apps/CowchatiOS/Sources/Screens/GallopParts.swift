import SwiftUI
import UIKit
import CowchatKit

enum GallopChrome {
    /// Navigation titles take Season Mix, as the Mac's room header does.
    static func apply() {
        _ = SeasonFontProvider.fontsRegistered
        let provider = SeasonFontProvider()
        let text = UIColor(SemanticColor.textPrimary)
        let bar = UINavigationBar.appearance()
        bar.largeTitleTextAttributes = [.font: provider.nativeFont(for: .h2), .foregroundColor: text]
        bar.titleTextAttributes = [.font: provider.nativeFont(for: .h5), .foregroundColor: text]
    }
}

struct GallopButtonStyle: ButtonStyle {
    enum Kind { case primary, secondary, outline }

    var kind = Kind.primary
    var fillsWidth = true
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let fill: Color
        let text: Color
        switch kind {
        case .primary:
            fill = configuration.isPressed ? SemanticColor.buttonPrimaryPressed : SemanticColor.buttonPrimaryDefault
            text = SemanticColor.buttonPrimaryTextDefault
        case .secondary:
            fill = configuration.isPressed ? SemanticColor.buttonSecondaryPressed : SemanticColor.buttonSecondaryDefault
            text = SemanticColor.buttonSecondaryTextDefault
        case .outline:
            fill = configuration.isPressed ? SemanticColor.surface600 : SemanticColor.surface700
            text = SemanticColor.textPrimary
        }
        return configuration.label
            .gallopText(.bodyLStrong, color: text)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            .frame(maxWidth: fillsWidth ? .infinity : nil)
            .background(fill, in: Capsule())
            .overlay { if kind == .outline { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) } }
            .opacity(isEnabled ? 1 : 0.4)
    }
}

/// A text button for secondary routes, such as the sign-in screen's links.
struct GallopLinkButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .gallopText(.bodyS, color: SemanticColor.textTertiary)
            .opacity(configuration.isPressed ? 0.6 : 1)
            .frame(minHeight: 36)
            .contentShape(Rectangle())
    }
}

extension View {
    func gallopField() -> some View {
        gallopText(.bodyL, color: SemanticColor.textPrimary)
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(SemanticColor.textfieldDefault, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }
    }

    /// A grouped form drawn on Gallop surfaces instead of the system's.
    func gallopForm() -> some View {
        scrollContentBackground(.hidden)
            .background(SemanticColor.surface400.ignoresSafeArea())
    }

    func gallopFormRow() -> some View {
        listRowBackground(SemanticColor.surface600)
            .listRowSeparatorTint(SemanticColor.borderDefault)
    }
}

struct AgentAvatar: View {
    let name: String
    var size: CGFloat = 24

    var body: some View {
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(SemanticColor.surfaceGlassOnDefault)
            .overlay {
                Text(name.trimmingCharacters(in: .whitespaces).prefix(1).uppercased())
                    .font(.system(size: size * 0.42, weight: .bold, design: .rounded))
                    .foregroundStyle(SemanticColor.surfaceGlassOnTextDefault)
            }
            .overlay {
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .stroke(SemanticColor.borderDefault, lineWidth: 0.5)
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// The Mac sidebar's mark for where a room lives: a laptop for a server on the network, the Network glyph for Global.
struct ServerMark: View {
    let server: ServerChoice?

    var body: some View {
        Group {
            switch server {
            case .tcp:
                Image(systemName: "laptopcomputer")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(SemanticColor.iconTertiary)
            default:
                GallopIconView(icon: .network, fallbackSystemName: "globe", size: 16)
                    .foregroundStyle(Palette.nugget500)
            }
        }
        .frame(width: 20, height: 20)
        .accessibilityHidden(true)
    }
}

enum GallopTime {
    /// "now", "34m", "5h", then a date, as the Mac's sidebar and message rows show it.
    static func short(_ date: Date, now: Date = Date()) -> String {
        let seconds = now.timeIntervalSince(date)
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        if seconds < 86400 { return "\(Int(seconds / 3600))h" }
        return date.formatted(.dateTime.month(.abbreviated).day())
    }

    static func ago(_ date: Date, now: Date = Date()) -> String {
        let value = short(date, now: now)
        return now.timeIntervalSince(date) < 60 || now.timeIntervalSince(date) >= 86400 ? value : "\(value) ago"
    }
}
