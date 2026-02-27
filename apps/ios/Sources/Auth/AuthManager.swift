import Foundation
import UIKit
import os

// MARK: - Auth status response

struct AuthStatusResponse: Codable {
    let setupRequired: Bool
    let setupComplete: Bool
    let authenticated: Bool
    let authDisabled: Bool
    let hasPassword: Bool
    let setupCodeRequired: Bool
    let graphqlEnabled: Bool?

    enum CodingKeys: String, CodingKey {
        case setupRequired = "setup_required"
        case setupComplete = "setup_complete"
        case authenticated
        case authDisabled = "auth_disabled"
        case hasPassword = "has_password"
        case setupCodeRequired = "setup_code_required"
        case graphqlEnabled = "graphql_enabled"
    }
}

private struct GonStatusResponse: Codable {
    let graphqlEnabled: Bool?

    enum CodingKeys: String, CodingKey {
        case graphqlEnabled = "graphql_enabled"
    }
}

struct CreateApiKeyResponse: Codable {
    let id: Int
    let key: String
}

// MARK: - Auth errors

enum AuthError: LocalizedError {
    case invalidURL
    case networkError(Error)
    case serverError(Int, String)
    case setupRequired
    case invalidCredentials
    case noApiKey

    var errorDescription: String? {
        switch self {
        case .invalidURL:
            return "Invalid server URL"
        case .networkError(let error):
            return "Network error: \(error.localizedDescription)"
        case .serverError(let code, let message):
            return "Server error (\(code)): \(message)"
        case .setupRequired:
            return "Server requires initial setup"
        case .invalidCredentials:
            return "Invalid password"
        case .noApiKey:
            return "No API key available"
        }
    }
}

// MARK: - AuthManager

@MainActor
final class AuthManager: ObservableObject {
    @Published var servers: [ServerConnection] = []
    @Published var activeServer: ServerConnection?
    @Published var isAuthenticating = false
    @Published var authError: String?

    private let logger = Logger(subsystem: "org.moltis.ios", category: "auth")
    private let serversKey = "saved_servers"
    private let activeServerKey = "active_server_id"

    // MARK: - Server persistence

    func loadSavedServers() {
        guard let data = UserDefaults.standard.data(forKey: serversKey),
              let decoded = try? JSONDecoder().decode([ServerConnection].self, from: data) else {
            return
        }
        servers = decoded
        if let activeId = UserDefaults.standard.string(forKey: activeServerKey),
           let uuid = UUID(uuidString: activeId),
           let server = servers.first(where: { $0.id == uuid }),
           server.apiKey != nil {
            activeServer = server
        }
    }

    private func saveServers() {
        guard let data = try? JSONEncoder().encode(servers) else { return }
        UserDefaults.standard.set(data, forKey: serversKey)
        if let active = activeServer {
            UserDefaults.standard.set(active.id.uuidString, forKey: activeServerKey)
        }
    }

    // MARK: - Auth flow

    /// Check the authentication status of a server.
    func checkStatus(url: URL) async throws -> AuthStatusResponse {
        guard let statusURL = endpointURL(baseURL: url, endpointPath: "/api/auth/status") else {
            throw AuthError.invalidURL
        }

        var request = URLRequest(url: statusURL)
        request.httpMethod = "GET"
        request.timeoutInterval = 10

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AuthError.invalidURL
        }
        guard httpResponse.statusCode == 200 else {
            let body = String(data: data, encoding: .utf8) ?? "Unknown error"
            throw AuthError.serverError(httpResponse.statusCode, body)
        }

        return try JSONDecoder().decode(AuthStatusResponse.self, from: data)
    }

    /// Check whether GraphQL is enabled for this server.
    /// Returns nil when this cannot be determined from server responses.
    func checkGraphQLEnabled(url: URL) async -> Bool? {
        guard let gonURL = endpointURL(baseURL: url, endpointPath: "/api/gon") else {
            return nil
        }

        var request = URLRequest(url: gonURL)
        request.httpMethod = "GET"
        request.timeoutInterval = 10

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let httpResponse = response as? HTTPURLResponse,
                  httpResponse.statusCode == 200 else {
                return nil
            }

            let gon = try JSONDecoder().decode(GonStatusResponse.self, from: data)
            return gon.graphqlEnabled
        } catch {
            return nil
        }
    }

    /// Login with password, then create an API key for persistent access.
    func loginAndCreateApiKey(
        serverURL: URL,
        password: String,
        serverName: String
    ) async throws -> ServerConnection {
        isAuthenticating = true
        authError = nil
        defer { isAuthenticating = false }

        let baseURL = ServerConnection.normalizedURL(serverURL)

        // 1. Login to get session cookie
        let sessionCookie = try await login(baseURL: baseURL, password: password)

        // 2. Create an API key using the session
        let apiKey = try await createApiKey(baseURL: baseURL, sessionCookie: sessionCookie)

        // 3. Save the server
        let server = ServerConnection(name: serverName, url: serverURL)
        server.saveApiKey(apiKey)
        upsertAndActivate(server)

        logger.info("Authenticated to \(serverURL.absoluteString)")
        return server
    }

    /// Connect using an existing API key.
    func connectWithApiKey(
        serverURL: URL,
        apiKey: String,
        serverName: String
    ) async throws -> ServerConnection {
        isAuthenticating = true
        authError = nil
        defer { isAuthenticating = false }

        // Validate the key by checking auth status with it
        let baseURL = ServerConnection.normalizedURL(serverURL)
        var request = URLRequest(url: baseURL.appendingPathComponent("api/gon"))
        request.httpMethod = "GET"
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.timeoutInterval = 10

        let (_, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200 else {
            throw AuthError.invalidCredentials
        }

        let server = ServerConnection(name: serverName, url: serverURL)
        server.saveApiKey(apiKey)
        upsertAndActivate(server)

        logger.info("Connected to \(serverURL.absoluteString) with API key")
        return server
    }

    /// Switch to a different saved server.
    func switchServer(_ server: ServerConnection) {
        guard server.apiKey != nil else { return }
        activeServer = server
        saveServers()
    }

    /// Remove a saved server and its API key.
    func removeServer(_ server: ServerConnection) {
        server.deleteApiKey()
        servers.removeAll { $0.id == server.id }
        if activeServer?.id == server.id {
            activeServer = servers.first(where: { $0.apiKey != nil })
        }
        saveServers()
    }

    /// Disconnect from the active server (but keep it saved).
    func disconnect() {
        activeServer = nil
        UserDefaults.standard.removeObject(forKey: activeServerKey)
    }

    // MARK: - Private helpers

    private func upsertAndActivate(_ server: ServerConnection) {
        if let idx = servers.firstIndex(where: { $0.url == server.url }) {
            servers[idx] = server
        } else {
            servers.append(server)
        }
        activeServer = server
        saveServers()
    }

    private func endpointURL(baseURL: URL, endpointPath: String) -> URL? {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        let normalizedBasePath = components?.path.hasSuffix("/") == true
            ? String(components?.path.dropLast() ?? "")
            : (components?.path ?? "")
        components?.path = normalizedBasePath + endpointPath
        return components?.url
    }

    private func login(baseURL: URL, password: String) async throws -> String {
        let loginURL = baseURL.appendingPathComponent("api/auth/login")
        var request = URLRequest(url: loginURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(["password": password])
        request.timeoutInterval = 10

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AuthError.invalidURL
        }

        if httpResponse.statusCode == 401 {
            throw AuthError.invalidCredentials
        }
        guard httpResponse.statusCode == 200 else {
            let body = String(data: data, encoding: .utf8) ?? "Unknown error"
            throw AuthError.serverError(httpResponse.statusCode, body)
        }

        // Extract session cookie
        let cookies = HTTPCookieStorage.shared.cookies(for: loginURL) ?? []
        guard let sessionCookie = cookies.first(where: { $0.name == "moltis_session" }) else {
            // If no cookie found in storage, try to parse from headers
            if let setCookie = httpResponse.value(forHTTPHeaderField: "Set-Cookie"),
               let range = setCookie.range(of: "moltis_session=") {
                let valueStart = range.upperBound
                let valueEnd = setCookie[valueStart...].firstIndex(of: ";") ?? setCookie.endIndex
                return String(setCookie[valueStart..<valueEnd])
            }
            throw AuthError.serverError(200, "No session cookie received")
        }

        return sessionCookie.value
    }

    private func createApiKey(baseURL: URL, sessionCookie: String) async throws -> String {
        let apiKeysURL = baseURL.appendingPathComponent("api/auth/api-keys")
        var request = URLRequest(url: apiKeysURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("moltis_session=\(sessionCookie)", forHTTPHeaderField: "Cookie")
        request.timeoutInterval = 10

        let body: [String: Any] = [
            "label": "Moltis iOS (\(UIDevice.current.name))",
            "scopes": ["operator.read", "operator.write"]
        ]
        request.httpBody = try JSONSerialization.data(withJSONObject: body)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200 else {
            let body = String(data: data, encoding: .utf8) ?? "Unknown error"
            throw AuthError.serverError(
                (response as? HTTPURLResponse)?.statusCode ?? 0, body
            )
        }

        let decoded = try JSONDecoder().decode(CreateApiKeyResponse.self, from: data)
        return decoded.key
    }
}
