import Combine
import Foundation

final class AppSettings: ObservableObject {
    @Published var identityName = "Moltis"
    @Published var identityEmoji = ""
    @Published var identityTheme = ""
    @Published var identityUserName = ""
    @Published var identitySoul = ""

    @Published var environmentConfigDir = ""
    @Published var environmentDataDir = ""

    @Published var memoryEnabled = true
    @Published var memoryMode = "workspace"

    @Published var notificationsEnabled = true
    @Published var notificationsSoundEnabled = false

    @Published var cronJobs: [CronJobItem] = []
    @Published var heartbeatEnabled = true
    @Published var heartbeatIntervalMinutes = 5

    @Published var requirePassword = true
    @Published var passkeysEnabled = true
    @Published var tailscaleEnabled = false
    @Published var tailscaleHostname = ""

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

    @Published var monitoringEnabled = true
    @Published var metricsEnabled = true
    @Published var tracingEnabled = true

    @Published var logLevel = "info"
    @Published var persistLogs = true

    @Published var graphqlEnabled = false
    @Published var graphqlPath = "/graphql"

    @Published var httpdEnabled = false
    @Published var httpdBindMode = "loopback"
    @Published var httpdPort = "8080"

    let httpdBindModes = ["loopback", "all"]

    @Published var configurationToml = ""

    let memoryModes = ["workspace", "global", "off"]
    let sandboxBackends = ["auto", "docker", "apple-container"]
    let logLevels = ["trace", "debug", "info", "warn", "error"]
}
