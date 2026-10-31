//! The dock as the column's Direct view (WX-5d §3.1, `direct-1920x1080.png`).
//!
//! The home view of the window with the column layout, so it is drawn to
//! invite a conversation rather than to look like a raw dock: the chats as
//! the head's tabs (a state dot and the agent's name), the followed treatment
//! as chips, the transcript and its composer in the middle, and the
//! terminals in a pane under them, collapsed until one is wanted. With no
//! chat yet, the middle is one row per agent with Start.
//!
//! Only the composition lives here. The chats are `chat_view`'s, the
//! terminals `terminal_view`'s, and the processes behind both keep running
//! whichever view the column shows (`Dock::tick`).

use egui::{Context, Rect, RichText, Stroke};

use super::{Agent, Body, Dock, DockContext, DockOutput};
use crate::ui::theme::color32;

/// The column head's height, shared with every raylib view.
const HEAD: f32 = crate::ui::widgets::COLUMN_HEAD_HEIGHT;
/// The terminal pane's own head, and its default and least heights.
const PANE_HEAD: f32 = 30.0;
const PANE_DEFAULT: f32 = 250.0;
const PANE_MINIMUM: f32 = 90.0;
/// What the chat keeps above the pane, however tall the pane is dragged.
const CHAT_MINIMUM: f32 = 220.0;
/// How many earlier chats the head's `+` lists, and how wide.
const HEAD_MENU_CHATS: usize = 6;
const MENU_ROW_WIDTH: f32 = 260.0;

fn of(value: u32) -> egui::Color32 {
    color32::of(value)
}

fn micro(text: &str, colour: egui::Color32) -> RichText {
    RichText::new(text.to_uppercase())
        .size(crate::ui::theme::metric::UI_FONT_MICRO)
        .extra_letter_spacing(1.0)
        .color(colour)
}

/// A 6 px dot, painted (the interface face has no U+25CF).
fn dot(ui: &mut egui::Ui, colour: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, colour);
}

/// The same state drives the painted card and its report.
fn installed(agent: Agent) -> &'static str {
    let override_var = format!("MUSIALIZER_AGENT_{}", agent.command().to_uppercase());
    if std::env::var_os(&override_var).is_some_and(|value| !value.is_empty()) {
        return "installed";
    }
    use musializer_runtime::agent_process::Location;
    match musializer_runtime::agent_process::locate(agent.command()) {
        Location::Found(_) => "installed",
        Location::Checking => "checking",
        Location::NotFound => "not-found",
    }
}

impl Dock {
    /// Indices of the chats, or of the terminals, in tab order.
    fn indices(&self, chats: bool) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| matches!(tab.body, Body::Chat(_)) == chats)
            .map(|(index, _)| index)
            .collect()
    }

    /// The chat the column shows, if there is one.
    pub(super) fn current_chat(&self) -> Option<usize> {
        let chats = self.indices(true);
        chats
            .iter()
            .copied()
            .find(|index| Some(*index) == self.chat_active)
            .or_else(|| chats.last().copied())
    }

    /// The terminal the pane shows, if there is one.
    pub(super) fn current_terminal(&self) -> Option<usize> {
        let terminals = self.indices(false);
        terminals
            .iter()
            .copied()
            .find(|index| Some(*index) == self.terminal_active)
            .or_else(|| terminals.last().copied())
    }

    /// Opens a shell in the pane if there is none, and opens the pane: the
    /// probes' `dock-run=` and `dock-focus=` type into a terminal.
    pub fn ensure_terminal(&mut self, context: &DockContext) {
        if self.current_terminal().is_none() {
            self.open_tab(None, context, false);
        }
        self.terminal_open = true;
    }

    /// Notes the window's own treatment applies in the chat that is showing
    /// (§3.1): "the agent edited" and "the preview changed" in one place.
    /// Called every frame, whichever view the column shows.
    pub fn observe(&mut self, context: &DockContext) {
        let Some(following) = &context.following else {
            self.applied_seen = 0;
            return;
        };
        if following.applied <= self.applied_seen {
            self.applied_seen = following.applied;
            return;
        }
        self.applied_seen = following.applied;
        let place = context.now.split(" \u{00b7} ").next().unwrap_or("");
        let text = if place.is_empty() {
            format!("Window applied {}", following.name)
        } else {
            format!("Window applied {} at {place}", following.name)
        };
        if let Some(index) = self.current_chat() {
            if let Body::Chat(chat) = &mut self.tabs[index].body {
                let text = following.changed_sections.map_or(text.clone(), |count| {
                    format!(
                        "{text} · {count} section{} changed",
                        if count == 1 { "" } else { "s" }
                    )
                });
                chat.note(text, following.last_applied_at);
            }
        }
    }

    /// Builds the Direct view into `rect`, the column's whole rect.
    pub(super) fn show_column(
        &mut self,
        ctx: &Context,
        rect: Rect,
        context: &DockContext,
    ) -> DockOutput {
        let mut output = DockOutput::default();
        self.refresh_earlier(context);
        self.earlier_rects.clear();
        let palette = crate::ui::theme::palette();
        let surface = of(palette.ui_surface);
        let rule = of(palette.ui_rule);
        // No fade: the column's other views arrive at once, and egui's fade
        // runs on the wall clock, so a frame would depend on when it was drawn.
        egui::Area::new(egui::Id::new("musializer_dock_column"))
            .fade_in(false)
            .fixed_pos(rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_width(rect.width());
                ui.set_height(rect.height());
                ui.set_clip_rect(rect);
                ui.painter().rect_filled(rect, 0.0, surface);
                ui.painter().line_segment(
                    [rect.left_top(), rect.left_bottom()],
                    Stroke::new(1.0, rule),
                );

                // The head: the chats, `+`, and the terminal toggle.
                let head = Rect::from_min_size(rect.min, egui::vec2(rect.width(), HEAD));
                ui.painter().line_segment(
                    [
                        egui::pos2(head.left(), head.bottom() - 0.5),
                        egui::pos2(head.right(), head.bottom() - 0.5),
                    ],
                    Stroke::new(1.0, rule),
                );
                let mut head_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            egui::pos2(head.left() + 6.0, head.top()),
                            egui::pos2(head.right() - 8.0, head.bottom() - 1.0),
                        ))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                self.column_head(&mut head_ui, context);
                let mut top = head.bottom();

                // The followed treatment, as chips.
                if let Some(following) = &context.following {
                    let row = Rect::from_min_max(
                        egui::pos2(rect.left(), top),
                        egui::pos2(rect.right(), top + 82.0),
                    );
                    let mut row_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(row.shrink2(egui::vec2(14.0, 10.0)))
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );
                    output.stop_following = following_chips(&mut row_ui, following);
                    ui.painter().line_segment(
                        [
                            egui::pos2(row.left(), row.bottom() - 0.5),
                            egui::pos2(row.right(), row.bottom() - 0.5),
                        ],
                        Stroke::new(1.0, rule),
                    );
                    top = row.bottom();
                }

                // The terminal pane, from the bottom up.
                let pane_height = if self.terminal_open {
                    self.terminal_height.unwrap_or(PANE_DEFAULT).clamp(
                        PANE_MINIMUM,
                        (rect.bottom() - top - CHAT_MINIMUM).max(PANE_MINIMUM),
                    )
                } else {
                    PANE_HEAD
                };
                let pane = Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - pane_height),
                    rect.max,
                );
                let middle = Rect::from_min_max(
                    egui::pos2(rect.left() + 1.0, top),
                    egui::pos2(rect.right(), pane.top()),
                );

                let mut keyboard = false;
                let mut middle_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(middle)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                match self.current_chat() {
                    Some(index) => {
                        self.start_rects.clear();
                        self.chat_active = Some(index);
                        if let Body::Chat(chat) = &mut self.tabs[index].body {
                            keyboard |= chat.show_column(&mut middle_ui, context).has_focus;
                        }
                    }
                    None => self.empty_state(&mut middle_ui, context),
                }
                keyboard |= self.terminal_pane(ui, pane, context, &mut output);
                self.keyboard = keyboard || ctx.egui_wants_keyboard_input();
            });
        output
    }

    /// The head: one ghost tab per chat (a state dot and the agent's name,
    /// the shown one in ink over a 2 px ink rule), `+` for a new chat or
    /// terminal, and the terminal pane's toggle at the right.
    fn column_head(&mut self, ui: &mut egui::Ui, context: &DockContext) {
        let ink = color32::ui_ink();
        let weak = ui.visuals().weak_text_color();
        ui.spacing_mut().item_spacing.x = 2.0;
        // The head's controls are ghosts: a glyph until hovered.
        ui.style_mut().visuals.button_frame = false;
        let shown = self.current_chat();
        let mut select = None;
        if shown.is_none() {
            ui.add_space(8.0);
            ui.label(micro("Direct", weak));
            ui.add_space(6.0);
        }
        for index in self.indices(true) {
            let Body::Chat(chat) = &self.tabs[index].body else {
                continue;
            };
            let (busy, waiting, _) = chat.activity();
            let tone = if chat.failed() {
                color32::ui_danger()
            } else if waiting {
                color32::ui_warning()
            } else if busy || !chat.stopped() {
                color32::ui_success()
            } else {
                weak
            };
            let selected = shown == Some(index);
            let response = ui
                .scope(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        dot(ui, tone);
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(chat.agent.label())
                                .size(13.0)
                                .color(if selected { ink } else { weak }),
                        );
                        ui.add_space(8.0);
                    })
                    .response
                })
                .inner
                .interact(egui::Sense::click());
            if selected {
                let bar = Rect::from_min_max(
                    egui::pos2(response.rect.left(), ui.max_rect().bottom() - 1.0),
                    egui::pos2(response.rect.right(), ui.max_rect().bottom() + 1.0),
                );
                ui.painter().rect_filled(bar, 0.0, ink);
            }
            if response
                .on_hover_text(if waiting {
                    "Waiting for you to allow or deny"
                } else if busy {
                    "Working"
                } else {
                    "Show this chat"
                })
                .clicked()
            {
                select = Some(index);
            }
        }
        let mut open_chat = None;
        let mut open_terminal = false;
        let earlier = self.earlier_closed();
        let mut reopen = None;
        let mut past_rects = Vec::new();
        let plus = ui
            .menu_button(RichText::new("+").size(15.0).color(weak), |ui| {
                for agent in Agent::ALL {
                    if ui.button(format!("Chat with {}", agent.label())).clicked() {
                        open_chat = Some(agent);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("New terminal").clicked() {
                    open_terminal = true;
                    ui.close();
                }
                if !earlier.is_empty() {
                    ui.separator();
                    ui.label(micro("Earlier chats", weak));
                    for (index, chat) in earlier.iter().take(HEAD_MENU_CHATS).enumerate() {
                        let row = earlier_row(ui, chat, MENU_ROW_WIDTH);
                        past_rects.push((format!("past:{index}"), row.rect));
                        if row.clicked() {
                            reopen = Some(chat.id.clone());
                            ui.close();
                        }
                    }
                }
            })
            .response;
        self.earlier_rects.push(("head-plus".to_owned(), plus.rect));
        self.earlier_rects.append(&mut past_rects);
        plus.on_hover_text("A new chat or terminal, or an earlier chat");
        if let Some(index) = shown {
            if ui
                .add(egui::Button::new(RichText::new("\u{00d7}").color(weak)).frame(false))
                .on_hover_text("Close this chat. What was said stays under Earlier chats")
                .clicked()
            {
                if matches!(&self.tabs[index].body, Body::Chat(chat) if chat.needs_close_confirmation())
                {
                    self.close_chat_confirmation = Some(self.tabs[index].id);
                } else {
                    self.close_tab(index);
                }
            }
        }
        if let Some(id) = self.close_chat_confirmation {
            let mut close = false;
            let mut cancel = false;
            egui::Window::new("Close this chat?")
                .id(id.with("confirm_close"))
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("This stops any running turn. The chat stays under Earlier chats.");
                    ui.horizontal(|ui| {
                        cancel = ui.button("Keep chat").clicked();
                        close = ui.button("Close chat").clicked();
                    });
                });
            if close {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                    self.close_tab(index);
                }
            }
            if close || cancel {
                self.close_chat_confirmation = None;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let text = RichText::new(">_")
                .monospace()
                .size(12.0)
                .color(if self.terminal_open { ink } else { weak });
            if ui
                .add(egui::Button::selectable(self.terminal_open, text))
                .on_hover_text(if self.terminal_open {
                    "Hide the terminal"
                } else {
                    "Show the terminal"
                })
                .clicked()
            {
                self.terminal_open = !self.terminal_open;
            }
        });
        if let Some(index) = select {
            self.chat_active = Some(index);
            if let Body::Chat(chat) = &mut self.tabs[index].body {
                chat.focus_next = true;
            }
        }
        if let Some(agent) = open_chat {
            self.open_chat(agent, context);
        }
        if let Some(chat_id) = reopen {
            self.reopen(&chat_id);
        }
        if open_terminal {
            self.open_tab(None, context, true);
            self.terminal_open = true;
        }
    }

    /// No chat yet: an invitation, and one row per agent with Start.
    fn empty_state(&mut self, ui: &mut egui::Ui, context: &DockContext) {
        let palette = crate::ui::theme::palette();
        let ink = color32::ui_ink();
        let weak = ui.visuals().weak_text_color();
        let mut start = None;
        let mut starts = Vec::new();
        let mut reopen = None;
        let mut forget = None;
        let mut ask_forget = None;
        let mut keep = false;
        let earlier = self.earlier_closed();
        let mut rows = Vec::new();
        let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        scroll.show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(14, 18))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 10.0;
                    ui.label(
                        RichText::new("Talk the video through with an agent")
                            .size(crate::ui::theme::metric::UI_FONT_HEADER + 2.0)
                            .color(ink),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Say what a section should feel like, or ask for a still or a \
                             clip. It reads the project, writes the treatment, and the \
                             preview follows at the playhead.",
                            )
                            .color(weak),
                        )
                        .wrap(),
                    );
                    ui.add_space(6.0);
                    for agent in Agent::ALL {
                        let state = installed(agent);
                        let found = state == "installed";
                        if state == "checking" {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(50));
                        }
                        egui::Frame::new()
                            .fill(of(palette.ui_raised))
                            .stroke(Stroke::new(1.0, of(palette.ui_rule)))
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    dot(ui, if found { color32::ui_success() } else { weak });
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.label(
                                            RichText::new(agent.label()).size(14.0).color(ink),
                                        );
                                        ui.label(micro(
                                            match state {
                                                "installed" => "Installed",
                                                "checking" => "Checking…",
                                                _ => "Not found on PATH",
                                            },
                                            weak,
                                        ));
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let label = if self.agents_in_terminal {
                                                "Open"
                                            } else {
                                                "Start"
                                            };
                                            let button = egui::Button::new(
                                                RichText::new(label).color(if found {
                                                    of(palette.ui_surface)
                                                } else {
                                                    ink
                                                }),
                                            )
                                            .fill(if found { ink } else { of(palette.ui_control) })
                                            .corner_radius(4.0)
                                            .min_size(egui::vec2(64.0, 28.0));
                                            let tip = if self.agents_in_terminal {
                                                format!(
                                                    "{} in a terminal, in {}",
                                                    agent.label(),
                                                    context.place
                                                )
                                            } else {
                                                format!(
                                                    "Chat with {}, working in {}",
                                                    agent.label(),
                                                    context.place
                                                )
                                            };
                                            let response = ui.add(button).on_hover_text(tip);
                                            starts.push((agent.command(), response.rect, state));
                                            if response.clicked() {
                                                start = Some(agent);
                                            }
                                        },
                                    );
                                });
                            });
                    }
                    ui.checkbox(
                        &mut self.agents_in_terminal,
                        RichText::new("run in a terminal instead")
                            .size(12.0)
                            .color(weak),
                    )
                    .on_hover_text("Open the agent's own interface in the terminal pane");
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("in {}", context.place))
                                .size(12.0)
                                .color(weak),
                        )
                        .truncate(),
                    )
                    .on_hover_text("New chats and terminals start in this folder");
                    if !earlier.is_empty()
                        || self.earlier_error.is_some()
                        || self.earlier_refused > 0
                    {
                        ui.add_space(8.0);
                        ui.label(micro("Earlier chats", weak));
                    }
                    if let Some(error) = &self.earlier_error {
                        ui.label(RichText::new(error).size(12.0).color(color32::ui_danger()));
                    }
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, chat) in earlier.iter().enumerate() {
                        if self.forget_pending.as_deref() == Some(chat.id.as_str()) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "Forget \u{201c}{}\u{201d}?",
                                            chat.title
                                        ))
                                        .size(12.0)
                                        .color(ink),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(
                                    "Removes it from this list. The agent keeps its own history.",
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let no = ui.add(egui::Button::new(
                                            RichText::new("Keep").size(12.0).color(ink),
                                        ));
                                        let yes = ui.add(egui::Button::new(
                                            RichText::new("Forget")
                                                .size(12.0)
                                                .color(color32::ui_danger()),
                                        ));
                                        rows.push((format!("forget-yes:{index}"), yes.rect));
                                        rows.push((format!("forget-no:{index}"), no.rect));
                                        if yes.clicked() {
                                            forget = Some(chat.id.clone());
                                        }
                                        keep |= no.clicked();
                                    },
                                );
                            });
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let row = earlier_row(ui, chat, ui.available_width() - 28.0);
                            rows.push((format!("earlier:{index}"), row.rect));
                            if row.on_hover_text("Open this chat again").clicked() {
                                reopen = Some(chat.id.clone());
                            }
                            let close = ui
                                .add(
                                    egui::Button::new(RichText::new("\u{00d7}").color(weak))
                                        .frame(false),
                                )
                                .on_hover_text("Forget this chat");
                            rows.push((format!("forget:{index}"), close.rect));
                            if close.clicked() {
                                ask_forget = Some(chat.id.clone());
                            }
                        });
                    }
                    if self.earlier_refused > 0 {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!(
                                "{} saved chat{} could not be read and {} left alone.",
                                self.earlier_refused,
                                if self.earlier_refused == 1 { "" } else { "s" },
                                if self.earlier_refused == 1 {
                                    "is"
                                } else {
                                    "are"
                                },
                            ))
                            .size(11.0)
                            .color(weak),
                        );
                    }
                })
        });
        self.start_rects = starts;
        self.earlier_rects.append(&mut rows);
        if keep {
            self.forget_pending = None;
        }
        if ask_forget.is_some() {
            self.forget_pending = ask_forget;
        }
        if let Some(chat_id) = forget {
            self.forget(&chat_id);
        }
        if let Some(chat_id) = reopen {
            self.reopen(&chat_id);
        }
        if let Some(agent) = start {
            if self.agents_in_terminal {
                self.open_tab(Some(agent), context, true);
                self.terminal_open = true;
            } else {
                self.open_chat(agent, context);
            }
        }
    }

    /// The terminal pane: a splitter, a 30 px micro head, the terminal.
    /// Returns whether the terminal holds the keyboard.
    fn terminal_pane(
        &mut self,
        ui: &mut egui::Ui,
        pane: Rect,
        context: &DockContext,
        output: &mut DockOutput,
    ) -> bool {
        let palette = crate::ui::theme::palette();
        let rule = of(palette.ui_rule);
        let weak = ui.visuals().weak_text_color();
        ui.painter()
            .rect_filled(pane, 0.0, of(palette.ui_lane_trough));
        ui.painter()
            .line_segment([pane.left_top(), pane.right_top()], Stroke::new(1.0, rule));
        // The splitter: drag the pane's top edge.
        if self.terminal_open {
            let grip = Rect::from_min_max(
                egui::pos2(pane.left(), pane.top() - 3.0),
                egui::pos2(pane.right(), pane.top() + 3.0),
            );
            let response = ui.interact(grip, egui::Id::new("dock_pane_grip"), egui::Sense::drag());
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if let Some(pos) = response
                .interact_pointer_pos()
                .filter(|_| response.dragged())
            {
                self.terminal_height = Some(pane.bottom() - pos.y);
            }
        }
        let head = Rect::from_min_size(pane.min, egui::vec2(pane.width(), PANE_HEAD));
        ui.painter().line_segment(
            [
                egui::pos2(head.left(), head.bottom() - 0.5),
                egui::pos2(head.right(), head.bottom() - 0.5),
            ],
            Stroke::new(1.0, rule),
        );
        let terminal = self.current_terminal();
        let title =
            terminal.map_or_else(|| "Terminal".to_owned(), |index| self.tabs[index].title());
        let mut head_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(head.shrink2(egui::vec2(14.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut toggle = false;
        let mut open = false;
        let mut cycle = None;
        let mut close = false;
        head_ui.label(micro(&title, weak));
        head_ui
            .add(egui::Label::new(RichText::new(&context.place).size(11.0).color(weak)).truncate());
        head_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if terminal.is_some()
                && ui
                    .add(egui::Button::new(RichText::new("×").color(weak)).frame(false))
                    .on_hover_text("Close this terminal and stop its processes")
                    .clicked()
            {
                close = true;
            }
            let chevron = if self.terminal_open { "\u{2013}" } else { "+" };
            if self.terminal_open
                && ui
                    .add(egui::Button::new(RichText::new(chevron).color(weak)).frame(false))
                    .on_hover_text("Collapse the terminal")
                    .clicked()
            {
                toggle = true;
            }
            if ui
                .add(egui::Button::new(RichText::new("+").color(weak)).frame(false))
                .on_hover_text("New terminal")
                .clicked()
            {
                open = true;
            }
            let terminals = self.indices(false);
            if terminals.len() > 1 {
                if let Some(position) =
                    terminal.and_then(|index| terminals.iter().position(|t| *t == index))
                {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{}/{}", position + 1, terminals.len()))
                                    .size(11.0)
                                    .color(weak),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Next terminal")
                        .clicked()
                    {
                        cycle = Some(terminals[(position + 1) % terminals.len()]);
                    }
                }
            }
        });
        if close {
            if let Some(index) = terminal {
                self.close_tab(index);
            }
            if self.current_terminal().is_none() {
                self.terminal_open = false;
            }
            return false;
        }
        // A press on a collapsed head opens the pane.
        if !self.terminal_open
            && ui
                .interact(head, egui::Id::new("dock_pane_head"), egui::Sense::click())
                .on_hover_text("Show the terminal")
                .clicked()
        {
            toggle = true;
        }
        if toggle {
            self.terminal_open = !self.terminal_open;
        }
        if let Some(index) = cycle {
            self.terminal_active = Some(index);
        }
        if open {
            self.open_tab(None, context, true);
            self.terminal_open = true;
            return false;
        }
        if !self.terminal_open {
            return false;
        }
        let Some(index) = self.current_terminal().or_else(|| {
            // Opened with nothing in it: the pane starts a shell.
            self.open_tab(None, context, false);
            self.current_terminal()
        }) else {
            return false;
        };
        self.terminal_active = Some(index);
        let body = Rect::from_min_max(
            egui::pos2(pane.left() + 8.0, head.bottom() + 4.0),
            egui::pos2(pane.right() - 6.0, pane.bottom() - 4.0),
        );
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let tab_id = self.tabs[index].id;
        let mut keyboard = false;
        if let Body::Terminal(terminal) = &mut self.tabs[index].body {
            let restart = if let Some(session) = terminal.session.as_mut() {
                let view = terminal.view.show(&mut body_ui, tab_id, session);
                keyboard = view.has_focus;
                output.toggle |= view.toggle_dock;
                view.restart
            } else {
                body_ui.label(
                    RichText::new(format!(
                        "Could not start the terminal: {}",
                        terminal.error.as_deref().unwrap_or("unknown error")
                    ))
                    .color(color32::ui_danger()),
                );
                body_ui.button("Try again").clicked()
            };
            if restart {
                terminal.start(true);
            }
        }
        keyboard
    }
}

/// One saved chat: what it was about, over who and when.
fn earlier_row(
    ui: &mut egui::Ui,
    chat: &crate::ui::chat_store::ChatSummary,
    width: f32,
) -> egui::Response {
    use egui::text::{LayoutJob, TextWrapping};
    let palette = crate::ui::theme::palette();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 40.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, of(palette.ui_selected));
    }
    let weak = ui.visuals().weak_text_color();
    let agent = Agent::from_name(&chat.agent)
        .map_or_else(|| chat.agent.clone(), |agent| agent.label().to_owned());
    let age = crate::ui::preferences::recent::describe_age(
        Some(chat.updated_unix),
        Some(crate::ui::chat_store::now_unix()),
    )
    .unwrap_or_default();
    let line = |words: String, size: f32, colour: egui::Color32, top: f32| {
        let mut job = LayoutJob::simple_singleline(words, egui::FontId::proportional(size), colour);
        job.wrap = TextWrapping::truncate_at_width((rect.width() - 12.0).max(0.0));
        let galley = ui.painter().layout_job(job);
        ui.painter()
            .galley(egui::pos2(rect.left() + 6.0, top), galley, colour);
    };
    line(
        chat.title.clone(),
        13.0,
        color32::ui_ink(),
        rect.top() + 4.0,
    );
    line(
        format!("{agent} \u{00b7} {age}"),
        11.0,
        weak,
        rect.top() + 22.0,
    );
    response
}

fn following_status(following: &super::Following) -> String {
    let Some(at) = following.last_applied_at else {
        return "applies on save".to_owned();
    };
    let age = super::chat_view::note_age(at.elapsed().as_secs());
    following.changed_sections.map_or_else(
        || format!("applied · {age}"),
        |count| {
            format!(
                "{count} section{} changed · {age}",
                if count == 1 { "" } else { "s" }
            )
        },
    )
}

/// `following native.treatment.json · stop` and how often it applied.
/// Returns whether stop was pressed.
fn following_chips(ui: &mut egui::Ui, following: &super::Following) -> bool {
    let ink = color32::ui_ink();
    let weak = ui.visuals().weak_text_color();
    let edge = of(crate::ui::theme::palette().ui_control_edge);
    let chip = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, edge))
            .corner_radius(12.0)
            .inner_margin(egui::Margin::symmetric(9, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    add(ui);
                });
            });
    };
    let mut stop = false;
    ui.spacing_mut().item_spacing.x = 6.0;
    chip(ui, &mut |ui| {
        ui.label(RichText::new("following").size(12.0).color(weak));
        ui.add_sized(
            egui::vec2((ui.available_width() - 46.0).max(0.0), 16.0),
            egui::Label::new(RichText::new(&following.name).size(12.0).color(ink)).truncate(),
        );
        stop = ui
            .add(
                egui::Button::new(RichText::new("\u{00b7} stop").size(12.0).color(weak))
                    .frame(false),
            )
            .on_hover_text("Stop following; the preview keeps what was applied")
            .clicked();
    });
    chip(ui, &mut |ui| match &following.error {
        Some(error) => {
            ui.label(
                RichText::new("last change refused")
                    .size(12.0)
                    .color(color32::ui_danger()),
            )
            .on_hover_text(error.as_str());
        }
        None => {
            ui.label(
                RichText::new(following_status(following))
                    .size(12.0)
                    .color(weak),
            );
        }
    });
    stop
}

impl Dock {
    /// `dock:` for the Direct view: the tabs, who holds the keyboard, the
    /// pane, the shown chat and the pane's terminal, so the gate reads the
    /// chat and the shell from one line.
    pub(super) fn describe_column(&self, open: bool) -> String {
        let tabs: Vec<String> = self.tabs.iter().map(super::Tab::title).collect();
        let chat = self.current_chat().map_or_else(
            || "none".to_owned(),
            |index| match &self.tabs[index].body {
                Body::Chat(chat) => chat.describe(),
                Body::Terminal(_) => "none".to_owned(),
            },
        );
        let terminal = self.current_terminal().map_or_else(
            || "none".to_owned(),
            |index| match &self.tabs[index].body {
                Body::Terminal(terminal) => {
                    let (state, screen) = super::terminal_state(terminal);
                    format!("{state} screen={screen:?}")
                }
                Body::Chat(_) => "none".to_owned(),
            },
        );
        let at = |r: &egui::Rect| {
            format!(
                "{:.0},{:.0},{:.0},{:.0}",
                r.left(),
                r.top(),
                r.width(),
                r.height()
            )
        };
        let direct = match self.current_chat() {
            Some(index) => match &self.tabs[index].body {
                Body::Chat(chat) => format!(
                    "chat {}",
                    chat.layout
                        .iter()
                        .map(|(kind, rect)| format!("{kind}@{}", at(rect)))
                        .chain(
                            chat.menu_rows
                                .iter()
                                .map(|(row, rect)| format!("row:{row}@{}", at(rect)))
                        )
                        .collect::<Vec<_>>()
                        .join(";")
                ),
                Body::Terminal(_) => "none".to_owned(),
            },
            None => format!(
                "start {}",
                self.start_rects
                    .iter()
                    .map(|(name, rect, _)| format!("{name}@{}", at(rect)))
                    .collect::<Vec<_>>()
                    .join(";")
            ),
        };
        let earlier = format!(
            "{} refused={} error={:?} rows=[{}]",
            self.earlier.len(),
            self.earlier_refused,
            self.earlier_error.as_deref().unwrap_or(""),
            self.earlier_rects
                .iter()
                .map(|(name, rect)| format!("{name}@{}", at(rect)))
                .collect::<Vec<_>>()
                .join(";")
        );
        let agents = self
            .start_rects
            .iter()
            .map(|(name, _, state)| format!("{name}:{state}:settled={}", *state != "checking"))
            .collect::<Vec<_>>()
            .join(";");
        format!(
            "open={open} tabs={} [{}] keyboard={} pane={} direct=[{direct}] agents=[{agents}] earlier={earlier} chat=[{chat}] terminal=[{terminal}]",
            tabs.len(),
            tabs.join(", "),
            self.keyboard,
            if self.terminal_open { "open" } else { "closed" },
        )
    }
}
