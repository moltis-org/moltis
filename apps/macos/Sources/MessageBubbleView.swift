import SwiftUI

// MARK: - Theme colors matching the web UI (base.css)

/// Adaptive colors that match the Moltis web UI dark/light theme.
enum MoltisTheme {
    // ── Bubble backgrounds ──
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

    // ── Semantic colors ──
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
    static let surface = Color(
        light: Color(red: 0xf5 / 255, green: 0xf5 / 255, blue: 0xf5 / 255),
        dark: Color(red: 0x14 / 255, green: 0x16 / 255, blue: 0x1d / 255)
    )
    static let border = Color(
        light: Color(red: 0xe4 / 255, green: 0xe4 / 255, blue: 0xe7 / 255),
        dark: Color(red: 0x27 / 255, green: 0x27 / 255, blue: 0x2a / 255)
    )
}

private extension Color {
    init(light: Color, dark: Color) {
        self.init(nsColor: NSColor(
            name: nil,
            dynamicProvider: { appearance in
                let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
                return NSColor(isDark ? dark : light)
            }
        ))
    }
}

// MARK: - Message bubble view

struct MessageBubbleView: View {
    let message: ChatMessage

    private var isUser: Bool { message.role == .user }

    // ── Footer text (web UI: msg-model-footer) ──

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

    /// Speed color matching web UI thresholds (slow < 10, fast >= 25).
    private func speedColor(for message: ChatMessage) -> Color {
        guard let outTok = message.outputTokens, let ms = message.durationMs, ms > 0 else {
            return MoltisTheme.muted
        }
        let tokPerSec = Double(outTok) / (Double(ms) / 1000.0)
        if tokPerSec >= 25 { return MoltisTheme.ok }
        if tokPerSec < 10 { return MoltisTheme.error }
        return MoltisTheme.muted
    }

    // ── Body ──

    var body: some View {
        switch message.role {
        case .system:
            systemBadge(color: .secondary, icon: nil)
        case .error:
            errorBadge
        case .user, .assistant:
            chatBubble
        }
    }

    // ── System badge (centered capsule, web UI .msg.system) ──

    private func systemBadge(color: Color, icon: String?) -> some View {
        HStack(spacing: 6) {
            if let icon {
                Image(systemName: icon)
                    .font(.caption2)
                    .foregroundStyle(color)
            }
            Text(message.text)
                .font(.caption)
                .foregroundStyle(color)
            Text(shortTimeFormatter.string(from: message.createdAt))
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(.quaternary.opacity(0.5), in: Capsule())
        .frame(maxWidth: .infinity, alignment: .center)
        .padding(.vertical, 4)
    }

    // ── Error badge (centered, matching web UI .msg.error — center-aligned,
    //    error-colored text, no bubble background) ──

    private var errorBadge: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(MoltisTheme.error)
            Text(message.text)
                .font(.caption)
                .foregroundStyle(MoltisTheme.error)
            Text(shortTimeFormatter.string(from: message.createdAt))
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(MoltisTheme.error.opacity(0.08), in: Capsule())
        .frame(maxWidth: .infinity, alignment: .center)
        .padding(.vertical, 4)
    }

    // ── Chat bubble (user = right, assistant = left) ──

    private var chatBubble: some View {
        HStack {
            if isUser { Spacer(minLength: 80) }

            VStack(alignment: .leading, spacing: 6) {
                // Role + time header
                HStack {
                    Text(message.role.title)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(shortTimeFormatter.string(from: message.createdAt))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }

                // Message content (streaming states)
                if message.isStreaming && message.text.isEmpty {
                    ThinkingDotsView()
                        .frame(height: 20)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else if message.isStreaming {
                    HStack(spacing: 0) {
                        Text(message.text)
                            .textSelection(.enabled)
                        StreamingCursorView()
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    Text(message.text)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                // Image attachments (web UI: msg-image-row + msg-image-thumb)
                if !message.attachments.isEmpty {
                    imageAttachmentRow
                }

                // Model footer (web UI: msg-model-footer)
                if let metadata = metadataText {
                    Text(metadata)
                        .font(.caption2)
                        .foregroundStyle(speedColor(for: message))
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
            .padding(10)
            .frame(maxWidth: 640, alignment: .leading)
            .background(isUser ? MoltisTheme.userBg : MoltisTheme.assistantBg)
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .stroke(
                        isUser ? MoltisTheme.userBorder : MoltisTheme.assistantBorder,
                        lineWidth: 1
                    )
            }
            .clipShape(RoundedRectangle(cornerRadius: 12))

            if !isUser { Spacer(minLength: 80) }
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
    }

    // ── Image attachment thumbnails ──

    private var imageAttachmentRow: some View {
        HStack(spacing: 6) {
            ForEach(message.attachments) { attachment in
                switch attachment.kind {
                case let .image(image):
                    Image(nsImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(maxWidth: 120, maxHeight: 90)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                        .overlay {
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(MoltisTheme.border, lineWidth: 1)
                        }
                case .file:
                    HStack(spacing: 4) {
                        Image(systemName: "doc")
                            .font(.caption)
                        Text(attachment.name)
                            .font(.caption2)
                            .lineLimit(1)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(MoltisTheme.surface)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay {
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(MoltisTheme.border, lineWidth: 1)
                    }
                }
            }
        }
        .padding(.top, 4)
    }
}
