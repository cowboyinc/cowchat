import SwiftUI

/// Compact Slack hash mark, legible over every generated room avatar.
struct SlackRoomBadge: View {
    var body: some View {
        ZStack {
            Circle().fill(.white)
            SlackHash().stroke(Color(red: 0.29, green: 0.08, blue: 0.29), style: StrokeStyle(lineWidth: 1.8, lineCap: .round))
                .padding(4.5)
        }
        .frame(width: 19, height: 19)
        .overlay(Circle().stroke(.black.opacity(0.1), lineWidth: 0.5))
        .help("Connected to Slack")
        .accessibilityLabel("Connected to Slack")
    }
}

private struct SlackHash: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        for fraction in [0.32, 0.68] {
            let x = rect.minX + rect.width * fraction
            path.move(to: CGPoint(x: x + 0.6, y: rect.minY))
            path.addLine(to: CGPoint(x: x - 0.6, y: rect.maxY))
            let y = rect.minY + rect.height * fraction
            path.move(to: CGPoint(x: rect.minX, y: y + 0.6))
            path.addLine(to: CGPoint(x: rect.maxX, y: y - 0.6))
        }
        return path
    }
}
