import Apollo
import ApolloAPI
import Foundation
import os

// MARK: - GraphQL client

actor MoltisGraphQLClient {
    private let logger = Logger(subsystem: "org.moltis.ios", category: "graphql")
    private var server: ServerConnection?
    private var apolloClient: ApolloClient?

    func configure(server: ServerConnection) {
        self.server = server

        guard let apiKey = server.apiKey else {
            apolloClient = nil
            return
        }

        let store = ApolloStore(cache: InMemoryNormalizedCache())
        let transport = RequestChainNetworkTransport(
            urlSession: URLSession(configuration: .default),
            interceptorProvider: DefaultInterceptorProvider.shared,
            store: store,
            endpointURL: server.graphqlURL,
            additionalHeaders: [
                "Authorization": "Bearer \(apiKey)",
                "Content-Type": "application/json"
            ]
        )

        apolloClient = ApolloClient(networkTransport: transport, store: store)
    }

    // MARK: - Queries

    private func execute<Query: GraphQLQuery>(
        _ query: Query,
        operationName: String
    ) async throws -> Query.Data where Query.ResponseFormat == SingleResponseFormat {
        guard server != nil else {
            throw AuthError.serverError(0, "GraphQL client not configured")
        }
        guard let apolloClient else {
            throw AuthError.noApiKey
        }

        logger.debug("GraphQL request started: \(operationName, privacy: .public)")

        do {
            let response = try await apolloClient.fetch(
                query: query,
                cachePolicy: .networkOnly
            )

            if let errors = response.errors, !errors.isEmpty {
                let joined = errors.compactMap(\.message).joined(separator: " | ")
                logger.error(
                    "GraphQL resolver error op=\(operationName, privacy: .public) messages=\(joined, privacy: .public)"
                )
                throw AuthError.serverError(
                    0,
                    joined.isEmpty ? "GraphQL request failed" : joined
                )
            }

            guard let data = response.data else {
                throw AuthError.serverError(0, "No data in GraphQL response")
            }

            logger.debug("GraphQL request succeeded: \(operationName, privacy: .public)")
            return data
        } catch {
            logger.error(
                "GraphQL request failed op=\(operationName, privacy: .public) error=\(error.localizedDescription, privacy: .public)"
            )
            throw error
        }
    }

    private func execute<Mutation: GraphQLMutation>(
        _ mutation: Mutation,
        operationName: String
    ) async throws -> Mutation.Data where Mutation.ResponseFormat == SingleResponseFormat {
        guard server != nil else {
            throw AuthError.serverError(0, "GraphQL client not configured")
        }
        guard let apolloClient else {
            throw AuthError.noApiKey
        }

        logger.debug("GraphQL mutation started: \(operationName, privacy: .public)")

        do {
            let response = try await apolloClient.perform(mutation: mutation)

            if let errors = response.errors, !errors.isEmpty {
                let joined = errors.compactMap(\.message).joined(separator: " | ")
                logger.error(
                    "GraphQL resolver error op=\(operationName, privacy: .public) messages=\(joined, privacy: .public)"
                )
                throw AuthError.serverError(
                    0,
                    joined.isEmpty ? "GraphQL mutation failed" : joined
                )
            }

            guard let data = response.data else {
                throw AuthError.serverError(0, "No data in GraphQL response")
            }

            logger.debug("GraphQL mutation succeeded: \(operationName, privacy: .public)")
            return data
        } catch {
            logger.error(
                "GraphQL mutation failed op=\(operationName, privacy: .public) error=\(error.localizedDescription, privacy: .public)"
            )
            throw error
        }
    }

    // MARK: - Standard queries

    func fetchSessions() async throws -> [GQLSession] {
        let data = try await execute(MoltisAPI.FetchSessionsQuery(), operationName: "FetchSessions")
        return data.sessions.list.map { mapSession($0.fragments.sessionFields) }
    }

    func searchSessions(query searchQuery: String) async throws -> [GQLSession] {
        let data = try await execute(
            MoltisAPI.SearchSessionsQuery(query: searchQuery),
            operationName: "SearchSessions"
        )
        return data.sessions.search.map { mapSession($0.fragments.sessionFields) }
    }

    func fetchModels() async throws -> [GQLModel] {
        let data = try await execute(MoltisAPI.FetchModelsQuery(), operationName: "FetchModels")
        return data.models.list.map { model in
            GQLModel(
                id: model.id,
                name: model.name,
                provider: model.provider,
                tier: nil
            )
        }
    }

    private func mapSession(_ s: MoltisAPI.SessionFields) -> GQLSession {
        let key = s.key ?? ""
        return GQLSession(
            id: s.id ?? key,
            key: key,
            label: s.label,
            model: s.model,
            preview: s.preview,
            createdAt: s.createdAt,
            updatedAt: s.updatedAt,
            messageCount: s.messageCount,
            lastSeenMessageCount: s.lastSeenMessageCount,
            archived: s.archived
        )
    }

    func fetchStatus() async throws -> GQLStatus {
        let data = try await execute(MoltisAPI.FetchStatusQuery(), operationName: "FetchStatus")
        return GQLStatus(
            hostname: data.status.hostname,
            version: data.status.version,
            connections: data.status.connections,
            uptimeMs: data.status.uptimeMs
        )
    }

    func updateUserLocation(latitude: Double, longitude: Double) async throws -> Bool {
        let payload: [String: Any] = [
            "user_location": [
                "latitude": latitude,
                "longitude": longitude
            ]
        ]
        let payloadData = try JSONSerialization.data(withJSONObject: payload)
        guard let payloadString = String(data: payloadData, encoding: .utf8) else {
            throw AuthError.serverError(0, "Failed to encode location payload")
        }

        let data = try await execute(
            MoltisAPI.UpdateUserLocationMutation(input: payloadString),
            operationName: "UpdateUserLocation"
        )
        return data.agents.updateIdentity.ok
    }
}

// MARK: - GraphQL data models

struct GQLSession: Decodable, Identifiable, Equatable {
    let id: String
    let key: String
    let label: String?
    let model: String?
    let preview: String?
    let createdAt: Int?
    let updatedAt: Int?
    let messageCount: Int?
    let lastSeenMessageCount: Int?
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
