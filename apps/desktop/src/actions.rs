//! Actions and their default key bindings.

use gpui::{App, KeyBinding, actions};

actions!(moltis, [
    Quit,
    Hide,
    NewSession,
    AbortRun,
    OpenWebUi,
    OpenWebSettings,
    OpenWebProviders,
    OpenWebCrons,
    OpenWebSkills,
    OpenWebLogs,
    ShowServers,
    /// The servers screen with the "Add a server" form open.
    AddServer,
    ReconnectNow,
    SignOut,
    FocusComposer,
]);

actions!(text_input, [
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteToLineStart,
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectAll,
    Home,
    End,
    SelectHome,
    SelectEnd,
    Paste,
    Cut,
    Copy,
    Submit,
    Newline,
    ShowCharacterPalette,
]);

/// Key context of text inputs.
pub const INPUT_CONTEXT: &str = "TextInput";

pub fn bind_keys(cx: &mut App) {
    let input = Some(INPUT_CONTEXT);
    let platform = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let word = if cfg!(target_os = "macos") {
        "alt"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{platform}-q"), Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new(&format!("{platform}-n"), NewSession, None),
        KeyBinding::new(&format!("{platform}-."), AbortRun, None),
        KeyBinding::new(&format!("{platform}-,"), OpenWebSettings, None),
        KeyBinding::new(&format!("{platform}-shift-o"), OpenWebUi, None),
        KeyBinding::new(&format!("{platform}-shift-s"), ShowServers, None),
        KeyBinding::new(&format!("{platform}-l"), FocusComposer, None),
        KeyBinding::new("backspace", Backspace, input),
        KeyBinding::new("delete", Delete, input),
        KeyBinding::new(&format!("{word}-backspace"), DeleteWordLeft, input),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, input),
        KeyBinding::new("left", Left, input),
        KeyBinding::new("right", Right, input),
        KeyBinding::new("up", Up, input),
        KeyBinding::new("down", Down, input),
        KeyBinding::new(&format!("{word}-left"), WordLeft, input),
        KeyBinding::new(&format!("{word}-right"), WordRight, input),
        KeyBinding::new("shift-left", SelectLeft, input),
        KeyBinding::new("shift-right", SelectRight, input),
        KeyBinding::new("shift-up", SelectUp, input),
        KeyBinding::new("shift-down", SelectDown, input),
        KeyBinding::new(&format!("{word}-shift-left"), SelectWordLeft, input),
        KeyBinding::new(&format!("{word}-shift-right"), SelectWordRight, input),
        KeyBinding::new(&format!("{platform}-a"), SelectAll, input),
        KeyBinding::new("home", Home, input),
        KeyBinding::new("end", End, input),
        KeyBinding::new("cmd-left", Home, input),
        KeyBinding::new("cmd-right", End, input),
        KeyBinding::new("shift-home", SelectHome, input),
        KeyBinding::new("shift-end", SelectEnd, input),
        KeyBinding::new("cmd-shift-left", SelectHome, input),
        KeyBinding::new("cmd-shift-right", SelectEnd, input),
        KeyBinding::new(&format!("{platform}-v"), Paste, input),
        KeyBinding::new(&format!("{platform}-x"), Cut, input),
        KeyBinding::new(&format!("{platform}-c"), Copy, input),
        KeyBinding::new("enter", Submit, input),
        KeyBinding::new("shift-enter", Newline, input),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, input),
    ]);
}
