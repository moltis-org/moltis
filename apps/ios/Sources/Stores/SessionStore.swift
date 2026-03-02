import Foundation
import os

@MainActor
final class SessionStore: ObservableObject {
    @Published var sessions: [ChatSession] = []
    @Published var isLoading = false

    private weak var connectionStore: ConnectionStore?
    private let logger = Logger(subsystem: "org.moltis.ios", category: "sessions")

    init(connectionStore: ConnectionStore) {
        self.connectionStore = connectionStore
    }

    // MARK: - Load sessions

    func loadSessions() async {
        guard let graphqlClient = connectionStore?.graphqlClient else { return }
        isLoading = true
        defer { isLoading = false }

        do {
            let gqlSessions = try await graphqlClient.fetchSessions()
            // Server returns sessions pre-sorted: main first, then by updated_at DESC.
            sessions = gqlSessions.map { ChatSession.from($0) }
        } catch {
            logger.error("Failed to load sessions: \(error.localizedDescription)")
        }
    }

    // MARK: - Search

    func searchSessions(query: String) async {
        guard let graphqlClient = connectionStore?.graphqlClient else { return }
        guard !query.isEmpty else {
            await loadSessions()
            return
        }

        do {
            let gqlSessions = try await graphqlClient.searchSessions(query: query)
            sessions = gqlSessions.map { ChatSession.from($0) }
        } catch {
            logger.error("Failed to search sessions: \(error.localizedDescription)")
        }
    }

    // MARK: - Create session

    func createSession() async -> String? {
        let key = "session:\(UUID().uuidString.lowercased())"
        logger.info("Prepared new session key: \(key, privacy: .public)")
        return key
    }

    // MARK: - Delete session

    func deleteSession(key: String) async {
        guard let wsClient = connectionStore?.wsClient else { return }
        do {
            let params: [String: AnyCodable] = ["key": AnyCodable(key)]
            _ = try await wsClient.send(method: "sessions.delete", params: params)
            sessions.removeAll { $0.key == key }
        } catch {
            logger.error("Failed to delete session: \(error.localizedDescription)")
        }
    }
}
