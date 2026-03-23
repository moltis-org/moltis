import Combine
import Foundation

// MARK: - Network Audit Entry

struct NetworkAuditEntry: Identifiable {
    let id: UUID
    let timestamp: Date
    let domain: String
    let port: UInt16
    let networkProtocol: String
    let action: String
    let source: String
    let method: String?
    let url: String?

    var isAllowed: Bool { action == "allowed" || action == "approved_by_user" }
    var isDenied: Bool { action == "denied" || action == "timeout" }
}

// MARK: - Network Audit Store

final class NetworkAuditStore: ObservableObject {
    @Published private(set) var entries: [NetworkAuditEntry] = []
    @Published var filterAction = "all"
    @Published var searchText = ""
    @Published var isPaused = false

    /// Maximum entries kept in memory.
    private let maxEntries = 5000
    /// Buffer for entries received while paused.
    private var pauseBuffer: [NetworkAuditEntry] = []

    let filterActions = ["all", "allowed", "denied"]

    var filteredEntries: [NetworkAuditEntry] {
        let actionQuery = filterAction
        let searchQuery = searchText
            .trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        return entries.filter { entry in
            if actionQuery == "allowed", !entry.isAllowed { return false }
            if actionQuery == "denied", !entry.isDenied { return false }
            if !searchQuery.isEmpty,
               !entry.domain.lowercased().contains(searchQuery),
               !(entry.method?.lowercased().contains(searchQuery) ?? false),
               !(entry.url?.lowercased().contains(searchQuery) ?? false) {
                return false
            }
            return true
        }
    }

    var entryCount: Int { entries.count }
    var filteredCount: Int { filteredEntries.count }
    var allowedCount: Int { entries.filter(\.isAllowed).count }
    var deniedCount: Int { entries.filter(\.isDenied).count }

    // MARK: - Push

    func push(_ entry: NetworkAuditEntry) {
        if isPaused {
            pauseBuffer.append(entry)
            return
        }
        appendEntry(entry)
    }

    func resume() {
        isPaused = false
        for entry in pauseBuffer {
            appendEntry(entry)
        }
        pauseBuffer.removeAll()
    }

    func clear() {
        entries.removeAll()
        pauseBuffer.removeAll()
    }

    // MARK: - Export

    func exportJSONL() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        return filteredEntries.map { entry in
            var dict: [String: Any] = [
                "timestamp": formatter.string(from: entry.timestamp),
                "domain": entry.domain,
                "port": entry.port,
                "protocol": entry.networkProtocol,
                "action": entry.action,
                "source": entry.source
            ]
            if let method = entry.method { dict["method"] = method }
            if let url = entry.url { dict["url"] = url }
            guard let data = try? JSONSerialization.data(
                withJSONObject: dict, options: [.sortedKeys]
            ) else { return "{}" }
            return String(data: data, encoding: .utf8) ?? "{}"
        }.joined(separator: "\n")
    }

    func exportPlainText() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss.SSS"

        return filteredEntries.map { entry in
            let ts = formatter.string(from: entry.timestamp)
            let badge = entry.isAllowed ? "ALLOW" : "DENY"
            var line = "\(ts) [\(badge)] \(entry.domain):\(entry.port) (\(entry.networkProtocol))"
            if let method = entry.method, let url = entry.url {
                line += " \(method) \(url)"
            }
            line += " via=\(entry.source)"
            return line
        }.joined(separator: "\n")
    }

    // MARK: - Private

    private func appendEntry(_ entry: NetworkAuditEntry) {
        entries.append(entry)
        if entries.count > maxEntries {
            entries.removeFirst(entries.count - maxEntries)
        }
    }
}
