//! The conversation: header, streaming transcript, approvals, composer.

mod messages;

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
};

use gpui::{
    Anchor, AnyElement, Context, Entity, FocusHandle, Focusable, FollowMode, FontWeight,
    IntoElement, ListAlignment, ListState, ParentElement, Render, Styled, Subscription, Window,
    anchored, deferred, div, list, prelude::*, px,
};

use crate::{
    actions::{AbortRun, FocusComposer},
    assets::{Icon, icon},
    daemon::{Daemon, DaemonEvent, Phase},
    input::{InputEvent, TextInput},
    markdown::parse::{Block, parse},
    status_view::reconnect_text,
    theme::Theme,
    transcript::{Change, RunState},
    ui::{ButtonKind, button},
};

use moltis_client::protocol::client_api::ApprovalDecision;

pub struct ChatView {
    daemon: Entity<Daemon>,
    list: ListState,
    shown_session: Option<String>,
    composer: Entity<TextInput>,
    sending: bool,
    composer_version: u64,
    expanded: HashSet<u64>,
    /// Parsed markdown per item id, with the hash of the text it came from.
    markdown: HashMap<u64, (u64, Rc<Vec<Block>>)>,
    model_menu_open: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ChatView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ChatView {
    pub fn new(daemon: Entity<Daemon>, cx: &mut Context<Self>) -> Self {
        let composer =
            cx.new(|cx| TextInput::multi_line("Describe a task or ask a question", 10, cx));
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(FollowMode::Tail);
        let subscriptions = vec![
            cx.subscribe(&daemon, Self::on_daemon_event),
            cx.subscribe(&composer, |this, _, event, cx| match event {
                InputEvent::Submit => this.send(cx),
                InputEvent::Changed => this.composer_version += 1,
            }),
        ];
        let mut this = Self {
            daemon,
            list,
            shown_session: None,
            composer,
            sending: false,
            composer_version: 0,
            expanded: HashSet::new(),
            markdown: HashMap::new(),
            model_menu_open: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.sync_list(cx);
        this
    }

    pub fn composer_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.composer.read(cx).focus_handle(cx)
    }

    fn item_count(&self, cx: &gpui::App) -> usize {
        self.daemon
            .read(cx)
            .selected_transcript()
            .map_or(0, |t| t.items().len())
    }

    /// Rebuild the list if another session is now selected.
    fn sync_list(&mut self, cx: &mut Context<Self>) {
        let selected = self.daemon.read(cx).sessions().selected().map(String::from);
        if selected != self.shown_session {
            self.shown_session = selected;
            self.expanded.clear();
            self.markdown.clear();
            self.list.reset(self.item_count(cx));
            self.list.scroll_to_end();
        }
    }

    fn on_daemon_event(&mut self, _: Entity<Daemon>, event: &DaemonEvent, cx: &mut Context<Self>) {
        match event {
            DaemonEvent::Sessions => self.sync_list(cx),
            DaemonEvent::Transcript {
                session_key,
                change,
            } => {
                self.sync_list(cx);
                if self.shown_session.as_deref() != Some(session_key.as_str()) {
                    return;
                }
                match change {
                    Change::None => {},
                    Change::Appended { at, count } => self.list.splice(*at..*at, *count),
                    Change::Updated { index } => self.list.splice(*index..*index + 1, 1),
                    Change::Removed { index } => self.list.splice(*index..*index + 1, 0),
                    Change::Reset => {
                        self.markdown.clear();
                        self.list.reset(self.item_count(cx));
                    },
                }
                // Guard against drift between the list and the transcript.
                if self.list.item_count() != self.item_count(cx) {
                    self.list.reset(self.item_count(cx));
                }
                cx.notify();
            },
            DaemonEvent::Models | DaemonEvent::Approvals | DaemonEvent::Phase => cx.notify(),
            DaemonEvent::ProfileChanged(_) | DaemonEvent::Toast(_) => {},
        }
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        let text = self.composer.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        let Some(sent) = self.daemon.update(cx, |daemon, cx| daemon.send(text, cx)) else {
            return;
        };
        self.sending = true;
        let version = self.composer_version;
        self.list.scroll_to_end();
        cx.spawn(async move |this, cx| {
            let success = sent.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_send(success, version, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_send(&mut self, success: bool, version: u64, cx: &mut Context<Self>) {
        self.sending = false;
        // Keep the original draft until acceptance. A failed RPC (or an edit
        // while it was pending) must never erase user text.
        if success && self.composer_version == version {
            self.composer.update(cx, |input, cx| input.clear(cx));
        }
        cx.notify();
    }

    fn abort(&mut self, _: &AbortRun, _: &mut Window, cx: &mut Context<Self>) {
        self.daemon.update(cx, |daemon, cx| daemon.abort(cx));
    }

    fn focus_composer_action(
        &mut self,
        _: &FocusComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self.composer_focus(cx);
        window.focus(&focus, cx);
    }

    /// Markdown blocks of `text`, parsed once per text version.
    fn blocks(&mut self, id: u64, text: &str) -> Rc<Vec<Block>> {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some((cached, blocks)) = self.markdown.get(&id)
            && *cached == hash
        {
            return Rc::clone(blocks);
        }
        let blocks = Rc::new(parse(text));
        self.markdown.insert(id, (hash, Rc::clone(&blocks)));
        blocks
    }

    /// The chat title, centred in the band that also moves the window.
    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let title = self
            .daemon
            .read(cx)
            .sessions()
            .selected_entry()
            .map_or_else(|| "New chat".to_string(), |e| e.title().to_string());
        crate::titlebar::movable(
            div()
                .id("chat-titlebar")
                .flex_none()
                .h(px(crate::titlebar::INSET.max(40.)))
                .px_24()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(theme.text_muted)
                .child(div().min_w_0().truncate().child(title)),
        )
        .into_any_element()
    }

    /// The model name with a menu that opens upwards, above the composer.
    fn render_model_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let daemon = self.daemon.read(cx);
        let model_name = daemon
            .model()
            .and_then(|id| daemon.models().iter().find(|m| m.id == id))
            .map_or_else(|| "Default model".to_string(), |m| m.name().to_string());
        let models = daemon.models().to_vec();
        let current = daemon.model().map(String::from);
        let hover = theme.surface_hover;
        div()
            .relative()
            .child(
                div()
                    .id("model-picker")
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(theme.text)
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model_menu_open = !this.model_menu_open;
                        cx.notify();
                    }))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(model_name)
                    .child(icon(Icon::ChevronDown, 11.).text_color(theme.text_muted)),
            )
            .when(self.model_menu_open, |this| {
                this.child(deferred(
                    anchored().anchor(Anchor::BottomRight).child(
                        div()
                            .id("model-menu")
                            .mb_1()
                            .w(px(300.))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.surface)
                            .shadow_lg()
                            .p_1()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.model_menu_open = false;
                                cx.notify();
                            }))
                            .children(models.into_iter().enumerate().map(|(i, model)| {
                                let selected = current.as_deref() == Some(model.id.as_str());
                                let id = model.id.clone();
                                div()
                                    .id(("model", i))
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(selected, |this| this.bg(theme.selected))
                                    .hover(move |style| style.bg(hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.model_menu_open = false;
                                        this.daemon
                                            .update(cx, |daemon, cx| daemon.set_model(&id, cx));
                                    }))
                                    .child(div().text_sm().child(model.name().to_string()))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_muted)
                                            .child(model.provider.clone()),
                                    )
                            })),
                    ),
                ))
            })
            .into_any_element()
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = Theme::global(cx).clone();
        if let Some((running, bundled)) = self.daemon.read(cx).outdated().cloned() {
            return Some(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_4()
                    .py_1()
                    .bg(theme.brand.opacity(0.12))
                    .text_sm()
                    .child(format!(
                        "This app includes moltis {bundled}; the running daemon is {running}."
                    ))
                    .child(
                        button("restart-daemon", "Restart daemon", ButtonKind::Ghost, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.daemon
                                    .update(cx, |daemon, cx| daemon.restart_local_daemon(cx));
                            }),
                        ),
                    )
                    .into_any_element(),
            );
        }
        let Phase::Reconnecting { reason, retry_in } = self.daemon.read(cx).phase().clone() else {
            return None;
        };
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_4()
                .py_1()
                .bg(theme.warning.opacity(0.15))
                .text_sm()
                .child(reconnect_text(&reason, retry_in))
                .child(
                    button("retry-now", "Retry now", ButtonKind::Ghost, cx).on_click(cx.listener(
                        |this, _, _, cx| this.daemon.update(cx, |daemon, cx| daemon.retry_now(cx)),
                    )),
                )
                .into_any_element(),
        )
    }

    fn render_approvals(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let approvals = self.daemon.read(cx).approvals().to_vec();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(approvals.into_iter().map(|request| {
                let approve_id = request.request_id.clone();
                let deny_id = request.request_id.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.warning)
                    .bg(theme.warning.opacity(0.08))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Allow this command?"),
                    )
                    .child(
                        div()
                            .p_2()
                            .rounded_sm()
                            .bg(theme.code_background)
                            .font_family(theme.mono_font.clone())
                            .text_xs()
                            .child(request.command.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    gpui::ElementId::Name(format!("approve-{approve_id}").into()),
                                    "Approve",
                                    ButtonKind::Primary,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.daemon.update(cx, |daemon, cx| {
                                            daemon.resolve_approval(
                                                &approve_id,
                                                ApprovalDecision::Approved,
                                                cx,
                                            );
                                        });
                                    },
                                )),
                            )
                            .child(
                                button(
                                    gpui::ElementId::Name(format!("deny-{deny_id}").into()),
                                    "Deny",
                                    ButtonKind::Secondary,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.daemon.update(cx, |daemon, cx| {
                                            daemon.resolve_approval(
                                                &deny_id,
                                                ApprovalDecision::Denied,
                                                cx,
                                            );
                                        });
                                    },
                                )),
                            ),
                    )
            }))
            .into_any_element()
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let daemon = self.daemon.read(cx);
        let state = daemon
            .selected_transcript()
            .map(crate::transcript::Transcript::state)
            .unwrap_or_default();
        let busy = state != RunState::Idle;
        let connected = matches!(daemon.phase(), Phase::Connected);
        let server = daemon.profile().name.clone();
        let context = daemon.selected_context();
        let status = match state {
            RunState::Thinking => "Thinking…",
            RunState::RunningTool => "Running a tool…",
            RunState::Streaming => "Replying…",
            RunState::Idle => "⇧↩ for a new line",
        };
        let hover = theme.surface_hover;
        let action = if !connected {
            div()
                .text_xs()
                .text_color(theme.text_faint)
                .child("Offline")
                .into_any_element()
        } else if self.sending {
            div()
                .text_xs()
                .text_color(theme.text_faint)
                .child("Sending…")
                .into_any_element()
        } else if busy {
            div()
                .id("stop")
                .size(px(28.))
                .rounded_lg()
                .bg(theme.accent)
                .text_color(theme.accent_text)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.daemon.update(cx, |d, cx| d.abort(cx))))
                .child(icon(Icon::Stop, 18.).text_color(theme.accent_text))
                .into_any_element()
        } else {
            div()
                .id("send")
                .size(px(28.))
                .rounded_lg()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.text_faint)
                .hover(move |style| style.bg(hover))
                .on_click(cx.listener(|this, _, _, cx| this.send(cx)))
                .child(icon(Icon::ArrowUp, 16.).text_color(theme.text_muted))
                .into_any_element()
        };
        let picker = self.render_model_picker(cx);
        div()
            .w_full()
            .max_w(px(messages::COLUMN))
            .mx_auto()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().gap_2().child(
                    div()
                        .id("server-chip")
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py(px(3.))
                        .rounded_md()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface)
                        .text_xs()
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(crate::actions::ShowServers), cx);
                        })
                        .child(icon(Icon::Monitor, 13.).text_color(theme.text_muted))
                        .child(server),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .pl_3()
                    .pr_2()
                    .py_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface)
                    .child(div().flex_1().min_w_0().py_1().child(self.composer.clone()))
                    .child(action),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_1()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(div().flex_1().min_w_0().truncate().child(status))
                    .when_some(
                        context.and_then(|c| c.fraction().map(|f| (c, f))),
                        |this, (usage, fraction)| {
                            this.child(context_meter(
                                usage.used(),
                                usage.context_window,
                                fraction,
                                &theme,
                            ))
                        },
                    )
                    .child(picker),
            )
            .into_any_element()
    }
}

/// "38k / 200k" with a short bar that warms up as the window fills.
fn context_meter(used: u64, window: u64, fraction: f32, theme: &Theme) -> impl IntoElement {
    const WIDTH: f32 = 48.;
    let color = if fraction >= 0.9 {
        theme.danger
    } else if fraction >= 0.7 {
        theme.warning
    } else {
        theme.success
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(WIDTH))
                .h(px(4.))
                .rounded_full()
                .bg(theme.surface_hover)
                .child(
                    div()
                        .w(px((WIDTH * fraction).max(2.)))
                        .h_full()
                        .rounded_full()
                        .bg(color),
                ),
        )
        .child(format!(
            "{} / {}",
            format_tokens(used),
            format_tokens(window)
        ))
}

/// Compact token counts: 950, 38k, 1.2M.
fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{}k", tokens / 1_000),
        _ => {
            let tenths = tokens / 100_000;
            if tenths.is_multiple_of(10) {
                format!("{}M", tenths / 10)
            } else {
                format!("{}.{}M", tenths / 10, tenths % 10)
            }
        },
    }
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let empty = self.item_count(cx) == 0;
        let header = self.render_header(cx);
        let banner = self.render_banner(cx);
        let approvals = self.render_approvals(cx);
        let composer = self.render_composer(cx);
        let transcript = if empty {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xl()
                        .child(div().text_color(theme.brand).child("✳"))
                        .child(greeting(
                            self.daemon
                                .read(cx)
                                .identity()
                                .and_then(|identity| identity.user_name.as_deref()),
                        )),
                )
                .into_any_element()
        } else {
            list(
                self.list.clone(),
                cx.processor(|this, index: usize, window, cx| this.render_item(index, window, cx)),
            )
            .flex_1()
            .into_any_element()
        };
        div()
            .key_context("Chat")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::abort))
            .on_action(cx.listener(Self::focus_composer_action))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .child(header)
            .children(banner)
            .child(transcript)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_6()
                    .pb_3()
                    .pt_2()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(messages::COLUMN))
                            .mx_auto()
                            .child(approvals),
                    )
                    .child(composer),
            )
    }
}

/// The empty chat's prompt, by name when the daemon knows the owner's.
fn greeting(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("What's next, {name}?"),
        None => "What's next?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use {
        gpui::{AppContext, TestAppContext},
        moltis_client::{Endpoint, TlsTrust},
    };

    use super::*;

    fn chat(cx: &mut TestAppContext) -> Entity<ChatView> {
        let daemon = cx.new(|cx| {
            Daemon::disconnected_for_test(
                crate::servers::ServerProfile::direct(
                    "test",
                    Endpoint::parse("http://localhost:1", TlsTrust::System).unwrap(),
                ),
                cx,
            )
        });
        cx.new(|cx| ChatView::new(daemon, cx))
    }

    #[gpui::test]
    fn offline_enter_keeps_the_draft(cx: &mut TestAppContext) {
        let chat = chat(cx);
        chat.update(cx, |chat, cx| {
            chat.composer
                .update(cx, |input, cx| input.set_text("keep this draft", cx));
            chat.send(cx);
            assert_eq!(chat.composer.read(cx).text(), "keep this draft");
            assert!(!chat.sending);
        });
    }

    #[gpui::test]
    fn failed_submission_keeps_draft_and_success_clears_it(cx: &mut TestAppContext) {
        let chat = chat(cx);
        chat.update(cx, |chat, cx| {
            chat.composer
                .update(cx, |input, cx| input.set_text("draft", cx));
        });
        cx.run_until_parked();
        chat.update(cx, |chat, cx| {
            chat.sending = true;
            let version = chat.composer_version;
            chat.finish_send(false, version, cx);
            assert_eq!(chat.composer.read(cx).text(), "draft");
            assert!(!chat.sending);
            chat.finish_send(true, version, cx);
            assert_eq!(chat.composer.read(cx).text(), "");
        });
    }

    #[gpui::test]
    fn acknowledgement_preserves_edits_made_while_sending(cx: &mut TestAppContext) {
        let chat = chat(cx);
        let version = chat.update(cx, |chat, cx| {
            chat.sending = true;
            let version = chat.composer_version;
            chat.composer
                .update(cx, |input, cx| input.set_text("new draft", cx));
            version
        });
        cx.run_until_parked();
        chat.update(cx, |chat, cx| {
            chat.finish_send(true, version, cx);
            assert_eq!(chat.composer.read(cx).text(), "new draft");
            assert!(!chat.sending);
        });
    }

    #[gpui::test]
    fn repeated_enter_during_submission_does_not_clear_draft(cx: &mut TestAppContext) {
        let chat = chat(cx);
        chat.update(cx, |chat, cx| {
            chat.composer
                .update(cx, |input, cx| input.set_text("draft", cx));
            chat.sending = true;
            chat.send(cx);
            assert!(chat.sending);
            assert_eq!(chat.composer.read(cx).text(), "draft");
        });
    }

    #[test]
    fn token_counts_are_compact() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(38_400), "38k");
        assert_eq!(format_tokens(200_000), "200k");
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(1_250_000), "1.2M");
    }

    #[test]
    fn greeting_uses_the_owner_name() {
        assert_eq!(greeting(Some("Fabien")), "What's next, Fabien?");
        assert_eq!(greeting(Some("  ")), "What's next?");
        assert_eq!(greeting(None), "What's next?");
    }
}
