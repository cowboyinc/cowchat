import SwiftUI
import CowchatKit

struct ComposerView: View {
    @EnvironmentObject private var session: ChatSession
    let roomID: String

    @State private var text = ""
    @State private var draft = MentionDraft()
    @State private var error: String?
    @State private var isSending = false

    private var suggestions: [RoomMember] {
        MentionDraft.suggestions(text: text, members: session.members[roomID] ?? [])
    }

    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !isSending
    }

    var body: some View {
        VStack(spacing: 0) {
            if !suggestions.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack {
                        ForEach(suggestions) { member in
                            Button("@\(member.name)") { text = draft.accept(member, in: text) }
                                .buttonStyle(GallopButtonStyle(kind: .secondary, fillsWidth: false))
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 6)
                }
            }
            if let error {
                Text(error)
                    .gallopText(.bodyS, color: SemanticColor.textError)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message", text: $text, axis: .vertical)
                    .lineLimit(1...6)
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(SemanticColor.textfieldDefault, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 22, style: .continuous)
                            .stroke(SemanticColor.borderDefault, lineWidth: 1)
                    }
                    .accessibilityIdentifier("composer.field")
                Button(action: send) {
                    GallopIconView(icon: .send, fallbackSystemName: "arrow.up", size: 20)
                        .foregroundStyle(SemanticColor.buttonPrimaryIconDefault)
                        .frame(width: 44, height: 44)
                        .background(SemanticColor.buttonPrimaryDefault, in: Circle())
                        .opacity(canSend ? 1 : 0.4)
                }
                .disabled(!canSend)
                .accessibilityLabel("Send")
                .accessibilityIdentifier("composer.send")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(SemanticColor.surface500.ignoresSafeArea())
    }

    private func send() {
        let body = text
        let mentions = draft.mentionIDs(in: body)
        isSending = true
        error = nil
        Task {
            do {
                try await session.send(roomID: roomID, text: body, mentions: mentions)
                // Anything typed while it was sending stays.
                if text == body {
                    text = ""
                    draft.reset()
                }
            } catch {
                self.error = error.localizedDescription
            }
            isSending = false
        }
    }
}
