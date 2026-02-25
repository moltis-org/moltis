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
        editorRow("Cron definitions", text: $settings.cronJobs)
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
        editorRow("Channel rules", text: $settings.channelRules)
    }

    var hooksPane: some View {
        editorRow("Hooks config", text: $settings.hooksConfig)
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
        editorRow("MCP servers", text: $settings.mcpServers)
    }

    var skillsPane: some View {
        editorRow("Skill packs", text: $settings.skills)
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
        editorRow("moltis.toml", text: $settings.configurationToml, minHeight: 280)
    }
}

// MARK: - Helpers

private extension SettingsSectionContent {
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
}

// MARK: - Voice provider grid pane

struct VoiceProviderGridPane: View {
    @ObservedObject var providerStore: ProviderStore
    @ObservedObject var settings: AppSettings

    private let columns = [
        GridItem(.adaptive(minimum: 180, maximum: 260), spacing: 10)
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Toggle("Enable voice", isOn: $settings.voiceEnabled)

            if settings.voiceEnabled {
                LazyVGrid(columns: columns, spacing: 10) {
                    ForEach(VoiceProvider.all) { voiceProvider in
                        VoiceProviderCardView(
                            provider: voiceProvider,
                            isSelected: providerStore.selectedVoiceProviderName == voiceProvider.name,
                            onSelect: {
                                providerStore.selectedVoiceProviderName = voiceProvider.name
                                providerStore.voiceApiKeyDraft = ""
                                settings.voiceProvider = voiceProvider.name
                            }
                        )
                    }
                }

                if let selected = VoiceProvider.all.first(where: {
                    $0.name == providerStore.selectedVoiceProviderName
                }), selected.requiresApiKey {
                    VStack(alignment: .leading, spacing: 12) {
                        Text(selected.displayName)
                            .font(.title3.weight(.semibold))

                        SecureField("API Key", text: $providerStore.voiceApiKeyDraft)
                            .textFieldStyle(.roundedBorder)

                        Button("Save") {
                            settings.voiceApiKey = providerStore.voiceApiKeyDraft
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(
                            providerStore.voiceApiKeyDraft
                                .trimmingCharacters(in: .whitespacesAndNewlines)
                                .isEmpty
                        )
                    }
                    .padding()
                }
            }
        }
    }
}

// MARK: - Provider grid pane (used in LLMs section)

struct ProviderGridPane: View {
    @ObservedObject var providerStore: ProviderStore

    private let columns = [
        GridItem(.adaptive(minimum: 180, maximum: 260), spacing: 10)
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            LazyVGrid(columns: columns, spacing: 10) {
                ForEach(providerStore.knownProviders) { provider in
                    ProviderCardView(
                        provider: provider,
                        isConfigured: providerStore.isConfigured(provider.name),
                        isSelected: providerStore.selectedProviderName == provider.name,
                        onSelect: {
                            selectProvider(provider)
                        }
                    )
                }
            }

            ProviderConfigForm(providerStore: providerStore)
        }
        .onAppear {
            if providerStore.knownProviders.isEmpty {
                providerStore.loadAll()
            }
        }
    }

    private func selectProvider(_ provider: BridgeKnownProvider) {
        providerStore.selectedProviderName = provider.name
        providerStore.apiKeyDraft = ""
        providerStore.baseUrlDraft = provider.defaultBaseUrl ?? ""
        providerStore.selectedModelID = nil
    }
}
