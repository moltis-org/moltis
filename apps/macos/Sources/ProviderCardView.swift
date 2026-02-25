import SwiftUI

// MARK: - Shared card styling

private struct SelectableCardStyle: ViewModifier {
    let isSelected: Bool

    func body(content: Content) -> some View {
        content
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(isSelected ? Color.accentColor.opacity(0.12) : Color(nsColor: .controlBackgroundColor))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .stroke(
                        isSelected ? Color.accentColor : .secondary.opacity(0.2),
                        lineWidth: isSelected ? 2 : 1
                    )
            }
            .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

// MARK: - Provider card

struct ProviderCardView: View {
    let provider: BridgeKnownProvider
    let isConfigured: Bool
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(provider.displayName)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer()
                    if isConfigured {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                            .font(.body)
                    }
                }

                HStack(spacing: 6) {
                    Text(provider.authType)
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.secondary.opacity(0.15))
                        .clipShape(Capsule())

                    if provider.keyOptional {
                        Text("key optional")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .modifier(SelectableCardStyle(isSelected: isSelected))
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Voice provider card

struct VoiceProviderCardView: View {
    let provider: VoiceProvider
    let isSelected: Bool
    let onSelect: () -> Void

    var body: some View {
        Button(action: onSelect) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(provider.displayName)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer()
                    if !provider.requiresApiKey {
                        Image(systemName: "desktopcomputer")
                            .foregroundStyle(.secondary)
                            .font(.body)
                    }
                }

                HStack(spacing: 6) {
                    Text(provider.requiresApiKey ? "API key" : "Local")
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.secondary.opacity(0.15))
                        .clipShape(Capsule())
                }
            }
            .modifier(SelectableCardStyle(isSelected: isSelected))
        }
        .buttonStyle(.plain)
    }
}
