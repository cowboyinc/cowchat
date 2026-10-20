import AppKit
import SwiftUI
import UserNotifications

struct SocialNotificationSettings: View {
    @ObservedObject var notifications: SocialNotifications

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Toggle("Show Mac notifications", isOn: $notifications.enabled)
                .gallopText(.bodyMStrong)
            VStack(alignment: .leading, spacing: 14) {
                Toggle("New friend requests", isOn: $notifications.friendRequests)
                Toggle("Friend requests accepted", isOn: $notifications.acceptedRequests)
                Toggle("Room invitations", isOn: $notifications.roomInvitations)
            }
            .gallopText(.bodyM)
            .disabled(!notifications.enabled)
            Text("Notifications arrive while Cowchat is running, even when Friends is closed. Click one to open Friends or Invitations.")
                .gallopText(.bodyS, color: SemanticColor.textSecondary)
            if notifications.enabled {
                switch notifications.authorizationStatus {
                case .notDetermined:
                    Button("Allow notifications…") { Task { await notifications.requestPermission() } }
                        .buttonStyle(CowchatPillButtonStyle(prominent: true))
                case .denied:
                    Text("Notifications are blocked in macOS. Allow Cowchat in System Settings → Notifications.")
                        .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    Button("Open notification settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") {
                            NSWorkspace.shared.open(url)
                        }
                    }.buttonStyle(CowchatPillButtonStyle())
                default:
                    Label("Notifications allowed by macOS", systemImage: "checkmark.circle")
                        .gallopText(.bodyS, color: SemanticColor.textSecondary)
                    Button("Send test notification") { Task { await notifications.sendTestNotification() } }
                        .buttonStyle(CowchatPillButtonStyle())
                }
            }
            if let error = notifications.errorMessage {
                Text(error).gallopText(.bodyS, color: SemanticColor.notificationErrorTextDefault)
            }
        }
        .task { await notifications.refreshAuthorization() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await notifications.refreshAuthorization() }
        }
    }
}
