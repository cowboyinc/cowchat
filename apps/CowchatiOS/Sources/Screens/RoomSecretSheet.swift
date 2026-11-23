import SwiftUI
import CowchatKit

struct RoomSecretSheet: View {
    @EnvironmentObject private var session: ChatSession
    @Environment(\.dismiss) private var dismiss
    let roomID: String
    @State private var secret = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                SecureField("Room secret", text: $secret)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .gallopText(.bodyL, color: SemanticColor.textPrimary)
                    .gallopFormRow()
                if let error {
                    Text(error)
                        .gallopText(.bodyS, color: SemanticColor.textError)
                        .gallopFormRow()
                        .accessibilityIdentifier("secret.error")
                }
            }
            .gallopForm()
            .navigationTitle("Room secret")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        do {
                            try session.setSecret(secret.trimmingCharacters(in: .whitespacesAndNewlines), roomID: roomID)
                            dismiss()
                        } catch {
                            self.error = error.localizedDescription
                        }
                    }
                    .disabled(secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .presentationDetents([.medium])
    }
}
