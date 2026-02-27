import Foundation

struct ServerConnection: Identifiable, Codable, Equatable {
    var id: UUID
    var name: String
    var url: URL
    var keychainKey: String

    init(id: UUID = UUID(), name: String, url: URL) {
        self.id = id
        self.name = name
        self.url = url
        self.keychainKey = "apikey-\(id.uuidString)"
    }

    var apiKey: String? {
        KeychainHelper.loadString(key: keychainKey)
    }

    @discardableResult
    func saveApiKey(_ key: String) -> Bool {
        KeychainHelper.save(key: keychainKey, string: key)
    }

    func deleteApiKey() {
        KeychainHelper.delete(key: keychainKey)
    }

    /// Base URL with trailing slash stripped.
    var baseURL: URL {
        var urlString = url.absoluteString
        while urlString.hasSuffix("/") {
            urlString.removeLast()
        }
        return URL(string: urlString) ?? url
    }

    /// WebSocket URL for the chat endpoint.
    var wsURL: URL {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        components?.scheme = baseURL.scheme == "https" ? "wss" : "ws"
        components?.path += "/ws/chat"
        return components?.url ?? baseURL
    }

    /// GraphQL HTTP endpoint.
    var graphqlURL: URL {
        baseURL.appendingPathComponent("graphql")
    }
}
