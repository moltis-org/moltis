import SwiftUI

struct ChatView: View {
    @EnvironmentObject var chatStore: ChatStore
    @EnvironmentObject var connectionStore: ConnectionStore
    @EnvironmentObject var settingsStore: SettingsStore
    @EnvironmentObject var locationSharingStore: LocationSharingStore
    @EnvironmentObject var authManager: AuthManager

    @FocusState private var isInputFocused: Bool
    @State private var isSessionDrawerOpen = false
    @State private var sessionSearchText = ""
    @State private var searchDebounceTask: Task<Void, Never>?
    @State private var showModelPicker = false
    @State private var showSettings = false

    var body: some View {
        NavigationStack {
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    VStack(spacing: 0) {
                        topBar

                        // Message list
                        messageList

                        // Tool call banner
                        if settingsStore.showToolCalls, let toolCall = chatStore.activeToolCalls.first {
                            ToolCallBanner(toolCall: toolCall)
                        }

                        // Thinking text
                        if let thinkingText = chatStore.currentThinkingText,
                           chatStore.isStreaming {
                            Text(thinkingText)
                                .font(.caption)
                                .italic()
                                .foregroundStyle(.orange)
                                .lineLimit(2)
                                .padding(.horizontal, 16)
                                .padding(.vertical, 4)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }

                        // Input bar
                        inputBar
                    }

                    if isSessionDrawerOpen {
                        Color.black.opacity(0.38)
                            .ignoresSafeArea()
                            .contentShape(Rectangle())
                            .onTapGesture {
                                closeSessionDrawer()
                            }

                        sessionsDrawer(width: min(geometry.size.width * 0.84, 340))
                            .transition(.move(edge: .leading).combined(with: .opacity))
                    }
                }
            }
            .toolbar(.hidden, for: .navigationBar)
            .animation(.easeInOut(duration: 0.22), value: isSessionDrawerOpen)
            .sheet(isPresented: $showModelPicker) {
                NavigationStack {
                    ModelPickerView()
                        .environmentObject(connectionStore.modelStore)
                }
            }
            .sheet(isPresented: $showSettings) {
                SettingsView()
                    .environmentObject(connectionStore)
                    .environmentObject(settingsStore)
                    .environmentObject(locationSharingStore)
                    .environmentObject(authManager)
            }
        }
    }

    private var topBar: some View {
        HStack(spacing: 10) {
            circleTopButton(systemImage: "line.3.horizontal") {
                isInputFocused = false
                if connectionStore.sessionStore.sessions.isEmpty {
                    Task { await connectionStore.sessionStore.loadSessions() }
                }
                withAnimation {
                    isSessionDrawerOpen = true
                }
            }

            Button {
                isInputFocused = false
                connectionStore.modelStore.currentSessionKey = chatStore.currentSessionKey
                showModelPicker = true
            } label: {
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(modelSelection.model)
                            .font(.subheadline.weight(.semibold))
                            .lineLimit(1)
                            .foregroundStyle(.primary)

                        Text(modelSelection.provider)
                            .font(.caption)
                            .lineLimit(1)
                            .foregroundStyle(.secondary)
                    }

                    Spacer(minLength: 8)

                    Image(systemName: "chevron.down")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity)
                .background(.ultraThinMaterial)
                .clipShape(Capsule())
            }
            .buttonStyle(.plain)

            circleTopButton(systemImage: "gearshape") {
                isInputFocused = false
                showSettings = true
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 6)
    }

    private func circleTopButton(systemImage: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.headline)
                .foregroundStyle(.primary)
                .frame(width: 40, height: 40)
                .background(.ultraThinMaterial)
                .clipShape(Circle())
        }
        .buttonStyle(.plain)
    }

    private var modelSelection: (model: String, provider: String) {
        if let selected = selectedModel {
            return (selected.name, selected.provider)
        }
        if let selectedId = connectionStore.modelStore.selectedModelId {
            let parts = selectedId.components(separatedBy: "::")
            if parts.count >= 2 {
                let provider = parts[0]
                let model = parts.dropFirst().joined(separator: "::")
                return (model, provider)
            }
            return (selectedId, "Current model")
        }
        return ("Choose model", "Provider and model")
    }

    private var selectedModel: ModelInfo? {
        guard let id = connectionStore.modelStore.selectedModelId else { return nil }
        return connectionStore.modelStore.models.first { $0.id == id }
    }

    private var visibleSessions: [ChatSession] {
        connectionStore.sessionStore.sessions.filter { !$0.archived }
    }

    private func closeSessionDrawer() {
        withAnimation {
            isSessionDrawerOpen = false
        }
        sessionSearchText = ""
    }

    private func selectSession(_ session: ChatSession) {
        Task {
            await chatStore.switchSession(key: session.key)
            closeSessionDrawer()
        }
    }

    private func createSession() {
        Task {
            if let key = await connectionStore.sessionStore.createSession() {
                await chatStore.switchSession(key: key)
                await connectionStore.sessionStore.loadSessions()
            }
            closeSessionDrawer()
        }
    }

    private func deleteSession(_ session: ChatSession) {
        Task { await connectionStore.sessionStore.deleteSession(key: session.key) }
    }

    private func sessionsDrawer(width: CGFloat) -> some View {
        VStack(spacing: 0) {
            HStack {
                Text("Sessions")
                    .font(.headline)
                Spacer()
                Button {
                    createSession()
                } label: {
                    Image(systemName: "plus")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 32, height: 32)
                        .background(.quaternary.opacity(0.5))
                        .clipShape(Circle())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.top, 18)
            .padding(.bottom, 10)

            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                TextField("Search sessions", text: $sessionSearchText)
                    .font(.subheadline)
                    .textInputAutocapitalization(.never)
                    .disableAutocorrection(true)
                if !sessionSearchText.isEmpty {
                    Button {
                        sessionSearchText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(Color(uiColor: .systemGray5))
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .padding(.horizontal, 12)
            .padding(.bottom, 12)
            .onChange(of: sessionSearchText) { _, query in
                searchDebounceTask?.cancel()
                let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
                searchDebounceTask = Task {
                    if !trimmed.isEmpty {
                        try? await Task.sleep(for: .milliseconds(300))
                        guard !Task.isCancelled else { return }
                    }
                    await connectionStore.sessionStore.searchSessions(query: trimmed)
                }
            }

            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(visibleSessions) { session in
                        Button {
                            selectSession(session)
                        } label: {
                            SessionRow(
                                session: session,
                                isActive: session.key == chatStore.currentSessionKey
                            )
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .contextMenu {
                            Button(role: .destructive) {
                                deleteSession(session)
                            } label: {
                                Label("Delete", systemImage: "trash")
                            }
                        }
                    }
                }
                .padding(.horizontal, 8)
            }

            Spacer(minLength: 0)
        }
        .frame(width: width)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.regularMaterial)
        .shadow(color: .black.opacity(0.2), radius: 20, x: 0, y: 8)
        .onAppear {
            Task { await connectionStore.sessionStore.loadSessions() }
        }
    }

    // MARK: - Message list

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(chatStore.messages) { message in
                        MessageBubble(
                            message: message,
                            agentName: connectionStore.agentName
                        )
                        .id(message.id)
                    }

                    if chatStore.isStreaming && chatStore.messages.last?.isStreaming != true {
                        StreamingIndicator()
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .scrollDismissesKeyboard(.interactively)
            .simultaneousGesture(
                TapGesture().onEnded {
                    isInputFocused = false
                }
            )
            .onChange(of: chatStore.messages.count) {
                if let lastId = chatStore.messages.last?.id {
                    proxy.scrollTo(lastId, anchor: .bottom)
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
        let controlSize: CGFloat = 46
        let trimmedDraft = chatStore.draftMessage.trimmingCharacters(
            in: .whitespacesAndNewlines
        )
        let canSend = !trimmedDraft.isEmpty

        return HStack(spacing: 8) {
            TextField("Message...", text: $chatStore.draftMessage, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...5)
                .focused($isInputFocused)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(minHeight: controlSize)
                .background(.quaternary.opacity(0.45))
                .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
                .onSubmit {
                    Task { await chatStore.sendMessage() }
                }

            if chatStore.isStreaming {
                Button {
                    Task { await chatStore.abortGeneration() }
                } label: {
                    Image(systemName: "stop.fill")
                        .font(.headline.weight(.semibold))
                        .foregroundStyle(.white)
                        .frame(width: controlSize, height: controlSize)
                        .background(Color.red)
                        .clipShape(Circle())
                }
                .buttonStyle(.plain)
            } else {
                Button {
                    Task { await chatStore.sendMessage() }
                } label: {
                    Image(systemName: "arrow.up")
                        .font(.headline.weight(.semibold))
                        .foregroundStyle(canSend ? .white : .secondary)
                        .frame(width: controlSize, height: controlSize)
                        .background(canSend ? Color.blue : Color(uiColor: .systemGray5))
                        .clipShape(Circle())
                }
                .buttonStyle(.plain)
                .disabled(!canSend)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
