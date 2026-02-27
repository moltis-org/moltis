import Foundation
import UIKit
import os

// MARK: - WebSocket client

actor MoltisWSClient {
    typealias EventHandler = @Sendable (String, ChatEventPayload) -> Void
    typealias StateHandler = @Sendable (State) -> Void

    enum State: Sendable, Equatable {
        case disconnected
        case connecting
        case reconnecting(attempt: Int, nextRetryIn: TimeInterval)
        case connected
        case error(String)
    }

    private let logger = Logger(subsystem: "org.moltis.ios", category: "ws")

    private var webSocketTask: URLSessionWebSocketTask?
    private var session: URLSession?
    private var server: ServerConnection?
    private var apiKey: String?

    private(set) var state: State = .disconnected
    private var pendingRequests: [String: CheckedContinuation<RPCResponse, Error>] = [:]
    private var pendingRequestMethods: [String: String] = [:]
    private var eventHandlers: [EventHandler] = []
    private var stateHandlers: [StateHandler] = []
    private var receiveTask: Task<Void, Never>?
    private var reconnectTask: Task<Void, Never>?
    private var reconnectAttempts = 0
    private var shouldReconnect = false
    private var helloPayload: HelloOkPayload?

    var connectedServer: HelloOkPayload? { helloPayload }

    // MARK: - Event registration

    func onEvent(_ handler: @escaping EventHandler) {
        eventHandlers.append(handler)
    }

    func onStateChange(_ handler: @escaping StateHandler) {
        stateHandlers.append(handler)
        handler(state)
    }

    func clearEventHandlers() {
        eventHandlers.removeAll()
    }

    // MARK: - Connect

    func connect(to server: ServerConnection) async throws -> HelloOkPayload {
        disconnect()

        self.server = server
        self.apiKey = server.apiKey
        self.shouldReconnect = true
        self.reconnectAttempts = 0
        updateState(.connecting)

        guard let apiKey = server.apiKey else {
            updateState(.error("No API key"))
            throw AuthError.noApiKey
        }

        return try await performConnect(server: server, apiKey: apiKey)
    }

    private func performConnect(
        server: ServerConnection,
        apiKey: String
    ) async throws -> HelloOkPayload {
        let session = URLSession(configuration: .default)
        self.session = session

        var request = URLRequest(url: server.wsURL)
        request.timeoutInterval = TimeInterval(MoltisProtocol.handshakeTimeoutMs) / 1000.0
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")

        let task = session.webSocketTask(with: request)
        self.webSocketTask = task
        task.resume()

        // Start receive loop
        receiveTask = Task { [weak self] in
            await self?.receiveLoop()
        }

        // Capture device info (MainActor-isolated)
        let deviceName = await UIDevice.current.name
        let vendorId = await UIDevice.current.identifierForVendor?.uuidString ?? UUID().uuidString
        let appVersion = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String
            ?? "1.0.0"

        // Send connect handshake
        let connectParams = ConnectParams(
            minProtocol: MoltisProtocol.version,
            maxProtocol: MoltisProtocol.version,
            client: .init(
                id: "ios",
                displayName: deviceName,
                version: appVersion,
                platform: "ios",
                mode: "operator",
                instanceId: vendorId
            ),
            auth: .init(api_key: apiKey),
            locale: Locale.current.identifier,
            role: "operator"
        )

        let response = try await sendRequest(method: "connect", params: connectParams)

        guard response.ok == true, let payload = response.payload else {
            let errorMsg = response.error?.message ?? "Connection rejected"
            updateState(.error(errorMsg))
            throw AuthError.serverError(0, errorMsg)
        }

        // Decode hello-ok payload
        let jsonData = try JSONEncoder().encode(payload)
        let hello = try JSONDecoder().decode(HelloOkPayload.self, from: jsonData)
        self.helloPayload = hello
        updateState(.connected)
        reconnectAttempts = 0

        logger.info("Connected to \(server.url.absoluteString) (v\(hello.server?.version ?? "?"))")
        return hello
    }

    // MARK: - Disconnect

    func disconnect() {
        shouldReconnect = false
        reconnectTask?.cancel()
        reconnectTask = nil
        receiveTask?.cancel()
        receiveTask = nil
        webSocketTask?.cancel(with: .normalClosure, reason: nil)
        webSocketTask = nil
        session?.invalidateAndCancel()
        session = nil
        helloPayload = nil
        updateState(.disconnected)

        // Fail all pending requests
        for (_, continuation) in pendingRequests {
            continuation.resume(throwing: CancellationError())
        }
        pendingRequests.removeAll()
        pendingRequestMethods.removeAll()
    }

    // MARK: - Send RPC

    func send(method: String, params: [String: AnyCodable]? = nil) async throws -> RPCResponse {
        try await sendRequest(method: method, params: params)
    }

    private func sendRequest<P: Encodable>(
        method: String,
        params: P?
    ) async throws -> RPCResponse {
        guard let webSocketTask, webSocketTask.state == .running else {
            throw AuthError.serverError(0, "WebSocket not connected")
        }

        logger.debug("RPC -> \(method, privacy: .public)")

        let requestId = UUID().uuidString
        var payload: [String: AnyCodable] = [
            "type": AnyCodable("req"),
            "id": AnyCodable(requestId),
            "method": AnyCodable(method)
        ]

        if let params {
            let encoded = try JSONEncoder().encode(params)
            if let dict = try JSONSerialization.jsonObject(with: encoded) as? [String: Any] {
                payload["params"] = AnyCodable(dict)
            }
        }

        let data = try JSONEncoder().encode(payload)
        let message = URLSessionWebSocketTask.Message.string(String(data: data, encoding: .utf8) ?? "")

        try await webSocketTask.send(message)

        let response = try await withCheckedThrowingContinuation { continuation in
            pendingRequests[requestId] = continuation
            pendingRequestMethods[requestId] = method
        }

        if response.ok == false || response.error != nil {
            let code = response.error?.code ?? "unknown"
            let message = response.error?.message ?? "Unknown RPC error"
            throw AuthError.serverError(0, "[\(code)] \(message)")
        }

        return response
    }

    // MARK: - Receive loop

    private func receiveLoop() async {
        guard let webSocketTask else { return }

        while !Task.isCancelled {
            do {
                let message = try await webSocketTask.receive()
                switch message {
                case .string(let text):
                    handleMessage(text)
                case .data(let data):
                    if let text = String(data: data, encoding: .utf8) {
                        handleMessage(text)
                    }
                @unknown default:
                    break
                }
            } catch {
                if !Task.isCancelled {
                    logger.warning("WebSocket receive error: \(error.localizedDescription)")
                    if shouldReconnect {
                        scheduleReconnect()
                    } else {
                        updateState(.error(error.localizedDescription))
                    }
                }
                return
            }
        }
    }

    private func handleMessage(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }

        do {
            let frame = try JSONDecoder().decode(RPCResponse.self, from: data)

            switch frame.type {
            case .res:
                // Match response to pending request
                if let id = frame.id {
                    let method = pendingRequestMethods.removeValue(forKey: id) ?? "unknown"
                    guard let continuation = pendingRequests.removeValue(forKey: id) else {
                        return
                    }
                    if frame.ok == false || frame.error != nil {
                        let code = frame.error?.code ?? "unknown"
                        let message = frame.error?.message ?? "Unknown RPC error"
                        logger.error(
                            "RPC <- error method=\(method, privacy: .public) code=\(code, privacy: .public) message=\(message, privacy: .public)"
                        )
                    } else {
                        logger.debug("RPC <- ok \(method, privacy: .public)")
                    }
                    continuation.resume(returning: frame)
                }

            case .event:
                guard let eventName = frame.event else { return }

                // Handle tick (heartbeat)
                if eventName == "tick" { return }

                if eventName == "chat" {
                    guard let payloadValue = frame.payload else { return }
                    let payloadData = try JSONEncoder().encode(payloadValue)
                    let chatPayload = try JSONDecoder().decode(
                        ChatEventPayload.self, from: payloadData
                    )
                    for handler in eventHandlers {
                        handler(eventName, chatPayload)
                    }
                    return
                }

                // Non-chat events observed by iOS (e.g. models.updated) do not
                // necessarily match ChatEventPayload. Forward them with an empty
                // payload so subscribers can still react on event name.
                for handler in eventHandlers {
                    handler(eventName, .empty)
                }

            case nil:
                // Unknown or unrecognized frame type — ignore silently.
                break
            }
        } catch {
            let preview = text.prefix(300)
            if let decodingError = error as? DecodingError {
                logger.warning("Failed to decode message: \(decodingError) raw=\(preview, privacy: .public)")
            } else {
                logger.warning("Failed to decode message: \(error.localizedDescription) raw=\(preview, privacy: .public)")
            }
        }
    }

    // MARK: - Reconnect

    private func scheduleReconnect() {
        guard shouldReconnect, reconnectTask == nil else { return }

        let nextAttempt = reconnectAttempts + 1
        let delay = min(1.0 * pow(1.5, Double(reconnectAttempts)), 5.0)
        updateState(.reconnecting(attempt: nextAttempt, nextRetryIn: delay))

        reconnectTask = Task { [weak self] in
            guard let self else { return }

            try? await Task.sleep(for: .seconds(delay))

            guard !Task.isCancelled else { return }
            await self.attemptReconnect()
        }
    }

    private func attemptReconnect() {
        guard shouldReconnect, let server, let apiKey else { return }

        reconnectAttempts += 1
        reconnectTask = nil
        updateState(.connecting)

        let attempt = reconnectAttempts
        logger.info("Reconnecting (attempt \(attempt))...")

        Task {
            do {
                _ = try await performConnect(server: server, apiKey: apiKey)
            } catch {
                logger.warning("Reconnect failed: \(error.localizedDescription)")
                scheduleReconnect()
            }
        }
    }

    private func updateState(_ newState: State) {
        state = newState
        for handler in stateHandlers {
            handler(newState)
        }
    }
}
