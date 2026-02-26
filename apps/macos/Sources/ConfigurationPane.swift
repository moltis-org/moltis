import AppKit
import SwiftUI

// MARK: - Configuration Pane (full-screen TOML editor)

struct ConfigurationPane: View {
    @ObservedObject var settings: AppSettings
    @State private var loadError: String?
    @State private var saveStatus: SaveStatus?
    @State private var hasUnsavedChanges = false

    private enum SaveStatus {
        case saved
        case error(String)
    }

    private var configFilePath: String {
        let dir = settings.environmentConfigDir
        guard !dir.isEmpty else { return "" }
        let path = (dir as NSString).appendingPathComponent("moltis.toml")
        return path
    }

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            editor
        }
        .onAppear { loadFromDisk() }
    }

    // MARK: - Toolbar

    private var toolbar: some View {
        HStack(spacing: 10) {
            // File path
            if !configFilePath.isEmpty {
                HStack(spacing: 4) {
                    Image(systemName: "doc.text")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    Text(configFilePath)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            } else {
                Text("Config directory not set")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
            }

            Spacer()

            // Status indicator
            statusView

            // View in Finder
            Button {
                revealInFinder()
            } label: {
                Label("Reveal in Finder", systemImage: "folder")
            }
            .controlSize(.small)
            .disabled(configFilePath.isEmpty)

            // Reload
            Button {
                loadFromDisk()
            } label: {
                Label("Reload", systemImage: "arrow.clockwise")
            }
            .controlSize(.small)
            .disabled(configFilePath.isEmpty)

            // Save
            Button {
                saveToDisk()
            } label: {
                Label("Save", systemImage: "square.and.arrow.down")
            }
            .controlSize(.small)
            .disabled(configFilePath.isEmpty || !hasUnsavedChanges)
            .keyboardShortcut("s", modifiers: .command)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    @ViewBuilder
    private var statusView: some View {
        if let error = loadError {
            HStack(spacing: 4) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 10))
                    .foregroundStyle(.orange)
                Text(error)
                    .font(.system(size: 10))
                    .foregroundStyle(.orange)
                    .lineLimit(1)
            }
        } else if let status = saveStatus {
            switch status {
            case .saved:
                HStack(spacing: 4) {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(.green)
                    Text("Saved")
                        .font(.system(size: 10))
                        .foregroundStyle(.green)
                }
            case let .error(msg):
                HStack(spacing: 4) {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(.red)
                    Text(msg)
                        .font(.system(size: 10))
                        .foregroundStyle(.red)
                        .lineLimit(1)
                }
            }
        } else if hasUnsavedChanges {
            Text("Modified")
                .font(.system(size: 10))
                .foregroundStyle(.orange)
        }
    }

    // MARK: - Editor

    private var editor: some View {
        ConfigTextEditor(text: $settings.configurationToml)
            .onChange(of: settings.configurationToml) { _, _ in
                hasUnsavedChanges = true
                saveStatus = nil
            }
    }

    // MARK: - File I/O

    private func loadFromDisk() {
        let path = configFilePath
        guard !path.isEmpty else {
            loadError = "No config directory"
            return
        }

        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            do {
                let content = try String(contentsOf: url, encoding: .utf8)
                settings.configurationToml = content
                loadError = nil
                hasUnsavedChanges = false
                saveStatus = nil
            } catch {
                loadError = "Read failed: \(error.localizedDescription)"
            }
        } else {
            settings.configurationToml = ""
            loadError = nil
            hasUnsavedChanges = false
        }
    }

    private func saveToDisk() {
        let path = configFilePath
        guard !path.isEmpty else { return }

        let url = URL(fileURLWithPath: path)

        // Ensure directory exists
        let dir = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(
            at: dir, withIntermediateDirectories: true
        )

        do {
            try settings.configurationToml.write(
                to: url, atomically: true, encoding: .utf8
            )
            hasUnsavedChanges = false
            saveStatus = .saved

            // Auto-clear "Saved" after 3 seconds
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                if case .saved = self.saveStatus {
                    self.saveStatus = nil
                }
            }
        } catch {
            saveStatus = .error(error.localizedDescription)
        }
    }

    private func revealInFinder() {
        let path = configFilePath
        guard !path.isEmpty else { return }

        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            // Reveal the directory instead
            let dir = url.deletingLastPathComponent()
            NSWorkspace.shared.open(dir)
        }
    }
}

// MARK: - Full-height monospace text editor (NSTextView)

private struct ConfigTextEditor: NSViewRepresentable {
    @Binding var text: String

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor
        scrollView.wantsLayer = true
        scrollView.layer?.cornerRadius = 6
        scrollView.layer?.masksToBounds = true
        scrollView.layer?.borderWidth = 1
        scrollView.layer?.borderColor = NSColor.separatorColor.cgColor

        let textView = NSTextView()
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.allowsUndo = true
        textView.usesFindPanel = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.font = .monospacedSystemFont(ofSize: 13, weight: .regular)
        textView.textColor = .labelColor
        textView.backgroundColor = .textBackgroundColor
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = true
        textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.autoresizingMask = [.width, .height]
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )

        // 1.3x line height for comfortable reading (CotEditor default)
        let paragraphStyle = NSMutableParagraphStyle()
        paragraphStyle.lineHeightMultiple = 1.3
        textView.defaultParagraphStyle = paragraphStyle
        textView.typingAttributes[.paragraphStyle] = paragraphStyle

        scrollView.documentView = textView
        context.coordinator.textView = textView

        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView else {
            return
        }
        if textView.string != text {
            textView.string = text
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        @Binding var text: String
        weak var textView: NSTextView?

        init(text: Binding<String>) {
            _text = text
        }

        func textDidChange(_ notification: Notification) {
            guard let tv = notification.object as? NSTextView else {
                return
            }
            text = tv.string
        }
    }
}
