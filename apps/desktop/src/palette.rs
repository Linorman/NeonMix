//! 快速操作 (⌘K): one searchable list of navigation, room, sending and mixer
//! actions built from the current authoritative state. Only actions the
//! current identity may perform are listed; dangerous ones still confirm.
use super::*;
use crate::widgets::Tone;
use egui::{Align2, CornerRadius, Margin};

#[derive(Default)]
pub struct Palette {
    query: String,
    selected: usize,
    /// IME preedit is active: Enter belongs to the input method, not to us.
    composing: bool,
    focused: bool,
    selected_cmd: Option<Cmd>,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Hash)]
enum Cmd {
    Go(Page),
    LanguageSettings,
    HubStart,
    HubStop,
    SenderStart,
    SenderStop,
    MasterMute(bool),
    LaneMute(u64, bool),
    LaneSolo(u64, bool),
    Restore,
    Invite,
    Discover,
    Export,
    Hide,
    Quit,
}

struct Entry {
    group: String,
    title: String,
    keywords: String,
    hint: Option<String>,
    tone: Tone,
    cmd: Cmd,
}

impl Desktop {
    pub(crate) fn open_palette(&mut self) {
        if self.confirm.is_none() {
            self.palette = Some(Palette::default());
            self.palette_since = Instant::now();
        }
    }

    fn palette_entries(&self) -> Vec<Entry> {
        let mut list = Vec::new();
        let mut push = |group: Message, title: Message, keywords: Message, hint, tone, cmd| {
            list.push(Entry {
                group: self.tr(&group),
                title: self.tr(&title),
                keywords: self.tr(&keywords),
                hint,
                tone,
                cmd,
            });
        };
        for (i, (page, title)) in Page::ALL.into_iter().enumerate() {
            push(
                Message::PaletteGoGroup,
                Message::PaletteGo {
                    page: Box::new(title),
                },
                match page {
                    Page::Live => Message::PaletteAliasLive,
                    Page::Mixer => Message::PaletteAliasMixer,
                    Page::Hub => Message::PaletteAliasHub,
                    Page::Sender => Message::PaletteAliasSender,
                    Page::Devices => Message::PaletteAliasDevices,
                    Page::Diagnostics => Message::PaletteAliasDiagnostics,
                    Page::About => Message::PaletteAliasAbout,
                },
                (i < 6).then(|| widgets::command_hint(&(i + 1).to_string())),
                Tone::Neutral,
                Cmd::Go(page),
            );
        }
        push(
            Message::PaletteDisplayGroup,
            Message::PaletteLanguage,
            Message::PaletteAliasLanguage,
            None,
            Tone::Neutral,
            Cmd::LanguageSettings,
        );
        if self.undo_available()
            && let Some(undo) = &self.undo
        {
            push(
                Message::PaletteMixGroup,
                Message::PaletteRestore {
                    change: Box::new(undo.label.clone()),
                },
                Message::PaletteAliasRestore,
                Some(widgets::command_hint("Z")),
                Tone::Accent,
                Cmd::Restore,
            );
        }
        let status = self.status.as_ref();
        let configured = status.is_some_and(|s| s.hub_settings.is_some());
        let sharing = status.is_some_and(|s| s.hub.running) || self.pending("hub-start");
        if configured && !sharing {
            push(
                Message::PaletteRoomGroup,
                Message::PaletteStartSharing,
                Message::PaletteAliasShareStart,
                None,
                Tone::Success,
                Cmd::HubStart,
            );
        }
        if sharing {
            push(
                Message::PaletteRoomGroup,
                Message::PaletteStopSharing,
                Message::PaletteAliasShareStop,
                None,
                Tone::Danger,
                Cmd::HubStop,
            );
        }
        let sending = status.is_some_and(|s| s.sender.running);
        let binding_on = self
            .binding
            .as_ref()
            .and_then(|b| b["enabled"].as_bool())
            .unwrap_or(false);
        if !sending && binding_on && self.sender_allowed() {
            push(
                Message::PaletteSendGroup,
                Message::PaletteStartSending,
                Message::PaletteAliasSendStart,
                None,
                Tone::Success,
                Cmd::SenderStart,
            );
        }
        if sending {
            push(
                Message::PaletteSendGroup,
                Message::PaletteStopSending,
                Message::PaletteAliasSendStop,
                None,
                Tone::Danger,
                Cmd::SenderStop,
            );
        }
        if let Some(state) = &self.snapshot
            && self.writable()
        {
            if self.controls_room() {
                let muted = state.output.muted;
                push(
                    Message::PaletteMixGroup,
                    if muted {
                        Message::ShellMasterUnmute
                    } else {
                        Message::ShellMasterMute
                    },
                    Message::PaletteAliasMasterMute,
                    None,
                    Tone::Warning,
                    Cmd::MasterMute(!muted),
                );
            }
            for lane in self.lanes(state) {
                if lane.can_mix {
                    push(
                        Message::PaletteMixGroup,
                        if lane.muted {
                            Message::PaletteUnmuteLane {
                                name: lane.name.clone(),
                            }
                        } else {
                            Message::PaletteMuteLane {
                                name: lane.name.clone(),
                            }
                        },
                        Message::PaletteAliasLaneMute,
                        None,
                        Tone::Warning,
                        Cmd::LaneMute(lane.key, !lane.muted),
                    );
                }
                if lane.can_solo {
                    push(
                        Message::PaletteMixGroup,
                        if lane.solo {
                            Message::PaletteUnsoloLane {
                                name: lane.name.clone(),
                            }
                        } else {
                            Message::PaletteSoloLane {
                                name: lane.name.clone(),
                            }
                        },
                        Message::PaletteAliasLaneSolo,
                        None,
                        Tone::Solo,
                        Cmd::LaneSolo(lane.key, !lane.solo),
                    );
                }
            }
            if self.admin() {
                push(
                    Message::PaletteRoomGroup,
                    Message::PaletteInvite,
                    Message::PaletteAliasInvite,
                    None,
                    Tone::Accent,
                    Cmd::Invite,
                );
            }
            push(
                Message::PaletteDiagnosticsGroup,
                Message::PaletteExport,
                Message::PaletteAliasExport,
                None,
                Tone::Neutral,
                Cmd::Export,
            );
        }
        push(
            Message::PaletteSendGroup,
            Message::PaletteDiscover,
            Message::PaletteAliasDiscover,
            None,
            Tone::Neutral,
            Cmd::Discover,
        );
        push(
            Message::PaletteWindowGroup,
            Message::ShellHide,
            Message::PaletteAliasHide,
            None,
            Tone::Neutral,
            Cmd::Hide,
        );
        push(
            Message::PaletteWindowGroup,
            Message::ShellQuit,
            Message::PaletteAliasQuit,
            None,
            Tone::Danger,
            Cmd::Quit,
        );
        list
    }

    pub(crate) fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut palette) = self.palette.take() else {
            return;
        };
        // Track IME composition so Enter that commits a candidate is not
        // taken as "run the highlighted action".
        let mut committed_text = false;
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit(text)) => {
                        palette.composing = !text.is_empty()
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_))
                    | egui::Event::Ime(egui::ImeEvent::Disabled) => {
                        palette.composing = false;
                        committed_text = true;
                    }
                    _ => {}
                }
            }
        });
        let needle = palette.query.trim().to_lowercase();
        let entries: Vec<Entry> = self
            .palette_entries()
            .into_iter()
            .filter(|e| {
                needle.is_empty()
                    || e.title.to_lowercase().contains(&needle)
                    || e.keywords.contains(&needle)
                    || e.group.contains(&needle)
            })
            .collect();
        if palette.generation != self.localization.renderer.generation() {
            palette.selected = palette
                .selected_cmd
                .as_ref()
                .and_then(|cmd| entries.iter().position(|e| &e.cmd == cmd))
                .unwrap_or(0);
            palette.generation = self.localization.renderer.generation();
        }
        if !palette.composing {
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    palette.selected += 1;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    palette.selected = palette.selected.saturating_sub(1);
                }
            });
        }
        palette.selected = palette.selected.min(entries.len().saturating_sub(1));
        // While composing, Enter belongs to the input method: swallow it so
        // the search field keeps focus and receives the committed text.
        if palette.composing {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        let enter = !palette.composing
            && !committed_text
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut run = enter
            .then(|| entries.get(palette.selected).map(|e| e.cmd.clone()))
            .flatten();
        let width = (ctx.screen_rect().width() - 48.0).min(520.0);
        // Opens with a short fade and settle from slightly above.
        let appear = animation::fade_in(ctx, self.palette_since.elapsed(), 0.14);
        let top = (ctx.screen_rect().height() * 0.1).min(64.0) - 8.0 * (1.0 - appear);
        // Reserve search, footer and frame space inside the minimum window.
        let list_height = (ctx.screen_rect().height() - top - 120.0).clamp(80.0, 340.0);
        let response = egui::Modal::new(egui::Id::new("palette"))
            .area(
                egui::Modal::default_area(egui::Id::new("palette-area"))
                    .anchor(Align2::CENTER_TOP, egui::vec2(0.0, top)),
            )
            .backdrop_color(theme::shadow(0.45))
            .frame(crate::shell::overlay_frame(ctx).inner_margin(Margin::same(10)))
            .show(ctx, |ui| {
                ui.set_opacity(0.4 + 0.6 * appear);
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let (icon, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 30.0), egui::Sense::hover());
                    icons::paint(
                        ui.painter(),
                        icons::square(icon.center(), 16.0),
                        icons::Icon::Search,
                        theme::text_3(),
                    );
                    let search_width = (ui.available_width()
                        - if palette.query.is_empty() { 0.0 } else { 32.0 })
                    .max(20.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut palette.query)
                            .id(egui::Id::new("palette-query"))
                            .hint_text(crate::localization::text(ui, &Message::PaletteSearch))
                            .frame(false)
                            .font(egui::FontId::proportional(16.0))
                            .desired_width(search_width),
                    );
                    if !palette.query.is_empty()
                        && widgets::small_button(ui, true, "×")
                            .on_hover_text(crate::localization::text(ui, &Message::CommonClear))
                            .clicked()
                    {
                        palette.query.clear();
                        palette.selected = 0;
                        edit.request_focus();
                        ui.ctx().request_repaint();
                    }
                    if !palette.focused {
                        edit.request_focus();
                        palette.focused = true;
                    }
                    if edit.changed() {
                        palette.selected = 0;
                    }
                });
                ui.separator();
                if entries.is_empty() {
                    ui.add_space(10.0);
                    widgets::note(ui, crate::localization::text(ui, &Message::PaletteEmpty));
                    ui.add_space(6.0);
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut last_group = "";
                        for (i, entry) in entries.iter().enumerate() {
                            if entry.group != last_group {
                                last_group = entry.group.as_str();
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&entry.group)
                                        .size(11.0)
                                        .color(theme::text_3()),
                                );
                            }
                            let selected = i == palette.selected;
                            let (rect, _allocation) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 32.0),
                                egui::Sense::hover(),
                            );
                            let row = ui.interact(
                                rect,
                                ui.make_persistent_id(("palette-command", &entry.cmd)),
                                egui::Sense::click(),
                            );
                            row.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    true,
                                    selected,
                                    &entry.title,
                                )
                            });
                            if row.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                            {
                                palette.selected = i;
                            }
                            if selected {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(8),
                                    theme::text().gamma_multiply(0.08),
                                );
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        rect.left_top() + egui::vec2(0.0, 8.0),
                                        egui::vec2(2.5, rect.height() - 16.0),
                                    ),
                                    CornerRadius::same(2),
                                    theme::accent(),
                                );
                                ui.scroll_to_rect(rect, None);
                            }
                            ui.painter().circle_filled(
                                egui::pos2(rect.left() + 12.0, rect.center().y),
                                3.0,
                                entry.tone.color(),
                            );
                            ui.painter().text(
                                egui::pos2(rect.left() + 24.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                &entry.title,
                                egui::FontId::proportional(theme::BODY),
                                if selected {
                                    theme::text()
                                } else {
                                    theme::text_2()
                                },
                            );
                            if let Some(hint) = &entry.hint {
                                ui.painter().text(
                                    egui::pos2(rect.right() - 10.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    hint,
                                    egui::FontId::monospace(11.0),
                                    theme::text_3(),
                                );
                            }
                            if row.clicked() {
                                run = Some(entry.cmd.clone());
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    widgets::kbd(ui, "↑↓");
                    widgets::note(ui, crate::localization::text(ui, &Message::PaletteSelect));
                    widgets::kbd(ui, "↩");
                    widgets::note(ui, crate::localization::text(ui, &Message::PaletteExecute));
                    widgets::kbd(ui, "esc");
                    widgets::note(ui, crate::localization::text(ui, &Message::PaletteClose));
                });
            });
        if let Some(cmd) = run {
            self.run_command(ctx, cmd);
            return;
        }
        if !response.should_close() {
            palette.selected_cmd = entries.get(palette.selected).map(|e| e.cmd.clone());
            self.palette = Some(palette);
        }
    }

    fn run_command(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Go(page) => self.navigate(page),
            Cmd::LanguageSettings => {
                self.navigate(Page::About);
                self.localization.focus_language = true;
            }
            Cmd::HubStart => self.request(Request::HubStart),
            Cmd::HubStop => self.request(Request::HubStop),
            Cmd::SenderStart => self.request(Request::SenderStart {
                options: SenderOptions {
                    credential: self.credential.clone(),
                    hub: self.hub(),
                    output_binding: PathBuf::from("output"),
                },
            }),
            Cmd::SenderStop => self.stop_sender(),
            Cmd::MasterMute(on) => self.mix_change(
                if on {
                    Message::ShellMasterMute
                } else {
                    Message::ShellMasterUnmute
                },
                Write::Control(Operation::OutputMix {
                    gain_db: None,
                    muted: Some(on),
                }),
                Write::Control(Operation::OutputMix {
                    gain_db: None,
                    muted: Some(!on),
                }),
            ),
            Cmd::LaneMute(id, on) => self.lane_toggle(id, Some(on), None),
            Cmd::LaneSolo(id, on) => self.lane_toggle(id, None, Some(on)),
            Cmd::Restore => self.restore_last(),
            Cmd::Invite => self.request(Request::Invite {
                credential: self.credential.clone(),
                hub: self.hub(),
                out: PathBuf::from(format!("invitations/{}.json", uuid::Uuid::new_v4())),
                seconds: 120,
            }),
            Cmd::Discover => {
                self.navigate(Page::Sender);
                self.request(Request::Discover { seconds: 3 });
            }
            Cmd::Export => self.request(Request::ExportDiagnostics {
                credential: self.credential.clone(),
                hub: self.hub(),
            }),
            Cmd::Hide => {
                if self.tray.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            Cmd::Quit => self.confirmation(
                Message::ShellQuit,
                Message::ShellQuitConsequence,
                Request::Shutdown,
                egui::Id::new("palette-quit"),
            ),
        }
    }
}
