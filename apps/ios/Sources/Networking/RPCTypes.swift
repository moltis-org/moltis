import Foundation

// MARK: - Protocol constants

enum MoltisProtocol {
    static let version = 3
    static let maxPayloadBytes = 524_288
    static let tickIntervalMs = 30_000
    static let handshakeTimeoutMs = 10_000
}

// MARK: - Frame types

enum RPCFrameType: String {
    case res
    case event
}

struct RPCResponse: Decodable {
    let type: RPCFrameType?
    let id: String?
    let ok: Bool?
    let payload: AnyCodable?
    let error: RPCError?
    // Event fields
    let event: String?
    let seq: UInt64?

    private enum CodingKeys: String, CodingKey {
        case type, id, ok, payload, error, event, seq
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        // Decode type as String then map to RPCFrameType — unknown types become nil
        // instead of throwing a decode error.
        let typeString = try container.decodeIfPresent(String.self, forKey: .type)
        self.type = typeString.flatMap { RPCFrameType(rawValue: $0) }
        self.id = try container.decodeIfPresent(String.self, forKey: .id)
        self.ok = try container.decodeIfPresent(Bool.self, forKey: .ok)
        self.payload = try container.decodeIfPresent(AnyCodable.self, forKey: .payload)
        self.error = try container.decodeIfPresent(RPCError.self, forKey: .error)
        self.event = try container.decodeIfPresent(String.self, forKey: .event)
        self.seq = try container.decodeIfPresent(UInt64.self, forKey: .seq)
    }
}

struct RPCError: Decodable {
    let code: String
    let message: String
    let retryable: Bool?
    let retryAfterMs: Int?
}

// MARK: - Hello-OK payload

struct HelloOkPayload: Decodable {
    let type: String?
    let `protocol`: Int?
    let server: ServerInfo?
    let features: Features?
    let auth: AuthInfo?
    let policy: PolicyInfo?

    struct ServerInfo: Decodable {
        let version: String?
        let commit: String?
        let host: String?
        let connId: String?
    }

    struct Features: Decodable {
        let methods: [String]?
        let events: [String]?
    }

    struct AuthInfo: Decodable {
        let role: String?
        let scopes: [String]?
    }

    struct PolicyInfo: Decodable {
        let maxPayload: Int?
        let tickIntervalMs: Int?
    }
}

// MARK: - Connect params

struct ConnectParams: Encodable {
    let minProtocol: Int
    let maxProtocol: Int
    let client: ClientInfo
    let auth: AuthParam
    let locale: String
    let role: String

    struct ClientInfo: Encodable {
        let id: String
        let displayName: String
        let version: String
        let platform: String
        let mode: String
        let instanceId: String
    }

    struct AuthParam: Encodable {
        // swiftlint:disable:next identifier_name
        let api_key: String
    }
}

// MARK: - AnyCodable helper

struct AnyCodable: Codable, Equatable {
    let value: Any

    init(_ value: Any) {
        self.value = value
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            value = NSNull()
        } else if let bool = try? container.decode(Bool.self) {
            value = bool
        } else if let int = try? container.decode(Int.self) {
            value = int
        } else if let double = try? container.decode(Double.self) {
            value = double
        } else if let string = try? container.decode(String.self) {
            value = string
        } else if let array = try? container.decode([AnyCodable].self) {
            value = array.map(\.value)
        } else if let dict = try? container.decode([String: AnyCodable].self) {
            value = dict.mapValues(\.value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unsupported type"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch value {
        case is NSNull:
            try container.encodeNil()
        case let bool as Bool:
            try container.encode(bool)
        case let int as Int:
            try container.encode(int)
        case let double as Double:
            try container.encode(double)
        case let string as String:
            try container.encode(string)
        case let array as [Any]:
            try container.encode(array.map { AnyCodable($0) })
        case let dict as [String: Any]:
            try container.encode(dict.mapValues { AnyCodable($0) })
        default:
            throw EncodingError.invalidValue(
                value,
                EncodingError.Context(
                    codingPath: container.codingPath,
                    debugDescription: "Unsupported type: \(type(of: value))"
                )
            )
        }
    }

    static func == (lhs: AnyCodable, rhs: AnyCodable) -> Bool {
        switch (lhs.value, rhs.value) {
        case (is NSNull, is NSNull):
            return true
        case let (lhs as Bool, rhs as Bool):
            return lhs == rhs
        case let (lhs as Int, rhs as Int):
            return lhs == rhs
        case let (lhs as Double, rhs as Double):
            return lhs == rhs
        case let (lhs as String, rhs as String):
            return lhs == rhs
        default:
            return false
        }
    }
}
