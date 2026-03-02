// swiftlint:disable file_length
import AppKit
import SwiftUI

/// Returns raw form controls for a given settings section.
/// Designed to be placed inside a `Form` `Section`.
struct SettingsSectionContent: View {
    let section: SettingsSection
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    var logStore: LogStore?
    @State private var securityCurrentPassword = ""
    @State private var securityNewPassword = ""
    @State private var securityConfirmPassword = ""
    @State private var securityResetConfirm = false
    @State private var editingPasskeyId: Int64?
    @State private var editingPasskeyName = ""

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
                    .onSubmit { settings.saveIdentity() }
                TextField("Emoji", text: $settings.identityEmoji)
                    .onSubmit { settings.saveIdentity() }
                TextField("Theme", text: $settings.identityTheme, prompt: Text("e.g. wise owl, chill fox"))
                    .onSubmit { settings.saveIdentity() }
            }
            Section("User") {
                TextField("Your name", text: $settings.identityUserName, prompt: Text("e.g. Alice"))
                    .onSubmit { settings.saveUserProfile() }
            }
            editorRow("Soul", text: $settings.identitySoul)
        }
    }

    var memoryPane: some View {
        Group {
            Section {
                if settings.memoryLoading {
                    ProgressView("Loading memory status…")
                } else {
                    LabeledContent("Files") {
                        Text("\(settings.memoryTotalFiles)")
                    }
                    LabeledContent("Chunks") {
                        Text("\(settings.memoryTotalChunks)")
                    }
                    LabeledContent("Embedding model") {
                        Text(settings.memoryEmbeddingModel)
                            .font(.system(.caption, design: .monospaced))
                    }
                    LabeledContent("Embeddings") {
                        Text(settings.memoryHasEmbeddings ? "Enabled" : "Disabled")
                    }
                    LabeledContent("Database size") {
                        Text(settings.memoryDbSizeDisplay)
                    }
                    if let error = settings.memoryError {
                        Text(error)
                            .font(.caption)
                            .foregroundStyle(.red)
                    }
                    Button("Refresh status") {
                        settings.loadMemorySettings()
                    }
                }
            } header: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Overview")
                        .textCase(nil)
                    Text(
                        "Configure long-term memory retrieval, citations, reranking, and session export."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textCase(nil)
                }
            }

            Section("Configuration") {
                Toggle("Enable memory retrieval (RAG)", isOn: $settings.memoryEnabled)
                Picker("Backend", selection: $settings.memoryBackend) {
                    ForEach(settings.memoryBackends, id: \.self) { backend in
                        if backend == "builtin" {
                            Text("Built-in (Recommended)").tag(backend)
                        } else {
                            Text("QMD").tag(backend)
                        }
                    }
                }
                Picker("Citations", selection: $settings.memoryCitations) {
                    ForEach(settings.memoryCitationModes, id: \.self) { mode in
                        switch mode {
                        case "auto":
                            Text("Auto (multi-file only)").tag(mode)
                        case "on":
                            Text("Always").tag(mode)
                        default:
                            Text("Never").tag(mode)
                        }
                    }
                }
                Toggle("Enable LLM reranking", isOn: $settings.memoryLlmReranking)
                Toggle("Export session transcripts to memory", isOn: $settings.memorySessionExport)

                HStack(spacing: 8) {
                    Button(settings.memorySaving ? "Saving…" : "Save") {
                        settings.saveMemory()
                    }
                    .disabled(settings.memorySaving)
                    if settings.memorySaved {
                        Text("Saved")
                            .font(.caption)
                            .foregroundStyle(.green)
                    }
                }
            }

            Section {
                if settings.memoryQmdAvailable {
                    Text(
                        settings.memoryQmdVersion.isEmpty
                            ? "QMD detected."
                            : "QMD detected (\(settings.memoryQmdVersion))."
                    )
                    .foregroundStyle(.secondary)
                } else {
                    Text(
                        "QMD was not detected in PATH. Install with `npm install -g @tobilu/qmd` and run `qmd daemon`."
                    )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let error = settings.memoryQmdError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                if !settings.memoryQmdFeatureEnabled {
                    Text("QMD feature is disabled in this build.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } header: {
                VStack(alignment: .leading, spacing: 4) {
                    Text("QMD")
                        .textCase(nil)
                    Text("QMD backend availability and installation status.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textCase(nil)
                }
            }
        }
    }

    var notificationsPane: some View {
        Group {
            Toggle("Enable notifications", isOn: $settings.notificationsEnabled)
                .onChange(of: settings.notificationsEnabled) { settings.saveNotifications() }
            Toggle("Play sounds", isOn: $settings.notificationsSoundEnabled)
                .onChange(of: settings.notificationsSoundEnabled) { settings.saveNotifications() }
        }
    }

    var cronsPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            if settings.cronJobs.isEmpty {
                SettingsEmptyState(
                    icon: "clock.arrow.circlepath",
                    title: "No Cron Jobs",
                    subtitle: "Scheduled tasks require the gateway to be running"
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
                .onChange(of: settings.heartbeatEnabled) { settings.saveHeartbeat() }
            Stepper(
                String(
                    format: NSLocalizedString(
                        "Interval: %d min",
                        comment: "Heartbeat interval in minutes"
                    ),
                    settings.heartbeatIntervalMinutes
                ),
                value: $settings.heartbeatIntervalMinutes,
                in: 1 ... 120
            )
            .onChange(of: settings.heartbeatIntervalMinutes) { settings.saveHeartbeat() }
        }
    }
}

// MARK: - Security

private extension SettingsSectionContent {
    var securityPane: some View {
        Group {
            Section {
                Text(
                    "Authentication here applies to the HTTP server only "
                        + "(web UI, API, and WebSocket). It does not affect "
                        + "local app-only usage when HTTP Server is off."
                )
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            Section("Status") {
                if settings.securityLoading {
                    ProgressView("Loading authentication status…")
                } else {
                    LabeledContent("Authentication") {
                        Text(settings.authDisabled ? "Disabled" : "Enabled")
                            .foregroundStyle(settings.authDisabled ? .red : .primary)
                    }
                    LabeledContent("Password") {
                        Text(settings.authHasPassword ? "Configured" : "Not configured")
                    }
                    LabeledContent("Passkeys") {
                        Text("\(settings.securityPasskeys.count)")
                    }
                    Button("Refresh status") {
                        settings.loadSecuritySettings()
                    }
                    .disabled(settings.securityBusy)
                }
            }

            Section(settings.authHasPassword ? "Change Password" : "Set Password") {
                if settings.authHasPassword {
                    SecureField("Current password", text: $securityCurrentPassword)
                }
                SecureField(
                    settings.authHasPassword ? "New password" : "Password",
                    text: $securityNewPassword
                )
                SecureField(
                    settings.authHasPassword ? "Confirm new password" : "Confirm password",
                    text: $securityConfirmPassword
                )

                Button(
                    settings.securityBusy
                        ? (settings.authHasPassword ? "Changing…" : "Setting…")
                        : (settings.authHasPassword ? "Change password" : "Set password")
                ) {
                    settings.securityError = nil
                    settings.securityMessage = nil

                    guard securityNewPassword == securityConfirmPassword else {
                        settings.securityError = "Passwords do not match."
                        return
                    }

                    let current = settings.authHasPassword ? securityCurrentPassword : nil
                    settings.changeAuthenticationPassword(
                        currentPassword: current,
                        newPassword: securityNewPassword
                    )
                    if settings.securityError == nil {
                        securityCurrentPassword = ""
                        securityNewPassword = ""
                        securityConfirmPassword = ""
                    }
                }
                .disabled(
                    settings.securityBusy
                        || securityNewPassword.isEmpty
                        || securityConfirmPassword.isEmpty
                        || (settings.authHasPassword && securityCurrentPassword.isEmpty)
                )

                if let message = settings.securityMessage {
                    Text(message)
                        .font(.caption)
                        .foregroundStyle(.green)
                }
                if let error = settings.securityError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                if let recoveryKey = settings.securityRecoveryKey {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("Save this recovery key")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text(recoveryKey)
                            .font(.system(.caption, design: .monospaced))
                            .textSelection(.enabled)
                        Button("Copy Recovery Key") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(recoveryKey, forType: .string)
                        }
                        .controlSize(.small)
                    }
                }
            }

            Section("Passkeys") {
                if settings.securityLoading {
                    ProgressView("Loading passkeys…")
                } else if settings.securityPasskeys.isEmpty {
                    Text("No passkeys registered.")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(settings.securityPasskeys) { passkey in
                        if editingPasskeyId == passkey.id {
                            HStack(spacing: 8) {
                                TextField("Passkey name", text: $editingPasskeyName)
                                Button("Save") {
                                    let name = editingPasskeyName.trimmingCharacters(in: .whitespacesAndNewlines)
                                    guard !name.isEmpty else {
                                        settings.securityError = "Passkey name cannot be empty."
                                        return
                                    }
                                    settings.renamePasskey(id: passkey.id, name: name)
                                    if settings.securityError == nil {
                                        editingPasskeyId = nil
                                        editingPasskeyName = ""
                                    }
                                }
                                .controlSize(.small)
                                .disabled(settings.securityBusy)
                                Button("Cancel") {
                                    editingPasskeyId = nil
                                    editingPasskeyName = ""
                                }
                                .controlSize(.small)
                            }
                        } else {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(passkey.name)
                                    Text(passkey.createdAt)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button("Rename") {
                                    editingPasskeyId = passkey.id
                                    editingPasskeyName = passkey.name
                                }
                                .controlSize(.small)
                                .disabled(settings.securityBusy)
                                Button("Remove", role: .destructive) {
                                    settings.removePasskey(id: passkey.id)
                                    if editingPasskeyId == passkey.id {
                                        editingPasskeyId = nil
                                        editingPasskeyName = ""
                                    }
                                }
                                .controlSize(.small)
                                .disabled(settings.securityBusy)
                            }
                        }
                    }
                }

                Text("Passkey registration uses WebAuthn in the web interface.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button("Open Web Security to Add Passkey") {
                    openWebSecurityInBrowser()
                }
                .disabled(settings.securityBusy)
            }

            Section("Danger Zone") {
                if settings.authSetupComplete {
                    if securityResetConfirm {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Are you sure? This removes password, passkeys, API keys, and sessions.")
                                .font(.caption)
                                .foregroundStyle(.red)
                            HStack(spacing: 8) {
                                Button("Yes, Remove All Authentication", role: .destructive) {
                                    settings.resetAuthentication()
                                    securityResetConfirm = false
                                }
                                .disabled(settings.securityBusy)
                                Button("Cancel") {
                                    securityResetConfirm = false
                                }
                                .disabled(settings.securityBusy)
                            }
                        }
                    } else {
                        Button("Remove All Authentication", role: .destructive) {
                            securityResetConfirm = true
                        }
                        .disabled(settings.securityBusy)
                    }
                } else {
                    Text("Authentication is not fully configured yet.")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .onAppear {
            settings.loadSecuritySettings()
        }
    }

    func openWebSecurityInBrowser() {
        let port = UInt16(settings.httpdPort) ?? 8080
        let address = "127.0.0.1:\(port)"
        guard let url = URL(string: "http://\(address)/settings/security") else {
            return
        }
        NSWorkspace.shared.open(url)
    }

    var tailscalePane: some View {
        Group {
            Picker("Tailscale mode", selection: $settings.tailscaleMode) {
                ForEach(settings.tailscaleModes, id: \.self) { mode in
                    Text(mode.capitalized).tag(mode)
                }
            }
            .onChange(of: settings.tailscaleMode) {
                settings.tailscaleEnabled = settings.tailscaleMode != "off"
                settings.saveTailscale()
            }
        }
    }
}

// MARK: - Integrations

private extension SettingsSectionContent {
    var channelsPane: some View {
        VStack(alignment: .leading, spacing: 16) {
            if settings.channels.isEmpty {
                SettingsEmptyState(
                    icon: "point.3.connected.trianglepath.dotted",
                    title: "No Channels",
                    subtitle: "Connect messaging platforms like Telegram, Teams, Discord, and WhatsApp"
                )
            } else {
                ForEach($settings.channels) { $item in
                    DisclosureGroup {
                        channelFields(item: $item)
                            .padding(.top, 4)
                    } label: {
                        channelLabel(item: $item)
                    }
                    if item.id != settings.channels.last?.id {
                        Divider()
                    }
                }
            }
            Menu {
                ForEach(ChannelItem.channelTypes, id: \.self) { channelType in
                    Button("Add \(ChannelItem.displayName(for: channelType))") {
                        settings.channels.append(ChannelItem(channelType: channelType))
                    }
                }
            } label: {
                Label("Add Channel", systemImage: "plus")
            }
        }
        .onChange(of: settings.channels) { settings.saveChannels() }
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
                settings.saveHooks()
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
                settings.saveMcp()
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
                settings.saveSkills()
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
        SandboxesPane(settings: settings)
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
            Toggle("Enable metrics collection", isOn: $settings.metricsEnabled)
                .onChange(of: settings.metricsEnabled) { settings.saveMonitoring() }
            Toggle("Enable Prometheus endpoint", isOn: $settings.prometheusEndpointEnabled)
                .onChange(of: settings.prometheusEndpointEnabled) { settings.saveMonitoring() }
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
                .onChange(of: settings.graphqlEnabled) { settings.saveGraphql() }
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
