import SwiftUI

struct ContentView: View {
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var authManager: AuthManager

    var body: some View {
        ChatView()
            .environmentObject(connectionStore.chatStore)
            .safeAreaInset(edge: .top, spacing: 0) {
                if !connectionStore.state.isConnected {
                    connectionBanner
                        .padding(.horizontal, 12)
                        .padding(.top, 6)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
            }
            .animation(.easeInOut(duration: 0.2), value: connectionStore.state.isConnected)
    }

    private var connectionBanner: some View {
        HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                Text(bannerStyle.title)
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(.primary)
                Text(bannerStyle.subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 10)

            Button {
                authManager.disconnect()
                Task { await connectionStore.disconnect() }
            } label: {
                Text("Cancel")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(bannerStyle.tint)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(bannerStyle.tint.opacity(0.12), in: Capsule())
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(bannerStyle.tint.opacity(0.35), lineWidth: 0.9)
        }
        .shadow(color: .black.opacity(0.22), radius: 14, x: 0, y: 6)
    }

    private var bannerStyle: (title: String, subtitle: String, tint: Color) {
        switch connectionStore.state {
        case .connecting:
            return ("Connecting to server", "Establishing secure session...", .blue)
        case .reconnecting(let attempt, let nextRetryIn):
            let seconds = max(1, Int(nextRetryIn.rounded(.up)))
            return ("Server unavailable", "Retrying in \(seconds)s (attempt \(attempt))...", .orange)
        case .error(let message):
            return ("Connection error", message, .red)
        case .disconnected:
            return ("Disconnected", "Reconnect from Settings or restart Moltis.", .secondary)
        case .connected:
            return ("Connected", "Connected to server.", .green)
        }
    }
}
