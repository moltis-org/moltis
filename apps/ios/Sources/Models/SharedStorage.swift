import Foundation

enum SharedStorage {
    static let appGroupIdentifier = "group.org.moltis.ios"

    static var containerURL: URL? {
        FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        )
    }

    /// Write data to the shared App Group container.
    static func write(_ data: Data, filename: String) throws {
        guard let containerURL else {
            throw StorageError.noContainer
        }
        let fileURL = containerURL.appendingPathComponent(filename)
        try data.write(to: fileURL, options: .atomic)
    }

    /// Read data from the shared App Group container.
    static func read(filename: String) throws -> Data {
        guard let containerURL else {
            throw StorageError.noContainer
        }
        let fileURL = containerURL.appendingPathComponent(filename)
        return try Data(contentsOf: fileURL)
    }

    enum StorageError: LocalizedError {
        case noContainer

        var errorDescription: String? {
            switch self {
            case .noContainer:
                return "App Group container not available"
            }
        }
    }
}
