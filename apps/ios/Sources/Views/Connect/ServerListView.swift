import SwiftUI

struct ServerListView: View {
    @EnvironmentObject var authManager: AuthManager
    @EnvironmentObject var connectionStore: ConnectionStore

    var body: some View {
        ForEach(authManager.servers) { server in
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(server.name)
                        .font(.body)
                    Text(server.url.absoluteString)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                if server.id == authManager.activeServer?.id {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                } else if server.apiKey != nil {
                    Button("Connect") {
                        authManager.switchServer(server)
                        Task {
                            await connectionStore.connect(
                                to: server, authManager: authManager
                            )
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                } else {
                    Text("No key")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                }
            }
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                Button(role: .destructive) {
                    authManager.removeServer(server)
                } label: {
                    Label("Delete", systemImage: "trash")
                }
            }
        }
    }
}
