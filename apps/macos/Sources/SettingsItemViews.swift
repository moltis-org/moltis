import SwiftUI

// MARK: - Channel Helpers

extension SettingsSectionContent {
    func channelLabel(item: Binding<ChannelItem>) -> some View {
        let channelType = item.wrappedValue.channelType
        let name = item.wrappedValue.name.trimmingCharacters(in: .whitespacesAndNewlines)
        let accountId = item.wrappedValue.accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = ChannelItem.displayName(for: channelType)
        let title = !name.isEmpty ? name : (!accountId.isEmpty ? accountId : "New \(displayName) Channel")

        return HStack {
            Text(title)
            Text(displayName)
                .font(.caption)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.quaternary)
                .clipShape(Capsule())
            Spacer()
            Toggle("", isOn: item.enabled)
                .labelsHidden()
            deleteButton {
                settings.channels.removeAll { $0.id == item.wrappedValue.id }
            }
        }
    }

    func channelFields(item: Binding<ChannelItem>) -> some View {
        Group {
            TextField("Name", text: item.name, prompt: Text("e.g. My Support Bot"))

            LabeledContent("Channel Type") {
                Text(ChannelItem.displayName(for: item.wrappedValue.channelType))
                    .foregroundStyle(.secondary)
            }

            channelTypeFields(item: item)
        }
    }

    @ViewBuilder
    func channelTypeFields(item: Binding<ChannelItem>) -> some View {
        switch item.wrappedValue.channelType {
        case "telegram":
            channelHelpText(
                title: "How to create a Telegram bot",
                steps: [
                    "Open @BotFather in Telegram",
                    "Send /newbot and follow the prompts",
                    "Copy the bot token and paste it below"
                ]
            )
            TextField("Bot Username", text: item.accountId, prompt: Text("e.g. my_assistant_bot"))
            SecureField("Bot Token", text: item.credential, prompt: Text("123456:ABC-DEF\u{2026}"))
            telegramChatLink(username: item.wrappedValue.accountId)

        case "msteams":
            channelHelpText(
                title: "Microsoft Teams setup",
                steps: [
                    "Create an Azure Bot registration \u{2014} copy the App ID and App Password",
                    "Use Bootstrap Teams to generate the messaging endpoint",
                    "CLI shortcut: moltis channels teams bootstrap"
                ]
            )
            TextField("App ID / Account ID", text: item.accountId, prompt: Text("Azure App ID or alias"))
            TextField("App ID override", text: item.appId, prompt: Text("Separate App ID if different"))
            SecureField("App Password", text: item.credential, prompt: Text("Azure client secret"))
            TextField("Webhook Secret", text: item.webhookSecret, prompt: Text("Optional verification secret"))

        case "discord":
            channelHelpText(
                title: "How to set up a Discord bot",
                steps: [
                    "Go to the Discord Developer Portal",
                    "Create Application \u{2192} Bot tab \u{2192} copy the bot token",
                    "Enable 'Message Content Intent' under Privileged Gateway Intents",
                    "Paste the token below"
                ]
            )
            TextField("Account ID", text: item.accountId, prompt: Text("Discord bot or app ID"))
            SecureField("Bot Token", text: item.credential, prompt: Text("Bot token from Developer Portal"))

        case "whatsapp":
            TextField("Account ID", text: item.accountId, prompt: Text("WhatsApp phone number or ID"))
            Text("Pairing is done from the web UI or CLI. No bot token needed.")
                .font(.caption)
                .foregroundStyle(.secondary)

        default:
            TextField("Bot Username / Account ID", text: item.accountId, prompt: Text("Bot username or account ID"))
            SecureField("Bot Token", text: item.credential, prompt: Text("Bot token or API key"))
        }
    }

    @ViewBuilder
    func telegramChatLink(username: String) -> some View {
        let trimmed = username.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty, let url = URL(string: "https://t.me/\(trimmed)") {
            HStack(spacing: 4) {
                Image(systemName: "paperplane.fill")
                    .font(.caption2)
                Link("Open t.me/\(trimmed) to chat with your bot", destination: url)
            }
            .font(.caption)
        }
    }

    func channelHelpText(title: String, steps: [String]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.caption)
                .fontWeight(.medium)
            ForEach(Array(steps.enumerated()), id: \.offset) { index, step in
                Text("\(index + 1). \(step)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
    }
}

// MARK: - Hook Helpers

extension SettingsSectionContent {
    func hookLabel(item: Binding<HookItem>) -> some View {
        HStack {
            Text(item.wrappedValue.name.isEmpty ? "Untitled Hook" : item.wrappedValue.name)
            Text(item.wrappedValue.event)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
            Spacer()
            Toggle("", isOn: item.enabled)
                .labelsHidden()
            deleteButton {
                settings.hooks.removeAll { $0.id == item.wrappedValue.id }
            }
        }
    }

    func hookFields(item: Binding<HookItem>) -> some View {
        Group {
            TextField("Name", text: item.name)
            Picker("Event", selection: item.event) {
                ForEach(HookItem.eventTypes, id: \.self) { event in
                    Text(event).tag(event)
                }
            }
            TextField("Command", text: item.command)
                .font(.system(.body, design: .monospaced))
        }
    }
}

// MARK: - MCP Helpers

extension SettingsSectionContent {
    func mcpLabel(item: Binding<McpServerItem>) -> some View {
        HStack {
            Text(item.wrappedValue.name.isEmpty ? "Untitled Server" : item.wrappedValue.name)
            Text(item.wrappedValue.transport.rawValue.uppercased())
                .font(.caption)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(.quaternary)
                .clipShape(Capsule())
            if item.wrappedValue.transport == .stdio, !item.wrappedValue.command.isEmpty {
                Text(item.wrappedValue.command)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Toggle("", isOn: item.enabled)
                .labelsHidden()
            deleteButton {
                settings.mcpServers.removeAll { $0.id == item.wrappedValue.id }
            }
        }
    }

    func mcpFields(item: Binding<McpServerItem>) -> some View {
        Group {
            TextField("Name", text: item.name)
            Picker("Transport", selection: item.transport) {
                ForEach(McpTransport.allCases, id: \.self) { transport in
                    Text(transport.rawValue.uppercased()).tag(transport)
                }
            }
            if item.wrappedValue.transport == .stdio {
                TextField("Command", text: item.command)
                    .font(.system(.body, design: .monospaced))
            } else {
                TextField("URL", text: item.url)
            }
        }
    }
}

// MARK: - Skill Helpers

extension SettingsSectionContent {
    func skillLabel(item: Binding<SkillPackItem>) -> some View {
        HStack {
            Text(
                item.wrappedValue.repoName.isEmpty
                    ? "Untitled Skill Pack" : item.wrappedValue.repoName
            )
            if !item.wrappedValue.source.isEmpty {
                Text(item.wrappedValue.source)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Toggle("", isOn: item.enabled)
                .labelsHidden()
            deleteButton {
                settings.skillPacks.removeAll { $0.id == item.wrappedValue.id }
            }
        }
    }

    func skillFields(item: Binding<SkillPackItem>) -> some View {
        Group {
            TextField("Source (URL or path)", text: item.source)
            TextField("Repository name", text: item.repoName)
            Toggle("Trusted", isOn: item.trusted)
        }
    }
}

// MARK: - Cron Job Helpers

extension SettingsSectionContent {
    func cronJobLabel(item: Binding<CronJobItem>) -> some View {
        HStack {
            Text(item.wrappedValue.name.isEmpty ? "Untitled Job" : item.wrappedValue.name)
            Text(cronScheduleSummary(item.wrappedValue))
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
            Spacer()
            Toggle("", isOn: item.enabled)
                .labelsHidden()
            deleteButton {
                settings.cronJobs.removeAll { $0.id == item.wrappedValue.id }
            }
        }
    }

    func cronJobFields(item: Binding<CronJobItem>) -> some View {
        Group {
            TextField("Name", text: item.name)
            Picker("Schedule type", selection: item.scheduleType) {
                ForEach(CronScheduleType.allCases, id: \.self) { schedType in
                    Text(schedType.rawValue).tag(schedType)
                }
            }
            switch item.wrappedValue.scheduleType {
            case .cron:
                TextField("Cron expression", text: item.cronExpr)
                    .font(.system(.body, design: .monospaced))
            case .interval:
                Stepper(
                    String(
                        format: NSLocalizedString(
                            "Every %d min",
                            comment: "Cron interval minutes"
                        ),
                        item.wrappedValue.intervalMinutes
                    ),
                    value: item.intervalMinutes,
                    in: 1 ... 1440
                )
            case .oneShot:
                EmptyView()
            }
            TextField("Message", text: item.message)
        }
    }

    func cronScheduleSummary(_ item: CronJobItem) -> String {
        switch item.scheduleType {
        case .cron:
            return item.cronExpr.isEmpty ? "no schedule" : item.cronExpr
        case .interval:
            return String(
                format: NSLocalizedString(
                    "every %dm",
                    comment: "Cron summary interval with minute suffix"
                ),
                item.intervalMinutes
            )
        case .oneShot:
            return "one-shot"
        }
    }
}
