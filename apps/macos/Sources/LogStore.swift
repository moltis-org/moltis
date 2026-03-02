import Combine
import Foundation

// MARK: - Log Level

enum LogLevel: String, CaseIterable, Comparable {
    case trace = "TRACE"
    case debug = "DEBUG"
    case info = "INFO"
    case warn = "WARN"
    case error = "ERROR"

    private var severity: Int {
        switch self {
        case .trace: return 0
        case .debug: return 1
        case .info: return 2
        case .warn: return 3
        case .error: return 4
        }
    }

    static func < (lhs: LogLevel, rhs: LogLevel) -> Bool {
        lhs.severity < rhs.severity
    }
}

// MARK: - Log Entry

struct LogEntry: Identifiable {
    let id: UUID
    let timestamp: Date
    let level: LogLevel
    let target: String
    let message: String
    let fields: [String: String]

    init(
        id: UUID = UUID(),
        timestamp: Date = Date(),
        level: LogLevel,
        target: String,
        message: String,
        fields: [String: String] = [:]
    ) {
        self.id = id
        self.timestamp = timestamp
        self.level = level
        self.target = target
        self.message = message
        self.fields = fields
    }
}

// MARK: - Log Store

final class LogStore: ObservableObject {
    @Published private(set) var entries: [LogEntry] = []
    @Published var filterLevel: LogLevel = .debug
    @Published var filterTarget = ""
    @Published var searchText = ""
    @Published var isPaused = false

    /// Maximum entries kept in memory.
    private let maxEntries = 5000
    /// Buffer for entries received while paused.
    private var pauseBuffer: [LogEntry] = []

    var filteredEntries: [LogEntry] {
        let minLevel = filterLevel
        let targetQuery = filterTarget.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let searchQuery = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        return entries.filter { entry in
            guard entry.level >= minLevel else { return false }

            if !targetQuery.isEmpty,
               !entry.target.lowercased().contains(targetQuery) {
                return false
            }

            if !searchQuery.isEmpty,
               !entry.message.lowercased().contains(searchQuery),
               !entry.fields.values.contains(where: { $0.lowercased().contains(searchQuery) }) {
                return false
            }

            return true
        }
    }

    var entryCount: Int { entries.count }
    var filteredCount: Int { filteredEntries.count }

    /// All unique targets seen across entries.
    var knownTargets: [String] {
        Array(Set(entries.map(\.target))).sorted()
    }

    // MARK: - Logging

    func log(
        _ level: LogLevel,
        target: String,
        message: String,
        fields: [String: String] = [:]
    ) {
        let entry = LogEntry(level: level, target: target, message: message, fields: fields)

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

    /// Exports all filtered entries as JSONL (one JSON object per line).
    func exportJSONL() -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        return filteredEntries.map { entry in
            var dict: [String: Any] = [
                "timestamp": formatter.string(from: entry.timestamp),
                "level": entry.level.rawValue,
                "target": entry.target,
                "message": entry.message
            ]
            if !entry.fields.isEmpty {
                dict["fields"] = entry.fields
            }
            guard let data = try? JSONSerialization.data(
                withJSONObject: dict, options: [.sortedKeys]
            ) else { return "{}" }
            return String(data: data, encoding: .utf8) ?? "{}"
        }.joined(separator: "\n")
    }

    /// Plain text of all filtered entries for clipboard.
    func exportPlainText() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss.SSS"

        return filteredEntries.map { entry in
            let ts = formatter.string(from: entry.timestamp)
            var line = "\(ts) [\(entry.level.rawValue)] \(entry.target): \(entry.message)"
            if !entry.fields.isEmpty {
                let fieldStr = entry.fields.map { "\($0.key)=\($0.value)" }.joined(separator: " ")
                line += " {\(fieldStr)}"
            }
            return line
        }.joined(separator: "\n")
    }

    // MARK: - Private

    private func appendEntry(_ entry: LogEntry) {
        entries.append(entry)
        if entries.count > maxEntries {
            entries.removeFirst(entries.count - maxEntries)
        }
    }
}
