import SwiftUI
import CowchatKit

struct RoomSecretSheet: View {
    @EnvironmentObject private var session: ChatSession
    @Environment(\.dismiss) private var dismiss
    let roomID: String
    @State private var secret = ""

    var body: some View {
        NavigationStack {
            Form {
                SecureField("Room secret", text: $secret)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .gallopFormRow()
            }
            .gallopForm()
            .navigationTitle("Room secret")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        session.setSecret(secret.trimmingCharacters(in: .whitespacesAndNewlines), roomID: roomID)
                        dismiss()
                    }
                    .disabled(secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .presentationDetents([.medium])
    }
}
