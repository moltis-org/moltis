import SwiftUI

struct ContentView: View {
    @EnvironmentObject var connectionStore: ConnectionStore

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
            ZStack {
                Circle()
                    .fill(.white.opacity(0.07))
                Circle()
                    .stroke(bannerStyle.tint.opacity(0.55), lineWidth: 1)
                Image(systemName: bannerStyle.symbol)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(bannerStyle.tint)
            }
            .frame(width: 42, height: 42)

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

            if bannerStyle.showSpinner {
                ProgressView()
                    .tint(bannerStyle.tint)
            }
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

    private var bannerStyle: (
        title: String, subtitle: String, symbol: String, tint: Color, showSpinner: Bool
    ) {
        switch connectionStore.state {
        case .connecting:
            return (
                "Connecting to server",
                "Establishing secure session...",
                "bolt.horizontal.circle.fill",
                .blue,
                true
            )
        case .reconnecting(let attempt, let nextRetryIn):
            let seconds = max(1, Int(nextRetryIn.rounded(.up)))
            return (
                "Server unavailable",
                "Retrying in \(seconds)s (attempt \(attempt))...",
                "arrow.clockwise.circle.fill",
                .orange,
                true
            )
        case .error(let message):
            return (
                "Connection error",
                message,
                "exclamationmark.triangle.fill",
                .red,
                false
            )
        case .disconnected:
            return (
                "Disconnected",
                "Reconnect from Settings or restart Moltis.",
                "wifi.slash",
                .secondary,
                false
            )
        case .connected:
            return (
                "Connected",
                "Connected to server.",
                "checkmark.circle.fill",
                .green,
                false
            )
        }
    }
}
