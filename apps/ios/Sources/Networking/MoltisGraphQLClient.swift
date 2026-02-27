import Foundation
import os

// MARK: - GraphQL types

struct GraphQLRequest: Encodable {
    let query: String
    let variables: [String: AnyCodable]?
    let operationName: String?

    init(query: String, variables: [String: AnyCodable]? = nil, operationName: String? = nil) {
        self.query = query
        self.variables = variables
        self.operationName = operationName
    }
}

struct GraphQLResponse<T: Decodable>: Decodable {
    let data: T?
    let errors: [GraphQLError]?
}

struct GraphQLError: Decodable, LocalizedError {
    let message: String
    let locations: [Location]?
    let path: [String]?

    struct Location: Decodable {
        let line: Int
        let column: Int
    }

    var errorDescription: String? { message }
}

// MARK: - GraphQL client

actor MoltisGraphQLClient {
    private let logger = Logger(subsystem: "org.moltis.ios", category: "graphql")
    private var server: ServerConnection?
    private let urlSession = URLSession.shared

    func configure(server: ServerConnection) {
        self.server = server
    }

    // MARK: - Queries

    func query<T: Decodable>(
        _ queryString: String,
        variables: [String: AnyCodable]? = nil,
        operationName: String? = nil,
        as type: T.Type
    ) async throws -> T {
        guard let server else {
            throw AuthError.serverError(0, "GraphQL client not configured")
        }
        guard let apiKey = server.apiKey else {
            throw AuthError.noApiKey
        }

        var request = URLRequest(url: server.graphqlURL)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer \(apiKey)", forHTTPHeaderField: "Authorization")
        request.timeoutInterval = 30

        let opName = operationName ?? "anonymous"
        logger.debug("GraphQL request started: \(opName, privacy: .public)")

        let gqlRequest = GraphQLRequest(
            query: queryString,
            variables: variables,
            operationName: operationName
        )
        request.httpBody = try JSONEncoder().encode(gqlRequest)

        let (data, response) = try await urlSession.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw AuthError.invalidURL
        }
        guard httpResponse.statusCode == 200 else {
            let body = String(data: data, encoding: .utf8) ?? "Unknown error"
            logger.error(
                "GraphQL HTTP error op=\(opName, privacy: .public) status=\(httpResponse.statusCode) body=\(body, privacy: .public)"
            )
            throw AuthError.serverError(httpResponse.statusCode, body)
        }

        let gqlResponse = try JSONDecoder().decode(GraphQLResponse<T>.self, from: data)
        if let errors = gqlResponse.errors, let first = errors.first {
            let joined = errors.map(\.message).joined(separator: " | ")
            logger.error(
                "GraphQL resolver error op=\(opName, privacy: .public) messages=\(joined, privacy: .public)"
            )
            throw first
        }
        guard let result = gqlResponse.data else {
            throw AuthError.serverError(0, "No data in GraphQL response")
        }
        logger.debug("GraphQL request succeeded: \(opName, privacy: .public)")
        return result
    }

    // MARK: - Standard queries

    func fetchSessions() async throws -> [GQLSession] {
        struct Response: Decodable {
            let sessions: SessionsData
            struct SessionsData: Decodable {
                let list: [GQLSession]
            }
        }
        let result: Response = try await query("""
            query {
                sessions {
                    list {
                        id
                        key
                        label
                        model
                        createdAt
                        updatedAt
                        messageCount
                        archived
                    }
                }
            }
            """, operationName: "FetchSessions", as: Response.self)
        return result.sessions.list
    }

    func searchSessions(query searchQuery: String) async throws -> [GQLSession] {
        struct Response: Decodable {
            let sessions: SessionsData
            struct SessionsData: Decodable {
                let search: [GQLSession]
            }
        }
        let result: Response = try await query("""
            query($query: String!) {
                sessions {
                    search(query: $query) {
                        id
                        key
                        label
                        model
                        createdAt
                        updatedAt
                        messageCount
                        archived
                    }
                }
            }
            """,
            variables: ["query": AnyCodable(searchQuery)],
            operationName: "SearchSessions",
            as: Response.self
        )
        return result.sessions.search
    }

    func fetchModels() async throws -> [GQLModel] {
        struct Response: Decodable {
            let models: ModelsData
            struct ModelsData: Decodable {
                let list: [GQLModel]
            }
        }
        let result: Response = try await query("""
            query {
                models {
                    list {
                        id
                        name
                        provider
                    }
                }
            }
            """, operationName: "FetchModels", as: Response.self)
        return result.models.list
    }

    func fetchStatus() async throws -> GQLStatus {
        struct Response: Decodable {
            let status: GQLStatus
        }
        let result: Response = try await query("""
            query {
                status {
                    hostname
                    version
                    connections
                    uptimeMs
                }
            }
            """, operationName: "FetchStatus", as: Response.self)
        return result.status
    }
}

// MARK: - GraphQL data models

struct GQLSession: Decodable, Identifiable, Equatable {
    let id: String
    let key: String
    let label: String?
    let model: String?
    let createdAt: String?
    let updatedAt: String?
    let messageCount: Int?
    let archived: Bool?
}

struct GQLModel: Decodable, Identifiable, Equatable {
    let id: String?
    let name: String?
    let provider: String?
    let tier: String?
}

struct GQLStatus: Decodable {
    let hostname: String?
    let version: String?
    let connections: Int?
    let uptimeMs: Int?
}
