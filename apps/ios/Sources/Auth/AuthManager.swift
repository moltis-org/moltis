import AuthenticationServices
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
    let hasPasskeys: Bool
    let setupCodeRequired: Bool
    let graphqlEnabled: Bool?

    enum CodingKeys: String, CodingKey {
        case setupRequired = "setup_required"
        case setupComplete = "setup_complete"
        case authenticated
        case authDisabled = "auth_disabled"
        case hasPassword = "has_password"
        case hasPasskeys = "has_passkeys"
        case setupCodeRequired = "setup_code_required"
        case graphqlEnabled = "graphql_enabled"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        setupRequired = try container.decode(Bool.self, forKey: .setupRequired)
        setupComplete = try container.decode(Bool.self, forKey: .setupComplete)
        authenticated = try container.decode(Bool.self, forKey: .authenticated)
        authDisabled = try container.decode(Bool.self, forKey: .authDisabled)
        hasPassword = try container.decode(Bool.self, forKey: .hasPassword)
        hasPasskeys = try container.decodeIfPresent(Bool.self, forKey: .hasPasskeys) ?? false
        setupCodeRequired = try container.decode(Bool.self, forKey: .setupCodeRequired)
        graphqlEnabled = try container.decodeIfPresent(Bool.self, forKey: .graphqlEnabled)
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

private struct PasskeyAuthBeginResponse: Codable {
    let challengeId: String
    let options: PasskeyPublicKeyCredentialRequestOptions

    enum CodingKeys: String, CodingKey {
        case challengeId = "challenge_id"
        case options
    }
}

private struct PasskeyPublicKeyCredentialRequestOptions: Codable {
    let publicKey: PasskeyRequestPublicKey

    enum CodingKeys: String, CodingKey {
        case publicKey = "publicKey"
    }
}

private struct PasskeyRequestPublicKey: Codable {
    let challenge: String
    let rpId: String?
    let allowCredentials: [PasskeyCredentialDescriptor]?
    let userVerification: String?

    enum CodingKeys: String, CodingKey {
        case challenge
        case rpId
        case allowCredentials
        case userVerification
    }
}

private struct PasskeyCredentialDescriptor: Codable {
    let id: String
}

private struct PasskeyAuthFinishRequest: Codable {
    let challengeId: String
    let credential: PasskeyFinishCredential

    enum CodingKeys: String, CodingKey {
        case challengeId = "challenge_id"
        case credential
    }
}

private struct PasskeyFinishCredential: Codable {
    let id: String
    let rawId: String
    let type: String
    let response: PasskeyFinishCredentialResponse
}

private struct PasskeyFinishCredentialResponse: Codable {
    let authenticatorData: String
    let clientDataJSON: String
    let signature: String
    let userHandle: String?
}

@available(iOS 15.0, *)
private final class PasskeyAssertionAuthorizationDelegate: NSObject,
    ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    private static var activeDelegates: [PasskeyAssertionAuthorizationDelegate] = []

    private var continuation: CheckedContinuation<ASAuthorizationPlatformPublicKeyCredentialAssertion, Error>?

    private init(continuation: CheckedContinuation<ASAuthorizationPlatformPublicKeyCredentialAssertion, Error>) {
        self.continuation = continuation
    }

    static func perform(
        request: ASAuthorizationPlatformPublicKeyCredentialAssertionRequest
    ) async throws -> ASAuthorizationPlatformPublicKeyCredentialAssertion {
        try await withCheckedThrowingContinuation { continuation in
            let delegate = PasskeyAssertionAuthorizationDelegate(continuation: continuation)
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = delegate
            controller.presentationContextProvider = delegate
            activeDelegates.append(delegate)
            controller.performRequests()
        }
    }

    func presentationAnchor(for _: ASAuthorizationController) -> ASPresentationAnchor {
        if let keyWindow = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows)
            .first(where: { $0.isKeyWindow })
        {
            return keyWindow
        }

        if let firstWindow = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows)
            .first
        {
            return firstWindow
        }

        return ASPresentationAnchor()
    }

    func authorizationController(
        controller _: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        guard let assertion = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion else {
            complete(.failure(AuthError.passkeyAuthorizationFailed))
            return
        }
        complete(.success(assertion))
    }

    func authorizationController(controller _: ASAuthorizationController, didCompleteWithError error: Error) {
        if let authError = error as? ASAuthorizationError,
           authError.code == .canceled
        {
            complete(.failure(AuthError.passkeyCancelled))
            return
        }
        complete(.failure(error))
    }

    private func complete(_ result: Result<ASAuthorizationPlatformPublicKeyCredentialAssertion, Error>) {
        guard let continuation else { return }
        self.continuation = nil
        Self.activeDelegates.removeAll { $0 === self }
        continuation.resume(with: result)
    }
}

// MARK: - Auth errors

enum AuthError: LocalizedError {
    case invalidURL
    case networkError(Error)
    case serverError(Int, String)
    case setupRequired
    case invalidCredentials
    case noApiKey
    case passkeyUnavailable
    case passkeyCancelled
    case passkeyChallengeInvalid
    case passkeyAuthorizationFailed

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
        case .passkeyUnavailable:
            return "Passkeys are not available on this device."
        case .passkeyCancelled:
            return "Passkey sign-in was cancelled."
        case .passkeyChallengeInvalid:
            return "Server returned an invalid passkey challenge."
        case .passkeyAuthorizationFailed:
            return "Passkey authorization failed."
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

    /// Login with passkey, then create an API key for persistent access.
    func loginWithPasskeyAndCreateApiKey(
        serverURL: URL,
        serverName: String
    ) async throws -> ServerConnection {
        isAuthenticating = true
        authError = nil
        defer { isAuthenticating = false }

        let baseURL = ServerConnection.normalizedURL(serverURL)

        // 1. Login with passkey to get a session cookie.
        let sessionCookie = try await loginWithPasskey(baseURL: baseURL)

        // 2. Create an API key using the session.
        let apiKey = try await createApiKey(baseURL: baseURL, sessionCookie: sessionCookie)

        // 3. Save the server.
        let server = ServerConnection(name: serverName, url: serverURL)
        server.saveApiKey(apiKey)
        upsertAndActivate(server)

        logger.info("Authenticated with passkey to \(serverURL.absoluteString)")
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

        if let sessionCookie = Self.sessionCookieValue(for: loginURL, response: httpResponse) {
            return sessionCookie
        }

        throw AuthError.serverError(200, "No session cookie received")
    }

    private func loginWithPasskey(baseURL: URL) async throws -> String {
        guard #available(iOS 15.0, *) else {
            throw AuthError.passkeyUnavailable
        }

        let beginURL = baseURL.appendingPathComponent("api/auth/passkey/auth/begin")
        var beginRequest = URLRequest(url: beginURL)
        beginRequest.httpMethod = "POST"
        beginRequest.timeoutInterval = 10

        let (beginData, beginResponse) = try await URLSession.shared.data(for: beginRequest)
        guard let beginHttpResponse = beginResponse as? HTTPURLResponse else {
            throw AuthError.invalidURL
        }
        guard beginHttpResponse.statusCode == 200 else {
            let body = String(data: beginData, encoding: .utf8) ?? "Unknown error"
            throw AuthError.serverError(beginHttpResponse.statusCode, body)
        }

        let beginPayload = try JSONDecoder().decode(PasskeyAuthBeginResponse.self, from: beginData)
        let options = beginPayload.options.publicKey
        guard let challenge = Self.decodeBase64URL(options.challenge) else {
            throw AuthError.passkeyChallengeInvalid
        }

        guard let rpID = Self.relyingPartyIdentifier(rpIdFromServer: options.rpId, baseURL: baseURL) else {
            throw AuthError.invalidURL
        }

        let assertion = try await createPasskeyAssertion(
            relyingPartyIdentifier: rpID,
            challenge: challenge,
            allowCredentials: options.allowCredentials,
            userVerification: options.userVerification
        )

        let finishPayload = PasskeyAuthFinishRequest(
            challengeId: beginPayload.challengeId,
            credential: PasskeyFinishCredential(
                id: Self.encodeBase64URL(assertion.credentialID),
                rawId: Self.encodeBase64URL(assertion.credentialID),
                type: "public-key",
                response: PasskeyFinishCredentialResponse(
                    authenticatorData: Self.encodeBase64URL(assertion.rawAuthenticatorData),
                    clientDataJSON: Self.encodeBase64URL(assertion.rawClientDataJSON),
                    signature: Self.encodeBase64URL(assertion.signature),
                    userHandle: assertion.userID.isEmpty ? nil : Self.encodeBase64URL(assertion.userID)
                )
            )
        )

        let finishURL = baseURL.appendingPathComponent("api/auth/passkey/auth/finish")
        var finishRequest = URLRequest(url: finishURL)
        finishRequest.httpMethod = "POST"
        finishRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        finishRequest.httpBody = try JSONEncoder().encode(finishPayload)
        finishRequest.timeoutInterval = 10

        let (finishData, finishResponse) = try await URLSession.shared.data(for: finishRequest)
        guard let finishHttpResponse = finishResponse as? HTTPURLResponse else {
            throw AuthError.invalidURL
        }
        guard finishHttpResponse.statusCode == 200 else {
            let body = String(data: finishData, encoding: .utf8) ?? "Unknown error"
            throw AuthError.serverError(finishHttpResponse.statusCode, body)
        }

        if let sessionCookie = Self.sessionCookieValue(for: finishURL, response: finishHttpResponse) {
            return sessionCookie
        }

        throw AuthError.serverError(200, "No session cookie received")
    }

    @available(iOS 15.0, *)
    private func createPasskeyAssertion(
        relyingPartyIdentifier: String,
        challenge: Data,
        allowCredentials: [PasskeyCredentialDescriptor]?,
        userVerification: String?
    ) async throws -> ASAuthorizationPlatformPublicKeyCredentialAssertion {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: relyingPartyIdentifier
        )
        let request = provider.createCredentialAssertionRequest(challenge: challenge)
        request.relyingPartyIdentifier = relyingPartyIdentifier
        request.userVerificationPreference = Self.userVerificationPreference(from: userVerification)

        if let allowCredentials, !allowCredentials.isEmpty {
            request.allowedCredentials = allowCredentials.compactMap { descriptor in
                guard let credentialID = Self.decodeBase64URL(descriptor.id) else {
                    return nil
                }
                return ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: credentialID)
            }
        }

        return try await PasskeyAssertionAuthorizationDelegate.perform(request: request)
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

    private static func relyingPartyIdentifier(rpIdFromServer: String?, baseURL: URL) -> String? {
        let rpId = normalizeHost(rpIdFromServer) ?? normalizeHost(baseURL.host)
        guard let rpId, !rpId.isEmpty else {
            return nil
        }
        return rpId
    }

    private static func normalizeHost(_ host: String?) -> String? {
        guard var host else { return nil }
        host = host.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        while host.hasSuffix(".") {
            host.removeLast()
        }
        return host.isEmpty ? nil : host
    }

    private static func userVerificationPreference(
        from value: String?
    ) -> ASAuthorizationPublicKeyCredentialUserVerificationPreference {
        switch value?.lowercased() {
        case "required":
            return .required
        case "discouraged":
            return .discouraged
        default:
            return .preferred
        }
    }

    private static func decodeBase64URL(_ value: String) -> Data? {
        var normalized = value
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")

        let remainder = normalized.count % 4
        if remainder != 0 {
            normalized += String(repeating: "=", count: 4 - remainder)
        }
        return Data(base64Encoded: normalized)
    }

    private static func encodeBase64URL(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    private static func sessionCookieValue(for url: URL, response: HTTPURLResponse) -> String? {
        let cookies = HTTPCookieStorage.shared.cookies(for: url) ?? []
        if let sessionCookie = cookies.first(where: { $0.name == "moltis_session" }) {
            return sessionCookie.value
        }

        if let setCookie = response.value(forHTTPHeaderField: "Set-Cookie"),
           let range = setCookie.range(of: "moltis_session=")
        {
            let valueStart = range.upperBound
            let valueEnd = setCookie[valueStart...].firstIndex(of: ";") ?? setCookie.endIndex
            return String(setCookie[valueStart..<valueEnd])
        }

        return nil
    }
}
