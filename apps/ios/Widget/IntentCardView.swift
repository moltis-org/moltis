import SwiftUI
import WidgetKit

/// A stacked card showing a completed step in the Live Activity.
struct IntentCardView: View {
    let stepText: String
    let isTopCard: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .font(.caption)
                .foregroundStyle(.green)

            Text(stepText)
                .font(.caption)
                .foregroundStyle(.primary)
                .lineLimit(1)

            Spacer()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.ultraThinMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .opacity(isTopCard ? 1.0 : 0.72)
        .scaleEffect(isTopCard ? 1.0 : 0.9)
        .offset(y: isTopCard ? 0 : 10)
    }
}
