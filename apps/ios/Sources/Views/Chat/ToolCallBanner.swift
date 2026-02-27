import SwiftUI

struct ToolCallBanner: View {
    let toolCall: ToolCallInfo

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: toolCall.icon)
                .font(.caption)
                .foregroundStyle(.blue)

            Text(toolCall.displayLabel)
                .font(.caption)
                .lineLimit(1)
                .foregroundStyle(.primary)

            Spacer()

            ProgressView()
                .controlSize(.small)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
