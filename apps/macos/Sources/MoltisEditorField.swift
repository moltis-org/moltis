import AppKit
import SwiftUI

// MARK: - MoltisEditorField (for multi-line text editing in forms)

struct MoltisEditorField: View {
    @Binding var text: String
    let minHeight: CGFloat

    init(text: Binding<String>, minHeight: CGFloat = 160) {
        _text = text
        self.minHeight = minHeight
    }

    var body: some View {
        TextEditor(text: $text)
            .font(.system(.body, design: .monospaced))
            .scrollContentBackground(.hidden)
            .padding(6)
            .frame(minHeight: minHeight)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay {
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(Color(nsColor: .separatorColor).opacity(0.4))
            }
    }
}
