import SwiftUI
import CowchatKit

/// First run and "Add connection" both show this.
struct SignInView: View {
    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.isPresented) private var isPresented
    @State private var form = SignInForm()
    @State private var error: String?
    @State private var showsHelp = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 0) {
                    Image("CowchatMark")
                        .resizable()
                        .frame(width: 56, height: 56)
                        .clipShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
                        .accessibilityHidden(true)
                        .padding(.bottom, 24)
                    Text("Cowchat")
                        .gallopText(.h2, color: SemanticColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    Text("Rooms for you and your agents.")
                        .gallopText(.bodyL, color: SemanticColor.textSecondary)
                        .multilineTextAlignment(.center)
                        .padding(.top, 10)
                    card.padding(.top, 28)
                    links.padding(.top, 14)
                }
                .frame(maxWidth: 560)
                .padding(.horizontal, 24)
                .padding(.top, 48)
                .padding(.bottom, 24)
                .frame(maxWidth: .infinity)
            }
            .scrollDismissesKeyboard(.interactively)
            .background(SemanticColor.surface500.ignoresSafeArea())
            .toolbar(.hidden, for: .navigationBar)
            .overlay(alignment: .topTrailing) {
                if isPresented {
                    Button("Cancel") { dismiss() }
                        .buttonStyle(GallopLinkButtonStyle())
                        .padding(.horizontal, 20)
                        .padding(.top, 8)
                        .accessibilityIdentifier("signin.cancel")
                }
            }
            .sheet(isPresented: $showsHelp) { SignInHelpSheet() }
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 14) {
            if form.place != .global { serverSection }
            if let url = form.providerURL { ProviderSignInSection(serverURL: url).id(url) }
            SecureField("Paste invite or key", text: $form.pasted)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .gallopField()
                .accessibilityIdentifier("signin.paste")
            if let found = form.found {
                HStack(spacing: 6) {
                    GallopIconView(icon: .checkmark, fallbackSystemName: "checkmark.circle", size: 16)
                        .foregroundStyle(SemanticColor.success)
                    Text(found)
                        .gallopText(.bodyM, color: SemanticColor.textSecondary)
                        .accessibilityIdentifier("signin.found")
                }
            }
            Button("Connect", action: connect)
                .buttonStyle(GallopButtonStyle())
                .disabled(!form.canConnect || store.isAdding)
                .accessibilityIdentifier("signin.connect")
            if let error {
                Text(error)
                    .gallopText(.bodyS, color: SemanticColor.textError)
                    .frame(maxWidth: .infinity)
                    .multilineTextAlignment(.center)
                    .accessibilityIdentifier("signin.error")
            }
            if store.isAdding {
                ProgressView("Connecting")
                    .gallopText(.bodyM, color: SemanticColor.textSecondary)
                    .frame(maxWidth: .infinity)
            }
        }
        .padding(24)
        .background(SemanticColor.surface600, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(SemanticColor.borderDefault, lineWidth: 0.5)
        }
        .shadow(color: .black.opacity(0.06), radius: 4, y: 4)
    }

    private var links: some View {
        VStack(spacing: 0) {
            Button("How do I get an invite or key?") { showsHelp = true }
                .accessibilityIdentifier("signin.help")
            if form.place == .global {
                Button("Use my own server") { form.place = .ownServer }
                    .accessibilityIdentifier("signin.ownServer")
            } else {
                Button("Use Global") { form.place = .global }
                    .accessibilityIdentifier("signin.useGlobal")
            }
        }
        .buttonStyle(GallopLinkButtonStyle())
    }

    private var serverSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker("Server", selection: $form.place) {
                Text("My server").tag(SignInForm.Place.ownServer)
                Text("Other server").tag(SignInForm.Place.otherServer)
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("signin.place")
            if form.place == .ownServer {
                TextField("Host", text: $form.host)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .gallopField()
                    .accessibilityIdentifier("signin.host")
                TextField("Port", text: $form.port)
                    .keyboardType(.numberPad)
                    .gallopField()
                    .accessibilityIdentifier("signin.port")
                Text("This connection is not encrypted.")
                    .gallopText(.caption, color: SemanticColor.textTertiary)
            } else {
                TextField("URL", text: $form.url)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .gallopField()
                    .accessibilityIdentifier("signin.url")
            }
        }
    }

    private func connect() {
        guard let server = form.server, let credential = form.credential else { return }
        error = nil
        Task {
            do {
                _ = try await store.add(server: server, credential: credential)
                form.pasted = ""
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}

private struct SignInHelpSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Invite: ask someone in the room to open Invites in the Cowchat Mac app and send you one.")
                    Text("Key: on your own server, copy the contents of ~/.cowchat/auth.key.")
                    Text("Paste either one here.")
                }
                .gallopText(.bodyL, color: SemanticColor.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
            }
            .background(SemanticColor.surface400.ignoresSafeArea())
            .navigationTitle("Invite or key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        .presentationDetents([.medium])
        .accessibilityIdentifier("signin.helpSheet")
    }
}
