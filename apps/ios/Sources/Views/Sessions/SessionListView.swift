import SwiftUI

struct SessionListView: View {
    @EnvironmentObject var sessionStore: SessionStore
    @EnvironmentObject var chatStore: ChatStore
    @State private var searchText = ""

    var body: some View {
        NavigationStack {
            List {
                ForEach(filteredSessions) { session in
                    SessionRow(session: session,
                               isActive: session.key == chatStore.currentSessionKey)
                        .contentShape(Rectangle())
                        .onTapGesture {
                            Task {
                                await chatStore.switchSession(key: session.key)
                            }
                        }
                }
                .onDelete(perform: deleteSessions)
            }
            .listStyle(.plain)
            .searchable(text: $searchText, prompt: "Search sessions")
            .onChange(of: searchText) { _, newValue in
                Task { await sessionStore.searchSessions(query: newValue) }
            }
            .navigationTitle("Sessions")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task {
                            if let key = await sessionStore.createSession() {
                                await chatStore.switchSession(key: key)
                            }
                        }
                    } label: {
                        Image(systemName: "plus")
                    }
                }
            }
            .refreshable {
                await sessionStore.loadSessions()
            }
            .overlay {
                if sessionStore.sessions.isEmpty && !sessionStore.isLoading {
                    ContentUnavailableView(
                        "No Sessions",
                        systemImage: "bubble.left.and.bubble.right",
                        description: Text("Start a new conversation")
                    )
                }
            }
        }
    }

    private var filteredSessions: [ChatSession] {
        sessionStore.sessions.filter { !$0.archived }
    }

    private func deleteSessions(at offsets: IndexSet) {
        let sessions = filteredSessions
        for index in offsets {
            let session = sessions[index]
            Task { await sessionStore.deleteSession(key: session.key) }
        }
    }
}
