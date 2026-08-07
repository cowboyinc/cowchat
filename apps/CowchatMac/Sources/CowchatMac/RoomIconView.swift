import CoreGraphics
import SwiftUI

/// Draws a procedural room icon. `.initials` is not handled here — that style
/// keeps the plain lettered avatar in `RoomAvatar`.
struct RoomIconView: View {
    let name: String
    let style: RoomIconStyle
    let size: CGFloat

    var body: some View {
        let traits = RoomIconTraitsCache.traits(for: name)
        Canvas(rendersAsynchronously: false) { context, _ in
            let painter = RoomIconPainter(size: size)
            painter.clipToDisc(&context)
            switch style {
            case .initials:
                break
            case .hide:
                painter.drawHide(traits.hide, opacity: 1, into: &context)
            case .brand:
                painter.drawBrandGround(traits.brand, into: &context)
                painter.drawBrand(traits.brand, ink: painter.brandInk(traits.brand), into: &context)
            case .bandana:
                painter.drawBandana(traits.bandana, into: &context)
            case .brandOnHide:
                painter.drawHide(traits.combo, opacity: 0.28, into: &context)
                let hide = RoomIconPalette.hides[traits.combo.paletteIndex]
                painter.drawBrand(traits.brand, ink: hide.burnInk, into: &context)
            case .cowFace:
                painter.drawCowFace(traits.face, into: &context)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// Every coordinate below is in unit space (0…1 across the icon) so the same
/// numbers hold at 40pt in the sidebar and at 96pt in settings.
struct RoomIconPainter {
    let size: CGFloat

    func point(_ x: Double, _ y: Double) -> CGPoint {
        CGPoint(x: x * Double(size), y: y * Double(size))
    }

    func length(_ value: Double) -> CGFloat {
        value * size
    }

    func rect(_ x: Double, _ y: Double, _ width: Double, _ height: Double) -> CGRect {
        CGRect(x: length(x), y: length(y), width: length(width), height: length(height))
    }

    func clipToDisc(_ context: inout GraphicsContext) {
        let box = CGRect(origin: .zero, size: CGSize(width: size, height: size))
        context.clip(to: Path(ellipseIn: box.insetBy(dx: 0.5, dy: 0.5)))
    }

    // MARK: - Hide

    func drawHide(_ traits: HideTraits, opacity: Double, into context: inout GraphicsContext) {
        let palette = RoomIconPalette.hides[traits.paletteIndex]
        fillField(palette.ground, into: &context)
        for patch in traits.patches {
            context.fill(blobPath(patch), with: .color(palette.markings.opacity(opacity)))
        }
    }

    /// A closed quadratic curve through the midpoints of a jittered ring —
    /// irregular like a real marking, without the polygon corners a plain
    /// point-to-point path would leave.
    private func blobPath(_ patch: HideTraits.Patch) -> Path {
        let count = patch.wobble.count
        let ring = (0..<count).map { step -> CGPoint in
            let angle = Double(step) / Double(count) * 2 * .pi
            let radius = patch.radius * patch.wobble[step]
            return point(patch.x + cos(angle) * radius, patch.y + sin(angle) * radius)
        }
        func midpoint(_ a: CGPoint, _ b: CGPoint) -> CGPoint {
            CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
        }
        var path = Path()
        path.move(to: midpoint(ring[count - 1], ring[0]))
        for step in 0..<count {
            let next = ring[(step + 1) % count]
            path.addQuadCurve(to: midpoint(ring[step], next), control: ring[step])
        }
        path.closeSubpath()
        return path
    }

    // MARK: - Brand

    func brandInk(_ traits: BrandTraits) -> Color {
        RoomIconPalette.brands[traits.paletteIndex].ink
    }

    func drawBrandGround(_ traits: BrandTraits, into context: inout GraphicsContext) {
        fillField(RoomIconPalette.brands[traits.paletteIndex].ground, into: &context)
    }

    func drawBrand(_ traits: BrandTraits, ink: Color, into context: inout GraphicsContext) {
        var glyph = context
        if traits.stance != .upright {
            let centre = point(0.5, 0.5)
            glyph.translateBy(x: centre.x, y: centre.y)
            glyph.rotate(by: .degrees(traits.stance.degrees))
            glyph.translateBy(x: -centre.x, y: -centre.y)
        }
        let scale = traits.mark.count > 1 ? 0.36 : 0.50
        var mark = glyph.resolve(
            Text(traits.mark)
                .font(.system(size: length(scale), weight: .heavy, design: .serif))
        )
        mark.shading = .color(ink)
        glyph.draw(mark, at: point(0.5, 0.51), anchor: .center)

        // Modifiers stay unrotated: on a real brand the bar or rocker is read
        // against the ground, not against the letter.
        let stroke = StrokeStyle(lineWidth: length(0.06), lineCap: .round)
        switch traits.modifier {
        case .none:
            break
        case .barOver:
            context.fill(Path(rect(0.25, 0.13, 0.50, 0.065)), with: .color(ink))
        case .barUnder:
            context.fill(Path(rect(0.25, 0.805, 0.50, 0.065)), with: .color(ink))
        case .rocking:
            var path = Path()
            path.move(to: point(0.20, 0.78))
            path.addQuadCurve(to: point(0.80, 0.78), control: point(0.50, 0.96))
            context.stroke(path, with: .color(ink), style: stroke)
        case .flying:
            // Above the shoulders, not beside the letters — at avatar size a
            // wing level with the mark reads as part of the glyph.
            for side in [(0.10, 0.34), (0.66, 0.90)] {
                var path = Path()
                path.move(to: point(side.0, 0.30))
                path.addQuadCurve(
                    to: point(side.1, 0.30),
                    control: point((side.0 + side.1) / 2, 0.13)
                )
                context.stroke(path, with: .color(ink), style: stroke)
            }
        case .walking:
            context.fill(Path(rect(0.28, 0.74, 0.07, 0.17)), with: .color(ink))
            context.fill(Path(rect(0.65, 0.74, 0.07, 0.17)), with: .color(ink))
        case .circled:
            let inset = length(0.13)
            let box = CGRect(origin: .zero, size: CGSize(width: size, height: size))
            context.stroke(
                Path(ellipseIn: box.insetBy(dx: inset, dy: inset)),
                with: .color(ink),
                style: StrokeStyle(lineWidth: length(0.05))
            )
        }
    }

    // MARK: - Bandana

    func drawBandana(_ traits: BandanaTraits, into context: inout GraphicsContext) {
        let palette = RoomIconPalette.bandanas[traits.paletteIndex]
        fillField(palette.ground, into: &context)

        let motif = motifPath(traits)
        let centre = point(0.5, 0.5)
        for spoke in 0..<traits.spokes {
            var layer = context
            layer.translateBy(x: centre.x, y: centre.y)
            layer.rotate(by: .degrees(Double(spoke) * 360 / Double(traits.spokes)))
            layer.translateBy(x: -centre.x, y: -centre.y)
            layer.fill(motif, with: .color(palette.ink))
        }

        context.fill(
            Path(ellipseIn: CGRect(
                x: centre.x - length(traits.hub),
                y: centre.y - length(traits.hub),
                width: length(traits.hub * 2),
                height: length(traits.hub * 2)
            )),
            with: .color(palette.ink)
        )

        let box = CGRect(origin: .zero, size: CGSize(width: size, height: size))
        context.stroke(
            Path(ellipseIn: box.insetBy(dx: length(0.055), dy: length(0.055))),
            with: .color(palette.ink),
            style: StrokeStyle(lineWidth: length(0.028), dash: [length(traits.dash), length(0.05)])
        )
    }

    /// One motif, drawn pointing straight up from the centre; the spoke loop
    /// rotates copies of it. Symmetry is what makes these tell apart at 40pt.
    private func motifPath(_ traits: BandanaTraits) -> Path {
        var path = Path()
        let outer = 0.5 - traits.outer
        let inner = 0.5 - traits.inner
        switch traits.motif {
        case .dots:
            path.addEllipse(in: CGRect(
                x: length(0.5 - 0.055), y: length(outer - 0.055),
                width: length(0.11), height: length(0.11)
            ))
            path.addEllipse(in: CGRect(
                x: length(0.5 - 0.033), y: length(inner - 0.033),
                width: length(0.066), height: length(0.066)
            ))
        case .diamond:
            path.move(to: point(0.5, outer - 0.075))
            path.addLine(to: point(0.575, outer))
            path.addLine(to: point(0.5, outer + 0.075))
            path.addLine(to: point(0.425, outer))
            path.closeSubpath()
            path.addRect(rect(0.478, inner - 0.04, 0.044, 0.08))
        case .paisley:
            path.move(to: point(0.5, outer - 0.08))
            path.addQuadCurve(to: point(0.5, inner), control: point(0.60, outer))
            path.addQuadCurve(to: point(0.5, outer - 0.08), control: point(0.40, outer))
            path.closeSubpath()
        }
        return path
    }

    // MARK: - Cow face

    func drawCowFace(_ traits: CowFaceTraits, into context: inout GraphicsContext) {
        let palette = RoomIconPalette.hides[traits.paletteIndex]
        fillField(RoomIconPalette.faceField, into: &context)
        let outline = palette.isDarkGround ? palette.ink : palette.markings

        switch traits.horns {
        case .polled:
            break
        case .nubs:
            for x in [0.30, 0.70] {
                context.fill(
                    Path(ellipseIn: CGRect(
                        x: length(x - 0.06), y: length(0.19),
                        width: length(0.12), height: length(0.12)
                    )),
                    with: .color(outline)
                )
            }
        case .upright:
            drawMirrored(into: &context, color: outline) { path in
                path.move(to: self.point(0.29, 0.36))
                path.addQuadCurve(to: self.point(0.24, 0.13), control: self.point(0.16, 0.24))
            }
        case .longhorn:
            drawMirrored(into: &context, color: outline) { path in
                path.move(to: self.point(0.28, 0.35))
                path.addQuadCurve(to: self.point(0.05, 0.44), control: self.point(0.04, 0.28))
            }
        }

        // Wide ears are what keep a hornless head from reading as a pig.
        context.fill(rotatedEllipse(0.17, 0.47, 0.175, 0.09, -20), with: .color(palette.markings))
        context.fill(rotatedEllipse(0.83, 0.47, 0.175, 0.09, 20), with: .color(palette.markings))

        context.fill(
            Path(ellipseIn: CGRect(
                x: length(0.20), y: length(0.22),
                width: length(0.60), height: length(0.64)
            )),
            with: .color(palette.ground)
        )

        switch traits.eyePatch {
        case .none:
            break
        case .left, .right:
            let x = traits.eyePatch == .left ? 0.36 : 0.64
            context.fill(
                Path(ellipseIn: CGRect(
                    x: length(x - 0.14), y: length(0.34),
                    width: length(0.28), height: length(0.22)
                )),
                with: .color(palette.markings)
            )
        }

        if traits.forelock {
            var path = Path()
            path.move(to: point(0.34, 0.28))
            path.addQuadCurve(to: point(0.66, 0.28), control: point(0.50, 0.14))
            path.addQuadCurve(to: point(0.34, 0.28), control: point(0.50, 0.34))
            path.closeSubpath()
            context.fill(path, with: .color(palette.markings))
        }

        context.fill(
            Path(ellipseIn: CGRect(
                x: length(0.355), y: length(0.635),
                width: length(0.29), height: length(0.20)
            )),
            with: .color(palette.muzzle)
        )
        for x in [0.44, 0.56] {
            context.fill(
                Path(ellipseIn: CGRect(
                    x: length(x - 0.026), y: length(0.705),
                    width: length(0.052), height: length(0.07)
                )),
                with: .color(palette.markings)
            )
        }

        let eye = palette.isDarkGround ? palette.ink : Palette.bison800
        for x in [0.39, 0.61] {
            context.fill(
                Path(ellipseIn: CGRect(
                    x: length(x - 0.045), y: length(0.44),
                    width: length(0.09), height: length(0.09)
                )),
                with: .color(eye)
            )
        }

        if traits.earTag {
            let tag = Path(roundedRect: rect(0.755, 0.50, 0.12, 0.15), cornerRadius: length(0.03))
            context.fill(tag, with: .color(Palette.nugget500))
            context.stroke(tag, with: .color(Palette.nugget700), style: StrokeStyle(lineWidth: length(0.02)))
        }
    }

    private func drawMirrored(
        into context: inout GraphicsContext,
        color: Color,
        _ build: (inout Path) -> Void
    ) {
        var path = Path()
        build(&path)
        let style = StrokeStyle(lineWidth: length(0.055), lineCap: .round)
        context.stroke(path, with: .color(color), style: style)
        context.stroke(
            path.applying(
                CGAffineTransform(translationX: size, y: 0).scaledBy(x: -1, y: 1)
            ),
            with: .color(color),
            style: style
        )
    }

    private func rotatedEllipse(
        _ x: Double, _ y: Double,
        _ radiusX: Double, _ radiusY: Double,
        _ degrees: Double
    ) -> Path {
        let centre = point(x, y)
        let box = CGRect(
            x: centre.x - length(radiusX), y: centre.y - length(radiusY),
            width: length(radiusX * 2), height: length(radiusY * 2)
        )
        let transform = CGAffineTransform(translationX: centre.x, y: centre.y)
            .rotated(by: degrees * .pi / 180)
            .translatedBy(x: -centre.x, y: -centre.y)
        return Path(ellipseIn: box).applying(transform)
    }

    private func fillField(_ color: Color, into context: inout GraphicsContext) {
        context.fill(
            Path(CGRect(origin: .zero, size: CGSize(width: size, height: size))),
            with: .color(color)
        )
    }
}
