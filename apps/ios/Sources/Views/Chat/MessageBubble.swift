import SwiftUI

// MARK: - Theme (matching macOS app / web UI)

enum MoltisTheme {
    static let userBg = Color(
        light: Color(red: 0xf0 / 255, green: 0xf0 / 255, blue: 0xf0 / 255),
        dark: Color(red: 0x1e / 255, green: 0x20 / 255, blue: 0x28 / 255)
    )
    static let userBorder = Color(
        light: Color(red: 0xd4 / 255, green: 0xd4 / 255, blue: 0xd8 / 255),
        dark: Color(red: 0x2a / 255, green: 0x2d / 255, blue: 0x36 / 255)
    )
    static let assistantBg = Color(
        light: Color(red: 0xf5 / 255, green: 0xf5 / 255, blue: 0xf5 / 255),
        dark: Color(red: 0x1a / 255, green: 0x1d / 255, blue: 0x25 / 255)
    )
    static let assistantBorder = Color(
        light: Color(red: 0xe4 / 255, green: 0xe4 / 255, blue: 0xe7 / 255),
        dark: Color(red: 0x27 / 255, green: 0x27 / 255, blue: 0x2a / 255)
    )
    static let error = Color(
        light: Color(red: 0xdc / 255, green: 0x26 / 255, blue: 0x26 / 255),
        dark: Color(red: 0xef / 255, green: 0x44 / 255, blue: 0x44 / 255)
    )
    static let ok = Color(
        light: Color(red: 0x16 / 255, green: 0xa3 / 255, blue: 0x4a / 255),
        dark: Color(red: 0x22 / 255, green: 0xc5 / 255, blue: 0x5e / 255)
    )
    static let muted = Color(
        light: Color(red: 0x71 / 255, green: 0x71 / 255, blue: 0x7a / 255),
        dark: Color(red: 0x71 / 255, green: 0x71 / 255, blue: 0x7a / 255)
    )
}

private extension Color {
    init(light: Color, dark: Color) {
        self.init(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark ? UIColor(dark) : UIColor(light)
        })
    }
}

// MARK: - Time formatter

private let shortTimeFormatter: DateFormatter = {
    let fmt = DateFormatter()
    fmt.dateStyle = .none
    fmt.timeStyle = .short
    return fmt
}()

// MARK: - Message bubble

struct MessageBubble: View {
    let message: ChatMessage
    var agentName: String?

    private var isUser: Bool { message.role == .user }

    private var roleLabel: String {
        if isUser { return message.role.title }
        if let name = agentName, !name.isEmpty { return name }
        return message.role.title
    }

    private var metadataText: String? {
        guard message.role == .assistant, !message.isStreaming else { return nil }
        var parts: [String] = []
        if let provider = message.provider {
            if let model = message.model {
                parts.append("\(provider) / \(model)")
            } else {
                parts.append(provider)
            }
        }
        if let inTok = message.inputTokens, let outTok = message.outputTokens {
            parts.append("\(inTok) in / \(outTok) out")
        }
        if let outTok = message.outputTokens, let ms = message.durationMs, ms > 0 {
            let tokPerSec = Double(outTok) / (Double(ms) / 1000.0)
            if tokPerSec >= 100 {
                parts.append(String(format: "%.0f tok/s", tokPerSec))
            } else if tokPerSec >= 10 {
                parts.append(String(format: "%.1f tok/s", tokPerSec))
            } else {
                parts.append(String(format: "%.2f tok/s", tokPerSec))
            }
        }
        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }

    private func speedColor(for message: ChatMessage) -> Color {
        guard let outTok = message.outputTokens, let ms = message.durationMs, ms > 0 else {
            return MoltisTheme.muted
        }
        let tokPerSec = Double(outTok) / (Double(ms) / 1000.0)
        if tokPerSec >= 25 { return MoltisTheme.ok }
        if tokPerSec < 10 { return MoltisTheme.error }
        return MoltisTheme.muted
    }

    var body: some View {
        switch message.role {
        case .system:
            systemBadge
        case .error:
            errorBadge
        case .user, .assistant:
            chatBubble
        }
    }

    // MARK: - System badge

    private var systemBadge: some View {
        Text(message.text)
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.vertical, 4)
    }

    // MARK: - Error badge

    private var errorBadge: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(MoltisTheme.error)
            Text(message.text)
                .font(.caption)
                .foregroundStyle(MoltisTheme.error)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(MoltisTheme.error.opacity(0.08), in: Capsule())
        .frame(maxWidth: .infinity, alignment: .center)
        .padding(.vertical, 4)
    }

    // MARK: - Chat bubble

    private var chatBubble: some View {
        HStack {
            if isUser { Spacer(minLength: 60) }

            VStack(alignment: .leading, spacing: 6) {
                // Role + time header
                HStack {
                    Text(roleLabel)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(shortTimeFormatter.string(from: message.createdAt))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }

                // Message text
                if message.isStreaming && message.text.isEmpty {
                    StreamingIndicator()
                        .frame(height: 20)
                } else {
                    Text(message.text)
                        .font(.body)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                // Metadata footer
                if let metadata = metadataText {
                    Text(metadata)
                        .font(.caption2)
                        .foregroundStyle(speedColor(for: message))
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
            .padding(10)
            .background(isUser ? MoltisTheme.userBg : MoltisTheme.assistantBg)
            .overlay {
                RoundedRectangle(cornerRadius: 14)
                    .stroke(
                        isUser ? MoltisTheme.userBorder : MoltisTheme.assistantBorder,
                        lineWidth: 1
                    )
            }
            .clipShape(RoundedRectangle(cornerRadius: 14))

            if !isUser { Spacer(minLength: 60) }
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
    }
}
