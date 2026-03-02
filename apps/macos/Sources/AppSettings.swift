// swiftlint:disable file_length
import Combine
import Foundation

// swiftlint:disable type_body_length
final class AppSettings: ObservableObject {
    @Published var identityName = "Moltis"
    @Published var identityEmoji = ""
    @Published var identityTheme = ""
    @Published var identityUserName = ""
    @Published var identitySoul = ""

    @Published var environmentConfigDir = ""
    @Published var environmentDataDir = ""
    @Published var envVars: [EnvVarItem] = []
    @Published var environmentVaultStatus = "disabled"
    @Published var newEnvKey = ""
    @Published var newEnvValue = ""
    @Published var envMessage: String?
    @Published var envError: String?
    @Published var updatingEnvVarId: Int64?
    @Published var updatingEnvValue = ""
    @Published var environmentBusy = false

    @Published var memoryEnabled = true
    @Published var memoryBackend = "builtin"
    @Published var memoryCitations = "auto"
    @Published var memoryLlmReranking = false
    @Published var memorySessionExport = false
    @Published var memoryQmdFeatureEnabled = true
    @Published var memoryStatusAvailable = false
    @Published var memoryTotalFiles = 0
    @Published var memoryTotalChunks = 0
    @Published var memoryEmbeddingModel = "none"
    @Published var memoryDbSize: UInt64 = 0
    @Published var memoryDbSizeDisplay = "0 B"
    @Published var memoryHasEmbeddings = false
    @Published var memoryQmdAvailable = false
    @Published var memoryQmdVersion = ""
    @Published var memoryQmdError: String?
    @Published var memoryLoading = false
    @Published var memorySaving = false
    @Published var memorySaved = false
    @Published var memoryError: String?

    @Published var notificationsEnabled = true
    @Published var notificationsSoundEnabled = false

    @Published var cronJobs: [CronJobItem] = []
    @Published var heartbeatEnabled = true
    @Published var heartbeatIntervalMinutes = 5

    @Published var authDisabled = false
    @Published var authHasPassword = false
    @Published var authHasPasskeys = false
    @Published var authSetupComplete = false
    @Published var securityPasskeys: [BridgeAuthPasskeyEntry] = []
    @Published var securityLoading = false
    @Published var securityBusy = false
    @Published var securityMessage: String?
    @Published var securityError: String?
    @Published var securityRecoveryKey: String?
    @Published var tailscaleEnabled = false
    @Published var tailscaleMode = "off"

    @Published var channels: [ChannelItem] = []
    @Published var hooks: [HookItem] = []

    @Published var llmProvider = "openai"
    @Published var llmModel = "gpt-4.1"
    @Published var llmApiKey = ""

    @Published var mcpServers: [McpServerItem] = []
    @Published var skillPacks: [SkillPackItem] = []

    @Published var voiceEnabled = false
    @Published var voiceProvider = "none"
    @Published var voiceApiKey = ""

    @Published var terminalEnabled = false
    @Published var terminalShell = "/bin/zsh"

    @Published var sandboxEnabled = false
    @Published var containerImage = ""
    @Published var debugEnabled = false

    @Published var sandboxBackend = "auto"
    @Published var sandboxImage = "moltis/sandbox:latest"
    @Published var sandboxLoading = false
    @Published var sandboxError: String?
    @Published var sandboxRuntimeBackend = "none"
    @Published var sandboxRuntimeOS = "unknown"
    @Published var sandboxRuntimeDefaultImage = "ubuntu:25.10"
    @Published var sandboxDefaultImageDraft = ""
    @Published var sandboxDefaultImageSaving = false
    @Published var sandboxDefaultImageError: String?
    @Published var sandboxDefaultImageMessage: String?

    @Published var sandboxImages: [BridgeSandboxImageEntry] = []
    @Published var sandboxImagesLoading = false
    @Published var sandboxImagesBusy = false
    @Published var sandboxImagesError: String?
    @Published var sandboxImagesMessage: String?

    @Published var sandboxBuildName = ""
    @Published var sandboxBuildBase = "ubuntu:25.10"
    @Published var sandboxBuildPackages = ""
    @Published var sandboxBuilding = false
    @Published var sandboxBuildStatus = ""
    @Published var sandboxBuildWarning = ""

    @Published var sandboxContainers: [BridgeSandboxContainerEntry] = []
    @Published var sandboxContainersLoading = false
    @Published var sandboxContainersBusy = false
    @Published var sandboxContainersError: String?
    @Published var sandboxDiskUsage: BridgeSandboxDiskUsagePayload?

    @Published var sandboxSharedHomeEnabled = false
    @Published var sandboxSharedHomeMode = "off"
    @Published var sandboxSharedHomePath = ""
    @Published var sandboxSharedHomeConfiguredPath = ""
    @Published var sandboxSharedHomeLoading = false
    @Published var sandboxSharedHomeSaving = false
    @Published var sandboxSharedHomeError: String?
    @Published var sandboxSharedHomeMessage: String?

    @Published var metricsEnabled = true
    @Published var prometheusEndpointEnabled = true

    @Published var logLevel = "info"

    @Published var graphqlEnabled = false
    @Published var graphqlPath = "/graphql"

    @Published var httpdEnabled = false
    @Published var httpdBindMode = "loopback"
    @Published var httpdPort = "8080"

    let httpdBindModes = ["loopback", "all"]

    @Published var configurationToml = ""

    let memoryBackends = ["builtin", "qmd"]
    let memoryCitationModes = ["auto", "on", "off"]
    let sandboxBackends = ["auto", "docker", "apple-container"]
    let logLevels = ["trace", "debug", "info", "warn", "error"]
    let tailscaleModes = ["off", "serve", "funnel"]

    var sandboxRuntimeAvailable: Bool {
        sandboxRuntimeBackend != "none"
    }

    /// Whether settings have been loaded from the backend at least once.
    @Published private(set) var isLoaded = false

    // MARK: - Private state

    let client = MoltisClient()
    /// Raw config dictionary for round-tripping. Modified in-place by section
    /// save methods and sent back to Rust as the full config JSON.
    var rawConfig: [String: Any] = [:]
    private var cancellables = Set<AnyCancellable>()

    init() {
        // Debounce soul text saves so we don't hit FFI on every keystroke.
        $identitySoul
            .dropFirst()
            .debounce(for: .seconds(0.8), scheduler: RunLoop.main)
            .removeDuplicates()
            .sink { [weak self] _ in self?.saveSoul() }
            .store(in: &cancellables)
    }

    // MARK: - Load

    /// Loads all settings from the Rust backend (config file + identity files).
    func load() {
        do {
            let result = try client.getConfig()
            rawConfig = result.config
            environmentConfigDir = result.configDir
            environmentDataDir = result.dataDir
            populateFromConfig(result.config)
            loadMemorySettings()
            loadSecuritySettings()
        } catch {
            logSettingsError("load config", error)
        }

        loadEnvironmentVariables()

        do {
            let identity = try client.getIdentity()
            identityName = identity.name
            identityEmoji = identity.emoji ?? ""
            identityTheme = identity.theme ?? ""
            identityUserName = identity.userName ?? ""
        } catch {
            logSettingsError("load identity", error)
        }

        do {
            identitySoul = try client.getSoul() ?? ""
        } catch {
            logSettingsError("load soul", error)
        }

        isLoaded = true
    }

    // MARK: - Section saves

    func saveIdentity() {
        let name = identityName.isEmpty ? nil : identityName
        let emoji = identityEmoji.isEmpty ? nil : identityEmoji
        let theme = identityTheme.isEmpty ? nil : identityTheme
        do {
            try client.saveIdentity(name: name, emoji: emoji, theme: theme)
        } catch {
            logSettingsError("save identity", error)
        }
    }

    func saveUserProfile() {
        let name = identityUserName.isEmpty ? nil : identityUserName
        do {
            try client.saveUserProfile(name: name)
        } catch {
            logSettingsError("save user profile", error)
        }
    }

    func saveSoul() {
        let text = identitySoul.isEmpty ? nil : identitySoul
        do {
            try client.saveSoul(text)
        } catch {
            logSettingsError("save soul", error)
        }
    }

    func loadMemorySettings() {
        memoryLoading = true
        memoryError = nil
        memoryQmdError = nil

        do {
            let config = try client.memoryConfigGet()
            applyMemoryConfig(config)
            syncMemoryConfigIntoRawConfig()
        } catch {
            memoryError = error.localizedDescription
            logSettingsError("load memory config", error)
        }

        do {
            let status = try client.memoryStatus()
            applyMemoryStatus(status)
        } catch {
            memoryError = error.localizedDescription
            logSettingsError("load memory status", error)
        }

        do {
            let qmdStatus = try client.memoryQmdStatus()
            applyMemoryQmdStatus(qmdStatus)
        } catch {
            memoryQmdError = error.localizedDescription
            logSettingsError("load memory qmd status", error)
        }

        memoryLoading = false
    }

    func saveNotifications() {
        setConfigValue(notificationsEnabled, at: ["notifications", "enabled"])
        setConfigValue(notificationsSoundEnabled, at: ["notifications", "sound"])
        persistConfig("notifications")
    }

    func saveHeartbeat() {
        setConfigValue(heartbeatEnabled, at: ["heartbeat", "enabled"])
        let durationStr = "\(heartbeatIntervalMinutes)m"
        setConfigValue(durationStr, at: ["heartbeat", "every"])
        persistConfig("heartbeat")
    }

    func saveMemory() {
        memoryError = nil
        memorySaving = true
        memorySaved = false

        do {
            let updated = try client.memoryConfigUpdate(
                backend: memoryBackend,
                citations: memoryCitations,
                llmReranking: memoryLlmReranking,
                disableRag: !memoryEnabled,
                sessionExport: memorySessionExport
            )
            applyMemoryConfig(updated)
            syncMemoryConfigIntoRawConfig()
            memorySaved = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
                self?.memorySaved = false
            }
        } catch {
            memoryError = error.localizedDescription
            logSettingsError("save memory", error)
        }

        memorySaving = false
    }

    func loadSecuritySettings() {
        securityLoading = true
        securityError = nil

        do {
            let status = try client.authStatus()
            applyAuthStatus(status)
        } catch {
            securityError = error.localizedDescription
            logSettingsError("load auth status", error)
        }

        do {
            let passkeys = try client.authListPasskeys()
            securityPasskeys = passkeys
            authHasPasskeys = !passkeys.isEmpty
        } catch {
            if securityError == nil {
                securityError = error.localizedDescription
            }
            logSettingsError("load passkeys", error)
        }

        securityLoading = false
    }

    func changeAuthenticationPassword(currentPassword: String?, newPassword: String) {
        guard newPassword.count >= 8 else {
            securityError = "New password must be at least 8 characters."
            securityMessage = nil
            return
        }

        securityBusy = true
        securityError = nil
        securityMessage = nil
        securityRecoveryKey = nil
        let hadPassword = authHasPassword

        do {
            let result = try client.authPasswordChange(
                currentPassword: currentPassword,
                newPassword: newPassword
            )
            securityMessage = hadPassword ? "Password changed." : "Password set."
            securityRecoveryKey = result.recoveryKey
            loadSecuritySettings()
        } catch {
            securityError = error.localizedDescription
            logSettingsError("change password", error)
        }

        securityBusy = false
    }

    func resetAuthentication() {
        securityBusy = true
        securityError = nil
        securityMessage = nil
        securityRecoveryKey = nil

        do {
            try client.authReset()
            securityMessage = "Authentication disabled."
            loadSecuritySettings()
        } catch {
            securityError = error.localizedDescription
            logSettingsError("reset auth", error)
        }

        securityBusy = false
    }

    func removePasskey(id: Int64) {
        securityBusy = true
        securityError = nil
        securityMessage = nil

        do {
            try client.authRemovePasskey(id: id)
            securityMessage = "Passkey removed."
            loadSecuritySettings()
        } catch {
            securityError = error.localizedDescription
            logSettingsError("remove passkey", error)
        }

        securityBusy = false
    }

    func renamePasskey(id: Int64, name: String) {
        securityBusy = true
        securityError = nil
        securityMessage = nil

        do {
            try client.authRenamePasskey(id: id, name: name)
            securityMessage = "Passkey renamed."
            loadSecuritySettings()
        } catch {
            securityError = error.localizedDescription
            logSettingsError("rename passkey", error)
        }

        securityBusy = false
    }

    func saveTailscale() {
        setConfigValue(tailscaleMode, at: ["tailscale", "mode"])
        persistConfig("tailscale")
    }

    func saveMonitoring() {
        setConfigValue(metricsEnabled, at: ["metrics", "enabled"])
        setConfigValue(prometheusEndpointEnabled, at: ["metrics", "prometheus_endpoint"])
        persistConfig("monitoring")
    }

    func saveGraphql() {
        setConfigValue(graphqlEnabled, at: ["graphql", "enabled"])
        persistConfig("graphql")
    }

    func saveSandbox() {
        setConfigValue(sandboxBackend, at: ["tools", "exec", "sandbox", "backend"])
        let image: Any = sandboxImage.isEmpty ? NSNull() : sandboxImage
        setConfigValue(image, at: ["tools", "exec", "sandbox", "image"])
        persistConfig("sandbox")
    }

    func saveChannels() {
        // Convert ChannelItem array back to config shape.
        // Each channel type is a HashMap<String, Value> keyed by account ID.
        let existingChannelsConfig = rawConfig["channels"] as? [String: Any] ?? [:]
        var channelsByType = emptyChannelConfigMap()

        for channel in channels {
            guard let accountId = normalizedChannelAccountId(channel.accountId) else { continue }
            let existingEntry = existingChannelEntry(
                from: existingChannelsConfig,
                channelType: channel.channelType,
                accountId: accountId
            )
            let entry = mergedChannelEntry(
                for: channel,
                accountId: accountId,
                existingEntry: existingEntry
            )

            channelsByType[channel.channelType, default: [:]][accountId] = entry
        }

        for (channelType, entries) in channelsByType {
            setConfigValue(entries, at: ["channels", channelType])
        }
        persistConfig("channels")
    }

    private func emptyChannelConfigMap() -> [String: [String: Any]] {
        var channelsByType: [String: [String: Any]] = [:]
        for channelType in ChannelItem.channelTypes {
            channelsByType[channelType] = [:]
        }
        return channelsByType
    }

    private func normalizedChannelAccountId(_ accountId: String) -> String? {
        let trimmed = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private func existingChannelEntry(
        from channelsConfig: [String: Any],
        channelType: String,
        accountId: String
    ) -> [String: Any] {
        let existingTypeMap = channelsConfig[channelType] as? [String: Any]
        return existingTypeMap?[accountId] as? [String: Any] ?? [:]
    }

    private func mergedChannelEntry(
        for channel: ChannelItem,
        accountId: String,
        existingEntry: [String: Any]
    ) -> [String: Any] {
        var entry = existingEntry
        entry["enabled"] = channel.enabled

        let name = channel.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.isEmpty {
            entry.removeValue(forKey: "name")
        } else {
            entry["name"] = name
        }

        switch channel.channelType {
        case "msteams":
            applyTeamsChannelFields(channel, accountId: accountId, to: &entry)
        case "telegram", "discord":
            applyTokenChannelFields(channel, to: &entry)
        case "whatsapp":
            clearChannelCredentials(&entry)
        default:
            break
        }
        return entry
    }

    private func applyTeamsChannelFields(
        _ channel: ChannelItem,
        accountId: String,
        to entry: inout [String: Any]
    ) {
        let appId = channel.appId.trimmingCharacters(in: .whitespacesAndNewlines)
        entry["app_id"] = appId.isEmpty ? accountId : appId

        let appPassword = channel.credential.trimmingCharacters(in: .whitespacesAndNewlines)
        if appPassword.isEmpty {
            entry.removeValue(forKey: "app_password")
        } else {
            entry["app_password"] = appPassword
        }

        let webhookSecret = channel.webhookSecret.trimmingCharacters(in: .whitespacesAndNewlines)
        if webhookSecret.isEmpty {
            entry.removeValue(forKey: "webhook_secret")
        } else {
            entry["webhook_secret"] = webhookSecret
        }

        entry.removeValue(forKey: "token")
        entry.removeValue(forKey: "bot_token")
    }

    private func applyTokenChannelFields(
        _ channel: ChannelItem,
        to entry: inout [String: Any]
    ) {
        let token = channel.credential.trimmingCharacters(in: .whitespacesAndNewlines)
        if token.isEmpty {
            entry.removeValue(forKey: "token")
            entry.removeValue(forKey: "bot_token")
        } else {
            entry["token"] = token
            entry.removeValue(forKey: "bot_token")
        }

        entry.removeValue(forKey: "app_password")
        entry.removeValue(forKey: "app_id")
        entry.removeValue(forKey: "webhook_secret")
    }

    private func clearChannelCredentials(_ entry: inout [String: Any]) {
        entry.removeValue(forKey: "token")
        entry.removeValue(forKey: "bot_token")
        entry.removeValue(forKey: "app_password")
        entry.removeValue(forKey: "app_id")
        entry.removeValue(forKey: "webhook_secret")
    }

    func saveHooks() {
        let hookEntries: [[String: Any]] = hooks.map { hook in
            var entry: [String: Any] = [
                "name": hook.name,
                "command": hook.command,
                "events": [hook.event]
            ]
            if !hook.enabled {
                entry["timeout"] = 0
            }
            return entry
        }
        setConfigValue(["hooks": hookEntries], at: ["hooks"])
        persistConfig("hooks")
    }

    func saveMcp() {
        var servers: [String: Any] = [:]
        for item in mcpServers {
            let key = item.name.isEmpty ? "unnamed" : item.name
            var entry: [String: Any] = [
                "enabled": item.enabled,
                "transport": item.transport.rawValue
            ]
            if item.transport == .stdio, !item.command.isEmpty {
                entry["command"] = item.command
            }
            if item.transport == .sse, !item.url.isEmpty {
                entry["url"] = item.url
            }
            servers[key] = entry
        }
        setConfigValue(servers, at: ["mcp", "servers"])
        persistConfig("mcp")
    }

    func saveSkills() {
        let paths: [String] = skillPacks.map { $0.source }
        let autoLoad: [String] = skillPacks.filter { $0.enabled }.map { $0.source }
        setConfigValue(paths, at: ["skills", "search_paths"])
        setConfigValue(autoLoad, at: ["skills", "auto_load"])
        persistConfig("skills")
    }

    func saveVoice() {
        setConfigValue(voiceEnabled, at: ["voice", "tts", "enabled"])
        if !voiceProvider.isEmpty, voiceProvider != "none" {
            setConfigValue(voiceProvider, at: ["voice", "tts", "provider"])
        }
        persistConfig("voice")
    }
}
// swiftlint:enable type_body_length

// MARK: - Config population and persistence

extension AppSettings {
    func populateFromConfig(_ config: [String: Any]) {
        populateToggles(from: config)
        populateCollections(from: config)
    }

    private func applyMemoryConfig(_ config: BridgeMemoryConfigPayload) {
        memoryBackend = config.backend
        memoryCitations = config.citations
        memoryEnabled = !config.disableRag
        memoryLlmReranking = config.llmReranking
        memorySessionExport = config.sessionExport
        memoryQmdFeatureEnabled = config.qmdFeatureEnabled
    }

    private func applyMemoryStatus(_ status: BridgeMemoryStatusPayload) {
        memoryStatusAvailable = status.available
        memoryTotalFiles = status.totalFiles
        memoryTotalChunks = status.totalChunks
        memoryEmbeddingModel = status.embeddingModel
        memoryDbSize = status.dbSize
        memoryDbSizeDisplay = status.dbSizeDisplay
        memoryHasEmbeddings = status.hasEmbeddings
        if let statusError = status.error, !statusError.isEmpty {
            memoryError = statusError
        }
    }

    private func applyMemoryQmdStatus(_ status: BridgeMemoryQmdStatusPayload) {
        memoryQmdFeatureEnabled = status.featureEnabled
        memoryQmdAvailable = status.available
        memoryQmdVersion = status.version ?? ""
        memoryQmdError = status.error
    }

    private func applyAuthStatus(_ status: BridgeAuthStatusPayload) {
        authDisabled = status.authDisabled
        authHasPassword = status.hasPassword
        authHasPasskeys = status.hasPasskeys
        authSetupComplete = status.setupComplete
    }

    private func syncMemoryConfigIntoRawConfig() {
        setConfigValue(memoryBackend, at: ["memory", "backend"])
        setConfigValue(memoryCitations, at: ["memory", "citations"])
        setConfigValue(!memoryEnabled, at: ["memory", "disable_rag"])
        setConfigValue(memoryLlmReranking, at: ["memory", "llm_reranking"])
        setConfigValue(memorySessionExport, at: ["memory", "session_export"])
    }

    private func populateToggles(from config: [String: Any]) {
        if let notifications = config["notifications"] as? [String: Any] {
            notificationsEnabled = notifications["enabled"] as? Bool ?? true
            notificationsSoundEnabled = notifications["sound"] as? Bool ?? false
        }
        if let heartbeat = config["heartbeat"] as? [String: Any] {
            heartbeatEnabled = heartbeat["enabled"] as? Bool ?? true
            if let every = heartbeat["every"] as? String {
                heartbeatIntervalMinutes = parseMinutes(from: every) ?? 5
            }
        }
        if let memory = config["memory"] as? [String: Any] {
            memoryEnabled = !(memory["disable_rag"] as? Bool ?? false)
            memoryBackend = memory["backend"] as? String ?? "builtin"
            memoryCitations = memory["citations"] as? String ?? "auto"
            memoryLlmReranking = memory["llm_reranking"] as? Bool ?? false
            memorySessionExport = memory["session_export"] as? Bool ?? false
            memoryHasEmbeddings = memory["disable_rag"] as? Bool == true ? false : true
        }
        if let tailscale = config["tailscale"] as? [String: Any] {
            tailscaleMode = tailscale["mode"] as? String ?? "off"
            tailscaleEnabled = tailscaleMode != "off"
        }
        if let metrics = config["metrics"] as? [String: Any] {
            metricsEnabled = metrics["enabled"] as? Bool ?? true
            prometheusEndpointEnabled = metrics["prometheus_endpoint"] as? Bool ?? true
        }
        if let graphql = config["graphql"] as? [String: Any] {
            graphqlEnabled = graphql["enabled"] as? Bool ?? false
        }
        if let tools = config["tools"] as? [String: Any],
           let exec = tools["exec"] as? [String: Any],
           let sandbox = exec["sandbox"] as? [String: Any] {
            sandboxBackend = sandbox["backend"] as? String ?? "auto"
            sandboxImage = sandbox["image"] as? String ?? "moltis/sandbox:latest"
            sandboxRuntimeDefaultImage = sandboxImage
            sandboxDefaultImageDraft = sandboxImage
        }
        if let voice = config["voice"] as? [String: Any],
           let tts = voice["tts"] as? [String: Any] {
            voiceEnabled = tts["enabled"] as? Bool ?? false
            voiceProvider = tts["provider"] as? String ?? "none"
        }
    }

    private func populateCollections(from config: [String: Any]) {
        populateChannels(from: config)
        populateHooksAndServers(from: config)
    }

    private func populateChannels(from config: [String: Any]) {
        channels = []
        guard let channelsConfig = config["channels"] as? [String: Any] else { return }
        for channelType in ChannelItem.channelTypes {
            guard let typeMap = channelsConfig[channelType] as? [String: Any] else { continue }
            for (accountId, value) in typeMap {
                guard let entry = value as? [String: Any] else { continue }
                let credential: String
                switch channelType {
                case "msteams":
                    credential = entry["app_password"] as? String ?? ""
                case "telegram", "discord":
                    credential = (entry["token"] as? String) ?? (entry["bot_token"] as? String) ?? ""
                case "whatsapp":
                    credential = ""
                default:
                    credential = ""
                }

                channels.append(ChannelItem(
                    name: entry["name"] as? String ?? "",
                    accountId: accountId,
                    channelType: channelType,
                    credential: credential,
                    appId: entry["app_id"] as? String ?? accountId,
                    webhookSecret: entry["webhook_secret"] as? String ?? "",
                    enabled: entry["enabled"] as? Bool ?? true
                ))
            }
        }
    }

    private func populateHooksAndServers(from config: [String: Any]) {
        hooks = []
        if let hooksConfig = config["hooks"] as? [String: Any],
           let hooksList = hooksConfig["hooks"] as? [[String: Any]] {
            for entry in hooksList {
                let events = entry["events"] as? [String] ?? []
                hooks.append(HookItem(
                    name: entry["name"] as? String ?? "",
                    event: events.first ?? "on_message",
                    command: entry["command"] as? String ?? "",
                    enabled: true
                ))
            }
        }
        mcpServers = []
        if let mcp = config["mcp"] as? [String: Any],
           let servers = mcp["servers"] as? [String: Any] {
            for (name, value) in servers {
                guard let entry = value as? [String: Any] else { continue }
                let transportStr = entry["transport"] as? String ?? "stdio"
                let transport: McpTransport = transportStr == "sse" ? .sse : .stdio
                mcpServers.append(McpServerItem(
                    name: name,
                    transport: transport,
                    command: entry["command"] as? String ?? "",
                    url: entry["url"] as? String ?? "",
                    enabled: entry["enabled"] as? Bool ?? true
                ))
            }
        }
        skillPacks = []
        if let skills = config["skills"] as? [String: Any] {
            let searchPaths = skills["search_paths"] as? [String] ?? []
            let autoLoad = Set(skills["auto_load"] as? [String] ?? [])
            for path in searchPaths {
                skillPacks.append(SkillPackItem(
                    source: path,
                    repoName: URL(fileURLWithPath: path).lastPathComponent,
                    enabled: autoLoad.contains(path)
                ))
            }
        }
    }

    /// Sets a value in the raw config dictionary at the given key path.
    func setConfigValue(_ value: Any, at keyPath: [String]) {
        guard !keyPath.isEmpty else { return }
        if keyPath.count == 1 {
            rawConfig[keyPath[0]] = value
            return
        }
        // Walk into nested dictionaries, creating them as needed.
        var current = rawConfig
        var parents: [([String: Any], String)] = []

        for key in keyPath.dropLast() {
            parents.append((current, key))
            current = current[key] as? [String: Any] ?? [:]
        }
        guard let lastKey = keyPath.last else { return }
        current[lastKey] = value

        // Walk back up rebuilding parent dictionaries.
        for (parent, key) in parents.reversed() {
            var rebuilt = parent
            rebuilt[key] = current
            current = rebuilt
        }
        rawConfig = current
    }

    /// Persists the current rawConfig to the backend.
    func persistConfig(_ context: String) {
        do {
            try client.saveConfig(rawConfig)
        } catch {
            logSettingsError("save \(context)", error)
        }
    }

    /// Parses a duration string like "30m" or "1h" into minutes.
    func parseMinutes(from duration: String) -> Int? {
        let trimmed = duration.trimmingCharacters(in: .whitespaces)
        if trimmed.hasSuffix("m"), let value = Int(trimmed.dropLast()) {
            return value
        }
        if trimmed.hasSuffix("h"), let value = Int(trimmed.dropLast()) {
            return value * 60
        }
        if trimmed.hasSuffix("s"), let value = Int(trimmed.dropLast()) {
            return max(1, value / 60)
        }
        return Int(trimmed)
    }

    func logSettingsError(_ context: String, _ error: Error) {
        print("[AppSettings] Failed to \(context): \(error.localizedDescription)")
    }
}
