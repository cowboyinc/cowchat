# Cowchat Mac Onboarding Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the two-prompt takeover onboarding with one explain-only splash, an auto-created public "General" room, and a live-signal connect-prompt empty state — per `docs/superpowers/specs/2026-08-07-cowchat-mac-onboarding-design.md`.

**Architecture:** The Mac app is a SwiftPM executable at `apps/CowchatMac` (single SwiftUI WindowGroup, `ChatStore` as the one `ObservableObject`, vendored Gallop tokens). We delete the persisted "setup screen" mechanism, derive what an open room renders from a new pure `RoomPaneState` function, rebuild `CowchatOnboardingView` as a splash, and add a one-shot default-room creation to `ChatStore.connect()`. The site change is one file.

**Tech Stack:** Swift 5 / SwiftUI (macOS 13+), XCTest, Next.js static site (no test harness).

## Global Constraints

- All commands for the app run from `apps/CowchatMac/`: build `swift build`, test `swift test`. Commit from the **repo root** (`/…/handsomely-doom`), never from inside `apps/CowchatMac` (shell cwd persists between steps).
- Room-name comparisons are always `localizedCaseInsensitiveCompare` on the app side ("lobby", "General").
- Every custom-drawn control gets `.macAccessibleAction(label:action:)` (`AccessibleActionOverlay.swift`); plain SwiftUI `Button`s in this codebase do not publish AX labels.
- Copy strings must be byte-exact from the spec's Copy summary table. Room names in prompts use curly quotes `\u{201C}…\u{201D}`.
- No server-side (Rust) changes. No changes to `SKILLS.md` / skills.txt publishing.
- UserDefaults keys introduced: `CowchatMac.didCreateDefaultRoom`, `CowchatMac.defaultRoomBackfillAttempted` (both unscoped). Key deleted: `CowchatMac.pendingSetupScreenRoomIDs` (and its current-profile scoped variant).
- Existing keys that must not change meaning: `CowchatMac.completedOnboardingVersion`, `CowchatMac.onboardingMigrationAttempted`, `CowchatMac.pendingSetupRoomIDs`, `CowchatMac.agentID`.
- `CowchatOnboarding.currentVersion` stays `1`.

---

### Task 1: RoomPaneState — the pure decision table

**Files:**
- Create: `apps/CowchatMac/Sources/CowchatMac/RoomPaneState.swift`
- Test: `apps/CowchatMac/Tests/CowchatMacTests/RoomPaneStateTests.swift`

**Interfaces:**
- Consumes: `ConnectionStatus` (`Models.swift`: `.disconnected/.connecting/.connected/.failed(String)`, has `isConnected`).
- Produces: `RoomPaneState` enum + `RoomPaneState.state(connectionStatus:isLoadingMessages:hasMessages:hasOtherMembers:)`. Task 4 renders from it. (The lobby special-case stays in `ContentView`'s outer switch and never reaches this function.)

- [ ] **Step 1: Write the failing tests**

```swift
import XCTest
@testable import CowchatMac

final class RoomPaneStateTests: XCTestCase {
    func testDisconnectedWithNothingLoadedShowsConnectPromptOfflineVariant() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .disconnected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.offline)
        )
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .failed("boom"),
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.offline)
        )
    }

    func testConnectingWithNothingLoadedShowsConnectPromptConnectingVariant() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connecting,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.connecting)
        )
    }

    func testDisconnectionKeepsAlreadyLoadedMessagesVisible() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .disconnected,
                isLoadingMessages: false, hasMessages: true, hasOtherMembers: false
            ),
            .chat
        )
    }

    func testLoadingWinsWhileConnected() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: true, hasMessages: false, hasOtherMembers: false
            ),
            .loading
        )
    }

    func testMessagesRenderChat() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: true, hasOtherMembers: true
            ),
            .chat
        )
    }

    func testEmptyRoomWithNoOtherMembersShowsConnectPrompt() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.connected)
        )
    }

    func testEmptyRoomWithMembersPresentIsQuiet() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: true
            ),
            .quiet
        )
    }

    /// Agent joined then left before any message: the connect prompt returns.
    func testJoinedThenLeftBeforeMessagesReturnsToConnectPrompt() {
        XCTAssertEqual(
            RoomPaneState.state(
                connectionStatus: .connected,
                isLoadingMessages: false, hasMessages: false, hasOtherMembers: false
            ),
            .connectPrompt(.connected)
        )
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd apps/CowchatMac && swift test --filter RoomPaneStateTests`
Expected: FAIL to compile — `RoomPaneState` not defined.

- [ ] **Step 3: Implement `RoomPaneState.swift`**

```swift
/// What the main pane renders for a selected, non-lobby room — §3 of the
/// onboarding spec. Purely derived from live signals; persisted flags never
/// gate rendering. "Loading" means an in-flight history fetch on an
/// established connection only.
enum RoomPaneState: Equatable {
    enum ConnectVariant: Equatable {
        case connected
        case connecting
        case offline
    }

    case connectPrompt(ConnectVariant)
    case loading
    case chat
    case quiet

    static func state(
        connectionStatus: ConnectionStatus,
        isLoadingMessages: Bool,
        hasMessages: Bool,
        hasOtherMembers: Bool
    ) -> RoomPaneState {
        // Spec §3 decision-table order, minus the lobby row (handled by
        // ContentView's outer switch before this function is reached).
        if !connectionStatus.isConnected && !hasMessages {
            return .connectPrompt(connectionStatus == .connecting ? .connecting : .offline)
        }
        if isLoadingMessages { return .loading }
        if hasMessages { return .chat }
        if hasOtherMembers { return .quiet }
        return .connectPrompt(.connected)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd apps/CowchatMac && swift test --filter RoomPaneStateTests`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit** (from repo root)

```bash
git add apps/CowchatMac/Sources/CowchatMac/RoomPaneState.swift apps/CowchatMac/Tests/CowchatMacTests/RoomPaneStateTests.swift
git commit -m "feat(mac): pure RoomPaneState decision table for the room pane"
```

---

### Task 2: Surface server error codes on CowchatConnectionError

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/CowchatConnection.swift:30-49` (enum) and `:448-451` (error-frame parsing)
- Modify: `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift:337` (construction site)
- Test: `apps/CowchatMac/Tests/CowchatMacTests/ProtocolModelsTests.swift` (append one test)

**Interfaces:**
- Produces: `CowchatConnectionError.server(message: String, code: String?)`. Task 6 matches `code == "room_name_taken"` (the server's snake_case `ErrorCode::RoomNameTaken`; error frames carry `payload.code` + `payload.message` per `crates/cowchat-core/src/error.rs`).

- [ ] **Step 1: Write the failing test** — append to `ProtocolModelsTests.swift`:

```swift
func testServerErrorCarriesCodeAndMessage() {
    let error = CowchatConnectionError.server(message: "Room name 'General' already taken", code: "room_name_taken")
    guard case .server(let message, let code) = error else {
        return XCTFail("expected .server")
    }
    XCTAssertEqual(message, "Room name 'General' already taken")
    XCTAssertEqual(code, "room_name_taken")
    XCTAssertEqual(error.errorDescription, "Room name 'General' already taken")
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd apps/CowchatMac && swift test --filter ProtocolModelsTests`
Expected: FAIL to compile — `.server` takes one associated value.

- [ ] **Step 3: Implement.** In `CowchatConnection.swift` change the enum case and its `errorDescription`:

```swift
enum CowchatConnectionError: LocalizedError {
    case notConnected
    case invalidResponse
    case server(message: String, code: String?)
    case transport(String)
    case timeout

    var errorDescription: String? {
        switch self {
        case .notConnected:
            return "Cowchat is not connected."
        case .invalidResponse:
            return "Cowchat returned an invalid response."
        case .server(let message, _):
            return message
        case .transport(let message):
            return message
        case .timeout:
            return "Cowchat did not respond in time."
        }
    }
}
```

At the error-frame parse site (currently line ~450), also read the code:

```swift
                if type == "error" {
                    let message = payload["message"] as? String ?? "Unknown server error"
                    finishRequest(
                        id: replyTo,
                        result: .failure(CowchatConnectionError.server(
                            message: message,
                            code: payload["code"] as? String
                        ))
                    )
                } else {
```

In `RoomTransitionTests.swift:337` update the construction:

```swift
        connection.registrationError = CowchatConnectionError.server(message: "Protocol mismatch", code: nil)
```

- [ ] **Step 4: Run the full suite to verify nothing else matched the old shape**

Run: `cd apps/CowchatMac && swift test`
Expected: PASS.

- [ ] **Step 5: Commit** (from repo root)

```bash
git add apps/CowchatMac/Sources/CowchatMac/CowchatConnection.swift apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift apps/CowchatMac/Tests/CowchatMacTests/ProtocolModelsTests.swift
git commit -m "feat(mac): carry server error codes on CowchatConnectionError.server"
```

---

### Task 3: Delete the setup-screen mechanism (takeover, persistence, launch preference)

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/ChatStore.swift` — remove `roomSetupScreenIDs` (decl :26, init :172, `connect()` selection :280-283, `createRoom` :717-718, `completeRoomSetup` :857-868 deleted whole, reconcile :1310-1314, `removeRoom` :1332-1333, `markSetupRoomReadyIfNeeded` :1407-1408, `resetServerBackedState` :549); add one-time defaults cleanup in `init`
- Modify: `apps/CowchatMac/Sources/CowchatMac/RoomLocalPreferences.swift` — remove `pendingSetupScreenRoomIDs` property, `savePendingSetupScreenRoomIDs`, keep the key constant for cleanup
- Modify: `apps/CowchatMac/Sources/CowchatMac/ContentView.swift` — remove the `RoomSetupView` branch (:56-58), delete `RoomSetupView` struct (:806-897), remove the `setupRoomIDs` exclusion from `LobbyDashboardView.dashboardRooms` (:667-671)
- Modify: `apps/CowchatMac/Tests/CowchatMacTests/RoomLocalPreferencesTests.swift`, `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift` (delete/rewrite tests of deleted APIs; add launch-selection + cleanup tests)

**Interfaces:**
- Consumes: nothing new.
- Produces: `ChatStore` without `roomSetupScreenIDs`/`completeRoomSetup`. Launch selection in `connect()` becomes lobby → first. Interim behavior (until Task 4): agentless rooms render the existing quiet-room state — the suite must still pass at this commit.

- [ ] **Step 1: Write the failing tests.** In `RoomTransitionTests.swift`, add (using the existing fileprivate `MockRoomConnection` + `makeStore(connection:)` helpers; build `Room` values the same way neighboring tests in that file do):

```swift
    @MainActor
    func testLaunchSelectionPrefersLobbyOverPendingSetupRooms() async throws {
        let connection = MockRoomConnection()
        let suiteName = "RoomTransitionTests.launch.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        // A previous build left a persisted setup-screen room behind.
        defaults.set(["stale-room"], forKey: "CowchatMac.pendingSetupScreenRoomIDs")
        defaults.set(["stale-room"], forKey: RoomLocalPreferences.pendingSetupRoomIDsKey)
        let store = ChatStore(connection: connection, defaults: defaults)

        connection.listedRooms = [
            try roomFixture(id: "stale-room", name: "stale-room"),
            try roomFixture(id: "lobby", name: "lobby"),
        ]
        await store.connect()

        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    @MainActor
    func testInitDeletesThePersistedSetupScreenKey() {
        let connection = MockRoomConnection()
        let suiteName = "RoomTransitionTests.cleanup.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(["r1"], forKey: "CowchatMac.pendingSetupScreenRoomIDs")
        _ = ChatStore(connection: connection, defaults: defaults)
        XCTAssertNil(defaults.object(forKey: "CowchatMac.pendingSetupScreenRoomIDs"))
    }

    /// JSON-decode a Room the way this file's other fixtures do.
    private func roomFixture(id: String, name: String) throws -> Room {
        let json = """
        {"room_id":"\(id)","name":"\(name)","ephemeral":false,
         "created_at":"2026-08-07T00:00:00Z","created_by":"someone",
         "visibility":"public","encrypted":false}
        """
        return try JSONDecoder().decode(Room.self, from: Data(json.utf8))
    }
```

(If `RoomTransitionTests` already has a reusable room factory with a compatible signature, use it instead of `roomFixture` — do not duplicate. Check lines ~798, ~891, ~1131.)

- [ ] **Step 2: Run to verify the launch-selection test fails**

Run: `cd apps/CowchatMac && swift test --filter RoomTransitionTests/testLaunchSelectionPrefersLobbyOverPendingSetupRooms`
Expected: FAIL — selection lands on `stale-room` (setup-screen preference still in place).

- [ ] **Step 3: Implement the removals.**

`ChatStore.swift`:
1. Delete `@Published private(set) var roomSetupScreenIDs: Set<String> = []` (line 26) and every read/write listed in **Files** above. In `connect()` the initial selection becomes:

```swift
                let initial = rooms.first(where: { $0.name.lowercased() == "lobby" })
                    ?? rooms.first
```

2. In `createRoom`, keep the `setupRoomIDs` insert + save; delete the two `roomSetupScreenIDs` lines.
3. Delete `func completeRoomSetup(_ room: Room) async` entirely.
4. In `markSetupRoomReadyIfNeeded`, delete the two `roomSetupScreenIDs` lines; keep the rest.
5. In `reconcileLocalRoomPreferences` and `removeRoom` and `resetServerBackedState`, delete the `roomSetupScreenIDs` lines.
6. In `init`, after `localPreferences = …` / preference loads, add the one-time cleanup:

```swift
        // One-time cleanup: the setup-screen takeover was removed in the
        // onboarding redesign (spec 2026-08-07); stale persisted IDs would
        // otherwise sit orphaned forever.
        defaults.removeObject(forKey: RoomLocalPreferences.pendingSetupScreenRoomIDsKey)
        if let scope = connectionProfile.persistentIdentityScope {
            defaults.removeObject(
                forKey: "\(RoomLocalPreferences.pendingSetupScreenRoomIDsKey).\(scope)"
            )
        }
```

`RoomLocalPreferences.swift`: delete the `pendingSetupScreenRoomIDs` computed property and `savePendingSetupScreenRoomIDs`; **keep** `static let pendingSetupScreenRoomIDsKey` with a comment that it exists only for the init cleanup.

`ContentView.swift`:
1. Main-pane switch (lines 51-66) loses the middle branch:

```swift
                if let room = store.selectedRoom {
                    if room.name.localizedCaseInsensitiveCompare("lobby") == .orderedSame {
                        LobbyDashboardView(room: room, isSidebarVisible: $isSidebarVisible)
                            .id("lobby-\(room.id)")
                    } else {
                        ChatRoomView(room: room, isSidebarVisible: $isSidebarVisible)
                            .id(room.id)
                    }
                } else {
                    EmptyChatView()
                }
```

2. Delete the whole `private struct RoomSetupView` (lines 806-897).
3. `LobbyDashboardView.dashboardRooms` stops hiding agentless rooms (the filter existed for the takeover flow; it would hide the future General from the dashboard forever):

```swift
    private var dashboardRooms: [Room] {
        store.unarchivedRooms.filter { $0.id != room.id }
    }
```

`RoomLocalPreferencesTests.swift`: delete the `pendingSetupScreenRoomIDs` round-trip assertions (lines ~14, ~23).

`RoomTransitionTests.swift`: delete `testPendingSetupScreenResumesAcrossRelaunchAndDismissalPersists` and rewrite `testCollaboratorJoiningWhileSetupIsOpenTransitionsDirectlyToChat` to assert on the kept mechanism instead (drop `completeRoomSetup` calls and `roomSetupScreenIDs` assertions; keep assertions that `setupRoomIDs` empties and `roomReadyNotice` stays nil for the selected room when a collaborator appears). Any other compile error mentioning `roomSetupScreenIDs` or `completeRoomSetup` in tests: delete that assertion, keep the surrounding test if it still tests something real.

- [ ] **Step 4: Run the full suite**

Run: `cd apps/CowchatMac && swift test`
Expected: PASS, including both new tests.

- [ ] **Step 5: Commit** (from repo root)

```bash
git add -A apps/CowchatMac
git commit -m "feat(mac): remove the setup-screen takeover and its launch-selection trap"
```

---

### Task 4: Connect state — prompt as a live-signal room empty state

**Files:**
- Create: `apps/CowchatMac/Sources/CowchatMac/RoomConnectState.swift`
- Modify: `apps/CowchatMac/Sources/CowchatMac/ContentView.swift` — `ChatRoomView` (pane-state wiring, header caption, overlay)
- Modify: `apps/CowchatMac/Sources/CowchatMac/ChatStore.swift` — member polling covers the selected connect-state room
- Test: extend `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift`

**Interfaces:**
- Consumes: `RoomPaneState` (Task 1), `store.connectPrompt(for:)`, `SemanticColor`/`gallopText`/`.macAccessibleAction`/`GallopIconView` (existing).
- Produces: `RoomConnectStateView(roomName: String, prompt: String, variant: RoomPaneState.ConnectVariant)` and `ConnectTroubleshootingView()` (Task 7 reuses the latter). `ChatStore.selectedRoomAwaitingFirstAgent: Bool`.

- [ ] **Step 1: Write the failing test** — the polling hole: a failed `listAgents` during `select()` must be healed by the 2s poll even for a room not in `setupRoomIDs`. Add to `RoomTransitionTests.swift`:

```swift
    @MainActor
    func testSelectedAgentlessRoomIsPolledForMembersEvenWhenNotInSetupSet() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        let room = try roomFixture(id: "r-empty", name: "empty-room")
        connection.listedRooms = [room]
        await store.connect()
        await store.select(room: room)
        XCTAssertTrue(store.roomMembers.isEmpty)
        XCTAssertTrue(store.setupRoomIDs.isEmpty)
        XCTAssertTrue(store.selectedRoomAwaitingFirstAgent)

        // An agent connects between event pushes; only polling can see it.
        connection.agentsByRoom["r-empty"] = [
            AgentPresence(agentID: "other-agent", name: "codex")
        ]
        await store.pollSetupRoomReadiness()
        XCTAssertTrue(store.roomMembers.contains { $0.agentID == "other-agent" })
    }
```

(`AgentPresence(agentID:name:)` is valid — the struct's memberwise init in `Models.swift` defaults every other field.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd apps/CowchatMac && swift test --filter testSelectedAgentlessRoomIsPolledForMembersEvenWhenNotInSetupSet`
Expected: FAIL — `selectedRoomAwaitingFirstAgent` doesn't exist / members never refresh.

- [ ] **Step 3: Implement the ChatStore polling extension.**

```swift
    /// True while the selected room should render the connect state: non-lobby,
    /// connected, nothing said yet, nobody else here. Keeps the 2s poll honest
    /// about who is actually in the room (spec §3 "member-truth refresh").
    var selectedRoomAwaitingFirstAgent: Bool {
        guard connectionStatus.isConnected,
              let room = selectedRoom,
              room.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame,
              messages.isEmpty, !isLoadingMessages else { return false }
        return !roomMembers.contains { $0.id != agentID }
    }
```

In `startSetupReadinessPolling` (line ~1120) widen both emptiness guards from `!setupRoomIDs.isEmpty` to `(!setupRoomIDs.isEmpty || selectedRoomAwaitingFirstAgent)` (three places: the entry guard, the loop condition, the post-poll break). In `pollSetupRoomReadiness` (line ~1145), append after the `setupRoomIDs` loop:

```swift
        if selectedRoomAwaitingFirstAgent { await refreshMembers() }
```

At the end of `select(room:)` (after `await transition.value`), add `startSetupReadinessPolling()` so entering a connect-state room starts the poll.

- [ ] **Step 4: Run the new test — verify it passes**

Run: `cd apps/CowchatMac && swift test --filter testSelectedAgentlessRoomIsPolledForMembersEvenWhenNotInSetupSet`
Expected: PASS.

- [ ] **Step 5: Create `RoomConnectState.swift`** (view code; verified by build + Task 10 manual pass):

```swift
import AppKit
import SwiftUI

/// The connect-prompt empty state an agentless room renders in place of the
/// old RoomSetupView takeover (spec §3). Dumb view: everything injected.
struct RoomConnectStateView: View {
    let roomName: String
    let prompt: String
    let variant: RoomPaneState.ConnectVariant

    @State private var hasCopiedPrompt = false
    @State private var showsSlowJoinHint = false
    @State private var isTroubleshootingPresented = false

    var body: some View {
        VStack(spacing: 22) {
            HStack(spacing: 14) {
                Image(systemName: "list.bullet.rectangle")
                Image(systemName: "arrow.right")
                Image(systemName: "sparkles")
            }
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(SemanticColor.iconPrimary)

            Text("Paste this prompt into an AI chatbot")
                .gallopText(.h5, color: SemanticColor.textPrimary)

            HStack(alignment: .bottom, spacing: 14) {
                Text(prompt)
                    .textSelection(.enabled)
                    .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                Button(hasCopiedPrompt ? "Copied" : "Copy") { copyPrompt() }
                    .buttonStyle(.plain)
                    .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .padding(.horizontal, 18)
                    .frame(height: 38)
                    .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
                    .macAccessibleAction(label: "Copy connect prompt", action: copyPrompt)
            }
            .padding(18)
            .frame(maxWidth: 620)
            .background(
                SemanticColor.surface600,
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(SemanticColor.borderDefault, lineWidth: 1)
            }

            statusLine

            if showsSlowJoinHint, variant == .connected {
                Text("First join can take a few minutes while your agent installs cowchat.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            }

            Button("Not connecting?") { isTroubleshootingPresented = true }
                .buttonStyle(.plain)
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .macAccessibleAction(label: "Connection troubleshooting") {
                    isTroubleshootingPresented = true
                }
                .popover(isPresented: $isTroubleshootingPresented) {
                    ConnectTroubleshootingView()
                        .frame(width: 360)
                        .padding(18)
                }
        }
        .padding(28)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: "\(roomName)-\(variant == .connected)") {
            showsSlowJoinHint = false
            guard variant == .connected else { return }
            try? await Task.sleep(nanoseconds: 60_000_000_000)
            if !Task.isCancelled { showsSlowJoinHint = true }
        }
    }

    private var statusLine: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(variant == .connected ? SemanticColor.success : SemanticColor.warning)
                .frame(width: 7, height: 7)
                .modifier(PulsingDot(active: variant == .connected))
            Text(statusText)
                .gallopText(.caption, color: SemanticColor.textTertiary)
        }
    }

    private var statusText: String {
        switch variant {
        case .connected: return "Connected — waiting for your first agent…"
        case .connecting: return "Connecting…"
        case .offline: return "Offline — reconnect before pasting."
        }
    }

    private func copyPrompt() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(prompt, forType: .string)
        hasCopiedPrompt = true
    }
}

/// Subtle opacity pulse for the waiting dot; no motion for static variants.
private struct PulsingDot: ViewModifier {
    let active: Bool
    @State private var dimmed = false

    func body(content: Content) -> some View {
        content
            .opacity(active && dimmed ? 0.35 : 1)
            .animation(
                active ? .easeInOut(duration: 1.1).repeatForever(autoreverses: true) : .default,
                value: dimmed
            )
            .onAppear { if active { dimmed = true } }
    }
}

/// The three real first-run failure modes, one line each (spec §5). Shared by
/// the connect state's popover and EmptyChatView's connection-failed variant.
struct ConnectTroubleshootingView: View {
    @State private var hasCopiedBrewCommand = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            item(
                title: "Older server on port 9229",
                body: "If you've installed cowchat with Homebrew before, an older server may be answering. Run the command below, then quit and reopen Cowchat."
            )
            HStack(spacing: 10) {
                Text("brew upgrade cowchat")
                    .gallopText(.code, color: SemanticColor.textSecondary)
                    .textSelection(.enabled)
                Button(hasCopiedBrewCommand ? "Copied" : "Copy") {
                    let pasteboard = NSPasteboard.general
                    pasteboard.clearContents()
                    pasteboard.setString("brew upgrade cowchat", forType: .string)
                    hasCopiedBrewCommand = true
                }
                .buttonStyle(.plain)
                .gallopText(.bodySStrong, color: SemanticColor.buttonSecondaryTextDefault)
                .macAccessibleAction(label: "Copy brew upgrade command") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString("brew upgrade cowchat", forType: .string)
                    hasCopiedBrewCommand = true
                }
            }
            item(
                title: "Private rooms need the key",
                body: "Agents must connect with the key at ~/.cowchat/auth.key to see a private room — or make the room public."
            )
            item(
                title: "Your agent needs internet",
                body: "Your agent fetches the Cowchat skill from cowchat.cowboy.inc — it needs internet even though Cowchat itself is local."
            )
        }
    }

    private func item(title: String, body: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
            Text(body)
                .gallopText(.caption, color: SemanticColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
```

(`.gallopText(.code, …)` is valid: `GallopTextStyle.code` is defined in `GallopTokens.swift:803` — mono, 14pt.)

- [ ] **Step 6: Wire `ChatRoomView`.** In `ContentView.swift`:

Add to `ChatRoomView`:

```swift
    private var paneState: RoomPaneState {
        RoomPaneState.state(
            connectionStatus: store.connectionStatus,
            isLoadingMessages: store.isLoadingMessages,
            hasMessages: !store.messages.isEmpty,
            hasOtherMembers: store.roomMembers.contains { $0.id != store.agentID }
        )
    }

    private var connectVariant: RoomPaneState.ConnectVariant? {
        if case .connectPrompt(let variant) = paneState { return variant }
        return nil
    }
```

In `body`'s `ZStack` (lines ~978-990), replace the quiet-room overlay condition:

```swift
                if let connectVariant {
                    RoomConnectStateView(
                        roomName: room.name,
                        prompt: store.connectPrompt(for: room),
                        variant: connectVariant
                    )
                } else if paneState == .quiet {
                    quietRoom
                        .allowsHitTesting(true)
                }
```

(Keep the existing loading `ProgressView` condition and the composer untouched; the composer stays available in both empty states.)

In `chatHeader`, the caption line becomes:

```swift
                Text(connectVariant != nil ? "No agents here yet" : presenceSummary)
                    .gallopText(
                        .caption,
                        color: connectVariant == nil && presenceSummary.contains("active")
                            ? SemanticColor.warning : SemanticColor.textTertiary
                    )
                    .lineLimit(1)
```

- [ ] **Step 7: Build + full suite**

Run: `cd apps/CowchatMac && swift build && swift test`
Expected: builds; all tests PASS.

- [ ] **Step 8: Commit** (from repo root)

```bash
git add -A apps/CowchatMac
git commit -m "feat(mac): connect prompt as a live-signal room empty state"
```

---

### Task 5: Splash rebuild (explain-only, one button)

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/CowchatOnboarding.swift` (rebuild view; delete prompt/sheet)
- Modify: `apps/CowchatMac/Sources/CowchatMac/CowchatMacApp.swift:124-129` (onComplete no longer presents Create Room)
- Test: `apps/CowchatMac/Tests/CowchatMacTests/CowchatOnboardingTests.swift`

**Interfaces:**
- Consumes: `CapsulePillButtonStyle` (already private in this file — stays), `CowchatAppDelegate.applicationIcon()`.
- Produces: `CowchatOnboardingView(onComplete:)` unchanged signature. `CowchatOnboarding.collaborationPrompt` is **deleted** — after this task the room prompt (`ChatStore.connectPromptText`) is the app's only prompt string.

- [ ] **Step 1: Update the tests first.** In `CowchatOnboardingTests.swift`, delete `testPromptUsesCurrentCowchatNameAndCanonicalSkillURL` (the constant it locks is being deleted). Keep the four migration tests untouched — migration semantics are locked and unchanged.

- [ ] **Step 2: Run to verify the suite fails**

Run: `cd apps/CowchatMac && swift test --filter CowchatOnboardingTests`
Expected: PASS for the four kept tests (deletion first is safe) — then proceed; the compile failure comes when the constant is deleted and any leftover reference breaks.

- [ ] **Step 3: Rebuild the view.** Replace `CowchatOnboarding.swift`'s enum + view (keep `CapsulePillButtonStyle` at the bottom of the file exactly as is):

```swift
import AppKit
import SwiftUI

enum CowchatOnboarding {
    static let currentVersion = 1
    static let completedVersionKey = "CowchatMac.completedOnboardingVersion"
    static let migrationAttemptedKey = "CowchatMac.onboardingMigrationAttempted"

    static func migrateExistingUser(defaults: UserDefaults, hadExistingAgentID: Bool) {
        guard defaults.object(forKey: migrationAttemptedKey) == nil else { return }
        defaults.set(true, forKey: migrationAttemptedKey)
        guard hadExistingAgentID,
              defaults.object(forKey: completedVersionKey) == nil else { return }
        defaults.set(currentVersion, forKey: completedVersionKey)
    }
}

struct CowchatOnboardingView: View {
    let onComplete: () -> Void

    private static let steps: [(icon: String, caption: String)] = [
        ("list.bullet.rectangle", "Copy the prompt from your first room"),
        ("arrow.right", "Paste it into an AI chatbot"),
        ("sparkles", "Watch your agents work together live"),
    ]

    var body: some View {
        ZStack {
            SemanticColor.surface500

            VStack(spacing: 28) {
                appIcon

                VStack(spacing: 8) {
                    Text("Howdy… Welcome to Cowchat!")
                        .gallopText(.h4, color: SemanticColor.textPrimary)
                    Text("Cowchat is a small chat server your agents connect to. They join rooms, send messages, and collaborate in real time.")
                        .gallopText(.bodyL, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 580)
                }

                HStack(alignment: .top, spacing: 28) {
                    ForEach(Array(Self.steps.enumerated()), id: \.offset) { _, step in
                        VStack(spacing: 10) {
                            Image(systemName: step.icon)
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(SemanticColor.iconPrimary)
                            Text(step.caption)
                                .gallopText(.caption, color: SemanticColor.textTertiary)
                                .multilineTextAlignment(.center)
                        }
                        .frame(width: 150)
                    }
                }

                Button {
                    onComplete()
                } label: {
                    Text("Get started")
                        .gallopText(.bodyMStrong)
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(CapsulePillButtonStyle(prominent: true))
                .macAccessibleAction(label: "Get started", action: onComplete)
            }
            .padding(54)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 900, minHeight: 600)
        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 1)
                .allowsHitTesting(false)
        }
        .ignoresSafeArea(.container, edges: .top)
    }

    private var appIcon: some View {
        Group {
            if let icon = CowchatAppDelegate.applicationIcon() {
                Image(nsImage: icon)
                    .resizable()
                    .scaledToFit()
            } else {
                Image(systemName: "bubble.left.and.bubble.right.fill")
                    .resizable()
                    .scaledToFit()
                    .padding(20)
                    .foregroundStyle(SemanticColor.iconPrimary)
            }
        }
        .frame(width: 88, height: 88)
        .background(
            SemanticColor.surface600,
            in: RoundedRectangle(cornerRadius: 22, style: .continuous)
        )
        .shadow(
            color: SemanticColor.surfaceGlassBorderShadow,
            radius: 18,
            y: 8
        )
        .accessibilityHidden(true)
    }
}
```

In `CowchatMacApp.swift`, the completion closure no longer opens the Create Room sheet:

```swift
                if completedOnboardingVersion < CowchatOnboarding.currentVersion {
                    CowchatOnboardingView {
                        completedOnboardingVersion = CowchatOnboarding.currentVersion
                    }
                } else {
```

(Leave the ⌘N `.disabled(…)` command and the `ContentView { completedOnboardingVersion = 0 }` replay hook exactly as they are.)

- [ ] **Step 4: Build + full suite** (a leftover `collaborationPrompt` reference anywhere fails the build — that's the point)

Run: `cd apps/CowchatMac && swift build && swift test`
Expected: PASS.

- [ ] **Step 5: Commit** (from repo root)

```bash
git add apps/CowchatMac/Sources/CowchatMac/CowchatOnboarding.swift apps/CowchatMac/Sources/CowchatMac/CowchatMacApp.swift apps/CowchatMac/Tests/CowchatMacTests/CowchatOnboardingTests.swift
git commit -m "feat(mac): explain-only splash — no prompt, no consent sheet, no forced room"
```

---

### Task 6: Default room "General" + lobby-name guard

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/ChatStore.swift` (`ensureDefaultRoomIfNeeded`, backfill, `createRoom`/`rename` name guard, call site in `connect()`)
- Modify: `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift` (pin legacy helper + new tests)

**Interfaces:**
- Consumes: `CowchatConnectionError.server(message:code:)` (Task 2), `CowchatOnboarding.completedVersionKey/currentVersion`.
- Produces: `ChatStore.didCreateDefaultRoomKey` / `ChatStore.defaultRoomBackfillKey` (static `String`s), `static func suppressDefaultRoomForExistingInstalls(defaults:)`. Room copy: name **General**, description **"Where your agents meet and work together"**, public, permanent.

- [ ] **Step 1: Pin legacy tests first.** `makeStore(connection:)` in `RoomTransitionTests.swift` uses fresh defaults, which after this task would make every legacy test a "fresh install" and hit the mock's `fatalError("set roomToCreate before creating")`. Add one line to `makeStore` (and `makeProfileSwitchingStore`):

```swift
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
```

Also audit `LocalServerIntegrationTests.swift` — if it constructs a `ChatStore` with fresh defaults and connects to a live server, pin the same flag there.

- [ ] **Step 2: Write the failing tests** (in `RoomTransitionTests.swift`; the fresh-install helper deliberately omits the pin):

```swift
    @MainActor
    private func makeFreshInstallStore(connection: MockRoomConnection) -> ChatStore {
        let suiteName = "RoomTransitionTests.fresh.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return ChatStore(connection: connection, defaults: defaults)
    }

    @MainActor
    func testFreshInstallAutoCreatesPublicGeneralAndSelectsIt() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby")]
        connection.roomToCreate = try roomFixture(id: "general-id", name: "General")
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertTrue(connection.operations.contains("create:General"))
        XCTAssertEqual(store.selectedRoomID, "general-id")
        XCTAssertTrue(store.setupRoomIDs.contains("general-id"))
    }

    @MainActor
    func testAutoCreateSkipsWhenUserRoomsExist() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [
            try roomFixture(id: "lobby", name: "lobby"),
            try roomFixture(id: "mine", name: "my-room"),
        ]
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertFalse(connection.operations.contains { $0.hasPrefix("create:") })
        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    @MainActor
    func testAutoCreateSelectsExistingVisibleGeneralInstead() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [
            try roomFixture(id: "lobby", name: "lobby"),
            try roomFixture(id: "existing-general", name: "general"),
        ]
        let store = makeFreshInstallStore(connection: connection)

        await store.connect()

        XCTAssertFalse(connection.operations.contains { $0.hasPrefix("create:") })
        XCTAssertEqual(store.selectedRoomID, "existing-general")
    }

    @MainActor
    func testAutoCreateRunsOnlyOnce() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby")]
        connection.roomToCreate = try roomFixture(id: "general-id", name: "General")
        let store = makeFreshInstallStore(connection: connection)
        await store.connect()

        // User destroyed General; a later connect must not resurrect it.
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby")]
        await store.connect()

        XCTAssertEqual(connection.operations.filter { $0 == "create:General" }.count, 1)
    }

    @MainActor
    func testNameTakenSetsGuardFlagInsteadOfRetryLooping() async throws {
        let connection = MockRoomConnection()
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby")]
        connection.createRoomError = CowchatConnectionError.server(
            message: "Room name 'General' already taken", code: "room_name_taken"
        )
        let suiteName = "RoomTransitionTests.taken.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        let store = ChatStore(connection: connection, defaults: defaults)

        await store.connect()

        XCTAssertTrue(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
        XCTAssertNil(store.errorMessage)  // silent fallback, no alert
        XCTAssertEqual(store.selectedRoomID, "lobby")
    }

    func testBackfillSuppressesAutoCreateForExistingInstalls() {
        let suiteName = "RoomTransitionTests.backfill.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        // Existing install: already past onboarding v1.
        defaults.set(CowchatOnboarding.currentVersion, forKey: CowchatOnboarding.completedVersionKey)

        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)

        XCTAssertTrue(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
        // And it never re-runs: a later reset of the guard flag sticks.
        defaults.removeObject(forKey: ChatStore.didCreateDefaultRoomKey)
        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)
        XCTAssertFalse(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
    }

    func testBackfillLeavesFreshInstallsEligible() {
        let suiteName = "RoomTransitionTests.backfill2.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)

        ChatStore.suppressDefaultRoomForExistingInstalls(defaults: defaults)

        XCTAssertFalse(defaults.bool(forKey: ChatStore.didCreateDefaultRoomKey))
    }

    @MainActor
    func testCreateAndRenameRejectLobbyNames() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby")]
        await store.connect()

        let created = await store.createRoom(
            name: "Lobby", description: "", ephemeral: false, isPublic: true
        )
        XCTAssertFalse(created)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertFalse(connection.operations.contains("create:Lobby"))
    }
```

The mock needs one new hook — add to `MockRoomConnection`:

```swift
    var createRoomError: Error?
```

and at the top of its `createRoom(…)`, after `operations.append`:

```swift
        if let createRoomError { throw createRoomError }
```

- [ ] **Step 3: Run to verify the new tests fail**

Run: `cd apps/CowchatMac && swift test --filter RoomTransitionTests`
Expected: new tests FAIL to compile (`didCreateDefaultRoomKey` etc. undefined); legacy tests still pass once the Step-1 pin exists (the flag key must be defined before the pin compiles — define the two static keys first if running strictly in order, or accept one combined red step).

- [ ] **Step 4: Implement.** In `ChatStore.swift`:

Static keys + backfill (place near `resolveAgentID`):

```swift
    static let didCreateDefaultRoomKey = "CowchatMac.didCreateDefaultRoom"
    static let defaultRoomBackfillKey = "CowchatMac.defaultRoomBackfillAttempted"

    /// One-shot: installs already past onboarding v1 never get an unrequested
    /// auto-created room (spec §2 — retroactive creation was reviewed out).
    /// Runs after migrateExistingUser so migrated users are stamped by the
    /// time it reads the completed version.
    static func suppressDefaultRoomForExistingInstalls(defaults: UserDefaults) {
        guard !defaults.bool(forKey: defaultRoomBackfillKey) else { return }
        defaults.set(true, forKey: defaultRoomBackfillKey)
        if defaults.integer(forKey: CowchatOnboarding.completedVersionKey)
            >= CowchatOnboarding.currentVersion {
            defaults.set(true, forKey: didCreateDefaultRoomKey)
        }
    }
```

In `init`, immediately after the `CowchatOnboarding.migrateExistingUser(…)` call:

```swift
        Self.suppressDefaultRoomForExistingInstalls(defaults: defaults)
```

The creation chain (private func; spec §2 ordered chain):

```swift
    private func ensureDefaultRoomIfNeeded() async {
        guard connectionProfile.kind == .local,
              !defaults.bool(forKey: Self.didCreateDefaultRoomKey) else { return }
        let expectedProfileGeneration = profileGeneration

        if let general = rooms.first(where: {
            $0.name.localizedCaseInsensitiveCompare("General") == .orderedSame
        }) {
            defaults.set(true, forKey: Self.didCreateDefaultRoomKey)
            await select(room: general)
            return
        }
        let hasUserRooms = rooms.contains {
            $0.name.localizedCaseInsensitiveCompare("lobby") != .orderedSame
        }
        guard !hasUserRooms, !rooms.isEmpty else {
            // CLI-first user (or an anomalous empty list): normal selection
            // stands. An empty list still sets the flag — the seeded lobby is
            // always present on a healthy server, so this state is not "fresh".
            defaults.set(true, forKey: Self.didCreateDefaultRoomKey)
            return
        }
        do {
            let room = try await connection.createRoom(
                name: "General",
                description: "Where your agents meet and work together",
                parentID: nil,
                ephemeral: false,
                isPublic: true
            )
            guard expectedProfileGeneration == profileGeneration else { return }
            if !rooms.contains(where: { $0.id == room.id }) {
                rooms.append(room)
                recordRoomMutation(roomID: room.id)
            }
            rooms.sort(by: roomSort)
            setupRoomIDs.insert(room.id)
            localPreferences.savePendingSetupRoomIDs(setupRoomIDs)
            defaults.set(true, forKey: Self.didCreateDefaultRoomKey)
            await select(room: room)
        } catch {
            guard expectedProfileGeneration == profileGeneration else { return }
            if let connectionError = error as? CowchatConnectionError,
               case .server(_, .some("room_name_taken")) = connectionError {
                // The name is squatted by a room this key can't see; retrying
                // can never succeed. Silent fallback to normal selection.
                defaults.set(true, forKey: Self.didCreateDefaultRoomKey)
            }
            // Transient failure: flag stays unset; the next launch retries once.
        }
    }
```

Call site in `connect(expectedProfile:…)` — after the initial-selection `if/else` block and its generation guard, **before** `startSetupReadinessPolling()`:

```swift
            await ensureDefaultRoomIfNeeded()
            guard expectedProfileGeneration == profileGeneration else { return }
            startSetupReadinessPolling()
```

Name guard in `createRoom` (before the `connection.createRoom` call) and in `rename` (after the empty check):

```swift
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmedName.localizedCaseInsensitiveCompare("lobby") != .orderedSame else {
            errorMessage = "\u{201C}lobby\u{201D} is reserved for the Lobby dashboard. Choose another name."
            return false
        }
```

(In `createRoom`, pass `trimmedName` to the connection instead of re-trimming inline; in `rename` the trimmed `name` local already exists — add the same guard there against `name`.)

- [ ] **Step 5: Run the full suite**

Run: `cd apps/CowchatMac && swift test`
Expected: PASS — all new tests plus every legacy test (the Step-1 pin keeps them out of the auto-create path).

- [ ] **Step 6: Commit** (from repo root)

```bash
git add -A apps/CowchatMac
git commit -m "feat(mac): auto-create public General room for fresh installs"
```

---

### Task 7: Failure visibility — spawn errors alert + failed empty state

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/ChatStore.swift:288-293` (connect catch)
- Modify: `apps/CowchatMac/Sources/CowchatMac/ContentView.swift` (`EmptyChatView`, lines ~1728-1805)
- Test: `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift`

**Interfaces:**
- Consumes: `LocalServerSupervisorError` (`LocalServerSupervisor.swift:4-27`), `ConnectTroubleshootingView` (Task 4), `store.reconnect` (existing).
- Produces: latched spawn failures set `store.errorMessage` → the existing `ContentView` alert fires. `EmptyChatView` gains a connection-failed variant.

- [ ] **Step 1: Write the failing test:**

```swift
    @MainActor
    func testBundledServerFailureSurfacesInErrorMessage() async {
        let connection = MockRoomConnection()
        let supervisor = MockLocalServerSupervisor()
        supervisor.launchError = LocalServerSupervisorError.launchFailed("spawn denied")
        connection.connectFailuresRemaining = 99
        let suiteName = "RoomTransitionTests.spawnfail.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set(true, forKey: ChatStore.didCreateDefaultRoomKey)
        let store = ChatStore(
            connection: connection,
            defaults: defaults,
            localServerSupervisor: supervisor,
            localServerRetryDelaysNanoseconds: []
        )

        await store.connect()

        XCTAssertNotNil(store.errorMessage)
        XCTAssertTrue(store.errorMessage?.contains("spawn denied") == true)
    }
```

(`MockLocalServerSupervisor` already exists in this file — check its property for injecting a launch error; if it's named differently than `launchError`, use its actual name, and add the property if absent: `var launchError: Error?` thrown at the top of `launchIfNeeded()`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd apps/CowchatMac && swift test --filter testBundledServerFailureSurfacesInErrorMessage`
Expected: FAIL — `errorMessage` is nil (connect path never sets it).

- [ ] **Step 3: Implement.** In `connect(expectedProfile:…)`'s catch block:

```swift
        } catch {
            guard expectedProfileGeneration == profileGeneration,
                  connectionProfile == expectedProfile else { return }
            connectionStatus = .failed(error.localizedDescription)
            // Spawn/launch failures are latched (no background retry can fix
            // them) — surface them on the alert instead of a footer tooltip.
            if error is LocalServerSupervisorError {
                errorMessage = error.localizedDescription
            }
            scheduleReconnect()
        }
```

In `EmptyChatView`, the `store.rooms.isEmpty` branch becomes:

```swift
            if store.rooms.isEmpty {
                if case .failed = store.connectionStatus {
                    connectionFailedState
                } else {
                    // Centered welcome IS the empty state, with a direct path to
                    // the first room (Patrick, 2026-08-06).
                    VStack(spacing: 20) {
                        welcome(alignment: .center)
                        // …existing New room button unchanged…
                    }
                    .padding(24)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            } else {
```

with the new variant (spec §5: status, Reconnect, troubleshooting inline):

```swift
    private var connectionFailedState: some View {
        VStack(spacing: 18) {
            GallopIconView(icon: .retry, fallbackSystemName: "wifi.slash", size: 24)
                .foregroundStyle(SemanticColor.iconTertiary)
            Text("Can't reach the local server")
                .gallopText(.h5, color: SemanticColor.textPrimary)
            Text(store.connectionStatus.failureMessage ?? "Connection failed")
                .gallopText(.caption, color: SemanticColor.textTertiary)
                .multilineTextAlignment(.center)

            Button {
                store.reconnect()
            } label: {
                Text("Reconnect")
                    .gallopText(.bodyMStrong, color: SemanticColor.buttonPrimaryTextDefault)
                    .padding(.horizontal, 20)
                    .frame(height: 38)
                    .background(SemanticColor.buttonPrimaryDefault, in: Capsule())
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Reconnect", action: store.reconnect)

            ConnectTroubleshootingView()
                .frame(maxWidth: 420)
                .padding(.top, 8)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
```

- [ ] **Step 4: Run the full suite**

Run: `cd apps/CowchatMac && swift test`
Expected: PASS.

- [ ] **Step 5: Commit** (from repo root)

```bash
git add -A apps/CowchatMac
git commit -m "feat(mac): surface connection failures — alert on spawn errors, failed empty state with troubleshooting"
```

---

### Task 8: Second-agent bridge hint

**Files:**
- Modify: `apps/CowchatMac/Sources/CowchatMac/ChatStore.swift` (published hint + trigger in `markSetupRoomReadyIfNeeded` + resets)
- Modify: `apps/CowchatMac/Sources/CowchatMac/ContentView.swift` (notice view + overlay)
- Test: `apps/CowchatMac/Tests/CowchatMacTests/RoomTransitionTests.swift`

**Interfaces:**
- Consumes: `markSetupRoomReadyIfNeeded` (existing), `RoomReadyNotice` glass recipe (copied, not shared — it's a small private struct).
- Produces: `ChatStore.secondAgentHintRoom: Room?` (published; nil = hidden). Copy: **"Add more agents anytime — Copy connect prompt in the ⋯ menu."**

- [ ] **Step 1: Write the failing test:**

```swift
    @MainActor
    func testFirstCollaboratorInSelectedRoomShowsSecondAgentHintOnce() async throws {
        let connection = MockRoomConnection()
        let store = makeStore(connection: connection)
        let room = try roomFixture(id: "r1", name: "my-room")
        connection.listedRooms = [try roomFixture(id: "lobby", name: "lobby"), room]
        await store.connect()
        _ = await store.createRoom(name: "my-room", description: "", ephemeral: false, isPublic: true)
        // createRoom in the mock needs roomToCreate set:
        // (set connection.roomToCreate = room before the call above)

        // Collaborator appears while the room is selected → in-place flip.
        connection.agentsByRoom["r1"] = [AgentPresence(agentID: "other", name: "codex")]
        await store.pollSetupRoomReadiness()

        XCTAssertEqual(store.secondAgentHintRoom?.id, "r1")
        XCTAssertNil(store.roomReadyNotice)

        // Once per room: re-entering the setup state can't re-trigger it.
        store.secondAgentHintRoom = nil
        connection.agentsByRoom["r1"] = []
        await store.pollSetupRoomReadiness()
        connection.agentsByRoom["r1"] = [AgentPresence(agentID: "other", name: "codex")]
        await store.pollSetupRoomReadiness()
        XCTAssertNil(store.secondAgentHintRoom)
    }
```

(Set `connection.roomToCreate = room` before the `createRoom` call — the inline comment marks the spot. `AgentPresence(agentID:name:)` is valid; see Task 4.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd apps/CowchatMac && swift test --filter testFirstCollaboratorInSelectedRoomShowsSecondAgentHintOnce`
Expected: FAIL — `secondAgentHintRoom` undefined.

- [ ] **Step 3: Implement store side.**

```swift
    @Published var secondAgentHintRoom: Room?
    private var secondAgentHintShownRoomIDs: Set<String> = []
```

`markSetupRoomReadyIfNeeded` last line becomes:

```swift
        if selectedRoomID != roomID {
            roomReadyNotice = room
        } else if secondAgentHintShownRoomIDs.insert(roomID).inserted {
            secondAgentHintRoom = room
        }
```

Reset both in `resetServerBackedState` (`secondAgentHintRoom = nil`, `secondAgentHintShownRoomIDs = []`) and clear the hint in `removeRoom` (`if secondAgentHintRoom?.id == roomID { secondAgentHintRoom = nil }`).

- [ ] **Step 4: Implement view side.** In `ContentView`'s overlay (next to `RoomReadyNotice`, lines ~150-158) — the two are mutually exclusive by construction:

```swift
        .overlay(alignment: .bottomTrailing) {
            if let room = store.roomReadyNotice {
                RoomReadyNotice(room: room)
                    .environmentObject(store)
                    .padding(18)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            } else if store.secondAgentHintRoom != nil {
                SecondAgentHintNotice()
                    .environmentObject(store)
                    .padding(18)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeInOut(duration: 0.2), value: store.roomReadyNotice?.id)
        .animation(.easeInOut(duration: 0.2), value: store.secondAgentHintRoom?.id)
```

New view (below `RoomReadyNotice`; copy its glass background block verbatim):

```swift
private struct SecondAgentHintNotice: View {
    @EnvironmentObject private var store: ChatStore

    var body: some View {
        HStack(spacing: 14) {
            Text("Add more agents anytime — Copy connect prompt in the ⋯ menu.")
                .gallopText(.bodySStrong, color: SemanticColor.textPrimary)
            Button {
                store.secondAgentHintRoom = nil
            } label: {
                GallopIconView(icon: .dismiss, fallbackSystemName: "xmark", size: 11)
                    .foregroundStyle(SemanticColor.iconTertiary)
            }
            .buttonStyle(.plain)
            .macAccessibleAction(label: "Dismiss hint") {
                store.secondAgentHintRoom = nil
            }
        }
        .padding(14)
        .background {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(SemanticColor.surfaceGlass500)
                }
                .overlay {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .stroke(SemanticColor.surfaceGlassBorderHighlight, lineWidth: 1)
                }
        }
        .shadow(color: .black.opacity(0.08), radius: 2, y: 1)
        .shadow(color: .black.opacity(0.04), radius: 0, y: 0.5)
        .task {
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            if !Task.isCancelled { store.secondAgentHintRoom = nil }
        }
    }
}
```

- [ ] **Step 5: Run the full suite**

Run: `cd apps/CowchatMac && swift test`
Expected: PASS.

- [ ] **Step 6: Commit** (from repo root)

```bash
git add -A apps/CowchatMac
git commit -m "feat(mac): one-time hint bridging to the second agent after first join"
```

---

### Task 9: Site — one prompt everywhere

**Files:**
- Modify: `site/app/page.tsx` (`PROMPT` constant line 3; step-2 body lines ~80-86)

**Interfaces:**
- Consumes: nothing. `site/public/skills.txt` publishing untouched.
- Produces: site prompt naming room "General" with create-public clause. Curly quotes around General exactly as shown.

- [ ] **Step 1: Update `PROMPT`** (copy-exact, including curly quotes around General and the straight apostrophes):

```tsx
const PROMPT = `You're going to collaborate with another AI chatbot in real time over Cowchat. Read the Cowchat skill, connect to the local server, join the exact room \u{201C}General\u{201D} (create it as a public room if it doesn't exist), start listening right away (don't wait for me to confirm), and give me a prompt I can paste into the other bot. https://cowchat.cowboy.inc/skills.txt`;
```

(Template literals don't process `\u{…}` differently than strings — both forms work; alternatively paste the literal “General” characters directly. Verify the rendered page shows “General” with curly quotes.)

- [ ] **Step 2: Update step-2 body copy** (draft per spec — flagged for Patrick's live review):

```tsx
            <p className="type-body-m mt-2 text-text-secondary">
              Paste the prompt below into one chatbot; it reads the skills file,
              joins your General room, and prints the prompt for your second
              agent. Anything that opens a socket and writes JSON can join.
            </p>
```

- [ ] **Step 3: Verify the build and the string**

Run: `cd site && npm run build && grep -c "General" out/index.html`
Expected: build succeeds; grep count ≥ 2 (prompt + step-2 copy). Also confirm: `grep "create it as a public room" out/index.html` matches.

- [ ] **Step 4: Cross-check app↔site prompt shape** (manual, the site has no test harness): the site prompt must contain the exact substrings `join the exact room “General”`, `https://cowchat.cowboy.inc/skills.txt`, and the chaining sentence `give me a prompt I can paste into the other bot` — compare against `ChatStore.connectPromptText(roomName: "General", connectionInstruction: "connect to the local server")` output: identical up to the added parenthetical and chaining sentence.

- [ ] **Step 5: Commit** (from repo root)

```bash
git add site/app/page.tsx
git commit -m "feat(site): connection prompt names the General room, created public if missing"
```

---

### Task 10: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Full app suite + build**

Run: `cd apps/CowchatMac && swift build && swift test`
Expected: all tests PASS, no warnings introduced in changed files.

- [ ] **Step 2: Site build**

Run: `cd site && npm run build`
Expected: static export succeeds.

- [ ] **Step 3: Manual dev-loop pass** (needs the workspace server: `cargo build -p cowchat-server && ./target/debug/cowchat-server serve` from the repo root, and a fake short HOME per the dev-loop memory — deep scratchpad paths break the Unix socket):

1. Fresh HOME first run: splash shows (explain-only, Get started) → main window lands in **General** showing the connect state with the pulsing "Connected — waiting for your first agent…" line.
2. Connect a Python agent (`examples/python/cowchat.py`) to room General → the connect state flips to live chat in place, **no** ready toast, and the one-time "Add more agents anytime…" hint appears then auto-dismisses.
3. Repeat with the Lobby selected while the agent joins a second room → the "{room} is ready" toast fires.
4. Quit and relaunch with an agentless created room → lands on the Lobby dashboard (not the connect state), and the agentless room IS visible in the dashboard grid.
5. Kill the workspace server, occupy 9229 with something that refuses the protocol (or nothing at all in a dev build, which lacks the bundled helper) → the failed empty state shows "Can't reach the local server" + Reconnect + the three troubleshooting items; for the missing-helper case the alert also fires.
6. Settings → "Show onboarding again" → splash replays; completing it does NOT create a second General.
7. Screenshot each state via the window-ID `screencapture` recipe for Patrick's copy review.

- [ ] **Step 4: Final commit if the manual pass produced fixes; otherwise done.**

## Deliberately not in this plan (spec'd out or deferred)

- The optional "agent online but not in this room" hint (spec marks it implementer-judgment on an already-available signal; the lobby's available-agents count is selected-room members, so no such global signal exists — skipped).
- Dark theme, vote/election UI, skills.txt changes, server-side changes, clawchat.live redirect.
- Final copy is subject to Patrick's live review pass over the built app (spec Copy summary table is the source).
