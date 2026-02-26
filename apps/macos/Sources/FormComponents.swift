import AppKit
import SwiftUI

// MARK: - VisualEffectBackground

struct VisualEffectBackground: NSViewRepresentable {
    let material: NSVisualEffectView.Material
    let blendingMode: NSVisualEffectView.BlendingMode

    init(
        material: NSVisualEffectView.Material = .underPageBackground,
        blendingMode: NSVisualEffectView.BlendingMode = .behindWindow
    ) {
        self.material = material
        self.blendingMode = blendingMode
    }

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = blendingMode
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ nsView: NSVisualEffectView, context: Context) {
        nsView.material = material
        nsView.blendingMode = blendingMode
    }
}

// MARK: - ChatInputField (Enter sends, Cmd+Enter adds newline)

struct ChatInputField: NSViewRepresentable {
    @Binding var text: String
    var onSend: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text, onSend: onSend)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = false

        let textView = NSTextView()
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.allowsUndo = true
        textView.font = .systemFont(ofSize: NSFont.systemFontSize)
        textView.textColor = .labelColor
        textView.backgroundColor = .controlBackgroundColor
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.textContainerInset = NSSize(width: 6, height: 6)
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true

        scrollView.documentView = textView
        context.coordinator.textView = textView

        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView else { return }
        if textView.string != text {
            textView.string = text
        }
        context.coordinator.onSend = onSend
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        @Binding var text: String
        var onSend: () -> Void
        weak var textView: NSTextView?

        init(text: Binding<String>, onSend: @escaping () -> Void) {
            _text = text
            self.onSend = onSend
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            text = textView.string
        }

        func textView(
            _ textView: NSTextView,
            doCommandBy commandSelector: Selector
        ) -> Bool {
            if commandSelector == #selector(NSResponder.insertNewline(_:)) {
                let flags = NSApp.currentEvent?.modifierFlags ?? []
                if flags.contains(.command) {
                    textView.insertNewlineIgnoringFieldEditor(nil)
                    return true
                }
                onSend()
                return true
            }
            return false
        }
    }
}

// MARK: - SettingsEmptyState

struct SettingsEmptyState: View {
    let icon: String
    let title: String
    let subtitle: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon)
                .font(.system(size: 28))
                .foregroundStyle(.tertiary)
            Text(title)
                .font(.headline)
                .foregroundStyle(.tertiary)
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(.quaternary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }
}

// MARK: - SearchablePopoverPicker (popover with search + list)

struct SearchableOption: Identifiable {
    let id: String
    let display: String
    let detail: String?
}

struct SearchablePopoverPicker: View {
    let label: String
    @Binding var selection: String?
    let options: [SearchableOption]

    init(
        label: String,
        selection: Binding<String?>,
        options: [SearchableOption]
    ) {
        self.label = label
        self._selection = selection
        self.options = options
    }

    init(
        label: String,
        selection: Binding<String?>,
        options: [(id: String, display: String)]
    ) {
        self.label = label
        self._selection = selection
        self.options = options.map { option in
            SearchableOption(id: option.id, display: option.display, detail: nil)
        }
    }

    @State private var isPresented = false
    @State private var searchText = ""

    private var filteredOptions: [SearchableOption] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if query.isEmpty { return options }
        return options.filter {
            $0.display.lowercased().contains(query)
                || ($0.detail?.lowercased().contains(query) ?? false)
        }
    }

    private var selectedDisplay: String {
        if let sel = selection, let match = options.first(where: { $0.id == sel }) {
            return match.display
        }
        return "Default"
    }

    var body: some View {
        LabeledContent(label) {
            Button {
                isPresented.toggle()
            } label: {
                HStack {
                    Text(selectedDisplay)
                        .lineLimit(1)
                    Spacer()
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: 260)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(.background, in: RoundedRectangle(cornerRadius: 6))
                .overlay {
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(.quaternary)
                }
            }
            .buttonStyle(.plain)
            .popover(isPresented: $isPresented) {
                VStack(spacing: 0) {
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass")
                            .foregroundStyle(.tertiary)
                            .font(.system(size: 12))
                        TextField("Search models...", text: $searchText)
                            .textFieldStyle(.plain)
                            .font(.system(size: 13))
                    }
                    .padding(8)

                    Divider()

                    List(selection: $selection) {
                        Text("Default").tag(nil as String?)

                        ForEach(filteredOptions, id: \.id) { option in
                            HStack {
                                Text(option.display)
                                    .lineLimit(1)
                                Spacer()
                                if let detail = option.detail {
                                    Text(detail)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .tag(Optional(option.id))
                        }
                    }
                    .listStyle(.plain)
                    .frame(width: 360, height: 280)
                }
                .onChange(of: selection) { _, _ in
                    isPresented = false
                    searchText = ""
                }
            }
        }
    }
}

// MARK: - SearchableComboBoxPicker (native NSComboBox)

struct SearchableComboBoxPicker: NSViewRepresentable {
    let label: String
    @Binding var selection: String?
    let options: [(id: String, display: String)]

    func makeCoordinator() -> Coordinator {
        Coordinator(selection: $selection, options: options)
    }

    func makeNSView(context: Context) -> NSView {
        let stack = NSStackView()
        stack.orientation = .horizontal
        stack.spacing = 8
        stack.alignment = .firstBaseline

        let labelView = NSTextField(labelWithString: label)
        labelView.font = .systemFont(ofSize: NSFont.systemFontSize)
        labelView.setContentHuggingPriority(.required, for: .horizontal)

        let combo = NSComboBox()
        combo.isEditable = true
        combo.completes = true
        combo.usesDataSource = false
        combo.hasVerticalScroller = true
        combo.numberOfVisibleItems = 12
        combo.font = .systemFont(ofSize: NSFont.systemFontSize)

        combo.addItems(withObjectValues: ["Default"] + options.map(\.display))

        if let sel = selection, let match = options.first(where: { $0.id == sel }) {
            combo.stringValue = match.display
        } else {
            combo.stringValue = "Default"
        }

        combo.delegate = context.coordinator
        combo.target = context.coordinator
        combo.action = #selector(Coordinator.comboBoxSelectionChanged(_:))
        context.coordinator.comboBox = combo

        stack.addArrangedSubview(labelView)
        stack.addArrangedSubview(combo)

        return stack
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.options = options
        context.coordinator.selectionBinding = $selection
    }

    final class Coordinator: NSObject, NSComboBoxDelegate {
        var selectionBinding: Binding<String?>
        var selection: String? {
            get { selectionBinding.wrappedValue }
            set { selectionBinding.wrappedValue = newValue }
        }
        var options: [(id: String, display: String)]
        weak var comboBox: NSComboBox?

        init(selection: Binding<String?>, options: [(id: String, display: String)]) {
            self.selectionBinding = selection
            self.options = options
        }

        @objc func comboBoxSelectionChanged(_ sender: NSComboBox) {
            resolve(sender.stringValue)
        }

        func comboBoxSelectionDidChange(_ notification: Notification) {
            guard let combo = notification.object as? NSComboBox else { return }
            let idx = combo.indexOfSelectedItem
            if idx == 0 {
                selection = nil
            } else if idx > 0, idx - 1 < options.count {
                selection = options[idx - 1].id
            }
        }

        func controlTextDidEndEditing(_ obj: Notification) {
            guard let combo = obj.object as? NSComboBox else { return }
            resolve(combo.stringValue)
        }

        private func resolve(_ text: String) {
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty || trimmed.lowercased() == "default" {
                selection = nil
            } else if let match = options.first(where: {
                $0.display.lowercased() == trimmed.lowercased()
            }) {
                selection = match.id
            }
        }
    }
}

// MARK: - ThinkingDotsView (animated bouncing dots)

struct ThinkingDotsView: View {
    @State private var animating = false

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { index in
                Circle()
                    .fill(Color.secondary.opacity(0.6))
                    .frame(width: 6, height: 6)
                    .offset(y: animating ? -4 : 2)
                    .animation(
                        .easeInOut(duration: 0.4)
                            .repeatForever(autoreverses: true)
                            .delay(Double(index) * 0.15),
                        value: animating
                    )
            }
        }
        .onAppear { animating = true }
    }
}

// MARK: - StreamingCursorView (blinking vertical bar)

struct StreamingCursorView: View {
    @State private var visible = true

    var body: some View {
        Rectangle()
            .fill(Color.accentColor)
            .frame(width: 2, height: 14)
            .opacity(visible ? 1 : 0)
            .animation(
                .easeInOut(duration: 0.5).repeatForever(autoreverses: true),
                value: visible
            )
            .onAppear { visible = false }
    }
}
