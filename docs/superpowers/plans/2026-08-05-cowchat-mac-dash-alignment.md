# Cowchat Mac — Dash Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align `apps/CowchatMac` with the Cowboy Desktop design system: vendored Dash-correct tokens/fonts/icons, native window chrome, a simplified flat sidebar with working/unread states, and Open-in-Claude/Codex hover actions.

**Architecture:** A vendored `Gallop/` layer (tokens, typography, Season fonts, SVG icons — copied from `~/Github/macos`, the Dash-reconciled source of truth) replaces the hand-rolled `GallopTheme.swift`. UI work then proceeds in phases over `ContentView.swift` and `ChatStore.swift`: shell → sidebar → read/working state → chat pane → sweep. Spec: `docs/superpowers/specs/2026-08-05-cowchat-mac-dash-alignment-design.md`.

**Tech Stack:** Swift 5.9 SwiftPM executable (`apps/CowchatMac`, macOS 13+), SwiftUI + AppKit, XCTest. No new dependencies.

## Global Constraints

- Work on branch `ui-dash-alignment`. Every command below runs from `apps/CowchatMac/` unless the path is absolute.
- Token/typography values are copied **verbatim** from `~/Github/macos/Gallop/` — never regenerate from the gallop repo (`Dash outranks Gallop`; gallop `main` is stale).
- macOS deployment floor stays 13 (`Package.swift` `.macOS(.v13)`); do not use macOS-14-only API.
- Test after every task: `swift test` (expect 139+ tests green; 4 integration tests auto-skip without a server).
- Commit after every task with the repo's prefix style (`ui:`, `feat:`, `refactor:`, `test:`) and the trailer `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- Codex checkpoints close phases (Tasks 4, 7, 9, 13, 15): run the `auditcodex` skill on the phase's commits; triage findings with `superpowers:receiving-code-review` before continuing.
- The `.gallopText(...)` call-site API and `macAccessibleAction` usage must survive every task — they are load-bearing for accessibility tests.
- Never delete `Sources/CowchatMac/Resources/CowchatIcon.png` or touch DMG signing logic.

---

## Phase 1 — Vendored design foundation

### Task 1: Vendor Dash-reconciled tokens (`HexColor` + `GallopTokens`)

**Files:**
- Create: `Sources/CowchatMac/Gallop/HexColor.swift` (copy of `~/Github/macos/Gallop/HexColor.swift`)
- Create: `Sources/CowchatMac/Gallop/GallopTokens.swift` (copy of `~/Github/macos/Gallop/Generated/GallopTokens.swift`)
- Test: `Tests/CowchatMacTests/GallopTokensParityTests.swift`

**Interfaces:**
- Consumes: nothing (leaf task).
- Produces: `Palette.<ramp><shade>: Color` (e.g. `Palette.nugget500`), `SemanticColor.<token>: Color` (e.g. `SemanticColor.surface500`, `.textPrimary`, `.borderDefault`, `.buttonPrimaryDefault`, `.surfaceGlass500`), `GallopTextStyle` static roles (`.h4`, `.bodyL`, `.bodyS`, `.caption`, …), and `HexColor.color(_:)` / `HexColor.adaptive(light:dark:)`. Old `GallopTheme` continues to coexist until Task 4.

- [ ] **Step 1: Write the failing parity test**

```swift
// Tests/CowchatMacTests/GallopTokensParityTests.swift
import AppKit
import SwiftUI
import XCTest
@testable import CowchatMac

final class GallopTokensParityTests: XCTestCase {
    /// Resolves an adaptive SwiftUI Color to its sRGB hex under the given appearance.
    private func hex(_ color: Color, appearance: NSAppearance.Name) -> String {
        var resolved = ""
        NSAppearance(named: appearance)!.performAsCurrentDrawingAppearance {
            let ns = NSColor(color).usingColorSpace(.sRGB)!
            resolved = String(
                format: "#%02X%02X%02X%02X",
                Int(round(ns.redComponent * 255)),
                Int(round(ns.greenComponent * 255)),
                Int(round(ns.blueComponent * 255)),
                Int(round(ns.alphaComponent * 255))
            )
        }
        return resolved
    }

    private func assertToken(
        _ color: Color, light: String, dark: String,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(hex(color, appearance: .aqua), light, "light", file: file, line: line)
        XCTAssertEqual(hex(color, appearance: .darkAqua), dark, "dark", file: file, line: line)
    }

    /// Values pinned verbatim from ~/Github/macos Gallop/Generated/GallopTokens.swift
    /// (the Dash-reconciled set — see docs/dash-design-system.md in that repo).
    func testDashReconciledSpotValues() {
        assertToken(SemanticColor.surface500, light: "#F9F7F5FF", dark: "#1D1916FF")
        assertToken(SemanticColor.textPrimary, light: "#1D1916FF", dark: "#F4F0EBFF")
        assertToken(SemanticColor.borderDefault, light: "#D9CFC4FF", dark: "#3D3530FF")
        assertToken(SemanticColor.buttonPrimaryDefault, light: "#FF9D14FF", dark: "#FF9D14FF")
        // The Dash-reconciled glass value the old bridge got wrong (was #FFFFFFCC):
        assertToken(SemanticColor.surfaceGlass500, light: "#FCFAF8B8", dark: "#2E2824CC")
        assertToken(Palette.hay50, light: "#FCFAF8FF", dark: "#FCFAF8FF")
        assertToken(Palette.nugget500, light: "#FF9D14FF", dark: "#FF9D14FF")
        assertToken(Palette.cactus500, light: hex(Palette.cactus500, appearance: .aqua),
                    dark: hex(Palette.cactus500, appearance: .darkAqua)) // existence check
    }

    func testTypeRoleTableMatchesGallop() {
        XCTAssertEqual(GallopTextStyle.h4.size, 20)
        XCTAssertEqual(GallopTextStyle.h4.family, .display)
        XCTAssertEqual(GallopTextStyle.bodyL.weight, 550)
        XCTAssertEqual(GallopTextStyle.bodyS.size, 13)
        XCTAssertEqual(GallopTextStyle.caption.size, 12)
    }
}
```

Note: before finalizing this test, open the two vendored files and confirm the exact hex for `surfaceGlass500` (`docs/dash-design-system.md` in the macos repo says `#FCFAF8` @ 72% = `B8`) and one `lantern`/`cactus` shade; replace the `cactus500` existence-check line with a literal assert of the real values you read.

- [ ] **Step 2: Run test to verify it fails**

Run: `swift test --filter GallopTokensParityTests 2>&1 | tail -5`
Expected: compile FAILURE — `cannot find 'SemanticColor' in scope`.

- [ ] **Step 3: Copy the vendored files verbatim**

```bash
mkdir -p Sources/CowchatMac/Gallop
cp ~/Github/macos/Gallop/HexColor.swift Sources/CowchatMac/Gallop/HexColor.swift
cp ~/Github/macos/Gallop/Generated/GallopTokens.swift Sources/CowchatMac/Gallop/GallopTokens.swift
```

Then edit only the header comment of the copied `GallopTokens.swift`: replace the first line with `// Vendored from cowboyinc/macos Gallop/Generated/GallopTokens.swift — do not edit; re-copy to sync.` and KEEP the existing WARNING paragraph about never regenerating from gallop main. If `GallopTokens.swift` references types beyond `HexColor`/CoreGraphics/SwiftUI (check imports and compile errors), copy the missing sibling file from `~/Github/macos/Gallop/` the same way rather than stubbing. `GallopTextStyle` is defined in `~/Github/macos/Gallop/Typography.swift` — if the tokens file's role statics fail to compile because the struct lives there, move ONLY the `GallopTextStyle` struct definition forward into this task (copy `Typography.swift` → `Sources/CowchatMac/Gallop/GallopTypography.swift` now; Task 2 then skips that copy step).

- [ ] **Step 4: Run tests to verify they pass**

Run: `swift test --filter GallopTokensParityTests 2>&1 | tail -5` then full `swift test 2>&1 | tail -3`
Expected: parity tests PASS; full suite still green (old `GallopTheme` untouched).

- [ ] **Step 5: Commit**

```bash
git add Sources/CowchatMac/Gallop Tests/CowchatMacTests/GallopTokensParityTests.swift
git commit -m "feat: vendor Dash-reconciled Gallop tokens" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

### Task 2: Vendor typography + bundle Season fonts

**Files:**
- Create: `Sources/CowchatMac/Gallop/GallopTypography.swift` (copy of `~/Github/macos/Gallop/Typography.swift`, if not already copied in Task 1 Step 3)
- Create: `Sources/CowchatMac/Gallop/SeasonFonts.swift` (adapted copy of `~/Github/macos/Gallop/SeasonFonts.swift`)
- Create: `Sources/CowchatMac/Resources/Fonts/SeasonSansUprightsVF.ttf`, `Sources/CowchatMac/Resources/Fonts/SeasonMixUprightsVF.ttf` (binary copies)
- Modify: `Package.swift` (resources list)
- Test: `Tests/CowchatMacTests/SeasonFontsTests.swift`

**Interfaces:**
- Consumes: `GallopTextStyle` (Task 1), `HexColor` module context.
- Produces: `SeasonFontProvider` (`.font(for:)`, `nativeFont` via protocol, `SeasonFontProvider.fontsRegistered: Bool`, `SeasonFontProvider.resourceBundle(searching:)`, `resourceBundleCandidates`), `SystemFontProvider`, and the vendored `.gallopText(_ style: GallopTextStyle)` modifier. Resource bundle name constant: `"CowchatMac_CowchatMac.bundle"`.

- [ ] **Step 1: Write the failing tests**

```swift
// Tests/CowchatMacTests/SeasonFontsTests.swift
import AppKit
import XCTest
@testable import CowchatMac

final class SeasonFontsTests: XCTestCase {
    /// POSITIVE PATH (Codex review requirement): under `swift test` the bundle
    /// must actually resolve and the Season families must register — a broken
    /// bundle must fail tests, not silently fall back to system fonts.
    func testResourceBundleResolvesUnderSwiftTest() {
        XCTAssertNotNil(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates),
            "CowchatMac_CowchatMac.bundle not found — check Package.swift .copy entries and candidates"
        )
    }

    func testSeasonFontsRegisterAndResolve() {
        XCTAssertTrue(SeasonFontProvider.fontsRegistered)
        let font = SeasonFontProvider.variableFont(family: "Season Sans VF", size: 16, weight: 550)
        XCTAssertEqual(font?.familyName, "Season Sans VF")
        let display = SeasonFontProvider.variableFont(family: "Season Mix VF", size: 20, weight: 780)
        XCTAssertEqual(display?.familyName, "Season Mix VF")
    }

    /// Fallback stays crash-free when the bundle is absent (COW-2689 lesson).
    func testMissingBundleFallsBackWithoutTrapping() {
        XCTAssertNil(SeasonFontProvider.resourceBundle(searching: [URL(fileURLWithPath: "/nonexistent")]))
        let style = GallopTextStyle.bodyM
        _ = SystemFontProvider().font(for: style) // must not trap
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `swift test --filter SeasonFontsTests 2>&1 | tail -5`
Expected: compile FAILURE — `cannot find 'SeasonFontProvider'`.

- [ ] **Step 3: Copy files and adapt**

```bash
cp ~/Github/macos/Gallop/Typography.swift Sources/CowchatMac/Gallop/GallopTypography.swift   # skip if done in Task 1
cp ~/Github/macos/Gallop/SeasonFonts.swift Sources/CowchatMac/Gallop/SeasonFonts.swift
mkdir -p Sources/CowchatMac/Resources/Fonts
cp ~/Github/macos/Gallop/Fonts/SeasonSansUprightsVF.ttf Sources/CowchatMac/Resources/Fonts/
cp ~/Github/macos/Gallop/Fonts/SeasonMixUprightsVF.ttf Sources/CowchatMac/Resources/Fonts/
```

In the copied `SeasonFonts.swift` make exactly these adaptations:
1. `static let resourceBundleName = "CowboyDesktopNative_Gallop.bundle"` → `"CowchatMac_CowchatMac.bundle"`.
2. Keep the `Bundle(for: GallopBundleFinder.self)` candidate probing **as-is** — it is what makes `swift test` resolve the bundle (Codex review finding; do not simplify to `Bundle.main` only). Add `Bundle.module.bundleURL` as an additional first candidate wrapped in `#if DEBUG` only if the candidates fail under `swift test` — try without it first.
3. Font subpath stays `Fonts/<name>.ttf` (that is where the `.copy` resource lands them).

In `Package.swift`, change the target's resources:

```swift
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
        .copy("Resources/Fonts"),
    ]
),
```

Build once (`swift build 2>&1 | tail -5`); if SwiftPM reports overlapping resources, apply the NOTE above (move dirs to `Sources/CowchatMac/Bundled/`, update both `cp` destinations and the `.copy` paths). In the copied `GallopTypography.swift`, find how the `gallopText` modifier resolves its font provider; if it uses a static/default provider, ensure the default is `SeasonFontProvider()` (which itself falls back to `SystemFontProvider` internally). If both this file and old `GallopTheme.swift` define a member named `gallopText` with an identical signature, rename NOTHING in the old file — the signatures differ (`GallopTextStyle` vs `GallopTheme.TypeRole`), so both overloads coexist until Task 4.

- [ ] **Step 4: Run tests**

Run: `swift test --filter SeasonFontsTests 2>&1 | tail -5`, then full `swift test 2>&1 | tail -3`
Expected: all PASS, including the positive-path bundle test.

- [ ] **Step 5: Commit**

```bash
git add -A Sources/CowchatMac Package.swift Tests/CowchatMacTests/SeasonFontsTests.swift
git commit -m "feat: vendor Gallop typography and bundle Season fonts" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

### Task 3: Vendor GallopIcon + SVG assets

**Files:**
- Create: `Sources/CowchatMac/Gallop/GallopIcon.swift` (adapted copy of `~/Github/macos/Gallop/GallopIcon.swift`)
- Create: `Sources/CowchatMac/Resources/Icons/svg/*.svg` (21 files, sources below)
- Modify: `Package.swift` (add `.copy("Resources/Icons")` beside the Fonts entry, same relocation rule as Task 2)
- Test: `Tests/CowchatMacTests/GallopIconTests.swift`

**Interfaces:**
- Consumes: `SeasonFontProvider.resourceBundle(searching:)` (Task 2).
- Produces: `GallopIcon` enum with `.image: Image?` — cases used by later tasks: `.add, .arrowUpRight, .chevronDown, .chevronRightSmall, .chevronDownExtraSmall, .chevronRightExtraSmall, .chevronUpExtraSmall, .copy, .dismiss, .edit, .ellipsis, .folder, .lock, .message, .search, .send, .settings, .sidebar, .thinking, .trash, .warning`. Plus helper view `GallopIconView(icon:fallbackSystemName:size:)`.

- [ ] **Step 1: Write the failing test**

```swift
// Tests/CowchatMacTests/GallopIconTests.swift
import XCTest
@testable import CowchatMac

final class GallopIconTests: XCTestCase {
    /// Every declared icon must have a decodable SVG in the bundle — parity
    /// in BOTH directions is checked so a stray/missing file fails loudly.
    func testEveryIconResolvesFromBundle() {
        for icon in GallopIcon.allCases {
            XCTAssertNotNil(icon.image, "Missing or undecodable SVG for \(icon.rawValue)")
        }
    }

    func testBundleHasNoOrphanSVGs() throws {
        let bundle = try XCTUnwrap(
            SeasonFontProvider.resourceBundle(searching: SeasonFontProvider.resourceBundleCandidates)
        )
        let urls = bundle.urls(forResourcesWithExtension: "svg", subdirectory: "Icons/svg") ?? []
        let onDisk = Set(urls.map { $0.deletingPathExtension().lastPathComponent })
        XCTAssertEqual(onDisk, Set(GallopIcon.allCases.map(\.rawValue)))
    }
}
```

- [ ] **Step 2: Run to verify failure** — `swift test --filter GallopIconTests 2>&1 | tail -5` → compile FAILURE (`GallopIcon` not found).

- [ ] **Step 3: Copy assets and write the adapted enum**

```bash
mkdir -p Sources/CowchatMac/Resources/Icons/svg
cd Sources/CowchatMac/Resources/Icons/svg
for f in add arrow-up-right chevron-down chevron-right-small chevron-down-extra-small \
         chevron-right-extra-small chevron-up-extra-small copy edit folder lock message \
         search send settings sidebar thinking warning; do
  cp ~/Github/macos/Gallop/Icons/svg/$f.svg .
done
# Three icons the mac port lacks come from the gallop web set (rename to kebab-case):
cp ~/Github/gallop/packages/foundations/src/icons/svg/Ellipsis.svg ellipsis.svg
cp ~/Github/gallop/packages/foundations/src/icons/svg/Trash.svg trash.svg
cp ~/Github/gallop/packages/foundations/src/icons/svg/Dismiss.svg dismiss.svg
cd - >/dev/null
```

Copy `~/Github/macos/Gallop/GallopIcon.swift` to `Sources/CowchatMac/Gallop/GallopIcon.swift`, then: (a) replace the enum's case list with exactly the 21 cases above (kebab-case raw values where the name has hyphens, e.g. `case arrowUpRight = "arrow-up-right"`, `case chevronRightSmall = "chevron-right-small"`); (b) keep `OriginalNativeImageCache`, template loading, and nil-on-miss behavior verbatim; (c) the `resourceBundle` already comes from `SeasonFontProvider` — keep that. Delete cases cowchat doesn't ship (logo, gas, network, etc.). Add the small helper view at the bottom of the same file (pattern from the cowboy app's `ConversationDashIcon`):

```swift
/// Renders a Gallop icon at a fixed size, falling back to an SF Symbol when
/// the resource bundle is missing.
struct GallopIconView: View {
    let icon: GallopIcon
    let fallbackSystemName: String
    var size: CGFloat = 16

    var body: some View {
        if let image = icon.image {
            image.resizable().scaledToFit().frame(width: size, height: size)
        } else {
            Image(systemName: fallbackSystemName)
                .font(.system(size: size * 0.82, weight: .medium))
                .frame(width: size, height: size)
        }
    }
}
```

Add `.copy("Resources/Icons")` to `Package.swift` (or `Bundled/Icons` if relocated in Task 2). Check the three web-set SVGs render as template images: `Ellipsis.svg`/`Trash.svg`/`Dismiss.svg` use `fill="currentColor"` or plain paths — if any renders empty via `NSImage`, open the SVG and replace `currentColor` with `#000000` (template mode re-tints it anyway).

- [ ] **Step 4: Run tests** — `swift test --filter GallopIconTests 2>&1 | tail -5` then full suite. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A Sources/CowchatMac Package.swift Tests/CowchatMacTests/GallopIconTests.swift
git commit -m "feat: vendor Gallop icon set with SVG assets" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

### Task 4: Cut all call sites over to SemanticColor; delete GallopTheme

**Files:**
- Create: `Sources/CowchatMac/Gallop/GallopCompat.swift`
- Modify: every file using `GallopColor`/`gallopText` — `ContentView.swift` (incl. line 4 typealias), `CowchatMacApp.swift:137`, `CowchatOnboarding.swift`, `MessagePreview.swift` (check), any other hit of `grep -rl "GallopColor\|GallopTheme" Sources/`
- Delete: `Sources/CowchatMac/GallopTheme.swift`, `Tests/CowchatMacTests/GallopThemeTests.swift`
- Test: existing suite (parity now covered by Task 1 tests)

**Interfaces:**
- Consumes: everything from Tasks 1–2.
- Produces: `typealias GallopColor = SemanticColor` semantics via direct use — after this task, color call sites read `SemanticColor.surface500` (a `Color`, **no `.color` suffix**), and text call sites read `.gallopText(.bodyM, color: SemanticColor.textTertiary)`. `GallopCompat.swift` provides: the color-parameter overload of `gallopText`, and any type-role statics missing from the vendored table (`dataLabel` if absent).

- [ ] **Step 1: Add the compat layer**

```swift
// Sources/CowchatMac/Gallop/GallopCompat.swift
import SwiftUI

extension View {
    /// Cowchat's historical call-site shape: role + optional semantic color.
    /// Routes through the vendored Gallop modifier for font metrics.
    func gallopText(_ style: GallopTextStyle, color: Color?) -> some View {
        Group {
            if let color {
                gallopText(style).foregroundStyle(color)
            } else {
                gallopText(style)
            }
        }
    }
}
```

Then check the vendored role table: `grep "static let" Sources/CowchatMac/Gallop/GallopTypography.swift Sources/CowchatMac/Gallop/GallopTokens.swift | grep GallopTextStyle`. Cowchat needs all of: `h4, h5, bodyL, bodyLStrong, bodyM, bodyMStrong, bodyS, bodySStrong, caption, dataLabel, code`. For each missing one, add to `GallopCompat.swift` with cowchat's current values from the old `GallopTheme.TypeRole.style` table, e.g. if `dataLabel` is missing:

```swift
extension GallopTextStyle {
    /// Cowchat-local role, not in the upstream table (values from the old bridge).
    static let dataLabel = GallopTextStyle(
        name: "data-label", family: .sans, weight: 550, size: 10, lineHeight: 16, letterSpacing: 0.05
    )
}
```

- [ ] **Step 2: Mechanical sweep**

```bash
cd Sources/CowchatMac
# 1) GallopColor.x.color  ->  SemanticColor.x
perl -pi -e 's/GallopColor\.([A-Za-z0-9]+)\.color/SemanticColor.\1/g' *.swift
# 2) gallopText color shorthand: color: .foo -> color: SemanticColor.foo
perl -pi -e 's/color: \.([a-zA-Z][A-Za-z0-9]*)\b/color: SemanticColor.\1/g' *.swift
# 3) App tint
perl -pi -e 's/GallopTheme\.ColorToken\.([A-Za-z0-9]+)\.color/SemanticColor.\1/g' *.swift
cd - >/dev/null
```

Then delete line 4 of `ContentView.swift` (`private typealias GallopColor = GallopTheme.ColorToken`), delete `Sources/CowchatMac/GallopTheme.swift` and `Tests/CowchatMacTests/GallopThemeTests.swift` (`git rm`). Build; chase residual errors by hand — expected stragglers: `GallopTheme.Appearance` references (none exist today outside the theme file, but verify with `grep -rn "GallopTheme" Sources/ Tests/`), the sed regex catching a non-gallopText `color: .black`-style argument (revert those specific lines — `git diff` review before committing is the step's safety net), and `MessagePreviewTests`/`RoomSidebarPresentationTests` referencing theme types (they don't today; confirm).

- [ ] **Step 3: Review the diff hunk-by-hunk**

Run: `git diff --stat && git diff | head -400`
Verify: only mechanical substitutions; no behavior edits. Any `color: SemanticColor.black`-style nonsense means the regex over-matched — fix by hand.

- [ ] **Step 4: Full test run** — `swift test 2>&1 | tail -3` → green (GallopThemeTests removed; parity lives in GallopTokensParityTests).

- [ ] **Step 5: Commit + Codex checkpoint (Phase 1)**

```bash
git add -A && git commit -m "refactor: replace hand-rolled GallopTheme with vendored Gallop layer" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

Invoke the `auditcodex` skill scoped to Phase 1 (Tasks 1–4 commits). Triage per `superpowers:receiving-code-review`; land fixes as `fix:` commits before Phase 2.

---

## Phase 2 — Shell and sidebar

### Task 5: Native window chrome + toolbar

**Files:**
- Modify: `Sources/CowchatMac/CowchatMacApp.swift` (scene modifiers + commands)
- Modify: `Sources/CowchatMac/ContentView.swift` (`ContentView.body`, `SidebarView.body`/`sidebarChrome`, `ChatRoomView.chatHeader`, `LobbyDashboardView`/`RoomSetupView`/`EmptyChatView` show-sidebar affordances)

**Interfaces:**
- Consumes: `SemanticColor`, `GallopIcon`, `GallopIconView` (Tasks 1–4).
- Produces: toolbar-owned sidebar toggle and New Room button; `ContentView` keeps `@State isSidebarVisible` and passes the same bindings as today (child view signatures unchanged).

- [ ] **Step 1: Scene changes in `CowchatMacApp.swift`**

Remove `.windowStyle(.hiddenTitleBar)` (line 145). After `.defaultSize(width: 1080, height: 740)` nothing else changes at scene level. In `.commands`, add the sidebar command after the existing New Room group:

```swift
CommandGroup(after: .sidebar) {
    Button("Toggle Sidebar") {
        NotificationCenter.default.post(name: .cowchatToggleSidebar, object: nil)
    }
    .keyboardShortcut("s", modifiers: [.control, .command])
}
```

And at file scope (bottom of `CowchatMacApp.swift`):

```swift
extension Notification.Name {
    /// ⌃⌘S — macOS reserves this for sidebar toggling; nothing binds it once
    /// the shell owns its own fixed column (cowboy-app pattern).
    static let cowchatToggleSidebar = Notification.Name("CowchatMac.toggleSidebar")
}
```

- [ ] **Step 2: ContentView shell + toolbar**

In `ContentView.body` (currently lines 41–120): delete the `.clipShape(RoundedRectangle(cornerRadius: 22...))`, the matching `.overlay { ...stroke... }`, and `.ignoresSafeArea(.container, edges: .top)`. Delete the divider `Rectangle().fill(...borderDefault...).frame(width: 1)` between the panes. Change the sidebar frame to `.frame(width: 280)`. Add, after `.background(SemanticColor.surface500)`:

```swift
.navigationTitle(store.selectedRoom?.name ?? "Cowchat")
.toolbar {
    ToolbarItem(placement: .navigation) {
        Button {
            withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
        } label: {
            GallopIconView(icon: .sidebar, fallbackSystemName: "sidebar.left", size: 17)
                .foregroundStyle(SemanticColor.iconTertiary)
                .frame(width: 36, height: 36)
                .background(Circle().fill(SemanticColor.surface700))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .focusable(false)
        .help(isSidebarVisible ? "Hide sidebar" : "Show sidebar")
        .macAccessibleAction(label: "Toggle sidebar") {
            withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
        }
    }
    ToolbarItem(placement: .primaryAction) {
        Button {
            store.presentCreateRoom()
        } label: {
            GallopIconView(icon: .edit, fallbackSystemName: "square.and.pencil", size: 17)
                .foregroundStyle(SemanticColor.iconSecondary)
                .frame(width: 36, height: 36)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .help("New room (⌘N)")
        .macAccessibleAction(label: "Create room") { store.presentCreateRoom() }
    }
}
.onReceive(NotificationCenter.default.publisher(for: .cowchatToggleSidebar)) { _ in
    withAnimation(.easeInOut(duration: 0.2)) { isSidebarVisible.toggle() }
}
```

- [ ] **Step 3: Remove superseded chrome**

In `SidebarView`: delete the whole `sidebarChrome` computed property and its call in `body` (the hide-sidebar + compose circle buttons — the toolbar owns both now). Replace the sidebar background `GallopColor.surfaceGlass500`→ already swept; set it to `SemanticColor.surface500`. In `ChatRoomView.chatHeader` (and the equivalents in `LobbyDashboardView`, `RoomSetupView`, `EmptyChatView`): remove the `if !isSidebarVisible { CircleIconButton(systemName: "rectangle.split.1x2"...) }` blocks (toolbar toggle is always visible) — keep the `isSidebarVisible` binding parameter on these views for now to minimize churn; remove `.padding(.leading, isSidebarVisible ? 18 : 104)`-style conditionals by using the constant smaller value (`18`).

- [ ] **Step 4: Build, test, eyeball**

Run: `swift test 2>&1 | tail -3` (green), then `swift run CowchatMac` briefly: standard titlebar visible, toolbar has sidebar toggle + compose, ⌃⌘S toggles, panes flush with no divider. Quit with ⌘Q.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "ui: adopt native window chrome with toolbar controls" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

### Task 6: Remove scope switcher and pinned-rooms concept

**Files:**
- Modify: `Sources/CowchatMac/ContentView.swift` (delete `SidebarScope` enum, `scopePicker`, `activeCount`, `pinnedRooms` view; simplify `baseRooms`/`archivedRooms`; `roomContextMenu`, `DashboardRoomCard` menu, `ChatRoomView` header menu lose their Pin items)
- Modify: `Sources/CowchatMac/RoomSidebarPresentation.swift` (delete `activeRooms`, `pinnedRooms`, `visiblePinnedRooms`, `roomsForRecencyGroups`, `LobbyPresentation` stays)
- Modify: `Sources/CowchatMac/ChatStore.swift` (delete `pinnedRoomIDs` published var + `isPinned`/`togglePinned` + every `localPreferences.savePinnedRoomIDs`/auto-pin-lobby site — lines ~170, 533, 716–738, 1262–1265, 1291–1292, 1300, 1317 today)
- Modify: `Sources/CowchatMac/RoomLocalPreferences.swift` (delete pinned keys/accessors/save)
- Modify: `Tests/CowchatMacTests/RoomSidebarPresentationTests.swift`, `Tests/CowchatMacTests/RoomLocalPreferencesTests.swift` (delete the corresponding cases)

**Interfaces:**
- Consumes: current sidebar structure (Task 5 state).
- Produces: `SidebarView.baseRooms` = `RoomSidebarPresentation.filteredRooms(from: store.unarchivedRooms, query:, matchingMessageRoomIDs:)` only. No pin API anywhere. Stored `CowchatMac.pinnedRoomIDs*` defaults are orphaned deliberately.

- [ ] **Step 1: Delete tests first** — remove every test referencing `activeRooms`, `pinnedRooms`, `visiblePinnedRooms`, `roomsForRecencyGroups`, pinned prefs. Run `swift test 2>&1 | tail -3` — still green (tests deleted, code unused-but-present).
- [ ] **Step 2: Delete the UI + store + prefs code** listed above. `baseRooms` becomes:

```swift
private var baseRooms: [Room] {
    RoomSidebarPresentation.filteredRooms(
        from: store.unarchivedRooms,
        query: store.searchText,
        matchingMessageRoomIDs: store.messageSearchRoomIDs
    )
}
```

and `archivedRooms(at:)` loses its `scope` branch the same way. `SidebarView.body` drops `scopePicker` + `pinnedRooms` rows. Context menus keep Rename/Archive only (chat header menu handled fully in Task 12 — here just delete its Pin button, lines ~1079–1081).

- [ ] **Step 3: Grep-verify zero references** — `grep -rn "pinned\|Pinned\|SidebarScope\|activeRooms" Sources/ Tests/` → no hits (except deliberate comment in RoomLocalPreferences explaining orphaned defaults, if you add one).
- [ ] **Step 4: Full test run** — green.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "ui: remove sidebar scope switcher and pinned rooms" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 7: Flat recency list + Dash row treatment

**Files:**
- Modify: `Sources/CowchatMac/RoomSidebarPresentation.swift` (replace `groups`/`groupTitle`/`RoomSidebarGroup` with `sortedByRecency`)
- Modify: `Sources/CowchatMac/ContentView.swift` (`SidebarView` list body; `RoomRow` restyle + hover; new `SidebarRowState`/`SidebarRowBackground`; footer icon buttons)
- Test: `Tests/CowchatMacTests/RoomSidebarPresentationTests.swift`

**Interfaces:**
- Consumes: `baseRooms` (Task 6), `SemanticColor`, `GallopIcon`.
- Produces: `RoomSidebarPresentation.sortedByRecency(_ rooms: [Room]) -> [Room]`; `SidebarRowState` (`normal/selected/hover`, `init(isSelected:isHovering:)`); `SidebarRowBackground(state:)` view; `RoomRow(room:messagePreview:isSelected:now:)` unchanged signature (unread/working params arrive in Tasks 8–9).

- [ ] **Step 1: Write failing sort tests**

```swift
// In RoomSidebarPresentationTests.swift
func testSortedByRecencyKeepsLobbyFirstThenRecency() {
    let lobby = makeRoom(name: "Lobby", lastActivity: "2026-08-01T00:00:00Z")
    let older = makeRoom(name: "alpha", lastActivity: "2026-08-03T00:00:00Z")
    let newer = makeRoom(name: "zulu", lastActivity: "2026-08-04T00:00:00Z")
    let sorted = RoomSidebarPresentation.sortedByRecency([older, lobby, newer])
    XCTAssertEqual(sorted.map(\.name), ["Lobby", "zulu", "alpha"])
}

func testSortedByRecencyTiebreaksOnName() {
    let a = makeRoom(name: "beta", lastActivity: "2026-08-04T00:00:00Z")
    let b = makeRoom(name: "Alpha", lastActivity: "2026-08-04T00:00:00Z")
    XCTAssertEqual(RoomSidebarPresentation.sortedByRecency([a, b]).map(\.name), ["Alpha", "beta"])
}
```

(Reuse the file's existing `makeRoom` helper; if it lacks a `lastActivity` parameter, extend it with a defaulted one.) Run — FAIL (`sortedByRecency` undefined).

- [ ] **Step 2: Implement + swap in the view**

In `RoomSidebarPresentation.swift`, delete `RoomSidebarGroup`, `groups(from:now:calendar:)`, `groupTitle`, and add:

```swift
/// Flat iMessage-style ordering. Lobby stays first — the existing roomSort
/// invariant (ChatStore.roomSort): it is the home surface, not a "pin".
static func sortedByRecency(_ rooms: [Room]) -> [Room] {
    rooms.sorted { lhs, rhs in
        let lhsIsLobby = lhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        let rhsIsLobby = rhs.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame
        if lhsIsLobby != rhsIsLobby { return lhsIsLobby }
        let l = lhs.activityDate ?? .distantPast
        let r = rhs.activityDate ?? .distantPast
        if l != r { return l > r }
        return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
    }
}
```

In `SidebarView.body`, replace the `pinnedRooms` + `ForEach(visibleGroups...)` block with a single flat `ForEach(RoomSidebarPresentation.sortedByRecency(baseRooms))` rendering the same `Button { RoomRow(...) }` rows (delete `visibleGroups(at:)` and `roomGroup(_:now:)`).

- [ ] **Step 3: Row + footer restyle**

Add next to `RoomRow`:

```swift
enum SidebarRowState {
    case normal, selected, hover
    init(isSelected: Bool, isHovering: Bool) {
        self = isSelected ? .selected : (isHovering ? .hover : .normal)
    }
}

/// Cowboy SidebarRowPill recipe at cowchat's row geometry (radius 12 — the
/// 100pt pill reads as a blob on two-line 54pt rows; divergence logged in spec).
struct SidebarRowBackground: View {
    let state: SidebarRowState
    private var shape: RoundedRectangle { RoundedRectangle(cornerRadius: 12, style: .continuous) }

    var body: some View {
        switch state {
        case .normal:
            shape.fill(Color.clear)
        case .selected:
            shape.fill(SemanticColor.surface400)
        case .hover:
            shape.fill(SemanticColor.surface600)
                .overlay(shape.strokeBorder(Color.black.opacity(0.08), lineWidth: 0.5))
                .shadow(color: Color.black.opacity(0.04), radius: 1.5, x: 0, y: 1)
        }
    }
}
```

Rework `RoomRow`: add `@State private var isHovering = false`; title `.gallopText(.bodySStrong, color: SemanticColor.textPrimary)`, preview `color: SemanticColor.textTertiary`, time `color: SemanticColor.textTertiary`; delete the amber `foregroundStyle(isSelected ? buttonPrimaryText… : …)` block and replace the background with `SidebarRowBackground(state: .init(isSelected: isSelected, isHovering: isHovering))` + `.onHover { isHovering = $0 }` (no animation — instantaneous, cowboy pattern). Footer buttons (`CircleIconButton(systemName: "magnifyingglass"…)` / `"gearshape"`): change `CircleIconButton` to accept `icon: GallopIcon?` + `fallbackSystemName: String` (keep the old `systemName:` initializer delegating with `icon: nil`), render via `GallopIconView`, and change its background to the ghost recipe — clear at rest, `Circle().fill(SemanticColor.buttonGhostHover)` while its own `@State isHovering` (add `.onHover`). Call sites: search → `icon: .search`, settings → `icon: .settings`, reconnect keeps SF `arrow.clockwise`.

- [ ] **Step 4: Full test run + visual check** — `swift test 2>&1 | tail -3` green; `swift run CowchatMac`: flat list, Lobby first, no headers, hover/selection per recipe. 
- [ ] **Step 5: Commit + Codex checkpoint (Phase 2)** — `git add -A && git commit -m "ui: flat recency sidebar with Dash row treatment" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`, then `auditcodex` over Tasks 5–7; triage before Phase 3.

---

## Phase 3 — Working / unread state

### Task 8: RoomReadState model + unread UI

**Files:**
- Create: `Sources/CowchatMac/RoomReadState.swift`
- Modify: `Sources/CowchatMac/RoomLocalPreferences.swift` (add data accessor), `Sources/CowchatMac/ChatStore.swift` (own + wire), `Sources/CowchatMac/ContentView.swift` (`RoomRow` unread visuals)
- Test: `Tests/CowchatMacTests/RoomReadStateTests.swift`

**Interfaces:**
- Consumes: `Room.activityDate` (`RoomSidebarPresentation.swift:147`), `RoomLocalPreferences` defaults plumbing.
- Produces: `struct RoomReadState: Codable, Equatable` with `hasSeeded`, `entries: [String: Double]`, `mutating func seed(rooms: [Room])`, `mutating func markRead(roomID: String, activityDate: Date?)`, `func isUnread(_ room: Room, selectedRoomID: String?) -> Bool`, `mutating func reconcile(validRoomIDs: Set<String>)`, `static func milliseconds(from: Date) -> Double`. `ChatStore.isUnread(_ room: Room) -> Bool`. `RoomRow` gains `let isUnread: Bool`.

- [ ] **Step 1: Write failing model tests**

```swift
// Tests/CowchatMacTests/RoomReadStateTests.swift
import XCTest
@testable import CowchatMac

final class RoomReadStateTests: XCTestCase {
    /// Room has a custom Codable implementation, so build fixtures by decoding
    /// the wire shape (cross-check key names against Models.swift CodingKeys).
    private func room(_ id: String, name: String = "room", activity: String) -> Room {
        let json = """
        {"room_id":"\(id)","name":"\(name)","ephemeral":false,\
        "created_at":"2026-08-01T00:00:00Z","visibility":"public",\
        "last_activity":"\(activity)","member_count":1,"encrypted":false}
        """
        return try! JSONDecoder().decode(Room.self, from: Data(json.utf8))
    }

    func testSeedMarksEverythingRead() {
        var state = RoomReadState()
        let r = room("a", activity: "2026-08-05T10:00:00Z")
        state.seed(rooms: [r])
        XCTAssertTrue(state.hasSeeded)
        XCTAssertFalse(state.isUnread(r, selectedRoomID: nil))
    }

    func testNewerActivityIsUnreadUntilMarkedRead() {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z")])
        let updated = room("a", activity: "2026-08-05T11:00:00Z")
        XCTAssertTrue(state.isUnread(updated, selectedRoomID: nil))
        state.markRead(roomID: "a", activityDate: updated.activityDate)
        XCTAssertFalse(state.isUnread(updated, selectedRoomID: nil))
    }

    func testSelectedRoomIsNeverUnread() {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z")])
        let updated = room("a", activity: "2026-08-05T11:00:00Z")
        XCTAssertFalse(state.isUnread(updated, selectedRoomID: "a"))
    }

    func testUnseededOrUnknownRoomIsNotUnreadBeforeSeedButUnknownAfter() {
        var state = RoomReadState()
        let r = room("a", activity: "2026-08-05T10:00:00Z")
        XCTAssertFalse(state.isUnread(r, selectedRoomID: nil)) // pre-seed: quiet
        state.seed(rooms: [])
        XCTAssertTrue(state.isUnread(r, selectedRoomID: nil))  // post-seed unknown: unread
    }

    func testReconcilePrunesAndRoundTripsThroughCodable() throws {
        var state = RoomReadState()
        state.seed(rooms: [room("a", activity: "2026-08-05T10:00:00Z"),
                           room("b", activity: "2026-08-05T10:00:00Z")])
        state.reconcile(validRoomIDs: ["a"])
        XCTAssertNil(state.entries["b"])
        let decoded = try JSONDecoder().decode(
            RoomReadState.self, from: JSONEncoder().encode(state)
        )
        XCTAssertEqual(decoded, state)
    }
}
```

Run — FAIL (`RoomReadState` undefined). (If `Room`'s memberwise init is unavailable due to custom `Codable`, add an internal test-only `Room.fixture(...)` helper in the test target mirroring the existing tests' construction pattern — check how `RoomSidebarPresentationTests.makeRoom` builds rooms and reuse that.)

- [ ] **Step 2: Implement the model**

```swift
// Sources/CowchatMac/RoomReadState.swift
import Foundation

/// Per-room last-seen state, modeled on the cowboy app's ConversationReadState:
/// epoch-milliseconds map, seeded all-read on first run, marked read on selection.
struct RoomReadState: Codable, Equatable {
    private(set) var hasSeeded = false
    private(set) var entries: [String: Double] = [:]

    static func milliseconds(from date: Date) -> Double {
        (date.timeIntervalSince1970 * 1000).rounded()
    }

    mutating func seed(rooms: [Room]) {
        guard !hasSeeded else { return }
        for room in rooms {
            if let activity = room.activityDate {
                entries[room.id] = Self.milliseconds(from: activity)
            } else {
                entries[room.id] = 0
            }
        }
        hasSeeded = true
    }

    mutating func markRead(roomID: String, activityDate: Date?) {
        let stamp = activityDate.map(Self.milliseconds(from:)) ?? 0
        if stamp >= (entries[roomID] ?? -1) { entries[roomID] = stamp }
    }

    func isUnread(_ room: Room, selectedRoomID: String?) -> Bool {
        guard hasSeeded, room.id != selectedRoomID,
              let activity = room.activityDate else { return false }
        guard let seen = entries[room.id] else { return true }
        return Self.milliseconds(from: activity) > seen
    }

    mutating func reconcile(validRoomIDs: Set<String>) {
        entries = entries.filter { validRoomIDs.contains($0.key) }
    }
}
```

- [ ] **Step 3: Persistence + store wiring**

`RoomLocalPreferences` gains (same scoped-key pattern as its ID sets):

```swift
static let roomReadStateKey = "CowchatMac.roomReadState"

var roomReadState: RoomReadState? {
    guard let data = defaults.data(forKey: scopedKey(Self.roomReadStateKey)) else { return nil }
    return try? JSONDecoder().decode(RoomReadState.self, from: data)
}

func saveRoomReadState(_ state: RoomReadState) {
    guard let data = try? JSONEncoder().encode(state) else { return }
    defaults.set(data, forKey: scopedKey(Self.roomReadStateKey))
}
```

`ChatStore` gains `@Published private(set) var readState = RoomReadState()` loaded in the same places `pinnedRoomIDs` used to load (init + profile switch: `readState = localPreferences.roomReadState ?? RoomReadState()`), plus:

```swift
func isUnread(_ room: Room) -> Bool {
    readState.isUnread(room, selectedRoomID: selectedRoomID)
}

private func markSelectedRoomRead() {
    guard let selectedRoomID,
          let room = rooms.first(where: { $0.id == selectedRoomID }) else { return }
    readState.markRead(roomID: selectedRoomID, activityDate: room.activityDate)
    localPreferences.saveRoomReadState(readState)
}
```

Wire calls: (a) end of `select(room:)`'s synchronous prefix (right after `selectedRoomID = room.roomID`) call `markSelectedRoomRead()`; (b) in `updateRoomActivity(from:)`, after the room's `lastActivity` bump, `if message.roomID == selectedRoomID { markSelectedRoomRead() }`; (c) in `refreshRooms` (after `rooms = refreshed.sorted...`) — `if !readState.hasSeeded { readState.seed(rooms: rooms); localPreferences.saveRoomReadState(readState) }` and always `readState.reconcile(validRoomIDs: Set(rooms.map(\.id)))`; then `markSelectedRoomRead()` to absorb refresh-driven bumps of the selected room.

- [ ] **Step 4: Unread visuals in `RoomRow`**

Add `let isUnread: Bool` (pass `store.isUnread(room)` from `SidebarView`; archive rows pass the same). Leading dot + weight bump per spec:

```swift
HStack(spacing: 10) {
    Circle()
        .fill(isUnread ? Palette.nugget500 : Color.clear)
        .frame(width: 7, height: 7)
    RoomAvatar(name: room.name, size: 40, accented: isSelected)
    // …existing VStack…
}
```

Title line becomes `.gallopText(isUnread ? .bodySStrong : .bodyS, color: SemanticColor.textPrimary)`; add `.accessibilityValue(isUnread ? Text("Unread") : Text(""))` on the row. Adjust `.padding(.horizontal, 8)` → `.padding(.trailing, 8).padding(.leading, 4)` so the dot column sits inside the pill.

- [ ] **Step 5: Run everything, commit** — `swift test 2>&1 | tail -3` green → `git add -A && git commit -m "feat: room read state with unread indicators" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 9: Working signal (thinking recency)

**Files:**
- Modify: `Sources/CowchatMac/RoomSidebarPresentation.swift` (pure predicate), `Sources/CowchatMac/ChatStore.swift` (`lastThinkingAt` tracking in `handleEvent`), `Sources/CowchatMac/ContentView.swift` (`RoomRow` glyph, TimelineView cadence, header/above-composer treatments)
- Test: `Tests/CowchatMacTests/RoomSidebarPresentationTests.swift`

**Interfaces:**
- Consumes: `ChatMessage.isThinking` (`Models.swift:169`), `handleEvent` message routing (`ChatStore.swift:910-928`), `GallopIcon.thinking`.
- Produces: `RoomSidebarPresentation.isWorking(lastThinkingAt: Date?, now: Date, window: TimeInterval) -> Bool` (default window 120); `ChatStore.lastThinkingAt: [String: Date]` published, `ChatStore.isWorking(_ room: Room, at now: Date) -> Bool`; `RoomRow` gains `let isWorking: Bool`.

- [ ] **Step 1: Failing predicate tests**

```swift
func testWorkingPredicateHonorsWindow() {
    let now = Date()
    XCTAssertFalse(RoomSidebarPresentation.isWorking(lastThinkingAt: nil, now: now, window: 120))
    XCTAssertTrue(RoomSidebarPresentation.isWorking(
        lastThinkingAt: now.addingTimeInterval(-30), now: now, window: 120))
    XCTAssertFalse(RoomSidebarPresentation.isWorking(
        lastThinkingAt: now.addingTimeInterval(-121), now: now, window: 120))
}
```

Run — FAIL.

- [ ] **Step 2: Implement predicate + store tracking**

```swift
// RoomSidebarPresentation.swift
/// Sidebar working signal: presence is selected-room-only and unattributable
/// per-room (presence_update has no room_id), so background rooms light up on
/// thinking-message recency instead — see the spec's §4 validation notes.
static func isWorking(lastThinkingAt: Date?, now: Date, window: TimeInterval = 120) -> Bool {
    guard let lastThinkingAt else { return false }
    return now.timeIntervalSince(lastThinkingAt) < window
}
```

`ChatStore`: add `@Published private(set) var lastThinkingAt: [String: Date] = [:]` and, in `handleEvent`'s `message_received` decode block, BEFORE the existing `if message.isThinking` branch behavior (keep `updateRoomActivity` call):

```swift
if message.isThinking {
    lastThinkingAt[message.roomID] = message.timestamp.cowchatDate ?? Date()
} else {
    // A completed turn clears the working indicator immediately.
    lastThinkingAt.removeValue(forKey: message.roomID)
}
```

Add `func isWorking(_ room: Room, at now: Date = Date()) -> Bool { RoomSidebarPresentation.isWorking(lastThinkingAt: lastThinkingAt[room.id], now: now) }`. Clear the map on profile switch (where `pinnedRoomIDs` used to reset) and on `room_destroyed` (`lastThinkingAt.removeValue(forKey: id)` inside `removeRoom` or the event case).

- [ ] **Step 3: Row glyph + cadence + header treatment**

`RoomRow` gains `let isWorking: Bool` (passed as `store.isWorking(room, at: now)`). In the title HStack's trailing cluster, before the timestamp Text:

```swift
if isWorking {
    GallopIconView(icon: .thinking, fallbackSystemName: "arrow.triangle.2.circlepath", size: 12)
        .foregroundStyle(SemanticColor.buttonPrimaryDefault)
        .accessibilityLabel("Agents working")
}
```

Sidebar `TimelineView(.periodic(from: .now, by: 60))` → `TimelineView(.periodic(from: .now, by: store.lastThinkingAt.isEmpty ? 60 : 10))`. Chat header presence line: when `presenceSummary` reflects active agents (contains "active"), render `.gallopText(.caption, color: SemanticColor.warning)` — `warning` resolves to nugget600-family amber (verify the vendored token; if `SemanticColor.warning` doesn't exist post-vendor, use `Palette.nugget600`). Above-composer thinking indicator (`messageList`'s `thinkingText` block, ContentView ~1183): replace `ProgressView().controlSize(.mini)` with `GallopIconView(icon: .thinking, fallbackSystemName: "arrow.triangle.2.circlepath", size: 16).foregroundStyle(SemanticColor.buttonPrimaryDefault)` and text `.gallopText(.bodyL, color: SemanticColor.textTertiary)` — the cowboy live-row recipe.

- [ ] **Step 4: Full test run** — green.
- [ ] **Step 5: Commit + Codex checkpoint (Phase 3)** — `git add -A && git commit -m "feat: sidebar working indicator from thinking recency" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`, then `auditcodex` over Tasks 8–9 (ask it specifically to probe read/working state races: mark-read on refresh, thinking map lifecycle across profile switches).

---

## Phase 4 — Chat pane

### Task 10: AgentAppResolver

**Files:**
- Create: `Sources/CowchatMac/AgentAppResolver.swift`
- Modify: `Sources/CowchatMac/ContentView.swift` (`AgentAvatar.appIcon` uses the resolver)
- Test: `Tests/CowchatMacTests/AgentAppResolverTests.swift`

**Interfaces:**
- Consumes: nothing new.
- Produces: `enum AgentAppResolver` with `struct ResolvedApp: Equatable { let displayName: String; let bundleID: String }`, `static func resolvedApp(forAgentNamed name: String) -> ResolvedApp?`, `static func applicationURL(for app: ResolvedApp) -> URL?`, `@MainActor static func open(_ app: ResolvedApp)`.

- [ ] **Step 1: Failing tests**

```swift
// Tests/CowchatMacTests/AgentAppResolverTests.swift
import XCTest
@testable import CowchatMac

final class AgentAppResolverTests: XCTestCase {
    func testKnownAgentNamesResolve() {
        XCTAssertEqual(
            AgentAppResolver.resolvedApp(forAgentNamed: "Claude Code")?.bundleID,
            "com.anthropic.claudefordesktop"
        )
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "codex-cli")?.bundleID, "com.openai.codex")
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "ChatGPT agent")?.bundleID, "com.openai.chat")
        XCTAssertEqual(AgentAppResolver.resolvedApp(forAgentNamed: "CLAUDE")?.displayName, "Claude")
    }

    func testUnknownAgentNameResolvesNil() {
        XCTAssertNil(AgentAppResolver.resolvedApp(forAgentNamed: "mystery-bot"))
    }
}
```

Run — FAIL.

- [ ] **Step 2: Implement**

```swift
// Sources/CowchatMac/AgentAppResolver.swift
import AppKit

/// Maps agent display names to installed companion apps. Single source of
/// truth for both the avatar app-icon lookup and the "Open in …" actions.
enum AgentAppResolver {
    struct ResolvedApp: Equatable {
        let displayName: String
        let bundleID: String
    }

    static func resolvedApp(forAgentNamed name: String) -> ResolvedApp? {
        let normalized = name.lowercased()
        if normalized.contains("claude") {
            return ResolvedApp(displayName: "Claude", bundleID: "com.anthropic.claudefordesktop")
        }
        if normalized.contains("codex") {
            return ResolvedApp(displayName: "Codex", bundleID: "com.openai.codex")
        }
        if normalized.contains("chatgpt") || normalized.contains("openai") {
            return ResolvedApp(displayName: "ChatGPT", bundleID: "com.openai.chat")
        }
        return nil
    }

    static func applicationURL(for app: ResolvedApp) -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: app.bundleID)
    }

    @MainActor
    static func open(_ app: ResolvedApp) {
        guard let url = applicationURL(for: app) else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        NSWorkspace.shared.openApplication(at: url, configuration: configuration)
    }
}
```

Refactor `AgentAvatar.appIcon` (ContentView ~1531-1548) to:

```swift
private var appIcon: NSImage? {
    guard let app = AgentAppResolver.resolvedApp(forAgentNamed: name),
          let appURL = AgentAppResolver.applicationURL(for: app) else { return nil }
    return NSWorkspace.shared.icon(forFile: appURL.path)
}
```

- [ ] **Step 3: Tests green, commit** — `swift test 2>&1 | tail -3` → `git add -A && git commit -m "refactor: extract AgentAppResolver from AgentAvatar" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 11: "Open in Claude/Codex" hover chip

**Files:**
- Modify: `Sources/CowchatMac/ContentView.swift` (`MessageFeedRow` agent branch + new `OpenInAgentAppChip` view)

**Interfaces:**
- Consumes: `AgentAppResolver` (Task 10), `GallopIcon.arrowUpRight`.
- Produces: `OpenInAgentAppChip(app:isVisible:)` private view; `MessageFeedRow` gains `@State private var isHovering`.

- [ ] **Step 1: Add the chip view** (below `MessageFeedRow`):

```swift
/// Cowboy hover pattern: layout-reserved, opacity-faded, hit-test-gated —
/// siblings never jump, VoiceOver gets a persistent action instead.
private struct OpenInAgentAppChip: View {
    let app: AgentAppResolver.ResolvedApp
    let isVisible: Bool
    @State private var isChipHovering = false

    var body: some View {
        Button {
            AgentAppResolver.open(app)
        } label: {
            HStack(spacing: 4) {
                Text("Open in \(app.displayName)")
                    .gallopText(.caption, color: SemanticColor.textSecondary)
                GallopIconView(icon: .arrowUpRight, fallbackSystemName: "arrow.up.right", size: 10)
                    .foregroundStyle(SemanticColor.iconSecondary)
            }
            .padding(.horizontal, 9)
            .frame(height: 22)
            .background(
                isChipHovering ? SemanticColor.buttonSecondaryHover : SemanticColor.surface600,
                in: Capsule()
            )
            .overlay { Capsule().stroke(SemanticColor.borderDefault, lineWidth: 1) }
        }
        .buttonStyle(.plain)
        .onHover { isChipHovering = $0 }
        .opacity(isVisible ? 1 : 0)
        .allowsHitTesting(isVisible)
        .accessibilityHidden(!isVisible)
        .help("Open \(app.displayName)")
    }
}
```

- [ ] **Step 2: Wire into the agent row**

In `MessageFeedRow`'s non-mine branch, add `@State private var isHovering = false` to the struct, and change the name HStack (~line 1412):

```swift
HStack(spacing: 8) {
    Text(message.agentName)
        .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
    Text(relativeTimestamp)
        .gallopText(.caption, color: SemanticColor.textTertiary)
    if let app = AgentAppResolver.resolvedApp(forAgentNamed: message.agentName),
       AgentAppResolver.applicationURL(for: app) != nil {
        OpenInAgentAppChip(app: app, isVisible: isHovering)
    }
}
```

and on the whole non-mine `HStack(alignment: .top, spacing: 11)` add:

```swift
.contentShape(Rectangle())
.onHover { isHovering = $0 }
.animation(.easeOut(duration: 0.12), value: isHovering)
.macAccessibleAction(label: openInLabel ?? "") { openInApp() }
```

with helpers on `MessageFeedRow` (guard the accessibility action registration when `openInLabel == nil` — follow `macAccessibleAction`'s existing `isEnabled` parameter, see `AccessibleActionOverlay.swift`):

```swift
private var resolvedApp: AgentAppResolver.ResolvedApp? {
    guard !isMine,
          let app = AgentAppResolver.resolvedApp(forAgentNamed: message.agentName),
          AgentAppResolver.applicationURL(for: app) != nil else { return nil }
    return app
}
private var openInLabel: String? { resolvedApp.map { "Open in \($0.displayName)" } }
private func openInApp() { if let resolvedApp { AgentAppResolver.open(resolvedApp) } }
```

- [ ] **Step 3: Build + visual check** — `swift test 2>&1 | tail -3` green; `swift run CowchatMac`, hover an agent message with Claude installed: chip fades in over 0.12s without layout shift; click activates the app.
- [ ] **Step 4: Commit** — `git add -A && git commit -m "feat: open-in-app hover chips on agent messages" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 12: Consolidated room menu in the toolbar

**Files:**
- Modify: `Sources/CowchatMac/ContentView.swift` (`ChatRoomView.chatHeader` trailing cluster → toolbar menu; `DashboardRoomCard` menu icon)

**Interfaces:**
- Consumes: toolbar from Task 5, `GallopIcon.ellipsis`/`.trash`.
- Produces: room actions menu at `ToolbarItem(placement: .primaryAction)` scoped to `ChatRoomView` (registered via `.toolbar` on `ChatRoomView`'s content — SwiftUI merges detail toolbars); destroy alert stays on `ChatRoomView`.

- [ ] **Step 1: Move + merge the menu**

Delete the whole trailing `HStack(spacing: 0)` in `chatHeader` (ellipsis Menu + divider + trash Button — ContentView ~1075-1136). Add to `ChatRoomView.body`, after the existing `.alert` modifier:

```swift
.toolbar {
    ToolbarItem(placement: .primaryAction) {
        Menu {
            Button("Rename room") { store.presentRename(room) }
                .disabled(!store.canRename(room))
            Button("Archive room") { Task { await store.archive(room) } }
            Divider()
            Button("Create nested room…") { store.presentCreateRoom(parentID: room.id) }
            if !store.connectionStatus.isConnected {
                Button("Reconnect") { store.start() }
            }
            Divider()
            Text(room.ephemeral ? "Temporary room" : "Persistent room")
            Text(room.visibility.capitalized)
            Divider()
            Button("Destroy room…", role: .destructive) {
                isDestroyConfirmationPresented = true
            }
            .disabled(!store.canDestroy(room) || isDestroyingRoom)
        } label: {
            GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 17)
                .foregroundStyle(SemanticColor.iconSecondary)
                .frame(width: 36, height: 36)
                .contentShape(Circle())
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .accessibilityLabel("Room actions")
    }
}
```

(No Pin item — removed in Task 6. The Destroy confirmation `.alert` and `isDestroyingRoom` state stay untouched.) `DashboardRoomCard`'s card menu label (~line 825) swaps its `systemImage: "ellipsis"` label for `GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 14)` with the same frame.

- [ ] **Step 2: Build + verify** — `swift test 2>&1 | tail -3`; run app: room menu in toolbar, Destroy inside it (destructive red), no trash button in header, header now title+presence only.
- [ ] **Step 3: Commit** — `git add -A && git commit -m "ui: consolidate room actions into toolbar menu" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 13: Dash composer, bubble, and disclosure treatments

**Files:**
- Modify: `Sources/CowchatMac/ContentView.swift` (`ChatRoomView.expandedComposer`, `composer` collapsed FAB, `MessageFeedRow` mine-branch bubble, `ExpandableMessageText` chevrons)

**Interfaces:**
- Consumes: tokens/icons; Dash measurements (in-line below, extracted from Figma node `4605:27623` on 2026-08-05).
- Produces: no new public symbols; visual-only.

- [ ] **Step 1: Composer field + buttons (Dash "Chat textfield": height 56, radius 28, 1pt `borderDefault`, fill `textfieldDefault`; 36pt circular controls; 20pt icons at 0.92 opacity)**

In `expandedComposer` (~ContentView 1293-1367): remove the top hairline `overlay` and change the container `.background(SemanticColor.surface600)` → `.background(SemanticColor.surface500)`. Replace the field's paddings/frame block:

```swift
ComposerTextField(
    text: $store.draft,
    placeholder: "Message \(room.name)",
    isEnabled: !room.encrypted,
    onSubmit: store.sendDraft
)
.frame(height: 22)
.padding(.horizontal, 16)
.frame(height: 44)
.background(
    isFieldHovering ? SemanticColor.textfieldHover : SemanticColor.textfieldDefault,
    in: RoundedRectangle(cornerRadius: 22, style: .continuous)
)
.overlay {
    RoundedRectangle(cornerRadius: 22, style: .continuous)
        .stroke(isFieldHovering ? SemanticColor.borderHover : SemanticColor.borderDefault, lineWidth: 1)
}
.onHover { isFieldHovering = $0 }
```

with `@State private var isFieldHovering = false` added to `ChatRoomView`. (44pt, not Dash's 56 — cowchat's composer is a docked bar, not the full-width hero field; radius stays height/2. This is a recorded judgment call; flag in commit message.) Send button (~1327-1342) becomes the cowboy circular ramp:

```swift
Button { store.sendDraft() } label: {
    GallopIconView(icon: .send, fallbackSystemName: "paperplane.fill", size: 18)
        .foregroundStyle(SemanticColor.buttonPrimaryIconDefault)
        .frame(width: 36, height: 36)
        .background(SemanticColor.buttonPrimaryDefault, in: Circle())
        .overlay { Circle().stroke(Palette.nugget300, lineWidth: 1) }
}
.buttonStyle(.plain)
.disabled(!canSend)
.opacity(canSend ? 1 : 0.4)
```

(keep the existing `macAccessibleAction`). The attach placeholder `CircleIconButton(systemName: "plus"…)` → `icon: .add` via the Task 7 `CircleIconButton` icon support; close button `xmark` → `icon: .dismiss`. Collapsed FAB (`composer`, ~1270-1289): label becomes `GallopIconView(icon: .edit, fallbackSystemName: "pencil", size: 16)`, colors unchanged (secondary ramp).

- [ ] **Step 2: Bubble radii (Dash user bubble: 24/24/8/24, 0.5pt border, gradient surface300→surface400 — cowchat's existing gradient already matches; text `bodyL`/`textPrimary`; max-width 720)**

In `MessageFeedRow` mine-branch (~1383-1406): keep the gradient; replace both `RoundedRectangle(cornerRadius: 20, style: .continuous)` (background + stroke) with:

```swift
UnevenRoundedRectangle(
    topLeadingRadius: 24, bottomLeadingRadius: 24,
    bottomTrailingRadius: 8, topTrailingRadius: 24, style: .continuous
)
```

stroke width 20→0.5, padding `18/13` → `.padding(.horizontal, 20).padding(.vertical, 16)`. Inside `ExpandableMessageText` prose (only affects both branches — acceptable): change the mine-branch text color by passing a `textColor` parameter: add `var textColor: Color = SemanticColor.textSecondary` to `ExpandableMessageText`, use it in the prose `.gallopText(.bodyL, color: textColor)` line, and pass `textColor: SemanticColor.textPrimary` from the mine branch. (Dash inner highlight shadows are intentionally skipped — sub-1pt inset shadows; recorded simplification.)

- [ ] **Step 3: Disclosure + quiet-room icons** — `ExpandableMessageText`'s `bubble.left` stays SF (no Gallop equivalent — matches spec gap policy) but the chevrons swap: `chevron.down`/`chevron.right` → `GallopIconView(icon: isExpanded ? .chevronDownExtraSmall : .chevronRightExtraSmall, fallbackSystemName: isExpanded ? "chevron.down" : "chevron.right", size: 10)`. `quietRoom`'s `bubble.left` → `GallopIconView(icon: .message, fallbackSystemName: "bubble.left", size: 24)`.
- [ ] **Step 4: Build, test, eyeball, commit + Codex checkpoint (Phase 4)** — `swift test 2>&1 | tail -3`; run app for bubble/composer check; `git add -A && git commit -m "ui: Dash composer, bubble, and disclosure treatments" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`; `auditcodex` over Tasks 10–13.

---

## Phase 5 — Sweep and ship

### Task 14: Cards, glass notice, onboarding, icon sweep

**Files:**
- Create: `Sources/CowchatMac/Gallop/GallopCard.swift` (adapted copy of `~/Github/macos/SharedUI/GallopCard.swift`)
- Modify: `Sources/CowchatMac/ContentView.swift` (`DashboardRoomCard`, `LobbyDashboardView` new-room card, `RoomReadyNotice`, `EmptyChatView`, `SettingsView`, `CreateRoomView`, `RenameRoomView` icon swaps), `Sources/CowchatMac/CowchatOnboarding.swift` (typography + buttons)

**Interfaces:**
- Consumes: everything prior.
- Produces: `.gallopCard(cornerRadius:)` View extension (identical to cowboy: `padding(16)` + `surface600` fill + `borderDefault` 1pt stroke, default radius 8).

- [ ] **Step 1: Vendor `gallopCard`** — copy the 17-line file, drop `import Gallop`, keep the recipe verbatim. Apply to `DashboardRoomCard`: replace its hand-rolled `padding(16)/background(surface600, radius 14)/overlay stroke` block with `.gallopCard()` (content keeps `minHeight: 132`; radius 14→8 is the Dash-correct value). Same for the "+ New Room" card in `LobbyDashboardView` (find its matching background/overlay pair).
- [ ] **Step 2: Glass capsule for `RoomReadyNotice`** — replace its current background with the cowboy AppStatusBar recipe:

```swift
.background {
    Capsule().fill(.ultraThinMaterial)
        .overlay { Capsule().fill(SemanticColor.surfaceGlass500) }
        .overlay { Capsule().stroke(SemanticColor.surfaceGlassBorderHighlight, lineWidth: 1) }
        .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.04), radius: 0, y: 0.5)
}
```

(Adjust to the notice's actual shape if it isn't a capsule — keep its current shape, swap fills/strokes/shadows per the recipe.)
- [ ] **Step 3: Onboarding** — "Howdy… Welcome to Cowchat!" title gets `.gallopText(.h4, color: SemanticColor.textPrimary)` (Season Mix display); primary/secondary buttons adopt the capsule ramp inline (12pt vertical / 20pt horizontal padding, fill `buttonPrimaryDefault`→`Hover`→`Pressed` via a small `ButtonStyle` copied from the cowboy `AuthPillButtonStyle` shape — include `@Environment(\.isEnabled)` and 0.5 disabled opacity; place it in `CowchatOnboarding.swift` as `private struct CapsulePillButtonStyle: ButtonStyle`).
- [ ] **Step 4: Icon sweep** — remaining SF swaps where Gallop covers: settings sheet `gearshape`→`.settings`, search fields `magnifyingglass`→`.search`, clear buttons `xmark.circle.fill`/`xmark`→`.dismiss`, `lock.fill`→`.lock` (12pt sizes), `square.and.pencil`→`.edit`, `chevron.right` breadcrumbs→`.chevronRightExtraSmall`, sheet copy buttons→`.copy`, warnings→`.warning`. Keep SF for: `archivebox`, `cloud`, `desktopcomputer`, `wifi.slash`, `arrow.clockwise`, `checkmark.circle.fill`, `person.2.slash`, `chevron.up.chevron.down`, `network`, `sparkles`, `list.bullet.rectangle`, `circle.lefthalf.filled`, `exclamationmark.triangle.fill`, `plus` outside the composer. Command: `grep -n "systemName:\|systemImage:" Sources/CowchatMac/*.swift` and walk each hit against this mapping.
- [ ] **Step 5: Test + run + commit** — `swift test 2>&1 | tail -3`; visual pass of lobby/onboarding/settings; `git add -A && git commit -m "ui: gallop cards, glass notice, onboarding and icon sweep" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"`

### Task 15: Packaging verification + final review

**Files:**
- Modify (only if verification fails): `apps/CowchatMac/build-app.sh`
- No new source files.

- [ ] **Step 1: Packaging checks**

```bash
./test-dmg-packaging.sh            # existing packaging test must stay green
./build-app.sh                     # then inspect the built app:
BUNDLE="$HOME/Applications/Cowchat.app/Contents/Resources/CowchatMac_CowchatMac.bundle"
ls "$BUNDLE/Fonts" "$BUNDLE/Icons/svg" | head   # both directories must list files
```

`build-app.sh:144` already does `cp -R "$RESOURCE_BUNDLE" .../Resources/` — the fonts/icons ride along automatically. If `Fonts`/`Icons` are missing from the built bundle, the `.copy` entries in `Package.swift` are wrong — fix there, not in the script. Launch the built app once: Season type renders (compare a title against `swift run` output), icons appear, no fallback glyphs.

- [ ] **Step 2: Full suite + workspace** — `swift test 2>&1 | tail -3` and repo-root `cargo build --workspace 2>&1 | tail -3` (untouched, but prove it).
- [ ] **Step 3: Final Codex review** — `auditcodex` over the whole branch (`git diff main...HEAD`); triage with `superpowers:receiving-code-review`; land fixes.
- [ ] **Step 4: Update screenshots/docs if desired** and finish with `superpowers:finishing-a-development-branch` (merge/PR decision belongs to Patrick).

---

## Coverage map (spec § → task)

| Spec section | Tasks |
|---|---|
| §1 foundation (tokens/fonts/icons/card/packaging) | 1, 2, 3, 4, 14, 15 |
| §2 shell/chrome | 5 |
| §3 sidebar simplification + rows | 6, 7 |
| §4 working/unread | 8, 9 |
| §5 chat pane (chips, menu, composer, bubble) | 10, 11, 12, 13 |
| §6 remaining surfaces | 14 |
| §7 divergence log | encoded in Tasks 7 (radius/width), 13 (44pt field, skipped inner shadows) |
| §8 testing | every task + 15 |
| §9 process (Codex checkpoints) | 4, 7, 9, 13, 15 |
