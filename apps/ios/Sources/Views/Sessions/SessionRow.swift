import SwiftUI
import UIKit

struct SessionRow: View {
    let session: ChatSession
    let isActive: Bool

    private static let relativeDateFormatter: RelativeDateTimeFormatter = {
        let fmt = RelativeDateTimeFormatter()
        fmt.unitsStyle = .abbreviated
        return fmt
    }()

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(session.title)
                    .font(.body)
                    .fontWeight(isActive ? .semibold : .regular)
                    .foregroundStyle(isActive ? Color.white : Color(uiColor: .label))
                    .lineLimit(1)

                if let preview = session.preview, !preview.isEmpty {
                    Text(preview)
                        .font(.caption)
                        .foregroundStyle(isActive ? Color.white.opacity(0.7) : Color(uiColor: .secondaryLabel))
                        .lineLimit(2)
                } else if let model = session.model {
                    Text(model)
                        .font(.caption)
                        .foregroundStyle(isActive ? Color.white.opacity(0.6) : Color(uiColor: .tertiaryLabel))
                        .lineLimit(1)
                }
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                Text(Self.relativeDateFormatter.localizedString(
                    for: session.updatedAt, relativeTo: Date()
                ))
                .font(.caption2)
                .foregroundStyle(isActive ? Color.white.opacity(0.6) : Color(uiColor: .tertiaryLabel))

                if session.messageCount > 0 {
                    let hasUnread = !isActive && session.unreadCount > 0
                    Text("\(session.messageCount)")
                        .font(.caption2)
                        .fontWeight(hasUnread ? .semibold : .medium)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .foregroundStyle(
                            isActive ? Color.white : (hasUnread ? Color.white : Color(uiColor: .secondaryLabel))
                        )
                        .background(
                            isActive
                                ? Color.white.opacity(0.25)
                                : (hasUnread ? Color.blue : Color(uiColor: .tertiarySystemFill))
                        )
                        .clipShape(Capsule())
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(isActive ? Color.blue : Color.clear)
        )
    }
}
#if DEBUG
private struct _PreviewChatSession: Identifiable {
    let id = UUID()
    let title: String
    let preview: String?
    let model: String?
    let updatedAt: Date
    let messageCount: Int
}

extension _PreviewChatSession {
    static let sample = _PreviewChatSession(
        title: "Sample Session",
        preview: "This is a preview of the latest message in the chat session.",
        model: "GPT-4o Mini",
        updatedAt: Date().addingTimeInterval(-3600),
        messageCount: 3
    )
}

#Preview("SessionRow") {
    VStack(spacing: 12) {
        SessionRow(
            session: ChatSession(
                title: _PreviewChatSession.sample.title,
                preview: _PreviewChatSession.sample.preview,
                updatedAt: _PreviewChatSession.sample.updatedAt,
                messageCount: _PreviewChatSession.sample.messageCount,
                model: _PreviewChatSession.sample.model
            ),
            isActive: true
        )
        SessionRow(
            session: ChatSession(
                title: "Another Session With Longer Title That Truncates",
                updatedAt: Date().addingTimeInterval(-86400 * 3),
                model: "Claude 3.5 Sonnet"
            ),
            isActive: false
        )
    }
    .padding()
}
#endif

