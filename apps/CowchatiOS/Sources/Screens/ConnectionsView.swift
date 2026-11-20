import SwiftUI
import CowchatKit

/// The connections as rows of a list, each opening its detail, with Add connection after them.
struct ConnectionRows: View {
    @EnvironmentObject private var store: ConnectionsStore
    @State private var isAdding = false

    var body: some View {
        ForEach(store.connections) { connection in
            // A link that carries its own destination: one declared by value does not resolve on an iPad.
            NavigationLink {
                ConnectionDetailView(connectionID: connection.id)
            } label: {
                ConnectionRow(connection: connection, account: store.account(for: connection.id))
            }
            .listRowBackground(Color.clear)
            .listRowSeparator(.visible)
            .listRowSeparatorTint(SemanticColor.borderDefault)
            .accessibilityIdentifier("connections.row")
        }
        Button("Add connection") { isAdding = true }
            .buttonStyle(GallopButtonStyle(kind: .secondary))
            .listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
            .accessibilityIdentifier("connections.add")
            .sheet(isPresented: $isAdding) { SignInView() }
    }
}

private struct ConnectionRow: View {
    @EnvironmentObject private var store: ConnectionsStore
    let connection: ConnectionRecord
    let account: AccountStore?

    var body: some View {
        HStack(spacing: 12) {
            ServerMark(server: connection.server)
            VStack(alignment: .leading, spacing: 0) {
                Text(connection.name).gallopText(.bodyL, color: SemanticColor.textPrimary)
                if let account {
                    // The handle arrives after the account loads, so this line watches the account.
                    AccountSubtitle(account: account, connection: connection, phase: store.phases[connection.id])
                } else {
                    Text(ConnectionState.subtitle(connection, source: store.source(of: connection.id), phase: store.phases[connection.id]))
                        .gallopText(.caption, color: SemanticColor.textTertiary)
                }
            }
        }
        .padding(.vertical, 4)
    }
}

private struct AccountSubtitle: View {
    @ObservedObject var account: AccountStore
    let connection: ConnectionRecord
    let phase: SessionPhase?

    var body: some View {
        Text(ConnectionState.subtitle(connection, source: .account(handle: account.account?.handle), phase: phase))
            .gallopText(.caption, color: SemanticColor.textTertiary)
    }
}

private struct ConnectionDetailView: View {
    let connectionID: String

    @EnvironmentObject private var store: ConnectionsStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var confirmsRemove = false

    private var connection: ConnectionRecord? { store.connections.first { $0.id == connectionID } }

    var body: some View {
        Form {
            if let connection {
                Section {
                    LabeledContent("Name") {
                        TextField("Name", text: $name)
                            .multilineTextAlignment(.trailing)
                            .onSubmit { store.rename(connectionID: connectionID, to: name) }
                            .accessibilityIdentifier("connections.name")
                    }
                    LabeledContent("Server", value: connection.server.label)
                    LabeledContent("Status", value: ConnectionState.title(store.phases[connectionID]))
                }
                .gallopText(.bodyL, color: SemanticColor.textPrimary)
                .gallopFormRow()
                if case .failed(let message) = store.phases[connectionID] {
                    Section {
                        Text(message).gallopText(.bodyM, color: SemanticColor.textError)
                        Button("Try again") { Task { await store.retry(connectionID: connectionID) } }
                            .gallopText(.bodyL)
                            .accessibilityIdentifier("connections.retry")
                    }
                    .gallopFormRow()
                }
                Section {
                    Button("Remove", role: .destructive) { confirmsRemove = true }
                        .gallopText(.bodyL)
                        .accessibilityIdentifier("connections.remove")
                }
                .gallopFormRow()
            }
        }
        .gallopForm()
        .navigationTitle(connection?.name ?? "")
        .onAppear { name = connection?.name ?? "" }
        .onDisappear { store.rename(connectionID: connectionID, to: name) }
        .confirmationDialog("Remove this connection?", isPresented: $confirmsRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                store.remove(connectionID: connectionID)
                dismiss()
            }
            .accessibilityIdentifier("connections.confirmRemove")
        }
    }
}

enum ConnectionState {
    static func subtitle(_ connection: ConnectionRecord, source: ConnectionSource, phase: SessionPhase?) -> String {
        let server = connection.name == connection.server.label ? nil : connection.server.label
        return [source.label, server, title(phase)].compactMap { $0 }.joined(separator: " · ")
    }

    static func title(_ phase: SessionPhase?) -> String {
        switch phase {
        case .connected: "Connected"
        case .connecting: "Connecting"
        case .reconnecting: "Reconnecting"
        case .failed: "Not connected"
        case .signedOut, nil: "Signed out"
        }
    }
}
