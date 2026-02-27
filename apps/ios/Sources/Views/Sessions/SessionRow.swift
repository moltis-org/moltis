import SwiftUI

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
                HStack {
                    Text(session.title)
                        .font(.body)
                        .fontWeight(isActive ? .semibold : .regular)
                        .lineLimit(1)

                    if isActive {
                        Circle()
                            .fill(.blue)
                            .frame(width: 6, height: 6)
                    }
                }

                Text(session.previewText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 4) {
                Text(Self.relativeDateFormatter.localizedString(
                    for: session.updatedAt, relativeTo: Date()
                ))
                .font(.caption2)
                .foregroundStyle(.tertiary)

                if session.messageCount > 0 {
                    Text("\(session.messageCount)")
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.quaternary)
                        .clipShape(Capsule())
                }
            }
        }
        .padding(.vertical, 4)
    }
}
