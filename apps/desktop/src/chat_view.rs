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
    daemon::{Daemon, DaemonEvent, Phase},
    input::{InputEvent, TextInput},
    markdown::parse::{Block, parse},
    status_view::reconnect_text,
    theme::Theme,
    transcript::{Change, RunState},
    ui::{ButtonKind, button, field, muted},
};

use moltis_client::protocol::client_api::ApprovalDecision;

pub struct ChatView {
    daemon: Entity<Daemon>,
    list: ListState,
    shown_session: Option<String>,
    composer: Entity<TextInput>,
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
            cx.new(|cx| TextInput::multi_line("Message Moltis…  (⇧↩ for a new line)", 10, cx));
        let list = ListState::new(0, ListAlignment::Top, px(1000.));
        list.set_follow_mode(FollowMode::Tail);
        let subscriptions = vec![
            cx.subscribe(&daemon, Self::on_daemon_event),
            cx.subscribe(&composer, |this, _, event, cx| {
                if *event == InputEvent::Submit {
                    this.send(cx);
                }
            }),
        ];
        let mut this = Self {
            daemon,
            list,
            shown_session: None,
            composer,
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
        let text = self.composer.read(cx).text().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.composer.update(cx, |input, cx| input.clear(cx));
        self.list.scroll_to_end();
        self.daemon.update(cx, |daemon, cx| daemon.send(text, cx));
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

    fn render_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::global(cx).clone();
        let daemon = self.daemon.read(cx);
        let title = daemon
            .sessions()
            .selected_entry()
            .map_or_else(|| "New chat".to_string(), |e| e.title().to_string());
        let model_name = daemon
            .model()
            .and_then(|id| daemon.models().iter().find(|m| m.id == id))
            .map_or_else(|| "Default model".to_string(), |m| m.name().to_string());
        let models = daemon.models().to_vec();
        let current = daemon.model().map(String::from);

        let picker = div()
            .relative()
            .child(
                button(
                    "model-picker",
                    format!("{model_name} ▾"),
                    ButtonKind::Secondary,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.model_menu_open = !this.model_menu_open;
                    cx.notify();
                })),
            )
            .when(self.model_menu_open, |this| {
                this.child(deferred(
                    anchored().anchor(Anchor::TopRight).child(
                        div()
                            .id("model-menu")
                            .mt_1()
                            .w(px(300.))
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
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
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .when(selected, |this| this.bg(theme.surface))
                                    .hover(|style| style.bg(theme.surface_hover))
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
            });

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px_4()
            .h(px(44.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(picker)
            .into_any_element()
    }

    fn render_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = Theme::global(cx).clone();
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
        let state = self
            .daemon
            .read(cx)
            .selected_transcript()
            .map(crate::transcript::Transcript::state)
            .unwrap_or_default();
        let busy = state != RunState::Idle;
        let status = match state {
            RunState::Thinking => Some("Thinking…"),
            RunState::RunningTool => Some("Running a tool…"),
            RunState::Idle | RunState::Streaming => None,
        };
        let connected = matches!(self.daemon.read(cx).phase(), Phase::Connected);
        let action = if busy {
            button("stop", "Stop", ButtonKind::Secondary, cx)
                .on_click(cx.listener(|this, _, _, cx| this.daemon.update(cx, |d, cx| d.abort(cx))))
        } else {
            button("send", "Send", ButtonKind::Primary, cx)
                .on_click(cx.listener(|this, _, _, cx| this.send(cx)))
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .when_some(status, |this, status| this.child(muted(status, cx)))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(field(cx).flex_1().py_2().child(self.composer.clone()))
                    .child(if connected {
                        action.into_any_element()
                    } else {
                        muted("Offline", cx).into_any_element()
                    }),
            )
            .into_any_element()
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
                .child(muted(
                    "Ask anything. Messages stream from your moltis daemon.",
                    cx,
                ))
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
                    .px_4()
                    .pb_4()
                    .pt_2()
                    .child(approvals)
                    .child(composer),
            )
    }
}
