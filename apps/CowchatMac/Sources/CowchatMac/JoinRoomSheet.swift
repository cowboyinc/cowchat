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
                SheetChoiceRow(name: "No session", isChosen: selectedSession == nil) {
                    selectedSession = nil
                }
                ForEach(showsAllSessions ? sessions : picked) { session in
                    PageRowSeam()
                    SheetChoiceRow(name: sessionName(session), fact: session.place,
                                   isChosen: selectedSession == session.id) {
                        selectedSession = session.id
                    } mark: {
                        HarnessMark(harness: session.harness, size: 24)
                    }
                }
                if hidden > 0 {
                    SessionsMoreRow(title: LiveSessions.moreTitle(hidden: hidden, showsAll: showsAllSessions),
                                    indent: SheetChoiceRow<EmptyView>.nameLeading, surface: .page) {
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
