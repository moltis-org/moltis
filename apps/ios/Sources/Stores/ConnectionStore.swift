import Foundation
import os

// MARK: - Connection state

enum ConnectionState: Equatable {
    case disconnected
    case connecting
    case reconnecting(attempt: Int, nextRetryIn: TimeInterval)
    case connected
    case error(String)

    var isConnected: Bool {
        if case .connected = self { return true }
        return false
    }

    var statusText: String {
        switch self {
        case .disconnected: return "Disconnected"
        case .connecting: return "Connecting..."
        case .reconnecting(let attempt, let nextRetryIn):
            let seconds = max(1, Int(nextRetryIn.rounded(.up)))
            return "Server unavailable. Retrying in \(seconds)s (attempt \(attempt))..."
        case .connected: return "Connected"
        case .error(let msg): return "Error: \(msg)"
        }
    }
}

// MARK: - Connection store

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var state: ConnectionState = .disconnected
    @Published var serverVersion: String?
    @Published var serverHost: String?
    @Published var agentName: String?
    @Published var agentEmoji: String?

    let wsClient = MoltisWSClient()
    let graphqlClient = MoltisGraphQLClient()

    private let logger = Logger(subsystem: "org.moltis.ios", category: "connection")

    lazy var chatStore: ChatStore = ChatStore(connectionStore: self)
    lazy var sessionStore: SessionStore = SessionStore(connectionStore: self)
    lazy var modelStore: ModelStore = ModelStore(connectionStore: self)

    init() {
        Task { [weak self] in
            await self?.wsClient.onStateChange { [weak self] wsState in
                Task { @MainActor in
                    self?.applyWSState(wsState)
                }
            }
        }
    }

    // MARK: - Connect

    func connect(to server: ServerConnection, authManager: AuthManager) async {
        state = .connecting
        await graphqlClient.configure(server: server)

        do {
            let hello = try await wsClient.connect(to: server)
            serverVersion = hello.server?.version
            serverHost = hello.server?.host
            state = .connected

            // Register event handlers
            await wsClient.clearEventHandlers()
            await wsClient.onEvent { [weak self] event, payload in
                Task { @MainActor in
                    self?.handleEvent(event: event, payload: payload)
                }
            }

            // Fetch identity
            await fetchIdentity()

            // Load initial data
            await sessionStore.loadSessions()
            await modelStore.loadModels()

        } catch {
            logger.error("Connection failed: \(error.localizedDescription)")
            state = .error(error.localizedDescription)
        }
    }

    func disconnect() async {
        await wsClient.disconnect()
        state = .disconnected
        serverVersion = nil
        serverHost = nil
        agentName = nil
        agentEmoji = nil
    }

    private func applyWSState(_ wsState: MoltisWSClient.State) {
        let wasConnected = state.isConnected
        switch wsState {
        case .disconnected:
            state = .disconnected
        case .connecting:
            state = .connecting
        case .reconnecting(let attempt, let nextRetryIn):
            state = .reconnecting(attempt: attempt, nextRetryIn: nextRetryIn)
        case .connected:
            state = .connected
            if !wasConnected {
                onReconnected()
            }
        case .error(let message):
            state = .error(message)
        }
    }

    /// Reload data after a successful reconnection.
    private func onReconnected() {
        Task {
            await fetchIdentity()
            await sessionStore.loadSessions()
            await modelStore.loadModels()
            // Location sharing reconnection is handled by MoltisApp via
            // .onChange(of: connectionStore.state).
        }
    }

    // MARK: - Event dispatch

    private func handleEvent(event: String, payload: ChatEventPayload) {
        switch event {
        case "chat":
            chatStore.handleChatEvent(payload)
        case "models.updated":
            Task { await modelStore.loadModels() }
        default:
            break
        }
    }

    // MARK: - Identity

    private func fetchIdentity() async {
        do {
            let response = try await wsClient.send(method: "agent.identity.get")
            if let payload = response.payload,
               let dict = payload.value as? [String: Any] {
                agentName = dict["name"] as? String
                agentEmoji = dict["emoji"] as? String
            }
        } catch {
            logger.debug("Could not fetch identity: \(error.localizedDescription)")
        }
    }
}
