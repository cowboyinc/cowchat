import SwiftUI
import CowchatKit

struct RoomView: View {
    @EnvironmentObject private var session: ChatSession
    @EnvironmentObject private var store: ConnectionsStore
    @EnvironmentObject private var navigator: Navigator
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    let roomID: String

    @State private var showSecretSheet = false
    @State private var confirmsRemove = false
    @State private var confirmsRemoveSecret = false
    @State private var confirmsLeave = false
    @State private var isAddingPerson = false
    @State private var showsPeople = false
    @State private var secretError: String?

    // Its own property: built inline in the modifier chain, this binding made the view too slow to type-check.
    private var showsSecretError: Binding<Bool> {
        Binding(get: { secretError != nil }, set: { if !$0 { secretError = nil } })
    }
    @State private var isRenaming = false
    @State private var newName = ""
    @State private var invite: InviteToShare?
    @State private var errorMessage: String?

    private var showsError: Binding<Bool> {
        Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })
    }
    // Held, not observed: only the pill redraws when the scroll position changes. If the whole room redrew,
    // the lazy stack would lay out again, move the bottom marker, and loop.
    @State private var scroll = ScrollTracker()
    @State private var openRuns: Set<String> = []
    @State private var now = Date()

    private var ref: RoomRef { RoomRef(connectionID: session.connectionID, roomID: roomID) }
    private var canRemove: Bool { store.canRemove(ref) }
    private var room: RoomSummary? { session.rooms.first { $0.id == roomID } }
    private var timeline: RoomTimeline { session.timelines[roomID] ?? RoomTimeline() }
    private var showsAll: Bool { session.showAllRoomIDs.contains(roomID) }
    private var feed: [FeedItem] { TimelineFeed.items(timeline.entries, showsAll: showsAll, openRuns: openRuns) }
    private var presence: String {
        PresenceLine.summary(members: session.members[roomID] ?? [], myAgentID: session.myAgentID, phase: session.phase)
    }
    private var needsSecret: Bool { (room?.isEncrypted ?? false) && !(room?.hasSecret ?? false) }

    // The screen is built in three steps. As one chain of some twenty modifiers it took the compiler long enough
    // to type-check that a slower build machine gave up on it.
    var body: some View {
        withSheets
            .task { await session.open(roomID: roomID) }
            .task {
                // Thinking signals expire on their own, so the feed re-reads the clock.
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(15))
                    now = Date()
                }
            }
            .onChange(of: session.pulses[roomID]) { now = Date() }
            .onDisappear { session.close(roomID: roomID) }
    }

    /// The feed, the composer and the toolbar.
    private var screen: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 20) {
                    if timeline.hasEarlier {
                        Button("Load earlier") { Task { await session.loadEarlier(roomID: roomID) } }
                            .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                            .disabled(timeline.isLoading)
                            .frame(maxWidth: .infinity)
                    }
                    let anchors = session.pulseAnchors(roomID: roomID, now: now)
                    ForEach(feed) { item in
                        switch item {
                        case .entry(let entry, let quiet):
                            // One view per row: a lazy stack re-lays out endlessly when rows vary in view count.
                            VStack(alignment: .leading, spacing: 8) {
                                EntryView(entry: entry, roomID: roomID).opacity(quiet ? 0.6 : 1)
                                ForEach(anchors.byMessageID[entry.id] ?? [], id: \.agentID) { PulsePill(pulse: $0) }
                            }
                        case .run(let id, let count, let isOpen):
                            Button {
                                if isOpen { openRuns.remove(id) } else { openRuns.insert(id) }
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: isOpen ? "chevron.down" : "chevron.right").font(.caption2)
                                    Text(TimelineFeed.runTitle(count: count)).gallopText(.caption)
                                }
                                .foregroundStyle(SemanticColor.textTertiary)
                            }
                            .accessibilityIdentifier("room.agentRun")
                        }
                    }
                    ForEach(anchors.unanchored, id: \.agentID) { PulsePill(pulse: $0) }
                    Color.clear.frame(height: 1).id(bottomID)
                        .onAppear { scroll.reachedBottom() }
                        .onDisappear { scroll.leftBottom(count: timeline.entries.count) }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
            }
            .background(SemanticColor.surface500.ignoresSafeArea())
            .defaultScrollAnchor(.bottom)
            .accessibilityIdentifier("room.timeline")
            // A room opens on its newest message, and turning the device keeps a reader who was there, there.
            .onAppear { DispatchQueue.main.async { proxy.scrollTo(bottomID, anchor: .bottom) } }
            .onChange(of: timeline.isLoading) { _, isLoading in
                if !isLoading, scroll.wasAtBottomRecently { proxy.scrollTo(bottomID, anchor: .bottom) }
            }
            .onReceive(NotificationCenter.default.publisher(for: UIDevice.orientationDidChangeNotification)) { _ in
                guard scroll.wasAtBottomRecently else { return }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { proxy.scrollTo(bottomID, anchor: .bottom) }
            }
            .onChange(of: scenePhase) { _, phase in
                // The system hides the views of a backgrounded app, which is not the reader scrolling away.
                scroll.isTracking = phase == .active
                if phase == .active, scroll.atBottom { proxy.scrollTo(bottomID, anchor: .bottom) }
            }
            .overlay(alignment: .bottom) {
                NewMessagesPill(scroll: scroll, count: timeline.entries.count) {
                    withAnimation { proxy.scrollTo(bottomID, anchor: .bottom) }
                }
            }
            .onChange(of: timeline.entries.last?.id) {
                // Your own message always comes into view; other people's only if you were at the bottom.
                if scroll.atBottom || timeline.entries.last?.isMine == true {
                    withAnimation { proxy.scrollTo(bottomID, anchor: .bottom) }
                }
            }
        }
        .navigationTitle(room?.name ?? "Room")
        .navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                if needsSecret {
                    Button { showSecretSheet = true } label: {
                        HStack(spacing: 8) {
                            GallopIconView(icon: .lock, fallbackSystemName: "lock.fill", size: 16)
                            Text("Enter room secret")
                        }
                    }
                    .buttonStyle(GallopButtonStyle(kind: .secondary))
                    .padding(.horizontal, 16)
                    .padding(.top, 8)
                    .background(SemanticColor.surface500.ignoresSafeArea())
                }
                ComposerView(roomID: roomID)
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Button { showsPeople = true } label: {
                    VStack(spacing: 0) {
                        Text(room?.name ?? "Room")
                            .gallopText(.bodyLStrong, color: SemanticColor.textPrimary)
                            .lineLimit(1)
                        Text(presence)
                            .gallopText(.caption, color: SemanticColor.textTertiary)
                            .lineLimit(1)
                            .accessibilityIdentifier("room.presence")
                    }
                }
                .accessibilityIdentifier("room.header")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Toggle("Show all messages", isOn: Binding(
                        get: { showsAll },
                        set: { session.setShowsAllMessages($0, roomID: roomID) }
                    ))
                    Divider()
                    Button("People") { showsPeople = true }
                    Button("Share invite") { shareInvite() }
                    Divider()
                    if room?.isEncrypted == true {
                        Button("Change secret") { showSecretSheet = true }
                        if room?.hasSecret == true {
                            Button("Remove secret", role: .destructive) { confirmsRemoveSecret = true }
                        }
                    }
                    if session.canRename(roomID: roomID) {
                        Button("Rename room") {
                            newName = room?.name ?? ""
                            isRenaming = true
                        }
                    }
                    if roomID != "lobby" {
                        if room?.isArchived == true {
                            Button("Unarchive room") { session.setArchived(false, roomID: roomID) }
                        } else {
                            Button("Archive room") {
                                session.setArchived(true, roomID: roomID)
                                navigator.roomPath = []
                            }
                        }
                    }
                    if let account = store.account(for: session.connectionID), account.canInvite(roomID: roomID) {
                        Button("Add person") { isAddingPerson = true }
                    }
                    if canRemove {
                        Button("Remove room", role: .destructive) { confirmsRemove = true }
                    }
                    if store.canLeave(ref) {
                        Button("Leave room", role: .destructive) { confirmsLeave = true }
                    }
                } label: {
                    GallopIconView(icon: .ellipsis, fallbackSystemName: "ellipsis", size: 20)
                        .foregroundStyle(SemanticColor.iconSecondary)
                }
                .accessibilityLabel("More")
                .accessibilityIdentifier("room.more")
            }
        }
    }

    /// The screen with its confirmations and alerts.
    private var withDialogs: some View {
        let removeTitle: String = "Remove \(room?.name ?? "")?"
        let leaveTitle: String = "Leave \(room?.name ?? "")?"
        let secretErrorText: String = secretError ?? ""
        let errorText: String = errorMessage ?? ""
        return screen
        .confirmationDialog("Remove the secret from this phone?", isPresented: $confirmsRemoveSecret, titleVisibility: .visible) {
            Button("Remove secret", role: .destructive) {
                do { try session.setSecret(nil, roomID: roomID) } catch { secretError = error.localizedDescription }
            }
        }
        .alert(secretErrorText, isPresented: showsSecretError) {
            Button("OK", role: .cancel) {}
        }
        .confirmationDialog(removeTitle, isPresented: $confirmsRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                Task {
                    await store.remove(room: ref)
                    dismiss()
                }
            }
            .accessibilityIdentifier("room.confirmRemove")
        }
        .confirmationDialog(leaveTitle, isPresented: $confirmsLeave, titleVisibility: .visible) {
            Button("Leave", role: .destructive) {
                Task {
                    await store.leave(ref)
                    if !store.rooms.contains(where: { $0.ref == ref }) { dismiss() }
                }
            }
            .accessibilityIdentifier("room.confirmLeave")
        }
        .alert("Rename room", isPresented: $isRenaming) {
            TextField("Room name", text: $newName)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            Button("Rename") { rename() }
            Button("Cancel", role: .cancel) {}
        }
        .alert(errorText, isPresented: showsError) {
            Button("OK", role: .cancel) {}
        }
    }

    /// The screen with the sheets it opens.
    private var withSheets: some View {
        withDialogs
            .sheet(isPresented: $isAddingPerson) {
                if let account = store.account(for: session.connectionID) {
                    AddPersonSheet(account: account, roomID: roomID)
                }
            }
            .sheet(isPresented: $showSecretSheet) { RoomSecretSheet(roomID: roomID) }
            .sheet(isPresented: $showsPeople) { RoomPeopleSheet(roomID: roomID) }
            .sheet(item: $invite) { ShareSheet(text: $0.token).presentationDetents([.medium, .large]) }
    }

    private func shareInvite() {
        Task {
            do {
                invite = InviteToShare(token: try await session.createInvite(roomID: roomID))
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    private func rename() {
        Task {
            do {
                try await session.rename(roomID: roomID, to: newName)
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }

    private let bottomID = "timeline.bottom"
}

@MainActor
final class ScrollTracker: ObservableObject {
    @Published private(set) var atBottom = true
    private(set) var countWhenLeft = 0
    /// False while the app is not in front, when views disappear for reasons other than scrolling.
    var isTracking = true

    func reachedBottom() {
        if !atBottom { atBottom = true }
    }

    private var leftAt = Date.distantPast

    /// True at the bottom, and for a moment after leaving it: a rotation or a reload moves the marker off screen
    /// before anyone has scrolled.
    var wasAtBottomRecently: Bool { atBottom || Date().timeIntervalSince(leftAt) < 1.5 }

    func leftBottom(count: Int) {
        guard atBottom, isTracking else { return }
        leftAt = Date()
        countWhenLeft = count
        atBottom = false
    }
}

private struct NewMessagesPill: View {
    @ObservedObject var scroll: ScrollTracker
    let count: Int
    let jump: () -> Void

    private var newCount: Int { scroll.atBottom ? 0 : max(0, count - scroll.countWhenLeft) }

    var body: some View {
        if newCount > 0 {
            Button(newCount == 1 ? "1 new message" : "\(newCount) new messages", action: jump)
                .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                .padding(.bottom, 8)
                .accessibilityIdentifier("room.newMessages")
        }
    }
}

private struct PulsePill: View {
    let pulse: AgentThinkingPulse

    var body: some View {
        Text(ThinkingPulsePresentation.label(for: pulse))
            .gallopText(.caption, color: SemanticColor.textSecondary)
            .lineLimit(2)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(SemanticColor.surface600, in: Capsule())
            .padding(.leading, 34)
            .accessibilityIdentifier("room.pulse")
    }
}

/// A long message shows its first lines until asked for the rest.
private struct CollapsingBody: View {
    let body_: TimelineEntry.Body
    @State private var isExpanded = false

    var body: some View {
        if case .text(let content) = body_, MessagePreview.needsDisclosure(for: content) {
            VStack(alignment: .leading, spacing: 8) {
                MessageBodyView(body_: isExpanded ? body_ : .text(MessagePreview.collapsedPreview(for: content).source))
                Button(isExpanded ? "Hide full response" : "Show full response") { isExpanded.toggle() }
                    .buttonStyle(GallopLinkButtonStyle())
                    .accessibilityIdentifier("message.disclosure")
            }
        } else {
            MessageBodyView(body_: body_)
        }
    }
}

/// What a message carries besides its words.
private struct EntryExtras: View {
    let entry: TimelineEntry
    let roomID: String

    var body: some View {
        if let pullRequest = entry.pullRequest { PullRequestRow(pullRequest: pullRequest) }
        if !entry.attachments.isEmpty { AttachmentsView(attachments: entry.attachments, roomID: roomID) }
    }
}

private struct EntryView: View {
    let entry: TimelineEntry
    let roomID: String

    private var mineShape: UnevenRoundedRectangle {
        UnevenRoundedRectangle(
            topLeadingRadius: 20, bottomLeadingRadius: 20, bottomTrailingRadius: 6, topTrailingRadius: 20,
            style: .continuous
        )
    }

    var body: some View {
        if entry.isMine {
            HStack(alignment: .bottom) {
                Spacer(minLength: 48)
                VStack(alignment: .trailing, spacing: 4) {
                    CollapsingBody(body_: entry.body)
                        .foregroundStyle(SemanticColor.textPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    EntryExtras(entry: entry, roomID: roomID)
                    Text(GallopTime.ago(entry.date)).gallopText(.caption, color: SemanticColor.textTertiary)
                }
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .background(
                    LinearGradient(
                        colors: [SemanticColor.surface300, SemanticColor.surface400],
                        startPoint: .topLeading, endPoint: .bottomTrailing
                    ),
                    in: mineShape
                )
                .overlay { mineShape.stroke(SemanticColor.borderDefault, lineWidth: 0.5) }
            }
            .accessibilityElement(children: .contain)
        } else {
            HStack(alignment: .top, spacing: 10) {
                AgentAvatar(name: entry.senderName)
                VStack(alignment: .leading, spacing: 4) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(entry.senderName)
                            .gallopText(.bodyMStrong, color: SemanticColor.textPrimary)
                            .lineLimit(1)
                        if entry.isAgent {
                            GallopIconView(icon: .chatbot, fallbackSystemName: "cpu", size: 12)
                                .foregroundStyle(SemanticColor.iconTertiary)
                                .accessibilityLabel("Agent")
                                .accessibilityIdentifier("message.agentMark")
                        }
                        Text(GallopTime.ago(entry.date)).gallopText(.caption, color: SemanticColor.textTertiary)
                    }
                    CollapsingBody(body_: entry.body)
                        .foregroundStyle(SemanticColor.textSecondary)
                    EntryExtras(entry: entry, roomID: roomID)
                }
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
