// swiftlint:disable file_length
import SwiftUI

let shortTimeFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .none
    formatter.timeStyle = .short
    return formatter
}()

/// Format token count with K/M suffixes (matches web UI formatTokens).
func formatTokens(_ count: UInt32) -> String {
    if count >= 1_000_000 {
        return String(format: "%.1fM", Double(count) / 1_000_000)
    }
    if count >= 1_000 {
        return String(format: "%.1fK", Double(count) / 1_000)
    }
    return "\(count)"
}

struct ContentView: View {
    @ObservedObject var chatStore: ChatStore
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        NavigationSplitView {
            SessionsSidebarView(chatStore: chatStore)
        } detail: {
            ChatDetailView(
                chatStore: chatStore,
                settings: settings,
                providerStore: providerStore
            ) {
                openWindow(id: "settings")
            }
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 1080, minHeight: 720)
    }
}

#Preview {
    let settings = AppSettings()
    let providerStore = ProviderStore()
    let store = ChatStore(settings: settings, providerStore: providerStore)
    return ContentView(chatStore: store, settings: settings, providerStore: providerStore)
}

private struct SessionsSidebarView: View {
    @ObservedObject var chatStore: ChatStore
    @State private var selectedKey: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Sessions")
                    .font(.title3.weight(.semibold))
                Spacer()
                Button {
                    chatStore.createSession()
                } label: {
                    Image(systemName: "plus")
                }
                .buttonStyle(.borderless)
                .help("Create a new session")
            }
            .padding(.horizontal, 12)

            ScrollViewReader { proxy in
                List(selection: $selectedKey) {
                    ForEach(chatStore.sessions) { session in
                        SessionRowView(session: session)
                            .tag(Optional(session.key))
                            .id(session.key)
                    }
                }
                .listStyle(.sidebar)
                .onChange(of: selectedKey) { _, newKey in
                    guard let newKey else { return }
                    chatStore.switchSession(key: newKey)
                }
                .onChange(of: chatStore.selectedSessionKey) { _, newKey in
                    selectedKey = newKey
                    // Only scroll for programmatic selection (e.g. new session)
                    if let newKey {
                        proxy.scrollTo(newKey, anchor: .center)
                    }
                }
                .onAppear {
                    selectedKey = chatStore.selectedSessionKey
                }
            }
        }
        .padding(.top, 12)
    }
}

private struct SessionRowView: View {
    let session: ChatSession

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(session.title)
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                Text(shortTimeFormatter.string(from: session.updatedAt))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }

            HStack(spacing: 6) {
                Text(session.previewText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if session.messageCount > 0 {
                    Spacer()
                    Text("\(session.messageCount)")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .padding(.vertical, 4)
    }
}

private struct ChatDetailView: View {
    @ObservedObject var chatStore: ChatStore
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    var openSettings: () -> Void

    private var sessionTitle: String {
        chatStore.selectedSession?.title ?? "No Session Selected"
    }

    private var sessionMessages: [ChatMessage] {
        chatStore.selectedSession?.messages ?? []
    }

    private var canSendMessage: Bool {
        let trimmed = chatStore.draftMessage.trimmingCharacters(
            in: .whitespacesAndNewlines
        )
        return !trimmed.isEmpty && !chatStore.isSending
    }

    var body: some View {
        VStack(spacing: 0) {
            headerBar

            sessionToolbar

            Divider()

            messageList

            if settings.debugEnabled {
                Divider()
                debugPanel
            }

            tokenBar

            Divider()

            inputBar
        }
    }

    private var headerBar: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(sessionTitle)
                    .font(.title3.weight(.semibold))
                HStack(spacing: 6) {
                    Text(chatStore.bridgeSummary)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let modelSummary = providerStore.selectedModelSummary {
                        Text("| \(modelSummary)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Spacer()

            Button {
                openSettings()
            } label: {
                Image(systemName: "gearshape")
            }
            .buttonStyle(.borderless)
            .help("Settings")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    private var sessionToolbar: some View {
        SessionToolbarView(
            providerStore: providerStore,
            settings: settings,
            chatStore: chatStore
        )
    }

    private var debugPanel: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Debug")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)

            if let lastAssistant = sessionMessages.last(where: { $0.role == .assistant }) {
                Text("Last response: \(lastAssistant.provider ?? "?") / \(lastAssistant.model ?? "?")")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                if let inTok = lastAssistant.inputTokens, let outTok = lastAssistant.outputTokens {
                    Text("Tokens: \(inTok) in / \(outTok) out")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                if let ms = lastAssistant.durationMs {
                    Text("Duration: \(ms)ms")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("No assistant messages yet")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }

            Text("Bridge: \(chatStore.bridgeSummary)")
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text("Config: \(settings.environmentConfigDir)")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.5))
    }

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    ForEach(sessionMessages) { message in
                        MessageBubbleView(message: message)
                            .id(message.id)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(16)
            }
            .background {
                VisualEffectBackground(material: .underPageBackground)
            }
            .onAppear {
                if let anchor = chatStore.selectedMessageAnchorID {
                    proxy.scrollTo(anchor, anchor: .bottom)
                }
            }
            .onChange(of: chatStore.selectedMessageAnchorID) { _, anchor in
                guard let anchor else {
                    return
                }
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo(anchor, anchor: .bottom)
                }
            }
        }
    }

    // ── Token bar (matches web UI .token-bar) ──

    private var tokenBarText: String {
        var parts: [String] = []

        if let session = chatStore.selectedSession {
            let totalIn = session.messages.compactMap(\.inputTokens).reduce(0, +)
            let totalOut = session.messages.compactMap(\.outputTokens).reduce(0, +)
            let total = totalIn + totalOut
            parts.append("\(formatTokens(totalIn)) in / \(formatTokens(totalOut)) out")
            parts.append("\(formatTokens(total)) tokens")
        } else {
            parts.append("0 in / 0 out")
            parts.append("0 tokens")
        }

        if let model = providerStore.selectedModelID {
            let provider = settings.llmProvider.isEmpty ? nil : settings.llmProvider
            if let provider {
                parts.append("\(provider) / \(model)")
            } else {
                parts.append(model)
            }
        }

        return parts.joined(separator: " \u{00B7} ")
    }

    private var tokenBar: some View {
        Text(tokenBarText)
            .font(.system(size: 10))
            .foregroundStyle(MoltisTheme.muted)
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(.horizontal, 12)
            .padding(.vertical, 2)
            .background(MoltisTheme.surface)
    }

    private var inputBar: some View {
        HStack(alignment: .center, spacing: 10) {
            ChatInputField(
                text: $chatStore.draftMessage,
                onSend: { chatStore.sendDraftMessage() }
            )
            .frame(minHeight: 44, maxHeight: 44)
            .background(Color(nsColor: .controlBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .stroke(.quaternary, lineWidth: 1)
            }

            if chatStore.isSending {
                Button {
                    chatStore.abortGeneration()
                } label: {
                    Image(systemName: "stop.fill")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.white)
                        .frame(width: 28, height: 28)
                        .background(Color.red)
                        .clipShape(Circle())
                }
                .buttonStyle(.borderless)
                .help("Stop generation")
            } else {
                Button {
                    chatStore.sendDraftMessage()
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 28))
                        .foregroundStyle(canSendMessage ? .blue : .secondary.opacity(0.4))
                }
                .buttonStyle(.borderless)
                .disabled(!canSendMessage)
                .help("Send message")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }
}

private struct SessionToolbarView: View {
    @ObservedObject var providerStore: ProviderStore
    @ObservedObject var settings: AppSettings
    @ObservedObject var chatStore: ChatStore
    @State private var showContextPopover = false
    @State private var showPeekPopover = false

    private var modelOptions: [SearchableOption] {
        providerStore.models.map {
            SearchableOption(id: $0.id, display: $0.displayName, detail: $0.provider)
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            modelPicker
            Divider().frame(height: 16)
            sandboxControls
            Divider().frame(height: 16)
            debugAndContext
            Divider().frame(height: 16)
            peekButton
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var modelPicker: some View {
        Group {
            if !modelOptions.isEmpty {
                SearchablePopoverPicker(
                    label: "",
                    selection: $providerStore.selectedModelID,
                    options: modelOptions
                )
                .controlSize(.small)
            }
        }
    }

    private var sandboxControls: some View {
        Group {
            Toggle("Sandbox", isOn: $settings.sandboxEnabled)
                .toggleStyle(.switch)
                .controlSize(.small)

            if settings.sandboxEnabled {
                TextField("Image", text: $settings.containerImage)
                    .textFieldStyle(.roundedBorder)
                    .controlSize(.small)
                    .frame(maxWidth: 160)
            }
        }
    }

    private var peekButton: some View {
        Button {
            chatStore.peekCurrentSession()
            showPeekPopover.toggle()
        } label: {
            Image(systemName: "eye")
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .help("Peek at activity")
        .popover(isPresented: $showPeekPopover) {
            PeekPopoverView(peekResult: chatStore.peekResult)
        }
    }

    private var debugAndContext: some View {
        Group {
            Toggle("Debug", isOn: $settings.debugEnabled)
                .toggleStyle(.switch)
                .controlSize(.small)

            Button {
                showContextPopover.toggle()
            } label: {
                Image(systemName: "doc.text.magnifyingglass")
            }
            .buttonStyle(.borderless)
            .controlSize(.small)
            .help("Session context")
            .popover(isPresented: $showContextPopover) {
                SessionContextPopover(
                    settings: settings,
                    providerStore: providerStore,
                    session: chatStore.selectedSession
                )
            }
        }
    }
}

private struct SessionContextPopover: View {
    @ObservedObject var settings: AppSettings
    @ObservedObject var providerStore: ProviderStore
    let session: ChatSession?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Session Context")
                .font(.headline)

            LabeledContent(
                "Config Dir",
                value: settings.environmentConfigDir.isEmpty ? "—" : settings.environmentConfigDir
            )
            LabeledContent("Provider", value: settings.llmProvider)
            LabeledContent("Model", value: providerStore.selectedModelID ?? "Default")

            if let session {
                let totalIn = session.messages.compactMap(\.inputTokens).reduce(0, +)
                let totalOut = session.messages.compactMap(\.outputTokens).reduce(0, +)
                LabeledContent("Input Tokens", value: "\(totalIn)")
                LabeledContent("Output Tokens", value: "\(totalOut)")
            }
        }
        .padding()
        .frame(minWidth: 280)
    }
}

private struct PeekPopoverView: View {
    let peekResult: BridgePeekResult?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Activity")
                .font(.headline)

            if let result = peekResult, result.active {
                if let text = result.thinkingText {
                    Text(text)
                        .font(.subheadline)
                        .italic()
                        .foregroundStyle(.orange)
                        .lineLimit(3)
                }
                if let toolCalls = result.toolCalls, !toolCalls.isEmpty {
                    ForEach(toolCalls) { tc in
                        HStack(spacing: 4) {
                            Image(systemName: "gearshape")
                                .foregroundStyle(.secondary)
                            Text(tc.name)
                                .font(.subheadline.monospaced())
                        }
                    }
                }
                if result.thinkingText == nil
                    && (result.toolCalls ?? []).isEmpty {
                    Text("Active (thinking…)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("Idle")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
        .padding()
        .frame(minWidth: 220)
    }
}
