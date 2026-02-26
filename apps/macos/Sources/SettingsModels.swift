import Foundation

// MARK: - Channels

struct ChannelItem: Identifiable, Equatable {
    let id: UUID
    var name: String
    var channelType: String
    var botToken: String
    var enabled: Bool

    init(
        id: UUID = UUID(),
        name: String = "",
        channelType: String = "telegram",
        botToken: String = "",
        enabled: Bool = true
    ) {
        self.id = id
        self.name = name
        self.channelType = channelType
        self.botToken = botToken
        self.enabled = enabled
    }

    static let channelTypes = ["telegram", "slack", "discord"]
}

// MARK: - Hooks

struct HookItem: Identifiable, Equatable {
    let id: UUID
    var name: String
    var event: String
    var command: String
    var enabled: Bool

    init(
        id: UUID = UUID(),
        name: String = "",
        event: String = "on_message",
        command: String = "",
        enabled: Bool = true
    ) {
        self.id = id
        self.name = name
        self.event = event
        self.command = command
        self.enabled = enabled
    }

    static let eventTypes = [
        "on_message",
        "on_session_start",
        "on_session_end",
        "on_tool_call",
        "on_error",
        "on_heartbeat"
    ]
}

// MARK: - MCP Servers

enum McpTransport: String, CaseIterable, Equatable {
    case stdio
    case sse
}

struct McpServerItem: Identifiable, Equatable {
    let id: UUID
    var name: String
    var transport: McpTransport
    var command: String
    var url: String
    var enabled: Bool

    init(
        id: UUID = UUID(),
        name: String = "",
        transport: McpTransport = .stdio,
        command: String = "",
        url: String = "",
        enabled: Bool = true
    ) {
        self.id = id
        self.name = name
        self.transport = transport
        self.command = command
        self.url = url
        self.enabled = enabled
    }
}

// MARK: - Skills

struct SkillPackItem: Identifiable, Equatable {
    let id: UUID
    var source: String
    var repoName: String
    var enabled: Bool
    var trusted: Bool

    init(
        id: UUID = UUID(),
        source: String = "",
        repoName: String = "",
        enabled: Bool = true,
        trusted: Bool = false
    ) {
        self.id = id
        self.source = source
        self.repoName = repoName
        self.enabled = enabled
        self.trusted = trusted
    }
}

// MARK: - Cron Jobs

enum CronScheduleType: String, CaseIterable, Equatable {
    case cron = "Cron"
    case interval = "Interval"
    case oneShot = "One-Shot"
}

struct CronJobItem: Identifiable, Equatable {
    let id: UUID
    var name: String
    var scheduleType: CronScheduleType
    var cronExpr: String
    var intervalMinutes: Int
    var message: String
    var enabled: Bool

    init(
        id: UUID = UUID(),
        name: String = "",
        scheduleType: CronScheduleType = .cron,
        cronExpr: String = "",
        intervalMinutes: Int = 60,
        message: String = "",
        enabled: Bool = true
    ) {
        self.id = id
        self.name = name
        self.scheduleType = scheduleType
        self.cronExpr = cronExpr
        self.intervalMinutes = intervalMinutes
        self.message = message
        self.enabled = enabled
    }
}
