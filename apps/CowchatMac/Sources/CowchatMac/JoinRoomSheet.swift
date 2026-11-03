import SwiftUI

/// Accepting an invitation, as the Inbox's Join sheet and from Friends, drawn from the Rename room sheet: 480 wide on
/// Surface/600 with 26 padding and its parts 20 apart. The room's name in Heading 4 with who invited 3 under it, then
/// the person's live sessions to join with, or none, then Decline at the left and Cancel and Join at the right.
/// Escape is Cancel and Return is Join.
struct SocialInvitationReview: View {
    let invitation: SocialRoomInvitation
    let sessions: [LiveSession]
    let sessionName: (LiveSession) -> String
    @Binding var selectedSession: String?
    let busy: Bool
    let error: String?
    let onCancel: () -> Void
    let onDecline: () -> Void
    let onAccept: () -> Void
    @State private var showsAllSessions = false

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            heading
            if !sessions.isEmpty { sessionChoice }
            VStack(alignment: .leading, spacing: 8) {
                if let error {
                    Text(error)
                        .gallopText(.caption, color: SemanticColor.textError)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                buttons
            }
        }
        .padding(26)
        .frame(width: 480, alignment: .leading)
        .fixedSize(horizontal: false, vertical: true)
        .background(SemanticColor.surface600)
    }

    /// The room's name, who invited, and a line when the room's messages also go to Slack.
    private var heading: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(invitation.roomName)
                .gallopText(.h4, color: SemanticColor.textPrimary)
                .lineLimit(1)
                .truncationMode(.tail)
            // copy:
            Text("Invited by \(invitation.from.displayName)")
                .gallopText(.bodyM, color: SemanticColor.textTertiary)
                .lineLimit(1)
                .truncationMode(.tail)
            if invitation.slackConnected {
                // copy:
                Text("Messages here also go to Slack")
                    .gallopText(.bodyS, color: SemanticColor.textTertiary)
            }
        }
    }

    /// "No session" first, then the sessions `LiveSessions.pick` chooses, newest first, and the more row for the rest.
    private var sessionChoice: some View {
        let picked = LiveSessions.pick(from: sessions)
        let hidden = sessions.count - picked.count
        return VStack(alignment: .leading, spacing: 4) {
            // copy:
            Text("Add a session")
                .gallopText(.caption, color: SemanticColor.textTertiary)
            VStack(spacing: 0) {
                // copy:
                JoinSessionRow(harness: nil, name: "No session", place: nil, isChosen: selectedSession == nil) {
                    selectedSession = nil
                }
                ForEach(showsAllSessions ? sessions : picked) { session in
                    PageRowSeam()
                    JoinSessionRow(harness: session.harness, name: sessionName(session), place: session.place,
                                   isChosen: selectedSession == session.id) {
                        selectedSession = session.id
                    }
                }
                if hidden > 0 {
                    SessionsMoreRow(title: LiveSessions.moreTitle(hidden: hidden, showsAll: showsAllSessions),
                                    indent: JoinSessionRow.nameLeading, surface: .page) {
                        showsAllSessions.toggle()
                    }
                }
            }
        }
    }

    /// Three 36 pills: Decline at the left, Cancel and Join 10 apart at the right. While busy, Join shows a spinner
    /// before its label and all three are disabled.
    private var buttons: some View {
        HStack(spacing: 10) {
            // copy:
            Button("Decline", action: onDecline)
                .macAccessibleAction(label: "Decline", isEnabled: !busy, action: onDecline)
            Spacer(minLength: 0)
            Button("Cancel", action: onCancel)
                .keyboardShortcut(.cancelAction)
                .macAccessibleAction(label: "Cancel", isEnabled: !busy, action: onCancel)
            Button(action: onAccept) {
                HStack(spacing: 6) {
                    if busy {
                        ProgressView().controlSize(.small).frame(width: 16, height: 16)
                    }
                    // copy:
                    Text("Join")
                }
            }
            .buttonStyle(CowchatPillButtonStyle(prominent: true))
            .keyboardShortcut(.defaultAction)
            .macAccessibleAction(label: "Join", isEnabled: !busy, action: onAccept)
        }
        .buttonStyle(CowchatPillButtonStyle())
        .disabled(busy)
    }
}

/// One choice in the Join sheet, from the agents row's parts with Inbox spacing: 44 tall, 16 in, the 24 harness mark,
/// 12, the name in Body M, the place in Body S Text/Tertiary (100, right-aligned), 12, and a 16 slot that holds the
/// tick on the chosen row, then 16. "No session" has no mark and no place. The choice has no fill, and the hover is the
/// page rows'.
private struct JoinSessionRow: View {
    /// Where the names start: the 16 in, the 24 mark and the 12 after it.
    static let nameLeading: CGFloat = 16 + 24 + 12

    let harness: LiveSession.Harness?
    let name: String
    let place: String?
    let isChosen: Bool
    let choose: () -> Void

    var body: some View {
        Button(action: choose) {
            HStack(spacing: 0) {
                ZStack {
                    if let harness { HarnessMark(harness: harness, size: 24) }
                }
                .frame(width: 24, height: 24)
                Spacer().frame(width: 12)
                Text(name)
                    .gallopText(.bodyM, color: SemanticColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let place {
                    Text(place)
                        .gallopText(.bodyS, color: SemanticColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .frame(width: 100, alignment: .trailing)
                }
                Spacer().frame(width: 12)
                ZStack {
                    if isChosen {
                        GallopIconView(icon: .checkmark, fallbackSystemName: "checkmark", size: 16)
                            .foregroundStyle(SemanticColor.iconPrimary)
                    }
                }
                .frame(width: 16, height: 16)
            }
            .padding(.horizontal, 16)
            .frame(height: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(PageRowButtonStyle())
        // copy:
        .macAccessibleAction(label: [name, place].compactMap { $0 }.joined(separator: ", "),
                             value: isChosen ? "chosen" : nil, action: choose)
    }
}
