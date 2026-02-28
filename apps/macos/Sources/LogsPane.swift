import AppKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Logs Pane (full-screen console, matches web UI page-logs)

struct LogsPane: View {
    @ObservedObject var logStore: LogStore

    var body: some View {
        VStack(spacing: 0) {
            logsToolbar
            Divider()
            logsList
        }
    }

    // MARK: - Toolbar

    private var logsToolbar: some View {
        HStack(spacing: 10) {
            // Level filter
            Picker("Level", selection: $logStore.filterLevel) {
                ForEach(LogLevel.allCases, id: \.self) { level in
                    Text(level.rawValue).tag(level)
                }
            }
            .frame(maxWidth: 120)

            // Target filter
            TextField("Target", text: $logStore.filterTarget)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11, design: .monospaced))
                .frame(maxWidth: 140)

            // Search
            HStack(spacing: 4) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.tertiary)
                    .font(.system(size: 10))
                TextField("Search...", text: $logStore.searchText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 11, design: .monospaced))
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(.background, in: RoundedRectangle(cornerRadius: 4))
            .overlay {
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(.quaternary)
            }
            .frame(maxWidth: 180)

            Spacer()

            // Actions
            Group {
                Button {
                    if logStore.isPaused {
                        logStore.resume()
                    } else {
                        logStore.isPaused = true
                    }
                } label: {
                    Image(systemName: logStore.isPaused ? "play.fill" : "pause.fill")
                }
                .help(logStore.isPaused ? "Resume" : "Pause")

                Button { logStore.clear() } label: {
                    Image(systemName: "trash")
                }
                .help("Clear")

                Button {
                    let text = logStore.exportPlainText()
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .help("Copy All")

                Button { downloadJSONL() } label: {
                    Image(systemName: "arrow.down.circle")
                }
                .help("Download JSONL")
            }
            .buttonStyle(.borderless)
            .controlSize(.small)

            // Entry count
            Text("\(logStore.filteredCount)/\(logStore.entryCount)")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    // MARK: - Console log area

    private var logsList: some View {
        Group {
            let filtered = logStore.filteredEntries
            if filtered.isEmpty {
                VStack(spacing: 6) {
                    Text("No log entries")
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(.tertiary)
                    Text(logStore.entries.isEmpty
                         ? "Logs appear here as you use the app"
                         : "Adjust filters to see entries")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.quaternary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color(nsColor: .textBackgroundColor))
            } else {
                ScrollViewReader { proxy in
                    List(filtered) { entry in
                        LogEntryRow(entry: entry)
                            .listRowInsets(EdgeInsets(
                                top: 0, leading: 6, bottom: 0, trailing: 6
                            ))
                            .listRowSeparator(.hidden)
                            .listRowBackground(rowBackground(entry.level))
                    }
                    .listStyle(.plain)
                    .font(.system(size: 11, design: .monospaced))
                    .onChange(of: logStore.entries.last?.id) { _, newID in
                        guard !logStore.isPaused, let newID else { return }
                        withAnimation(.easeOut(duration: 0.1)) {
                            proxy.scrollTo(newID, anchor: .bottom)
                        }
                    }
                }
            }
        }
    }

    private func rowBackground(_ level: LogLevel) -> Color {
        switch level {
        case .error: return MoltisTheme.error.opacity(0.06)
        case .warn: return Color.orange.opacity(0.04)
        default: return .clear
        }
    }

    // MARK: - Download

    private func downloadJSONL() {
        let panel = NSSavePanel()
        let jsonlType = UTType(filenameExtension: "jsonl") ?? .json
        panel.allowedContentTypes = [jsonlType]
        panel.nameFieldStringValue = "moltis-logs.jsonl"
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            let content = logStore.exportJSONL()
            try? content.write(to: url, atomically: true, encoding: .utf8)
        }
    }
}

// MARK: - Single log entry row (compact monospace console line)

private struct LogEntryRow: View {
    let entry: LogEntry

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss.SSS"
        return formatter
    }()

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            // Timestamp
            Text(Self.timeFormatter.string(from: entry.timestamp))
                .foregroundStyle(.secondary)
                .frame(width: 82, alignment: .leading)

            // Level badge
            Text(entry.level.rawValue)
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundStyle(.white)
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(levelColor(entry.level), in: RoundedRectangle(cornerRadius: 2))
                .frame(width: 48, alignment: .center)

            // Target
            Text(entry.target)
                .foregroundStyle(.tertiary)
                .frame(width: 110, alignment: .leading)
                .lineLimit(1)
                .padding(.leading, 6)

            // Message + inline fields
            Text(formattedMessage)
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, 6)

            Spacer(minLength: 0)
        }
        .padding(.vertical, 1)
    }

    private var formattedMessage: AttributedString {
        var result = AttributedString(entry.message)

        if !entry.fields.isEmpty {
            let fieldParts = entry.fields
                .sorted { $0.key < $1.key }
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: " ")

            var fieldsAttr = AttributedString("  \(fieldParts)")
            fieldsAttr.foregroundColor = .secondaryLabelColor
            result.append(fieldsAttr)
        }

        return result
    }

    private func levelColor(_ level: LogLevel) -> Color {
        switch level {
        case .trace: return .gray
        case .debug: return .blue
        case .info: return .green
        case .warn: return .orange
        case .error: return .red
        }
    }
}
