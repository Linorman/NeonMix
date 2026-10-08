//! Deterministic UI localization regression tests. No native desktop claims.
use super::*;
use neonmix_i18n::{Localizer, ResolvedLocale};

fn fixture() -> Desktop {
    let data: Value = serde_json::from_str(include_str!(
        "../../../docs/evidence/ui-rebuild-20261004/fixtures/full.json"
    ))
    .unwrap();
    let mut app = Desktop::empty(Client::new(".local/test-i18n"), true, false);
    app.load_preview(&data);
    app.snapshot.as_mut().unwrap().runtime_epoch = uuid::Uuid::new_v4();
    app.status.as_mut().unwrap().intent_version = neonmix_desktop_service::INTENT_VERSION;
    app.sync_command_context();
    // The mock backend receives writes. Scheduled network polls are excluded
    // so a locale change can be checked for unexpected control requests.
    app.online = false;
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_style(&ctx);
    theme::fallback_fonts(&ctx);
    ctx
}

fn select(app: &mut Desktop, locale: ResolvedLocale) {
    app.preferences.value.language = LanguagePreference::Explicit(locale.tag().into());
    app.localization.select(&app.preferences.value.language);
}

fn frame(ctx: &egui::Context, app: &mut Desktop) -> egui::FullOutput {
    frame_with_input(ctx, app, egui::vec2(1100.0, 760.0), Vec::new())
}

fn frame_with_input(
    ctx: &egui::Context,
    app: &mut Desktop,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    frame_at_scale(ctx, app, size, events, 1.0)
}

fn frame_at_scale(
    ctx: &egui::Context,
    app: &mut Desktop,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    scale: f32,
) -> egui::FullOutput {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        events,
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(scale);
    ctx.run(input, |ctx| {
        assert_eq!(
            ctx.screen_rect().size(),
            size,
            "headless logical viewport changed"
        );
        assert_eq!(ctx.pixels_per_point(), scale);
        // RawInput.screen_rect is logical points; native scale gives a
        // 600 × 440 logical viewport 1200 × 880 physical pixels at 200%.
        app.localization
            .frame(ctx, &app.preferences.value.language, true);
        app.show(ctx);
    })
}

#[test]
fn fifty_language_switches_preserve_drafts_targets_timers_and_backend_sessions() {
    let ctx = context();
    let mut app = fixture();
    let (work, requests) = mpsc::sync_channel(128);
    app.worker = Some(work);
    app.page = Page::Hub;
    app.room = "未保存的房间 🎧".into();
    app.search = "中文搜索 { $name }".into();
    app.airplay_name_drafts
        .insert("source:pending-source".into(), "未保存的来源别名".into());
    app.panels.insert("invite", true);
    app.panels.insert("airplay", false);
    app.selected_lane = app
        .snapshot
        .as_ref()
        .unwrap()
        .streams
        .keys()
        .next()
        .copied();
    let key = app.selected_lane.unwrap();
    app.lane_details.insert(key);
    app.gain_drafts.insert(key, 3.5);
    let undo_at = Instant::now() + Duration::from_secs(60);
    app.undo = Some(Undo {
        context: app.intents.current.clone().unwrap(),
        target: app
            .target_for(&Write::Control(Operation::StreamMix {
                stream_id: key,
                gain_db: Some(3.5),
                muted: None,
                solo: None,
            }))
            .unwrap(),
        after: Write::Control(Operation::StreamMix {
            stream_id: key,
            gain_db: Some(3.5),
            muted: None,
            solo: None,
        }),
        request_id: uuid::Uuid::new_v4(),
        label: Message::LaneGainChanged {
            name: "中文来源".into(),
            before: "0.0".into(),
            after: "+3.5".into(),
        },
        inverse: Write::Control(Operation::StreamMix {
            stream_id: key,
            gain_db: Some(0.0),
            muted: None,
            solo: None,
        }),
        at: undo_at,
    });
    let snapshot = app.snapshot.as_ref().unwrap();
    let target = snapshot
        .devices
        .values()
        .find(|d| d.role == Role::Member)
        .unwrap()
        .id;
    let confirm_request = Request::Control {
        credential: app.credential.clone(),
        hub: app.hub(),
        expected_revision: snapshot.revision,
        operation: Operation::Revoke { device_id: target },
    };
    let expected_request = serde_json::to_value(&confirm_request).unwrap();
    app.confirmation(
        Message::DevicesRevokeTitle {
            name: "中文设备".into(),
        },
        Message::DevicesEndThisDeviceSMediaAndControlConnections,
        confirm_request,
        egui::Id::new("persistent-confirmation-origin"),
    );
    app.message = Message::FaultRevisionConflict;
    app.error = true;
    select(&mut app, ResolvedLocale::ZhCn);
    frame(&ctx, &mut app);
    let message_since = app.message_since;
    let session = serde_json::to_value(app.snapshot.as_ref().unwrap()).unwrap();
    let drafts = app.airplay_name_drafts.clone();
    let panels = app.panels.clone();
    let original_events = app.events.len();
    for index in 0..50 {
        let locale = if index % 2 == 0 {
            ResolvedLocale::En
        } else {
            ResolvedLocale::ZhCn
        };
        select(&mut app, locale);
        frame(&ctx, &mut app);
        assert_eq!(app.localization.locale, locale);
        assert_eq!(app.page, Page::Hub);
        assert_eq!(app.selected_lane, Some(key));
        assert_eq!(app.search, "中文搜索 { $name }");
        assert_eq!(app.room, "未保存的房间 🎧");
        assert_eq!(app.airplay_name_drafts, drafts);
        assert_eq!(app.panels, panels);
        assert!(app.lane_details.contains(&key));
        assert_eq!(app.gain_drafts.get(&key), Some(&3.5));
        assert_eq!(app.undo.as_ref().unwrap().at, undo_at);
        assert_eq!(
            serde_json::to_value(&app.confirm.as_ref().unwrap().request).unwrap(),
            expected_request
        );
        assert_eq!(app.message_since, message_since);
        assert_eq!(app.events.len(), original_events);
        assert_eq!(
            serde_json::to_value(app.snapshot.as_ref().unwrap()).unwrap(),
            session
        );
        assert!(
            requests.try_recv().is_err(),
            "locale change sent backend work"
        );
        assert!(app.localization.renderer.diagnostics().is_empty());
    }
}

#[test]
fn actions_started_in_chinese_finish_in_the_current_english_locale() {
    for failed in [false, true] {
        let ctx = context();
        let mut app = fixture();
        let (work, requests) = mpsc::sync_channel(1);
        app.worker = Some(work);
        let (complete, results) = mpsc::sync_channel(1);
        app.results = Some(results);
        select(&mut app, ResolvedLocale::ZhCn);
        frame(&ctx, &mut app);
        app.request(Request::TestTone {
            output: app.output.clone(),
        });
        let Work::Action(request) = requests.try_recv().unwrap() else {
            panic!("expected action")
        };
        assert_eq!(app.message, Message::ShellProcessing);
        select(&mut app, ResolvedLocale::En);
        frame(&ctx, &mut app);
        assert!(app.busy);
        if failed {
            complete
                .send(Err(UiError {
                    fault: Some(neonmix_desktop_service::Fault::new(
                        neonmix_desktop_service::FaultCode::PermissionDenied,
                    )),
                }))
                .unwrap();
        } else {
            complete
                .send(Ok(Outcome::Action(Box::new(request), Value::Null)))
                .unwrap();
        }
        frame(&ctx, &mut app);
        let expected = if failed {
            Message::FaultPermissionDenied
        } else {
            Message::ShellCompleted
        };
        assert_eq!(app.message, expected);
        assert_eq!(
            app.tr(&expected),
            Localizer::new(ResolvedLocale::En).render(&expected)
        );
        assert_ne!(
            app.tr(&expected),
            Localizer::new(ResolvedLocale::ZhCn).render(&expected)
        );
        assert!(!app.busy);
        assert!(requests.try_recv().is_err());
    }
}

#[test]
fn current_frame_accessibility_names_change_while_source_node_ids_stay_stable() {
    let ctx = context();
    ctx.enable_accesskit();
    let mut app = fixture();
    let snapshot = app.snapshot.as_mut().unwrap();
    let stream = snapshot.streams.values().next().unwrap();
    snapshot.devices.get_mut(&stream.device_id).unwrap().name = "中文来源 🎧 { $status }".into();
    let mut nodes = Vec::new();
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        select(&mut app, locale);
        let output = frame(&ctx, &mut app);
        let tree = output.platform_output.accesskit_update.unwrap();
        let kind = app.tr(&Message::FlowNativeSender);
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, node)| {
                node.label().is_some_and(|label| {
                    label.contains("中文来源 🎧 { $status }") && label.contains(&kind)
                })
            })
            .expect("localized signal source is missing from current accessibility tree");
        nodes.push((*id, node.label().unwrap().to_owned()));
    }
    assert_eq!(nodes[0].0, nodes[1].0);
    assert_ne!(nodes[0].1, nodes[1].1);
    assert!(
        nodes
            .iter()
            .all(|(_, label)| label.contains("中文来源 🎧 { $status }"))
    );
}

#[test]
fn field_focus_selection_disclosure_and_scroll_persist_across_rendered_language_frames() {
    let ctx = context();
    let mut localizer = Localizer::new(ResolvedLocale::ZhCn);
    let mut draft = "未保存中文草稿".to_owned();
    let render = |localizer: &Localizer, draft: &mut String| {
        let mut ids = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 440.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let label = localizer.render(&Message::HubRoomName);
                    let field = widgets::field(ui, "i18n-room-draft", &label, draft, false);
                    let disclosure_id = ui.make_persistent_id("i18n-disclosure");
                    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                        ctx,
                        disclosure_id,
                        true,
                    );
                    state
                        .show_header(ui, |ui| {
                            ui.label(localizer.render(&Message::HubInviteDevice))
                        })
                        .body(|ui| ui.add_space(40.0));
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt("i18n-scroll")
                        .max_height(160.0)
                        .show(ui, |ui| ui.add_space(1000.0));
                    ids = Some((field.id, disclosure_id, scroll.id));
                });
            },
        );
        ids.unwrap()
    };
    let ids = render(&localizer, &mut draft);
    ctx.memory_mut(|memory| memory.request_focus(ids.0));
    let mut edit = egui::TextEdit::load_state(&ctx, ids.0).unwrap();
    let selection =
        egui::text::CCursorRange::two(egui::text::CCursor::new(1), egui::text::CCursor::new(3));
    edit.cursor.set_char_range(Some(selection));
    edit.store(&ctx, ids.0);
    let mut disclosure = egui::collapsing_header::CollapsingState::load(&ctx, ids.1).unwrap();
    disclosure.set_open(false);
    disclosure.store(&ctx);
    let mut scroll = egui::scroll_area::State::load(&ctx, ids.2).unwrap();
    scroll.offset.y = 120.0;
    scroll.store(&ctx, ids.2);
    localizer.set_locale(ResolvedLocale::En);
    let next_ids = render(&localizer, &mut draft);
    assert_eq!(ids, next_ids);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(ids.0));
    assert_eq!(
        egui::TextEdit::load_state(&ctx, ids.0)
            .unwrap()
            .cursor
            .char_range(),
        Some(selection)
    );
    assert!(
        !egui::collapsing_header::CollapsingState::load(&ctx, ids.1)
            .unwrap()
            .is_open()
    );
    assert!(
        (egui::scroll_area::State::load(&ctx, ids.2)
            .unwrap()
            .offset
            .y
            - 120.0)
            .abs()
            < 0.1
    );
    assert_eq!(draft, "未保存中文草稿");
}

const SURFACES: [(Page, bool); 8] = [
    (Page::Live, false),
    (Page::Mixer, false),
    (Page::Hub, false),
    (Page::Sender, false),
    (Page::Devices, false),
    (Page::Diagnostics, false),
    (Page::About, false),
    (Page::Hub, true),
];

fn assert_accessible_window_actions(output: &egui::FullOutput, app: &Desktop, size: egui::Vec2) {
    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
    let quit = app.tr(&Message::ShellQuit);
    let node = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some(quit.as_str()))
        .expect("sidebar quit action missing from current accessibility tree")
        .1
        .clone();
    let bounds = node.bounds().expect("quit action bounds");
    let sidebar = if size.x < 820.0 { 164.0 } else { 212.0 };
    assert!(
        bounds.x0 >= 0.0 && bounds.x1 <= sidebar + 1.0,
        "{:?} {:?}: quit action extends outside sidebar: {bounds:?}",
        app.localization.locale,
        size
    );
    assert!(
        bounds.y0 >= 0.0 && bounds.y1 <= f64::from(size.y),
        "{:?} {:?}: quit action extends outside window: {bounds:?}",
        app.localization.locale,
        size
    );
    let title = app.tr(&app.page.title());
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.value() == Some(title.as_str())
                || node.label() == Some(title.as_str())),
        "page title missing in {:?} {:?}",
        app.page,
        app.localization.locale
    );
    assert!(app.localization.renderer.diagnostics().is_empty());
}

#[test]
fn shell_actions_fit_with_long_room_names_in_both_languages() {
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        for (size, scale) in [
            (egui::vec2(600.0, 440.0), 1.0),
            (egui::vec2(600.0, 440.0), 2.0),
            (egui::vec2(1100.0, 760.0), 1.0),
            (egui::vec2(1100.0, 760.0), 2.0),
        ] {
            let ctx = context();
            ctx.enable_accesskit();
            let mut app = fixture();
            select(&mut app, locale);
            app.remote_room =
                Some("Studio · 音频制作与混音监听房间 — a very long room name".repeat(3));
            for page in [Page::Live, Page::Hub, Page::Mixer, Page::Diagnostics] {
                app.page = page;
                let output = frame_at_scale(&ctx, &mut app, size, Vec::new(), scale);
                let tree = output.platform_output.accesskit_update.unwrap();
                let sidebar = if size.x < 820.0 { 164.0 } else { 212.0 };
                let quick = app.tr(&Message::ShellQuickActions {
                    shortcut: widgets::command_hint("K"),
                });
                let mute = app.tr(&Message::ShellMasterMute);
                let title = app.tr(&page.title());
                let title_node = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| {
                        (node.label() == Some(title.as_str())
                            || node.value() == Some(title.as_str()))
                            && node
                                .bounds()
                                .is_some_and(|b| b.x0 >= sidebar && b.y0 < 50.0)
                    })
                    .expect("page heading must remain visible above the content");
                let title_bounds = title_node.1.bounds().unwrap();
                assert!(title_bounds.x1 <= f64::from(size.x));
                for label in [&quick, &mute] {
                    let nodes: Vec<_> = tree
                        .nodes
                        .iter()
                        .filter(|(_, node)| {
                            node.label() == Some(label.as_str())
                                && node.bounds().is_some_and(|b| b.y0 < 130.0)
                        })
                        .collect();
                    if label == &quick || page != Page::Mixer {
                        assert!(!nodes.is_empty(), "missing {label}: {locale:?} {page:?}");
                    }
                    for (_, node) in nodes {
                        let b = node.bounds().unwrap();
                        assert!(
                            b.x0 >= sidebar && b.x1 <= f64::from(size.x),
                            "{locale:?} {page:?} {size:?}: {label} outside content: {b:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn shell_identity_and_undo_stay_inside_their_panels() {
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        for size in [egui::vec2(600.0, 440.0), egui::vec2(1100.0, 760.0)] {
            let ctx = context();
            ctx.enable_accesskit();
            let mut app = fixture();
            select(&mut app, locale);
            app.undo = Some(Undo {
                context: app.intents.current.clone().unwrap(),
                target: intent::TargetKey::Master,
                request_id: uuid::Uuid::new_v4(),
                after: Write::Control(Operation::OutputMix {
                    gain_db: Some(app.snapshot.as_ref().unwrap().output.gain_db),
                    muted: None,
                }),
                label: Message::LaneGainChanged {
                    name: "An extremely long studio audio source name 中文设备".repeat(4),
                    before: "0.0".into(),
                    after: "+3.5".into(),
                },
                inverse: Write::Control(Operation::OutputMix {
                    gain_db: Some(0.0),
                    muted: None,
                }),
                at: Instant::now() + Duration::from_secs(30),
            });
            let undo = app.undo.as_ref().unwrap();
            app.intents.history.push_back(intent::Intent {
                id: undo.request_id,
                context: undo.context.clone(),
                route: intent::Route {
                    credential: app.credential.clone(),
                    hub: app.hub(),
                },
                target: undo.target.clone(),
                write: undo.after.clone(),
                label: Some(undo.label.clone()),
                guard: None,
                frozen: None,
                created_at: Instant::now(),
                phase: intent::Phase::Acknowledged,
            });
            assert!(app.undo_available());
            let output = frame_with_input(&ctx, &mut app, size, Vec::new());
            let tree = output.platform_output.accesskit_update.unwrap();
            let sidebar = if size.x < 820.0 { 164.0 } else { 212.0 };
            let identity = app.tr(&Message::ShellIdentity);
            let restore = app.tr(&Message::ShellRestoreButton {
                shortcut: widgets::command_hint("Z"),
            });
            let undo_label = app.tr(&Message::ShellAdjusted {
                change: app.tr(&app.undo.as_ref().unwrap().label),
            });
            for label in [&identity, &restore, &undo_label] {
                let nodes: Vec<_> = tree
                    .nodes
                    .iter()
                    .filter(|(_, node)| {
                        node.label() == Some(label.as_str()) || node.value() == Some(label.as_str())
                    })
                    .collect();
                assert!(!nodes.is_empty(), "missing {label}");
                for node in nodes {
                    let b = node.1.bounds().unwrap();
                    let (left, right) = if b.y0 >= f64::from(size.y - 36.0) {
                        (sidebar, f64::from(size.x))
                    } else {
                        (0.0, sidebar)
                    };
                    assert!(
                        b.x0 >= left && b.x1 <= right && b.y1 <= f64::from(size.y),
                        "{locale:?} {size:?}: {label} outside panel: {b:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn dual_language_eight_surfaces_cover_empty_full_busy_and_stale_states() {
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        let ctx = context();
        ctx.enable_accesskit();
        for (page, airplay_only) in SURFACES {
            for state in 0..4 {
                let mut app = fixture();
                app.page = page;
                app.preview = airplay_only;
                app.preview_airplay_only = airplay_only;
                match state {
                    0 => {
                        app.snapshot = None;
                        app.status = None;
                        app.diagnostics = None;
                        app.airplay = None;
                        app.fresh = None;
                        app.synced = None;
                    }
                    2 => {
                        app.busy = true;
                        app.inflight = Some("hub-start");
                    }
                    3 => {
                        let stale = Instant::now() - Duration::from_secs(10);
                        app.fresh = Some(stale);
                        app.synced = Some(stale);
                        app.message = Message::FaultOutputUnavailable;
                        app.error = true;
                    }
                    _ => {}
                }
                select(&mut app, locale);
                let size = egui::vec2(600.0, 440.0);
                eprintln!(
                    "state matrix: {locale:?}, {page:?}, airplay={airplay_only}, state={state}"
                );
                let output = frame_with_input(&ctx, &mut app, size, Vec::new());
                assert_accessible_window_actions(&output, &app, size);
                assert_eq!(app.page, page);
            }
        }
    }
}

#[test]
fn long_user_names_and_double_scale_keep_actions_and_page_names_reachable() {
    let name = "原始用户名称 🎧 { $name } ".repeat(8);
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        let ctx = context();
        ctx.enable_accesskit();
        for (page, airplay_only) in SURFACES {
            for (size, scale) in [
                (egui::vec2(600.0, 440.0), 1.0),
                (egui::vec2(600.0, 440.0), 2.0),
                (egui::vec2(1100.0, 760.0), 2.0),
                (egui::vec2(1600.0, 1000.0), 2.0),
            ] {
                let mut app = fixture();
                app.page = page;
                app.preview = airplay_only;
                app.preview_airplay_only = airplay_only;
                app.room = name.clone();
                app.remote_room = Some(name.clone());
                for device in app.snapshot.as_mut().unwrap().devices.values_mut() {
                    device.name = name.clone();
                }
                for device in &mut app.devices {
                    device.name = name.clone();
                }
                select(&mut app, locale);
                eprintln!(
                    "name/scale matrix: {locale:?}, {page:?}, airplay={airplay_only}, logical={size:?}, scale={scale}"
                );
                let output = frame_at_scale(&ctx, &mut app, size, Vec::new(), scale);
                assert_accessible_window_actions(&output, &app, size);
                assert_eq!(app.room, name);
                assert_eq!(app.remote_room.as_deref(), Some(name.as_str()));
                assert!(
                    app.snapshot
                        .as_ref()
                        .unwrap()
                        .devices
                        .values()
                        .all(|device| device.name == name)
                );
                if page == Page::Live {
                    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
                    assert!(
                        tree.nodes.iter().any(|(_, node)| node
                            .label()
                            .is_some_and(|label| label.contains(&name))),
                        "long original source name was lost in accessibility output"
                    );
                }
            }
        }
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn language_popup_opens_with_keyboard_and_escape_returns_focus_to_same_selector() {
    for locale in [ResolvedLocale::ZhCn, ResolvedLocale::En] {
        let ctx = context();
        ctx.enable_accesskit();
        let mut app = fixture();
        app.page = Page::About;
        app.localization.focus_language = true;
        select(&mut app, locale);
        frame(&ctx, &mut app);
        let origin = ctx
            .memory(|memory| memory.focused())
            .expect("language selector did not receive focus");
        let preference = app.preferences.value.language.clone();
        assert!(!egui::ComboBox::is_open(&ctx, origin));
        let output = frame_with_input(
            &ctx,
            &mut app,
            egui::vec2(1100.0, 760.0),
            vec![key(egui::Key::Space)],
        );
        assert!(
            egui::ComboBox::is_open(&ctx, origin),
            "focused language selector did not open with Space"
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        for name in ["English", "简体中文"] {
            assert!(
                tree.nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some(name) || node.value() == Some(name)),
                "language popup option {name} absent from accessibility tree"
            );
        }
        frame_with_input(
            &ctx,
            &mut app,
            egui::vec2(1100.0, 760.0),
            vec![key(egui::Key::Escape)],
        );
        assert!(!egui::ComboBox::is_open(&ctx, origin));
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(origin));
        assert_eq!(app.preferences.value.language, preference);
    }
}

#[test]
fn palette_preserves_query_and_selected_command_identity_across_language_switch() {
    let ctx = context();
    ctx.enable_accesskit();
    let mut app = fixture();
    let (work, requests) = mpsc::sync_channel(1);
    app.worker = Some(work);
    select(&mut app, ResolvedLocale::ZhCn);
    app.open_palette();
    frame(&ctx, &mut app);
    frame_with_input(
        &ctx,
        &mut app,
        egui::vec2(1100.0, 760.0),
        vec![egui::Event::Text("room".into())],
    );
    let mut output = frame(&ctx, &mut app);
    let target = app.tr(&Message::PaletteDiscover);
    let mut selected = false;
    for _ in 0..24 {
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        if tree.nodes.iter().any(|(_, node)| {
            node.label() == Some(target.as_str())
                && node.toggled() == Some(egui::accesskit::Toggled::True)
        }) {
            selected = true;
            break;
        }
        output = frame_with_input(
            &ctx,
            &mut app,
            egui::vec2(1100.0, 760.0),
            vec![key(egui::Key::ArrowDown)],
        );
    }
    assert!(selected, "could not select discovery by keyboard");
    select(&mut app, ResolvedLocale::En);
    let output = frame(&ctx, &mut app);
    let tree = output.platform_output.accesskit_update.as_ref().unwrap();
    let target = app.tr(&Message::PaletteDiscover);
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.label() == Some(target.as_str())
                && node.toggled() == Some(egui::accesskit::Toggled::True)),
        "locale change switched the selected command"
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.value() == Some("room")),
        "palette query was reset"
    );
    frame_with_input(
        &ctx,
        &mut app,
        egui::vec2(1100.0, 760.0),
        vec![key(egui::Key::Enter)],
    );
    assert_eq!(app.page, Page::Sender);
    assert!(matches!(
        requests.try_recv().unwrap(),
        Work::Action(Request::Discover { seconds: 3 })
    ));
    assert!(requests.try_recv().is_err());
}
