import SwiftUI

// MARK: - Channel Helpers

extension SettingsSectionContent {
    func channelLabel(item: Binding<ChannelItem>) -> some View {
        HStack {
            Text(item.wrappedValue.name.isEmpty ? "Untitled Channel" : item.wrappedValue.name)
            Text(item.wrappedValue.channelType)
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
            TextField("Name", text: item.name)
            Picker("Type", selection: item.channelType) {
                ForEach(ChannelItem.channelTypes, id: \.self) { type in
                    Text(type.capitalized).tag(type)
                }
            }
            SecureField("Bot Token", text: item.botToken)
        }
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
                    "Every \(item.wrappedValue.intervalMinutes) min",
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
            return "every \(item.intervalMinutes)m"
        case .oneShot:
            return "one-shot"
        }
    }
}
