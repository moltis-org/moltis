//! Choose, add and remove daemons.

use gpui::{
    Context, Entity, EventEmitter, FontWeight, IntoElement, ParentElement, Render, Styled, Window,
    div, prelude::*, px,
};

use moltis_client::{Endpoint, TlsTrust};

#[cfg(feature = "ssh")]
use moltis_client::ssh::SshTarget;

use crate::{
    input::TextInput,
    servers::{ServerKind, ServerProfile},
    theme::Theme,
    ui::{ButtonKind, button, card, field, label, muted},
};

/// Default daemon port when none is configured.
const DEFAULT_PORT: u16 = 18789;

pub enum ServersEvent {
    Connect(ServerProfile),
    Save(ServerProfile),
    Remove(String),
}

impl EventEmitter<ServersEvent> for ServersView {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Direct,
    #[cfg(feature = "ssh")]
    Ssh,
}

pub struct ServersView {
    profiles: Vec<ServerProfile>,
    active: Option<String>,
    adding: bool,
    mode: Mode,
    name: Entity<TextInput>,
    url: Entity<TextInput>,
    destination: Entity<TextInput>,
    remote_port: Entity<TextInput>,
    error: Option<String>,
}

impl ServersView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            profiles: Vec::new(),
            active: None,
            adding: false,
            mode: Mode::Direct,
            name: cx.new(|cx| TextInput::single_line("Studio", cx)),
            url: cx.new(|cx| TextInput::single_line("https://studio.local:18789", cx)),
            destination: cx.new(|cx| TextInput::single_line("user@studio", cx)),
            remote_port: cx.new(|cx| TextInput::single_line(DEFAULT_PORT.to_string(), cx)),
            error: None,
        }
    }

    /// The servers to list (the local daemon first) and the connected one.
    pub fn set_profiles(
        &mut self,
        profiles: Vec<ServerProfile>,
        active: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.adding = self.adding || profiles.is_empty();
        self.profiles = profiles;
        self.active = active;
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        match self.build_profile(cx) {
            Ok(profile) => {
                self.error = None;
                self.adding = false;
                for input in [&self.name, &self.url, &self.destination, &self.remote_port] {
                    input.update(cx, |input, cx| input.clear(cx));
                }
                cx.emit(ServersEvent::Save(profile.clone()));
                cx.emit(ServersEvent::Connect(profile));
            },
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn build_profile(&self, cx: &Context<Self>) -> Result<ServerProfile, String> {
        let name = self.name.read(cx).text().trim().to_string();
        match self.mode {
            Mode::Direct => {
                let url = self.url.read(cx).text().trim().to_string();
                // Without a scheme, assume the daemon's default HTTPS.
                let url = if url.contains("://") {
                    url
                } else {
                    format!("https://{url}")
                };
                let endpoint =
                    Endpoint::parse(&url, TlsTrust::System).map_err(|e| e.to_string())?;
                if !endpoint.protects_credentials() {
                    return Err("Use https:// (or an SSH tunnel) for other machines; plain http:// would send credentials unencrypted.".into());
                }
                let name = if name.is_empty() {
                    endpoint.host().to_string()
                } else {
                    name
                };
                Ok(ServerProfile::direct(name, endpoint))
            },
            #[cfg(feature = "ssh")]
            Mode::Ssh => {
                let destination = self.destination.read(cx).text().trim().to_string();
                let port_text = self.remote_port.read(cx).text().trim().to_string();
                let remote_port = if port_text.is_empty() {
                    DEFAULT_PORT
                } else {
                    port_text
                        .parse()
                        .map_err(|_| format!("{port_text:?} is not a port number"))?
                };
                if destination.is_empty() || destination.starts_with('-') {
                    return Err("Enter an SSH destination such as user@host.".into());
                }
                Ok(ServerProfile {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: if name.is_empty() {
                        destination.clone()
                    } else {
                        name
                    },
                    kind: ServerKind::Ssh {
                        target: SshTarget {
                            destination,
                            remote_port,
                            ssh_port: None,
                            identity_file: None,
                            tls: true,
                        },
                        tls: TlsTrust::System,
                    },
                })
            },
        }
    }

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let mode_card = |id: &'static str,
                         mode: Mode,
                         title: &'static str,
                         detail: &'static str,
                         cx: &mut Context<Self>| {
            let selected = self.mode == mode;
            div()
                .id(id)
                .flex_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(if selected {
                    theme.accent
                } else {
                    theme.border
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    cx.notify();
                }))
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(title))
                .child(div().text_xs().text_color(theme.text_muted).child(detail))
        };
        let modes = div().flex().gap_2().child(mode_card(
            "mode-direct",
            Mode::Direct,
            "Direct",
            "HTTPS to the daemon's port (LAN, Tailscale)",
            cx,
        ));
        #[cfg(feature = "ssh")]
        let modes = modes.child(mode_card(
            "mode-ssh",
            Mode::Ssh,
            "SSH tunnel",
            "Forward the remote loopback port with ssh",
            cx,
        ));
        let fields = match self.mode {
            Mode::Direct => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("URL", cx))
                .child(field(cx).child(self.url.clone())),
            #[cfg(feature = "ssh")]
            Mode::Ssh => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label(
                    "SSH destination (uses your ~/.ssh/config and agent)",
                    cx,
                ))
                .child(field(cx).child(self.destination.clone()))
                .child(label("Daemon port on that machine", cx))
                .child(field(cx).child(self.remote_port.clone())),
        };
        card(cx)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Add a server"),
            )
            .child(modes)
            .child(label("Name", cx))
            .child(field(cx).child(self.name.clone()))
            .child(fields)
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("add-server", "Add and connect", ButtonKind::Primary, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.add(cx))),
                    )
                    .when(!self.profiles.is_empty(), |this| {
                        this.child(
                            button("cancel-add", "Cancel", ButtonKind::Ghost, cx).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.adding = false;
                                    this.error = None;
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            )
    }
}

impl Render for ServersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let rows = self
            .profiles
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, profile)| {
                let active = self.active.as_deref() == Some(profile.id.as_str());
                let connect = profile.clone();
                let remove_id = profile.id.clone();
                div()
                    .id(("server", i))
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(if active {
                        theme.accent
                    } else {
                        theme.border
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ServersEvent::Connect(connect.clone()))
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(profile.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(profile.address()),
                            ),
                    )
                    .when(active, |this| {
                        this.child(div().text_xs().text_color(theme.accent).child("Connected"))
                    })
                    .when(!profile.is_local(), |this| {
                        this.child(
                            button(("remove", i), "Remove", ButtonKind::Ghost, cx).on_click(
                                cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(ServersEvent::Remove(remove_id.clone()));
                                }),
                            ),
                        )
                    })
            });
        div()
            .id("servers-view")
            .size_full()
            .overflow_y_scroll()
            .bg(theme.background)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(520.))
                    .max_w_full()
                    .py_8()
                    .px_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("Servers"))
                    .child(muted(
                        "Moltis Desktop connects to a running moltis daemon. Quitting the app leaves the daemon running.",
                        cx,
                    ))
                    .children(rows)
                    .child(if self.adding {
                        self.render_form(cx).into_any_element()
                    } else {
                        button("show-add", "+ Add server", ButtonKind::Secondary, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.adding = true;
                                cx.notify();
                            }))
                            .into_any_element()
                    }),
            )
    }
}
