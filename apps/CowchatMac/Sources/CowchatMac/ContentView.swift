import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var store: ChatStore
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 260, ideal: 310, max: 380)
        } detail: {
            if let room = store.selectedRoom {
                ChatRoomView(room: room)
                    .id(room.id)
            } else {
                EmptyChatView()
            }
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 880, minHeight: 580)
        .sheet(isPresented: $store.isCreateRoomPresented) {
            CreateRoomView()
                .environmentObject(store)
        }
        .alert("Cowchat", isPresented: Binding(
            get: { store.errorMessage != nil },
            set: { if !$0 { store.errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { store.errorMessage = nil }
        } message: {
            Text(store.errorMessage ?? "")
        }
        .task { store.start() }
    }
}

private struct SidebarView: View {
    @EnvironmentObject private var store: ChatStore

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 11) {
                ZStack {
                    Circle().fill(Color.accentColor.gradient)
                    Image(systemName: "bubble.left.and.bubble.right.fill")
                        .foregroundStyle(.white)
                        .font(.system(size: 15, weight: .semibold))
                }
                .frame(width: 34, height: 34)

                VStack(alignment: .leading, spacing: 1) {
                    Text("Cowchat").font(.headline)
                    Text("Local rooms").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    store.isCreateRoomPresented = true
                } label: {
                    Image(systemName: "square.and.pencil")
                        .font(.system(size: 16, weight: .semibold))
                }
                .buttonStyle(.plain)
                .help("Create room")
                .keyboardShortcut("n", modifiers: .command)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 13)

            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search rooms", text: $store.searchText)
                    .textFieldStyle(.plain)
                if !store.searchText.isEmpty {
                    Button { store.searchText = "" } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 11)
            .frame(height: 34)
            .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 9))
            .padding(.horizontal, 12)
            .padding(.bottom, 9)

            Divider()

            if store.filteredRooms.isEmpty {
                VStack(spacing: 9) {
                    Image(systemName: "bubble.left")
                        .font(.system(size: 28))
                        .foregroundStyle(.tertiary)
                    Text("No rooms found").font(.headline)
                    Text("Try another search or create a room.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List {
                    ForEach(store.filteredRooms) { room in
                        RoomRow(room: room, isSelected: store.selectedRoomID == room.id)
                            .contentShape(Rectangle())
                            .listRowBackground(
                                store.selectedRoomID == room.id
                                    ? Color.accentColor.opacity(0.92)
                                    : Color.clear
                            )
                            .onTapGesture { Task { await store.select(room: room) } }
                    }
                }
                .listStyle(.sidebar)
            }

            Divider()
            HStack(spacing: 8) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 8, height: 8)
                Text(store.connectionStatus.label)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if !store.connectionStatus.isConnected {
                    Button("Reconnect") { store.start() }
                        .buttonStyle(.plain)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(Color.accentColor)
                }
            }
            .padding(.horizontal, 15)
            .frame(height: 42)
        }
        .background(.regularMaterial)
    }

    private var statusColor: Color {
        switch store.connectionStatus {
        case .connected: return .green
        case .connecting: return .orange
        case .disconnected, .failed: return .red
        }
    }
}

private struct RoomRow: View {
    let room: Room
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 12) {
            RoomAvatar(name: room.name, size: 46)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 5) {
                    Text(room.name)
                        .font(.system(size: 14, weight: .semibold))
                        .lineLimit(1)
                    if room.encrypted {
                        Image(systemName: "lock.fill").font(.caption2)
                    }
                    Spacer()
                    Text((room.lastActivity ?? room.createdAt).cowchatTime)
                        .font(.caption2)
                        .foregroundStyle(isSelected ? .white.opacity(0.82) : .secondary)
                }
                HStack(spacing: 5) {
                    Text(room.description?.isEmpty == false ? room.description! : room.ephemeral ? "Ephemeral room" : "Open conversation")
                        .lineLimit(1)
                        .font(.subheadline)
                        .foregroundStyle(isSelected ? .white.opacity(0.86) : .secondary)
                    Spacer()
                    if let count = room.memberCount, count > 0 {
                        Text("\(count)")
                            .font(.caption2.weight(.bold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background((isSelected ? Color.white : Color.accentColor).opacity(0.18), in: Capsule())
                    }
                }
            }
        }
        .padding(.vertical, 5)
    }
}

private struct ChatRoomView: View {
    @EnvironmentObject private var store: ChatStore
    let room: Room

    var body: some View {
        VStack(spacing: 0) {
            chatHeader
            Divider()
            ZStack {
                Color(nsColor: NSColor(calibratedRed: 0.91, green: 0.94, blue: 0.96, alpha: 1))
                ChatBackdrop()
                messageList
            }
            Divider()
            composer
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var chatHeader: some View {
        VStack(spacing: 0) {
            HStack(spacing: 11) {
                RoomAvatar(name: room.name, size: 38)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 6) {
                        Text(room.name).font(.headline)
                        if room.encrypted { Image(systemName: "lock.fill").font(.caption) }
                    }
                    Text(displayedMemberCount == 1 ? "1 member" : "\(displayedMemberCount) members")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if room.ephemeral {
                    Label("Ephemeral", systemImage: "timer")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 18)
            .frame(height: 58)

            if !store.roomMembers.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 7) {
                        ForEach(store.roomMembers) { member in
                            MemberPresencePill(member: member)
                        }
                    }
                    .padding(.horizontal, 18)
                }
                .frame(height: 38)
            }
        }
        .background(.regularMaterial)
    }

    private var displayedMemberCount: Int {
        if !store.roomMembers.isEmpty { return store.roomMembers.count }
        return max(room.memberCount ?? 0, store.connectionStatus.isConnected ? 1 : 0)
    }

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(spacing: 7) {
                    if store.isLoadingMessages {
                        ProgressView().controlSize(.small).padding(.top, 28)
                    } else if store.messages.isEmpty {
                        Text("This room is quiet. Say hello.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 15)
                            .padding(.vertical, 9)
                            .background(.ultraThinMaterial, in: Capsule())
                            .padding(.top, 30)
                    }
                    ForEach(store.messages) { message in
                        MessageBubble(message: message, isMine: message.agentID == store.agentID)
                            .id(message.id)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 14)
            }
            .onChange(of: store.messages.count) { _ in
                if let last = store.messages.last {
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(last.id, anchor: .bottom) }
                }
            }
            .onAppear {
                if let last = store.messages.last { proxy.scrollTo(last.id, anchor: .bottom) }
            }
        }
    }

    private var composer: some View {
        VStack(spacing: 0) {
            if room.encrypted {
                Label("Encrypted rooms are not supported in this first macOS build.", systemImage: "lock.fill")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .padding(.top, 8)
            } else if !store.connectionStatus.isConnected {
                Label("Offline — you can keep typing and send after reconnecting.", systemImage: "wifi.slash")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 8)
            }
            HStack(alignment: .bottom, spacing: 10) {
                Image(systemName: "paperclip")
                    .font(.system(size: 18))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 8)
                ComposerTextField(
                    text: $store.draft,
                    placeholder: composerPlaceholder,
                    isEnabled: !room.encrypted,
                    onSubmit: store.sendDraft
                )
                    .frame(minHeight: 20)
                    .padding(.horizontal, 13)
                    .padding(.vertical, 9)
                    .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 18))
                Button { store.sendDraft() } label: {
                    Image(systemName: "paperplane.fill")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 36, height: 36)
                        .background(Color.accentColor.gradient, in: Circle())
                }
                .buttonStyle(.plain)
                .disabled(!canSend)
                .opacity(canSend ? 1 : 0.45)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 11)
        }
        .background(.regularMaterial)
    }

    private var composerPlaceholder: String {
        if room.encrypted { return "Encrypted room" }
        if !store.connectionStatus.isConnected { return "Draft a message while offline" }
        return "Message \(room.name)"
    }

    private var canSend: Bool {
        store.connectionStatus.isConnected
            && !room.encrypted
            && !store.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

private struct MemberPresencePill: View {
    let member: AgentPresence

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(Color.green)
                .frame(width: 7, height: 7)
            Text(member.name)
                .font(.caption.weight(.semibold))
            if let status = member.status, !status.isEmpty {
                Text(statusText(status))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Color.primary.opacity(0.06), in: Capsule())
        .help(helpText)
    }

    private func statusText(_ status: String) -> String {
        var parts = [status]
        if let detail = member.statusDetail, !detail.isEmpty { parts.append(detail) }
        if let progress = member.progress { parts.append("\(progress)%") }
        return parts.joined(separator: " · ")
    }

    private var helpText: String {
        guard let status = member.status else { return member.name }
        return "\(member.name): \(statusText(status))"
    }
}

private struct MessageBubble: View {
    let message: ChatMessage
    let isMine: Bool

    var body: some View {
        HStack {
            if isMine { Spacer(minLength: 90) }
            VStack(alignment: .leading, spacing: 3) {
                if !isMine {
                    Text(message.agentName)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(Color.accentColor)
                }
                ExpandableMessageText(content: message.content, isMine: isMine)
                HStack {
                    Spacer(minLength: 0)
                    Text(message.timestamp.cowchatTime)
                        .font(.system(size: 10))
                        .foregroundStyle(isMine ? .white.opacity(0.72) : .secondary)
                }
            }
            .frame(maxWidth: 640, alignment: .leading)
            .padding(.horizontal, 11)
            .padding(.vertical, 8)
            .background(isMine ? Color.accentColor.gradient : Color.white.gradient, in: RoundedRectangle(cornerRadius: 14))
            .shadow(color: .black.opacity(0.07), radius: 1.5, y: 1)
            if !isMine { Spacer(minLength: 90) }
        }
        .frame(maxWidth: .infinity, alignment: isMine ? .trailing : .leading)
    }
}

private struct ExpandableMessageText: View {
    let content: String
    let isMine: Bool
    @State private var isExpanded = false

    private let previewLength = 2_400

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(displayedContent)
                .textSelection(.enabled)
                .font(.system(size: 14))
                .foregroundStyle(isMine ? .white : .primary)
                .fixedSize(horizontal: false, vertical: true)
            if content.count > previewLength {
                Button(isExpanded ? "Show less" : "Show full message") {
                    isExpanded.toggle()
                }
                .buttonStyle(.plain)
                .font(.caption.weight(.semibold))
                .foregroundStyle(isMine ? .white.opacity(0.85) : Color.accentColor)
            }
        }
    }

    private var displayedContent: String {
        guard !isExpanded, content.count > previewLength else { return content }
        return String(content.prefix(previewLength)) + "…"
    }
}

private struct RoomAvatar: View {
    let name: String
    let size: CGFloat

    var body: some View {
        Circle()
            .fill(colors[index].gradient)
            .overlay {
                Text(initials)
                    .font(.system(size: size * 0.32, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
            }
            .frame(width: size, height: size)
    }

    private let colors: [Color] = [.blue, .purple, .pink, .orange, .teal, .indigo]
    private var index: Int { abs(name.unicodeScalars.reduce(0) { $0 + Int($1.value) }) % colors.count }
    private var initials: String {
        let words = name.split(separator: " ").prefix(2)
        let value = words.compactMap(\.first).map(String.init).joined()
        return value.isEmpty ? "#" : value.uppercased()
    }
}

private struct ChatBackdrop: View {
    var body: some View {
        GeometryReader { geometry in
            Canvas { context, size in
                for row in 0..<8 {
                    for column in 0..<10 {
                        let x = CGFloat(column) * 82 + (row.isMultiple(of: 2) ? 16 : 54)
                        let y = CGFloat(row) * 88 + 20
                        let rect = CGRect(x: x, y: y, width: 18, height: 18)
                        context.stroke(Path(ellipseIn: rect), with: .color(.blue.opacity(0.035)), lineWidth: 1)
                    }
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .allowsHitTesting(false)
    }
}

private struct EmptyChatView: View {
    var body: some View {
        ZStack {
            Color(nsColor: NSColor(calibratedRed: 0.91, green: 0.94, blue: 0.96, alpha: 1))
            VStack(spacing: 12) {
                Image(systemName: "bubble.left.and.bubble.right.fill")
                    .font(.system(size: 42))
                    .foregroundStyle(Color.accentColor)
                Text("Choose a Cowchat room")
                    .font(.title2.weight(.semibold))
                Text("Browse local rooms in the sidebar or create a new one.")
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct CreateRoomView: View {
    @EnvironmentObject private var store: ChatStore
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var description = ""
    @State private var ephemeral = false
    @State private var isPublic = false
    @State private var isCreating = false

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("New room").font(.title2.weight(.semibold))
                    Text("Create a conversation on your local Cowchat server.")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.plain).foregroundStyle(.tertiary)
            }

            Form {
                TextField("Room name", text: $name)
                TextField("Description (optional)", text: $description)
                Toggle("Ephemeral room", isOn: $ephemeral)
                Toggle("Public room", isOn: $isPublic)
            }
            .formStyle(.grouped)
            .scrollDisabled(true)

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create") {
                    isCreating = true
                    Task {
                        _ = await store.createRoom(
                            name: name,
                            description: description,
                            ephemeral: ephemeral,
                            isPublic: isPublic
                        )
                        isCreating = false
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isCreating)
            }
        }
        .padding(24)
        .frame(width: 460, height: 360)
    }
}
