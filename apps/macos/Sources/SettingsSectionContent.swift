import SwiftUI

/// Returns raw form controls for a given settings section.
/// Designed to be placed inside a `Form` `Section`.
struct SettingsSectionContent: View {
    let section: SettingsSection
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    var logStore: LogStore?

    var body: some View {
        switch section {
        case .identity: identityPane
        case .environment: environmentPane
        case .memory: memoryPane
        case .notifications: notificationsPane
        case .crons: cronsPane
        case .heartbeat: heartbeatPane
        case .security: securityPane
        case .tailscale: tailscalePane
        case .channels: channelsPane
        case .hooks: hooksPane
        case .llms: llmsPane
        case .mcp: mcpPane
        case .skills: skillsPane
        case .voice: voicePane
        case .sandboxes: sandboxesPane
        case .networkAudit: networkAuditPane
        case .monitoring: monitoringPane
        case .logs: logsPane
        case .graphql: graphqlPane
        case .httpd: httpdPane
        case .configuration: configurationPane
        }
    }
}

// MARK: - General

private extension SettingsSectionContent {
    var identityPane: some View {
        Group {
            Section("Agent") {
                TextField("Name", text: $settings.identityName, prompt: Text("e.g. Rex"))
                TextField("Emoji", text: $settings.identityEmoji)
                TextField("Theme", text: $settings.identityTheme, prompt: Text("e.g. wise owl, chill fox"))
            }
            Section("User") {
                TextField("Your name", text: $settings.identityUserName, prompt: Text("e.g. Alice"))
            }
            editorRow("Soul", text: $settings.identitySoul)
        }
    }

    var environmentPane: some View {
        Group {
            TextField("Config directory", text: $settings.environmentConfigDir)
            TextField("Data directory", text: $settings.environmentDataDir)
        }
    }

    var memoryPane: some View {
        Group {
            Toggle("Enable memory", isOn: $settings.memoryEnabled)
            Picker("Memory mode", selection: $settings.memoryMode) {
                ForEach(settings.memoryModes, id: \.self) { mode in
                    Text(mode.capitalized).tag(mode)
                }
            }
        }
    }

    var notificationsPane: some View {
        Group {
            Toggle("Enable notifications", isOn: $settings.notificationsEnabled)
            Toggle("Play sounds", isOn: $settings.notificationsSoundEnabled)
        }
    }

    var cronsPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.cronJobs.isEmpty {
                SettingsEmptyState(
                    icon: "clock.arrow.circlepath",
                    title: "No Cron Jobs",
                    subtitle: "Add scheduled tasks to run automatically"
                )
            } else {
                ForEach($settings.cronJobs) { $item in
                    DisclosureGroup {
                        cronJobFields(item: $item)
                    } label: {
                        cronJobLabel(item: $item)
                    }
                }
            }
            Button {
                settings.cronJobs.append(CronJobItem())
            } label: {
                Label("Add Cron Job", systemImage: "plus")
            }
        }
    }

    var heartbeatPane: some View {
        Group {
            Toggle("Enable heartbeat", isOn: $settings.heartbeatEnabled)
            Stepper(
                "Interval: \(settings.heartbeatIntervalMinutes) min",
                value: $settings.heartbeatIntervalMinutes,
                in: 1 ... 120
            )
        }
    }
}

// MARK: - Security

private extension SettingsSectionContent {
    var securityPane: some View {
        Group {
            Toggle("Require password login", isOn: $settings.requirePassword)
            Toggle("Enable passkeys", isOn: $settings.passkeysEnabled)
        }
    }

    var tailscalePane: some View {
        Group {
            Toggle("Enable Tailscale", isOn: $settings.tailscaleEnabled)
            TextField("Hostname", text: $settings.tailscaleHostname)
        }
    }
}

// MARK: - Integrations

private extension SettingsSectionContent {
    var channelsPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.channels.isEmpty {
                SettingsEmptyState(
                    icon: "point.3.connected.trianglepath.dotted",
                    title: "No Channels",
                    subtitle: "Connect messaging platforms like Telegram or Slack"
                )
            } else {
                ForEach($settings.channels) { $item in
                    DisclosureGroup {
                        channelFields(item: $item)
                    } label: {
                        channelLabel(item: $item)
                    }
                }
            }
            Button {
                settings.channels.append(ChannelItem())
            } label: {
                Label("Add Channel", systemImage: "plus")
            }
        }
    }

    var hooksPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.hooks.isEmpty {
                SettingsEmptyState(
                    icon: "wrench.and.screwdriver",
                    title: "No Hooks",
                    subtitle: "Run commands in response to events"
                )
            } else {
                ForEach($settings.hooks) { $item in
                    DisclosureGroup {
                        hookFields(item: $item)
                    } label: {
                        hookLabel(item: $item)
                    }
                }
            }
            Button {
                settings.hooks.append(HookItem())
            } label: {
                Label("Add Hook", systemImage: "plus")
            }
        }
    }

    var llmsPane: some View {
        ProviderGridPane(providerStore: providerStore)
    }

    var mcpPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.mcpServers.isEmpty {
                SettingsEmptyState(
                    icon: "link",
                    title: "No MCP Servers",
                    subtitle: "Connect external tools via Model Context Protocol"
                )
            } else {
                ForEach($settings.mcpServers) { $item in
                    DisclosureGroup {
                        mcpFields(item: $item)
                    } label: {
                        mcpLabel(item: $item)
                    }
                }
            }
            Button {
                settings.mcpServers.append(McpServerItem())
            } label: {
                Label("Add MCP Server", systemImage: "plus")
            }
        }
    }

    var skillsPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.skillPacks.isEmpty {
                SettingsEmptyState(
                    icon: "sparkles",
                    title: "No Skill Packs",
                    subtitle: "Install skill packs to extend capabilities"
                )
            } else {
                ForEach($settings.skillPacks) { $item in
                    DisclosureGroup {
                        skillFields(item: $item)
                    } label: {
                        skillLabel(item: $item)
                    }
                }
            }
            Button {
                settings.skillPacks.append(SkillPackItem())
            } label: {
                Label("Add Skill Pack", systemImage: "plus")
            }
        }
    }

    var voicePane: some View {
        VoiceProviderGridPane(
            providerStore: providerStore,
            settings: settings
        )
    }
}

// MARK: - Systems

private extension SettingsSectionContent {
    var sandboxesPane: some View {
        Group {
            Picker("Backend", selection: $settings.sandboxBackend) {
                ForEach(settings.sandboxBackends, id: \.self) { backend in
                    Text(backend.capitalized).tag(backend)
                }
            }
            TextField("Default image", text: $settings.sandboxImage)
        }
    }

    var networkAuditPane: some View {
        SettingsEmptyState(
            icon: "network.badge.shield.half.filled",
            title: "Network Audit",
            subtitle: "Select this section to view the full network audit log"
        )
    }

    var monitoringPane: some View {
        Group {
            Toggle("Enable monitoring", isOn: $settings.monitoringEnabled)
            Toggle("Enable metrics", isOn: $settings.metricsEnabled)
            Toggle("Enable tracing", isOn: $settings.tracingEnabled)
        }
    }

    @ViewBuilder
    var logsPane: some View {
        if let logStore {
            LogsPane(logStore: logStore)
        } else {
            SettingsEmptyState(
                icon: "doc.plaintext",
                title: "Logs Unavailable",
                subtitle: "Log store not connected"
            )
        }
    }

    var graphqlPane: some View {
        Group {
            Toggle("Enable GraphQL", isOn: $settings.graphqlEnabled)
            TextField("GraphQL path", text: $settings.graphqlPath)
        }
    }

    var httpdPane: some View {
        HttpdPane(settings: settings)
    }

    var configurationPane: some View {
        ConfigurationPane(settings: settings)
    }
}

// MARK: - Helpers

extension SettingsSectionContent {
    /// Full-width editor row with label above.
    func editorRow(
        _ title: String,
        text: Binding<String>,
        minHeight: CGFloat = 160
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .foregroundStyle(.secondary)
            MoltisEditorField(text: text, minHeight: minHeight)
        }
    }

    func deleteButton(action: @escaping () -> Void) -> some View {
        Button(role: .destructive, action: action) {
            Image(systemName: "trash")
                .foregroundStyle(.red)
        }
        .buttonStyle(.borderless)
    }
}
