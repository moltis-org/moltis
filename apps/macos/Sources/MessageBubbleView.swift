import SwiftUI

struct MessageBubbleView: View {
    let message: ChatMessage

    private var isUser: Bool {
        message.role == .user
    }

    private var bubbleFill: Color {
        switch message.role {
        case .user:
            return .accentColor.opacity(0.2)
        case .assistant:
            return Color(nsColor: .textBackgroundColor)
        case .system:
            return .yellow.opacity(0.18)
        case .error:
            return .red.opacity(0.18)
        }
    }

    private var bubbleBorder: Color {
        switch message.role {
        case .user:
            return .accentColor.opacity(0.4)
        case .assistant:
            return .secondary.opacity(0.25)
        case .system:
            return .yellow.opacity(0.5)
        case .error:
            return .red.opacity(0.5)
        }
    }

    private var metadataText: String? {
        guard message.role == .assistant else { return nil }

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
            parts.append(String(format: "%.1f tok/s", tokPerSec))
        }

        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }

    private func speedColor(for message: ChatMessage) -> Color {
        guard let outTok = message.outputTokens, let ms = message.durationMs, ms > 0 else {
            return .secondary
        }
        let tokPerSec = Double(outTok) / (Double(ms) / 1000.0)
        if tokPerSec >= 25 { return .green }
        if tokPerSec < 10 { return .red }
        return .secondary
    }

    var body: some View {
        HStack {
            if isUser {
                Spacer(minLength: 80)
            }

            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(message.role.title)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text(shortTimeFormatter.string(from: message.createdAt))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }

                Text(message.text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)

                if let metadata = metadataText {
                    Text(metadata)
                        .font(.caption2)
                        .foregroundStyle(speedColor(for: message))
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
            .padding(10)
            .frame(maxWidth: 640, alignment: .leading)
            .background(bubbleFill)
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .stroke(bubbleBorder, lineWidth: 1)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12))

            if !isUser {
                Spacer(minLength: 80)
            }
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
    }
}
