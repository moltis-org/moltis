import SwiftUI

// MARK: - Enums

enum SettingsGroup: String, CaseIterable, Hashable {
    case general = "General"
    case security = "Security"
    case integrations = "Integrations"
    case systems = "Systems"

    var sections: [SettingsSection] {
        SettingsSection.allCases.filter { $0.group == self }
    }
}

enum SettingsSection: String, CaseIterable, Hashable {
    case identity = "Identity"
    case environment = "Environment"
    case memory = "Memory"
    case notifications = "Notifications"
    case crons = "Crons"
    case heartbeat = "Heartbeat"
    case security = "Security"
    case tailscale = "Tailscale"
    case channels = "Channels"
    case hooks = "Hooks"
    case llms = "LLMs"
    case mcp = "MCP"
    case skills = "Skills"
    case voice = "Voice"
    case sandboxes = "Sandboxes"
    case networkAudit = "Network Audit"
    case monitoring = "Monitoring"
    case logs = "Logs"
    case graphql = "GraphQL"
    case httpd = "HTTP Server"
    case configuration = "Configuration"

    var title: String { rawValue }

    var icon: String {
        Self.iconMap[self] ?? "gearshape"
    }

    var iconColor: Color {
        Self.colorMap[self] ?? .gray
    }

    var group: SettingsGroup {
        Self.groupMap[self] ?? .general
    }

    // swiftlint:disable colon
    private static let iconMap: [SettingsSection: String] = [
        .identity:      "person.crop.circle.fill",
        .environment:   "terminal.fill",
        .memory:        "externaldrive.fill",
        .notifications: "bell.fill",
        .crons:         "clock.arrow.circlepath",
        .heartbeat:     "heart.text.square.fill",
        .security:      "lock.shield.fill",
        .tailscale:     "network",
        .channels:      "bubble.left.and.bubble.right.fill",
        .hooks:         "wrench.and.screwdriver.fill",
        .llms:          "cpu.fill",
        .mcp:           "link",
        .skills:        "sparkles",
        .voice:         "mic.fill",
        .sandboxes:     "shippingbox.fill",
        .networkAudit:  "network.badge.shield.half.filled",
        .monitoring:    "chart.bar.fill",
        .logs:          "doc.plaintext.fill",
        .graphql:       "point.3.connected.trianglepath.dotted",
        .httpd:         "server.rack",
        .configuration: "slider.horizontal.3"
    ]

    private static let colorMap: [SettingsSection: Color] = [
        .identity:      .blue,
        .environment:   .gray,
        .memory:        .purple,
        .notifications: .red,
        .crons:         .orange,
        .heartbeat:     .pink,
        .security:      .green,
        .tailscale:     .cyan,
        .channels:      .indigo,
        .hooks:         .brown,
        .llms:          .teal,
        .mcp:           .blue,
        .skills:        .yellow,
        .voice:         .mint,
        .sandboxes:     .orange,
        .networkAudit:  .cyan,
        .monitoring:    .green,
        .logs:          .secondary,
        .graphql:       .pink,
        .httpd:         .blue,
        .configuration: .purple
    ]

    private static let groupMap: [SettingsSection: SettingsGroup] = [
        .identity:      .general,
        .environment:   .general,
        .memory:        .general,
        .notifications: .general,
        .crons:         .general,
        .heartbeat:     .general,
        .security:      .security,
        .tailscale:     .security,
        .channels:      .integrations,
        .hooks:         .integrations,
        .llms:          .integrations,
        .mcp:           .integrations,
        .skills:        .integrations,
        .voice:         .integrations,
        .sandboxes:     .systems,
        .networkAudit:  .systems,
        .monitoring:    .systems,
        .logs:          .systems,
        .graphql:       .systems,
        .httpd:         .systems,
        .configuration: .systems
    ]
    // swiftlint:enable colon
}

// MARK: - Settings View (sidebar + detail like macOS System Settings)

struct SettingsView: View {
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    @ObservedObject var logStore: LogStore
    @ObservedObject var networkAuditStore: NetworkAuditStore
    @State private var selectedSection: SettingsSection? = .identity
    @State private var searchText = ""
    @FocusState private var focusedField: String?

    private var filteredGroups: [(group: SettingsGroup, sections: [SettingsSection])] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return SettingsGroup.allCases.compactMap { group in
            let sections = group.sections.filter { section in
                query.isEmpty || section.title.lowercased().contains(query)
            }
            return sections.isEmpty ? nil : (group, sections)
        }
    }

    var body: some View {
        NavigationSplitView {
            settingsSidebar
        } detail: {
            settingsDetail
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 720, minHeight: 520)
    }

    // MARK: Sidebar

    private var settingsSidebar: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.tertiary)
                    .font(.system(size: 12))
                TextField("Search", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
            }
            .padding(7)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
            .padding(.horizontal, 12)

            List(selection: $selectedSection) {
                ForEach(filteredGroups, id: \.group) { item in
                    Section(item.group.rawValue) {
                        ForEach(item.sections, id: \.self) { section in
                            Label {
                                Text(section.title)
                            } icon: {
                                SettingsIconView(
                                    systemName: section.icon,
                                    color: section.iconColor
                                )
                            }
                            .tag(section)
                        }
                    }
                }
            }
            .listStyle(.sidebar)
        }
        .padding(.top, 8)
    }

    // MARK: Detail

    private var settingsDetail: some View {
        Group {
            if let section = selectedSection {
                if section == .networkAudit {
                    NetworkAuditPane(store: networkAuditStore)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if section == .logs {
                    LogsPane(logStore: logStore)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if section == .configuration {
                    ConfigurationPane(settings: settings)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    Form {
                        SettingsSectionContent(
                            section: section,
                            settings: settings,
                            providerStore: providerStore,
                            logStore: logStore
                        )
                    }
                    .formStyle(.grouped)
                    .focused($focusedField, equals: "none")
                }
            } else {
                VStack(spacing: 8) {
                    Image(systemName: "gearshape")
                        .font(.system(size: 48))
                        .foregroundStyle(.tertiary)
                    Text("Select a setting")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}

// MARK: - Colorful icon badge (macOS System Settings style)

struct SettingsIconView: View {
    let systemName: String
    let color: Color

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 9, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 20, height: 20)
            .background(color, in: RoundedRectangle(cornerRadius: 5))
    }
}
