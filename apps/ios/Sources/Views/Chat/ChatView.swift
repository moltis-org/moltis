import SwiftUI

struct ChatView: View {
    @EnvironmentObject var chatStore: ChatStore
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var settingsStore: SettingsStore

    @FocusState private var isInputFocused: Bool

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Message list
                messageList

                // Tool call banner
                if settingsStore.showToolCalls, let toolCall = chatStore.activeToolCalls.first {
                    ToolCallBanner(toolCall: toolCall)
                }

                // Input bar
                inputBar
            }
            .navigationTitle(connectionStore.agentName ?? "Moltis")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    if let model = connectionStore.modelStore.selectedModelId {
                        Text(model)
                            .font(.caption2)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(.quaternary)
                            .clipShape(Capsule())
                    }
                }
            }
        }
    }

    // MARK: - Message list

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(chatStore.messages) { message in
                        MessageBubble(message: message)
                            .id(message.id)
                    }

                    if chatStore.isStreaming && chatStore.messages.last?.isStreaming != true {
                        StreamingIndicator()
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .onChange(of: chatStore.messages.count) {
                if let lastId = chatStore.messages.last?.id {
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(lastId, anchor: .bottom)
                    }
                }
            }
            .onChange(of: chatStore.messages.last?.text) {
                if let lastId = chatStore.messages.last?.id,
                   chatStore.messages.last?.isStreaming == true {
                    proxy.scrollTo(lastId, anchor: .bottom)
                }
            }
        }
    }

    // MARK: - Input bar

    private var inputBar: some View {
        HStack(spacing: 8) {
            TextField("Message...", text: $chatStore.draftMessage, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...5)
                .focused($isInputFocused)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.quaternary.opacity(0.5))
                .clipShape(RoundedRectangle(cornerRadius: 20))
                .onSubmit {
                    Task { await chatStore.sendMessage() }
                }

            if chatStore.isStreaming {
                Button {
                    Task { await chatStore.abortGeneration() }
                } label: {
                    Image(systemName: "stop.circle.fill")
                        .font(.title2)
                        .foregroundStyle(.red)
                }
            } else {
                Button {
                    Task { await chatStore.sendMessage() }
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title2)
                        .foregroundStyle(
                            chatStore.draftMessage.trimmingCharacters(
                                in: .whitespacesAndNewlines
                            ).isEmpty ? .gray : .blue
                        )
                }
                .disabled(
                    chatStore.draftMessage.trimmingCharacters(
                        in: .whitespacesAndNewlines
                    ).isEmpty
                )
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
