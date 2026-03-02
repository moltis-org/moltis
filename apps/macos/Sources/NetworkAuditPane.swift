import AppKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Network Audit Pane (full-screen console, matches LogsPane pattern)

struct NetworkAuditPane: View {
    @ObservedObject var store: NetworkAuditStore

    var body: some View {
        VStack(spacing: 0) {
            auditToolbar
            Divider()
            auditList
        }
    }

    // MARK: - Toolbar

    private var auditToolbar: some View {
        HStack(spacing: 10) {
            // Action filter
            Picker("Action", selection: $store.filterAction) {
                ForEach(store.filterActions, id: \.self) { action in
                    Text(action.capitalized).tag(action)
                }
            }
            .frame(maxWidth: 120)

            // Search
            HStack(spacing: 4) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.tertiary)
                    .font(.system(size: 10))
                TextField("Search domain...", text: $store.searchText)
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
            .frame(maxWidth: 200)

            Spacer()

            // Stats
            HStack(spacing: 8) {
                Label("\(store.allowedCount)", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .font(.system(size: 10, design: .monospaced))
                Label("\(store.deniedCount)", systemImage: "xmark.circle.fill")
                    .foregroundStyle(.red)
                    .font(.system(size: 10, design: .monospaced))
            }

            // Actions
            Group {
                Button {
                    if store.isPaused {
                        store.resume()
                    } else {
                        store.isPaused = true
                    }
                } label: {
                    Image(systemName: store.isPaused ? "play.fill" : "pause.fill")
                }
                .help(store.isPaused ? "Resume" : "Pause")

                Button { store.clear() } label: {
                    Image(systemName: "trash")
                }
                .help("Clear")

                Button {
                    let text = store.exportPlainText()
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
            Text("\(store.filteredCount)/\(store.entryCount)")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    // MARK: - Audit log area

    private var auditList: some View {
        Group {
            let filtered = store.filteredEntries
            if filtered.isEmpty {
                VStack(spacing: 6) {
                    Text("No network audit entries")
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(.tertiary)
                    Text(store.entries.isEmpty
                         ? "Network requests appear here when the proxy is active"
                         : "Adjust filters to see entries")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.quaternary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color(nsColor: .textBackgroundColor))
            } else {
                ScrollViewReader { proxy in
                    List(filtered) { entry in
                        NetworkAuditEntryRow(entry: entry)
                            .listRowInsets(EdgeInsets(
                                top: 0, leading: 6, bottom: 0, trailing: 6
                            ))
                            .listRowSeparator(.hidden)
                            .listRowBackground(rowBackground(entry))
                    }
                    .listStyle(.plain)
                    .font(.system(size: 11, design: .monospaced))
                    .onChange(of: store.entries.last?.id) { _, newID in
                        guard !store.isPaused, let newID else { return }
                        withAnimation(.easeOut(duration: 0.1)) {
                            proxy.scrollTo(newID, anchor: .bottom)
                        }
                    }
                }
            }
        }
    }

    private func rowBackground(_ entry: NetworkAuditEntry) -> Color {
        if entry.isDenied {
            return MoltisTheme.error.opacity(0.06)
        }
        return .clear
    }

    // MARK: - Download

    private func downloadJSONL() {
        let panel = NSSavePanel()
        let jsonlType = UTType(filenameExtension: "jsonl") ?? .json
        panel.allowedContentTypes = [jsonlType]
        panel.nameFieldStringValue = "network-audit.jsonl"
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            let content = store.exportJSONL()
            try? content.write(to: url, atomically: true, encoding: .utf8)
        }
    }
}

// MARK: - Single audit entry row

private struct NetworkAuditEntryRow: View {
    let entry: NetworkAuditEntry

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

            // Action badge
            Text(entry.isAllowed ? "ALLOW" : "DENY")
                .font(.system(size: 9, weight: .bold, design: .monospaced))
                .foregroundStyle(.white)
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(
                    entry.isAllowed ? Color.green : Color.red,
                    in: RoundedRectangle(cornerRadius: 2)
                )
                .frame(width: 52, alignment: .center)

            // Protocol
            Text(entry.networkProtocol.uppercased())
                .foregroundStyle(.tertiary)
                .frame(width: 60, alignment: .leading)
                .lineLimit(1)
                .padding(.leading, 6)

            // Domain:port
            Text("\(entry.domain):\(entry.port)")
                .textSelection(.enabled)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.leading, 6)
                .frame(minWidth: 160, alignment: .leading)

            // Method + URL (if present)
            if let method = entry.method {
                Text(method)
                    .foregroundStyle(.blue)
                    .frame(width: 44, alignment: .leading)
                    .padding(.leading, 6)
            }

            if let url = entry.url {
                Text(url)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.leading, 2)
            }

            Spacer(minLength: 0)

            // Source
            Text(entry.source)
                .font(.system(size: 9, design: .monospaced))
                .foregroundStyle(.quaternary)
                .frame(width: 70, alignment: .trailing)
        }
        .padding(.vertical, 1)
    }
}
