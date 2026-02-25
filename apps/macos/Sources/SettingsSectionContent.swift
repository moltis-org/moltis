import SwiftUI

/// Returns raw form controls for a given settings section.
/// Designed to be placed inside a `Form` `Section`.
struct SettingsSectionContent: View {
    let section: SettingsSection
    @ObservedObject var settings: AppSettings
    var providerStore: ProviderStore?

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
        case .terminal: terminalPane
        case .sandboxes: sandboxesPane
        case .monitoring: monitoringPane
        case .logs: logsPane
        case .graphql: graphqlPane
        case .configuration: configurationPane
        }
    }
}

// MARK: - General

private extension SettingsSectionContent {
    var identityPane: some View {
        Group {
            TextField("Display name", text: $settings.identityName)
            editorRow("Soul Prompt", text: $settings.identitySoul)
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

    @ViewBuilder
    var llmsPane: some View {
        if let providerStore {
            ProviderGridPane(providerStore: providerStore)
        } else {
            Group {
                TextField("Provider", text: $settings.llmProvider)
                TextField("Model", text: $settings.llmModel)
                SecureField("API key", text: $settings.llmApiKey)
            }
        }
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

    @ViewBuilder
    var voicePane: some View {
        if let providerStore {
            VoiceProviderGridPane(
                providerStore: providerStore,
                settings: settings
            )
        } else {
            Group {
                Toggle("Enable voice", isOn: $settings.voiceEnabled)
                SecureField("Voice API key", text: $settings.voiceApiKey)
            }
        }
    }
}

// MARK: - Systems

private extension SettingsSectionContent {
    var terminalPane: some View {
        Group {
            Toggle("Enable terminal tool", isOn: $settings.terminalEnabled)
            TextField("Default shell", text: $settings.terminalShell)
        }
    }

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

    var monitoringPane: some View {
        Group {
            Toggle("Enable monitoring", isOn: $settings.monitoringEnabled)
            Toggle("Enable metrics", isOn: $settings.metricsEnabled)
            Toggle("Enable tracing", isOn: $settings.tracingEnabled)
        }
    }

    var logsPane: some View {
        Group {
            Picker("Log level", selection: $settings.logLevel) {
                ForEach(settings.logLevels, id: \.self) { level in
                    Text(level.uppercased()).tag(level)
                }
            }
            Toggle("Persist logs to disk", isOn: $settings.persistLogs)
        }
    }

    var graphqlPane: some View {
        Group {
            Toggle("Enable GraphQL", isOn: $settings.graphqlEnabled)
            TextField("GraphQL path", text: $settings.graphqlPath)
        }
    }

    var configurationPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            editorRow("moltis.toml", text: $settings.configurationToml, minHeight: 280)
            Button("Validate") {}
                .disabled(true)
        }
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
