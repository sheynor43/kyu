use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{channel, Receiver, Sender};

use chrono::{Datelike, Local, NaiveDate};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, Ui,
    Vec2,
};

use crate::i18n::{t, T};
use crate::model::{day_of, Conn, Day, Event, Incoming, Item, LinkKind, Origin, Queue, Service, Status};
use crate::parse::find_links;
use crate::sources::{self, Sink, Worker};
use crate::store::{Lang, Reward, Settings, Store};
use crate::theme::{self, icon, ACCENT, BG_DEEP, CARD, MUTED, ON_ACCENT, RAISED, TEXT, TEXT2};

#[derive(Clone, Copy, PartialEq)]
enum View {
    Day(NaiveDate),
    Settings,
}

struct AddDraft {
    link: String,
    user: String,
    queue: Queue,
    error: bool,
}

pub struct App {
    store: Store,
    settings: Settings,
    draft: Settings,
    view: View,
    today: NaiveDate,
    cache: HashMap<NaiveDate, Day>,
    days: BTreeMap<NaiveDate, (usize, usize)>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    conn: HashMap<Service, Conn>,
    workers: Vec<Worker>,
    show_done: bool,
    add: Option<AddDraft>,
    last_id: u64,
    ctx: egui::Context,
    flash: Option<(String, f64)>,
    screenshot: Option<(String, u32)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let store = Store::open();
        let settings = store.load_settings();
        let today = day_of(Local::now(), settings.day_start_hour);
        let (tx, rx) = channel();
        let mut app = Self {
            days: BTreeMap::new(),
            draft: settings.clone(),
            view: View::Day(today),
            today,
            cache: HashMap::new(),
            tx,
            rx,
            conn: HashMap::new(),
            workers: Vec::new(),
            show_done: false,
            add: None,
            last_id: 0,
            ctx: cc.egui_ctx.clone(),
            flash: None,
            screenshot: std::env::var("KYU_SCREENSHOT").ok().map(|p| (p, 0)),
            store,
            settings,
        };
        match std::env::var("KYU_VIEW").as_deref() {
            Ok("settings") => app.view = View::Settings,
            Ok(d) => {
                if let Ok(d) = NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                    app.view = View::Day(d);
                }
            }
            _ => {}
        }
        if std::env::var("KYU_SHOW_DONE").is_ok() {
            app.show_done = true;
        }
        app.prune();
        app.refresh_days();
        if std::env::var("KYU_DEMO").is_ok() {
            // screenshot mode: no network, configured services shown as connected
            for (svc, on) in [
                (Service::Twitch, !app.settings.twitch_channel.is_empty()),
                (Service::DonationAlerts, !app.settings.da_token.is_empty()),
                (Service::DonatePay, !app.settings.dp_token.is_empty()),
            ] {
                if on {
                    app.conn.insert(svc, Conn::Online);
                }
            }
        } else {
            app.start_workers();
        }
        app
    }

    fn t(&self) -> &'static T {
        t(self.settings.lang)
    }

    fn start_workers(&mut self) {
        self.workers.clear();
        self.conn.clear();
        let sink = Sink::new(self.tx.clone(), self.ctx.clone());
        let s = &self.settings;
        if !s.twitch_channel.trim().is_empty() {
            let (ch, sk) = (s.twitch_channel.clone(), sink.clone());
            self.workers.push(sources::spawn(move |stop| sources::twitch::run(ch, sk, &stop)));
        }
        if !s.da_token.trim().is_empty() {
            let (tok, sk) = (s.da_token.clone(), sink.clone());
            self.workers.push(sources::spawn(move |stop| sources::donationalerts::run(tok, sk, &stop)));
        }
        if !s.dp_token.trim().is_empty() {
            let (tok, cur, sk) = (s.dp_token.clone(), s.dp_cursor, sink.clone());
            self.workers.push(sources::spawn(move |stop| sources::donatepay::run(tok, cur, sk, &stop)));
        }
    }

    fn prune(&mut self) {
        if self.settings.auto_delete {
            self.store.prune(self.today, self.settings.keep_days.max(1));
            self.cache.retain(|d, _| *d >= self.today - chrono::Duration::days(self.settings.keep_days as i64));
        }
    }

    fn refresh_days(&mut self) {
        self.days = self.store.list_days();
        self.days.entry(self.today).or_insert((0, 0));
    }

    fn day(&mut self, d: NaiveDate) -> &Day {
        self.cache.entry(d).or_insert_with(|| self.store.load_day(d))
    }

    fn edit_day(&mut self, d: NaiveDate, f: impl FnOnce(&mut Day)) {
        let store = &self.store;
        let day = self.cache.entry(d).or_insert_with(|| store.load_day(d));
        f(day);
        store.save_day(d, day);
        self.days.insert(d, (day.pending(), day.items.len()));
    }

    fn next_id(&mut self) -> u64 {
        let now = Local::now().timestamp_micros() as u64;
        self.last_id = now.max(self.last_id + 1);
        self.last_id
    }

    // ---- incoming messages ----

    fn poll_events(&mut self) {
        while let Ok(e) = self.rx.try_recv() {
            match e {
                Event::Message(m) => self.ingest(m),
                Event::Status(s, c) => {
                    self.conn.insert(s, c);
                }
                Event::RewardSeen { id, sample } => {
                    let known = &mut self.settings.known_rewards;
                    match known.iter_mut().find(|r| r.id == id) {
                        Some(r) => r.sample = sample,
                        None => known.push(Reward { id, sample }),
                    }
                    self.draft.known_rewards = known.clone();
                    self.store.save_settings(&self.settings);
                }
                Event::DonatePayCursor(c) => {
                    self.settings.dp_cursor = c;
                    self.draft.dp_cursor = c;
                    self.store.save_settings(&self.settings);
                }
            }
        }
    }

    /// Decides which queue a message lands in, then adds one item per link.
    fn ingest(&mut self, m: Incoming) {
        let s = &self.settings;
        let rules = s.link_rules();
        let links = find_links(&m.text, &rules);
        if links.is_empty() {
            return;
        }
        let mut queue = m.origin.queue();
        match &m.origin {
            Origin::Reward { reward_id } if !s.reward_ids.is_empty() && !s.reward_ids.contains(reward_id) => {
                queue = Queue::Free
            }
            Origin::Bits { bits } if !s.bits_enabled || *bits < s.bits_min => queue = Queue::Free,
            Origin::DonationAlerts { amount, .. } | Origin::DonatePay { amount, .. } if *amount < s.donation_min => {
                queue = Queue::Free
            }
            _ => {}
        }
        let (one_per_viewer, no_dupes) = (s.one_per_viewer, s.ignore_duplicates);
        let today = self.today;
        let now = Local::now();
        let message = strip_links(&m.text, &rules);

        for link in links {
            let id = self.next_id();
            let user = m.user.clone();
            let origin = m.origin.clone();
            let message = message.clone();
            self.edit_day(today, |day| {
                let pending = || day.items.iter().filter(|i| i.status == Status::Pending);
                if queue == Queue::Free && m.origin != Origin::Manual {
                    if one_per_viewer && pending().any(|i| i.queue == Queue::Free && i.user.eq_ignore_ascii_case(&user)) {
                        return;
                    }
                }
                if no_dupes {
                    if let Some(pos) = day.items.iter().position(|i| i.status == Status::Pending && i.url == link.url) {
                        // A paid repeat of a free link moves it up instead of adding a copy.
                        let old = &mut day.items[pos];
                        if old.queue == Queue::Free && queue != Queue::Free {
                            old.queue = queue;
                            old.origin = origin;
                            old.user = user;
                            if !message.is_empty() {
                                old.message = message;
                            }
                        }
                        return;
                    }
                }
                day.items.push(Item {
                    id,
                    received: now,
                    queue,
                    origin,
                    user,
                    kind: link.kind,
                    url: link.url,
                    message,
                    status: Status::Pending,
                    done_at: None,
                });
            });
        }
    }

    fn set_status(&mut self, d: NaiveDate, id: u64, st: Status) {
        self.edit_day(d, |day| {
            if let Some(i) = day.items.iter_mut().find(|i| i.id == id) {
                i.status = st;
                i.done_at = (st != Status::Pending).then(Local::now);
            }
        });
    }

    fn delete_item(&mut self, d: NaiveDate, id: u64) {
        self.edit_day(d, |day| day.items.retain(|i| i.id != id));
    }

    fn tick_day(&mut self) {
        let today = day_of(Local::now(), self.settings.day_start_hour);
        if today != self.today {
            let was_today = self.view == View::Day(self.today);
            self.today = today;
            self.prune();
            self.refresh_days();
            if was_today {
                self.view = View::Day(today);
            }
        }
    }

    // ---- ui ----

    fn sidebar(&mut self, ui: &mut Ui) {
        let tr = self.t();
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            mark(ui, r.center(), 8.0);
            ui.label(RichText::new("Kyu").family(theme::medium()).size(17.0).color(TEXT));
        });
        ui.add_space(24.0);

        section_label(ui, tr.days);
        let days: Vec<_> = self.days.iter().rev().map(|(d, c)| (*d, *c)).collect();
        let bottom_h = 32.0 * 4.0 + 64.0;
        ScrollArea::vertical()
            .max_height((ui.available_height() - bottom_h).max(64.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (d, (pending, total)) in days {
                    let label = if d == self.today { tr.today.to_string() } else { short_date(tr, d, self.today) };
                    let meta = if pending > 0 { pending.to_string() } else if total > 0 { icon::CHECK.into() } else { String::new() };
                    if nav_row(ui, None, &label, &meta, self.view == View::Day(d)).clicked() {
                        self.view = View::Day(d);
                    }
                }
            });

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            if nav_row(ui, Some(icon::SETTINGS), tr.settings, "", self.view == View::Settings).clicked() {
                self.draft = self.settings.clone();
                self.view = View::Settings;
            }
            ui.add_space(12.0);
            for (svc, name, configured) in [
                (Service::DonatePay, "DonatePay", !self.settings.dp_token.trim().is_empty()),
                (Service::DonationAlerts, "DonationAlerts", !self.settings.da_token.trim().is_empty()),
                (Service::Twitch, "Twitch", !self.settings.twitch_channel.trim().is_empty()),
            ] {
                let c = self.conn.get(&svc).cloned().unwrap_or(Conn::Off);
                status_row(ui, tr, name, configured, &c);
            }
            ui.add_space(4.0);
            section_label(ui, tr.connections);
        });
    }

    fn day_view(&mut self, ui: &mut Ui, d: NaiveDate) {
        let tr = self.t();
        let (pending, total) = self.days.get(&d).copied().unwrap_or((0, 0));

        // header
        ui.horizontal(|ui| {
            ui.set_height(40.0);
            let title = if d == self.today { tr.today.to_string() } else { long_date(tr, d) };
            ui.label(RichText::new(title).family(theme::medium()).size(24.0).color(TEXT));
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("{pending} {} · {} {}", tr.pending, total - pending, tr.done))
                    .monospace()
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if d == self.today && button(ui, Some(icon::PLUS), tr.add, false).clicked() {
                    self.add = Some(AddDraft { link: String::new(), user: String::new(), queue: Queue::Free, error: false });
                }
            });
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut changed = false;
            for q in Queue::ALL {
                let on = self.settings.shows(q);
                if toggle_chip(ui, theme::queue_dot(q), queue_name(tr, q), on).clicked() {
                    self.settings.toggle(q);
                    changed = true;
                }
            }
            if changed {
                self.draft.show_paid = self.settings.show_paid;
                self.draft.show_points = self.settings.show_points;
                self.draft.show_free = self.settings.show_free;
                self.store.save_settings(&self.settings);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (ic, label) = if self.show_done { (icon::EYE_OFF, tr.hide_done) } else { (icon::EYE, tr.show_done) };
                if toggle_chip_icon(ui, ic, label, self.show_done).clicked() {
                    self.show_done = !self.show_done;
                }
            });
        });
        ui.add_space(20.0);

        let visible: Vec<Queue> = Queue::ALL.into_iter().filter(|q| self.settings.shows(*q)).collect();
        if visible.is_empty() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| ui.label(RichText::new(tr.all_hidden).color(MUTED)));
            return;
        }

        let show_done = self.show_done;
        let items: Vec<Item> = self.day(d).items.clone();
        let mut actions: Vec<(u64, Action)> = Vec::new();
        let gap = 16.0;
        let n = visible.len() as f32;
        let col_w = ((ui.available_width() - gap * (n - 1.0)) / n).floor();
        let col_h = ui.available_height();

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for q in &visible {
                let mut list: Vec<&Item> = items
                    .iter()
                    .filter(|i| i.queue == *q && (show_done || i.status == Status::Pending))
                    .collect();
                // pending first in arrival order, finished ones after, most recent first
                list.sort_by_key(|i| (i.status != Status::Pending, if i.status == Status::Pending { i.id as i128 } else { -(i.id as i128) }));
                let count = items.iter().filter(|i| i.queue == *q && i.status == Status::Pending).count();
                ui.allocate_ui_with_layout(Vec2::new(col_w, col_h), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(col_w);
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        dot(ui, theme::queue_dot(*q));
                        ui.label(RichText::new(queue_name(tr, *q)).family(theme::medium()).color(TEXT));
                        ui.label(RichText::new(count.to_string()).monospace().color(MUTED));
                    });
                    ui.add_space(8.0);
                    ScrollArea::vertical().id_salt(("col", *q as u8)).auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        if list.is_empty() {
                            Frame::new()
                                .stroke(Stroke::new(1.0, theme::HAIRLINE))
                                .corner_radius(CornerRadius::same(12))
                                .inner_margin(Margin::same(20))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(tr.empty).color(MUTED));
                                });
                        }
                        for it in list {
                            if let Some(a) = card(ui, tr, it) {
                                actions.push((it.id, a));
                            }
                        }
                    });
                });
            }
        });

        for (id, a) in actions {
            match a {
                Action::Watched => self.set_status(d, id, Status::Watched),
                Action::Skip => self.set_status(d, id, Status::Skipped),
                Action::Undo => self.set_status(d, id, Status::Pending),
                Action::Delete => self.delete_item(d, id),
                Action::Copied => self.flash(self.t().copy),
            }
        }
    }

    fn flash(&mut self, msg: &str) {
        self.flash = Some((msg.to_string(), self.ctx.input(|i| i.time) + 1.6));
    }

    fn settings_view(&mut self, ui: &mut Ui) {
        let tr = self.t();
        let mut save = false;
        let mut cancel = false;
        let dirty = self.draft != self.settings;
        ui.horizontal(|ui| {
            ui.set_height(40.0);
            ui.label(RichText::new(tr.settings).family(theme::medium()).size(24.0).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                save = button(ui, None, tr.save, dirty).clicked();
                cancel = button(ui, None, tr.cancel, false).clicked();
            });
        });
        ui.add_space(20.0);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(640.0);
            let d = &mut self.draft;

            group(ui, tr.s_twitch, |ui| {
                field(ui, tr.s_channel, None, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.twitch_channel).hint_text(tr.s_channel_hint).desired_width(f32::INFINITY));
                });
                field(ui, tr.s_rewards, Some(tr.s_rewards_help), |ui| {
                    let mut any = d.reward_ids.is_empty();
                    if ui.checkbox(&mut any, tr.s_any_reward).changed() && any {
                        d.reward_ids.clear();
                    }
                    if d.known_rewards.is_empty() {
                        ui.label(RichText::new(tr.s_no_rewards_yet).color(MUTED).small());
                    }
                    for r in d.known_rewards.clone() {
                        let mut on = d.reward_ids.contains(&r.id);
                        let label = if r.sample.is_empty() { r.id.clone() } else { format!("«{}»", r.sample) };
                        let resp = ui.checkbox(&mut on, RichText::new(label).color(TEXT2)).on_hover_text(&r.id);
                        if resp.changed() {
                            if on {
                                d.reward_ids.push(r.id);
                            } else {
                                d.reward_ids.retain(|x| *x != r.id);
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.bits_enabled, tr.s_bits);
                    ui.add_enabled(d.bits_enabled, egui::DragValue::new(&mut d.bits_min).range(1..=100000));
                    ui.label(RichText::new(tr.s_bits_min).color(MUTED));
                });
            });

            group(ui, tr.s_donations, |ui| {
                field(ui, tr.s_da_token, Some(tr.s_da_help), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.da_token).password(true).desired_width(f32::INFINITY));
                });
                field(ui, tr.s_dp_token, Some(tr.s_dp_help), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.dp_token).password(true).desired_width(f32::INFINITY));
                });
                field(ui, tr.s_donation_min, Some(tr.s_donation_min_help), |ui| {
                    ui.add(egui::DragValue::new(&mut d.donation_min).range(0.0..=1_000_000.0).speed(10.0));
                });
            });

            group(ui, tr.s_links, |ui| {
                ui.label(RichText::new(tr.s_links_help).small().color(MUTED));
                ui.checkbox(&mut d.link_servers, tr.s_link_servers);
                ui.checkbox(&mut d.link_cloud, tr.s_link_cloud);
                ui.checkbox(&mut d.link_video, tr.s_link_video);
                field(ui, tr.s_extra_servers, Some(tr.s_extra_servers_help), |ui| {
                    ui.add_enabled(
                        d.link_servers,
                        egui::TextEdit::singleline(&mut d.extra_servers).hint_text("my-server.xyz, other.net").desired_width(f32::INFINITY),
                    );
                });
            });

            group(ui, tr.s_queue, |ui| {
                ui.checkbox(&mut d.one_per_viewer, tr.s_one_per_viewer);
                ui.checkbox(&mut d.ignore_duplicates, tr.s_duplicates);
                field(ui, tr.s_day_start, Some(tr.s_day_start_help), |ui| {
                    ui.add(egui::DragValue::new(&mut d.day_start_hour).range(0..=12).custom_formatter(|v, _| format!("{:02}:00", v as u32)));
                });
            });

            group(ui, tr.s_storage, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.auto_delete, tr.s_auto_delete);
                    ui.add_enabled(d.auto_delete, egui::DragValue::new(&mut d.keep_days).range(1..=3650));
                    ui.label(RichText::new(tr.s_keep_days).color(TEXT2));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if button(ui, Some(icon::FOLDER), tr.s_open_folder, false).clicked() {
                        let _ = open::that(&self.store.root);
                    }
                    ui.label(RichText::new(self.store.root.display().to_string()).monospace().color(MUTED));
                });
            });

            group(ui, tr.s_language, |ui| {
                ui.horizontal(|ui| {
                    for (l, name) in [(Lang::Ru, "Русский"), (Lang::En, "English")] {
                        if toggle_chip(ui, Color32::TRANSPARENT, name, d.lang == l).clicked() {
                            d.lang = l;
                        }
                    }
                });
            });

        });

        if cancel {
            self.draft = self.settings.clone();
            self.view = View::Day(self.today);
        }
        if save {
            let reconnect = self.draft.twitch_channel != self.settings.twitch_channel
                || self.draft.da_token != self.settings.da_token
                || self.draft.dp_token != self.settings.dp_token;
            let draft = std::mem::take(&mut self.draft);
            self.settings = Settings { dp_cursor: self.settings.dp_cursor, known_rewards: self.settings.known_rewards.clone(), ..draft };
            if self.settings.dp_token != self.store.load_settings().dp_token {
                self.settings.dp_cursor = 0;
            }
            self.draft = self.settings.clone();
            self.store.save_settings(&self.settings);
            self.today = day_of(Local::now(), self.settings.day_start_hour);
            self.prune();
            self.refresh_days();
            if reconnect {
                self.start_workers();
            }
            self.flash(self.t().saved);
        }
    }

    fn add_dialog(&mut self, ctx: &egui::Context) {
        let tr = self.t();
        let Some(a) = &mut self.add else { return };
        let mut close = false;
        let mut submit = false;
        let modal = egui::Modal::new(egui::Id::unique("add")).frame(
            Frame::new().fill(CARD).stroke(theme::hairline()).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(24)),
        );
        let resp = modal.show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(tr.a_title).family(theme::medium()).size(17.0).color(TEXT));
            ui.add_space(12.0);
            field(ui, tr.a_link, None, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut a.link).hint_text("https://…").desired_width(f32::INFINITY));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });
            field(ui, tr.a_user, None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut a.user).desired_width(f32::INFINITY));
            });
            field(ui, tr.a_queue, None, |ui| {
                ui.horizontal(|ui| {
                    for q in Queue::ALL {
                        if toggle_chip(ui, theme::queue_dot(q), queue_name(tr, q), a.queue == q).clicked() {
                            a.queue = q;
                        }
                    }
                });
            });
            if a.error {
                ui.label(RichText::new(tr.a_no_link).color(theme::NEGATIVE));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                submit |= button(ui, Some(icon::PLUS), tr.add, true).clicked();
                close |= button(ui, None, tr.cancel, false).clicked();
            });
        });
        if resp.should_close() {
            close = true;
        }
        if submit {
            if find_links(&a.link, &self.settings.link_rules()).is_empty() {
                a.error = true;
            } else {
                let user = if a.user.trim().is_empty() { "—".to_string() } else { a.user.trim().to_string() };
                let inc = Incoming { origin: Origin::Manual, user, text: a.link.clone() };
                let q = a.queue;
                self.add = None;
                let before: Vec<u64> = self.day(self.today).items.iter().map(|i| i.id).collect();
                self.ingest(inc);
                // manual items keep the queue the streamer picked
                self.edit_day(self.today, |day| {
                    for i in day.items.iter_mut().filter(|i| !before.contains(&i.id)) {
                        i.queue = q;
                    }
                });
                return;
            }
        }
        if close {
            self.add = None;
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, frames)) = &mut self.screenshot else { return };
        *frames += 1;
        if *frames == 10 {
            let (path, c) = (path.clone(), ctx.clone());
            ctx.request_screenshot(move |image| {
                let [w, h] = image.size;
                let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
                for p in &image.pixels {
                    out.extend_from_slice(&[p.r(), p.g(), p.b()]);
                }
                let _ = std::fs::write(&path, out);
                c.send_viewport_cmd(egui::ViewportCommand::Close);
                c.request_repaint();
            });
        }
        ctx.request_repaint();
    }
}

enum Action {
    Watched,
    Skip,
    Undo,
    Delete,
    Copied,
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.tick_day();
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("sidebar")
            .exact_size(240.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG_DEEP).inner_margin(Margin::symmetric(12, 0)))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(Frame::new().fill(theme::BG).inner_margin(Margin { left: 32, right: 32, top: 20, bottom: 0 }))
            .show(ui, |ui| match self.view {
                View::Day(d) => self.day_view(ui, d),
                View::Settings => self.settings_view(ui),
            });

        self.add_dialog(&ctx);

        if let Some((msg, until)) = &self.flash {
            let now = ctx.input(|i| i.time);
            if now < *until {
                egui::Area::new(egui::Id::unique("flash"))
                    .anchor(Align2::CENTER_BOTTOM, [120.0, -24.0])
                    .show(&ctx, |ui| {
                        Frame::new()
                            .fill(RAISED)
                            .stroke(theme::hairline())
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(16, 8))
                            .show(ui, |ui| ui.label(RichText::new(msg).color(TEXT)));
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            } else {
                self.flash = None;
            }
        }
        self.handle_screenshot(&ctx);
        // keeps the day rollover and reconnect statuses fresh even when idle
        ctx.request_repaint_after(std::time::Duration::from_secs(30));
    }
}

// ---- widgets ----

/// The app mark: an osu! hit circle with its approach ring.
fn mark(ui: &Ui, c: egui::Pos2, r: f32) {
    let p = ui.painter();
    p.circle_stroke(c, r + 1.0, Stroke::new(1.5, ACCENT));
    p.circle_filled(c, r - 3.0, ACCENT);
}

fn dot(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 3.5, color);
}

fn section_label(ui: &mut Ui, s: &str) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(s).small().color(MUTED));
    });
    ui.add_space(4.0);
}

fn nav_row(ui: &mut Ui, ic: Option<&str>, label: &str, meta: &str, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(8), RAISED);
        p.rect_filled(
            egui::Rect::from_min_size(rect.left_center() - Vec2::new(0.0, 7.0), Vec2::new(2.0, 14.0)),
            CornerRadius::same(1),
            ACCENT,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(6));
    }
    let mut x = rect.left() + 12.0;
    if let Some(ic) = ic {
        p.text(egui::pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, ic, theme::icon_font(16.0), TEXT2);
        x += 26.0;
    }
    let color = if active { TEXT } else { TEXT2 };
    p.text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), color);
    let meta_font = if meta == icon::CHECK { theme::icon_font(14.0) } else { FontId::monospace(12.0) };
    p.text(egui::pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, meta, meta_font, MUTED);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn status_row(ui: &mut Ui, tr: &T, name: &str, configured: bool, c: &Conn) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    let p = ui.painter();
    let (color, text) = match (configured, c) {
        (false, _) => (Color32::from_white_alpha(30), tr.not_set),
        (true, Conn::Off) => (Color32::from_white_alpha(30), tr.off),
        (true, Conn::Connecting) => (theme::queue_dot(Queue::Paid), tr.connecting),
        (true, Conn::Online) => (theme::POSITIVE, tr.online),
        (true, Conn::Error(_)) => (theme::NEGATIVE, tr.error),
    };
    p.circle_filled(egui::pos2(rect.left() + 16.0, rect.center().y), 3.5, color);
    p.text(egui::pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.0), TEXT2);
    p.text(egui::pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, text, FontId::monospace(12.0), MUTED);
    if let Conn::Error(e) = c {
        resp.on_hover_text(e);
    }
}

fn button(ui: &mut Ui, ic: Option<&str>, label: &str, primary: bool) -> egui::Response {
    let font = FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, TEXT);
    let icon_w = if ic.is_some() { 24.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + icon_w + 28.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (bg, fg) = if primary {
        (if resp.hovered() { Color32::from_rgb(0xff, 0xff, 0xff) } else { TEXT }, ON_ACCENT)
    } else {
        (if resp.hovered() { Color32::from_rgb(0x2b, 0x2b, 0x2e) } else { RAISED }, TEXT)
    };
    let p = ui.painter();
    let rect_p = if resp.is_pointer_button_down_on() { rect.shrink(0.5) } else { rect };
    p.rect(rect_p, CornerRadius::same(8), bg, if primary { Stroke::NONE } else { theme::hairline() }, egui::StrokeKind::Inside);
    let mut x = rect.left() + 14.0;
    if let Some(ic) = ic {
        p.text(egui::pos2(x + 8.0, rect.center().y), Align2::CENTER_CENTER, ic, theme::icon_font(16.0), if primary { fg } else { TEXT2 });
        x += icon_w;
    }
    p.galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn toggle_chip(ui: &mut Ui, color: Color32, label: &str, on: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, TEXT);
    let has_dot = color != Color32::TRANSPARENT;
    let size = Vec2::new(galley.size().x + if has_dot { 40.0 } else { 24.0 }, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let bg = if on { RAISED } else if resp.hovered() { Color32::from_white_alpha(6) } else { Color32::TRANSPARENT };
    p.rect(rect, CornerRadius::same(6), bg, theme::hairline(), egui::StrokeKind::Inside);
    let mut x = rect.left() + 12.0;
    if has_dot {
        let c = if on { color } else { Color32::from_white_alpha(40) };
        p.circle_filled(egui::pos2(x + 3.5, rect.center().y), 3.5, c);
        x += 16.0;
    }
    p.galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, if on { TEXT } else { MUTED });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn toggle_chip_icon(ui: &mut Ui, ic: &str, label: &str, on: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, TEXT);
    let size = Vec2::new(galley.size().x + 46.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    let bg = if on { RAISED } else if resp.hovered() { Color32::from_white_alpha(6) } else { Color32::TRANSPARENT };
    p.rect(rect, CornerRadius::same(6), bg, theme::hairline(), egui::StrokeKind::Inside);
    let fg = if on { TEXT } else { MUTED };
    p.text(egui::pos2(rect.left() + 20.0, rect.center().y), Align2::CENTER_CENTER, ic, theme::icon_font(14.0), fg);
    p.galley_with_override_text_color(egui::pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn icon_button(ui: &mut Ui, ic: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), RAISED);
    }
    p.text(rect.center(), Align2::CENTER_CENTER, ic, theme::icon_font(16.0), if resp.hovered() { TEXT } else { MUTED });
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The "watched" action, drawn as an approach circle that fills on hover.
fn watch_button(ui: &mut Ui, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let p = ui.painter();
    let c = rect.center();
    if resp.hovered() {
        p.circle_filled(c, 12.0, ACCENT);
        p.text(c, Align2::CENTER_CENTER, icon::CHECK, theme::icon_font(14.0), ON_ACCENT);
    } else {
        p.circle_stroke(c, 11.0, Stroke::new(1.5, ACCENT));
        p.text(c, Align2::CENTER_CENTER, icon::CHECK, theme::icon_font(14.0), ACCENT);
    }
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn card(ui: &mut Ui, tr: &T, it: &Item) -> Option<Action> {
    let mut action = None;
    let done = it.status != Status::Pending;
    let fade = |c: Color32| if done { c.gamma_multiply(0.55) } else { c };
    Frame::new()
        .fill(CARD)
        .stroke(theme::hairline())
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);

            ui.horizontal(|ui| {
                let (ic, kind) = match it.kind {
                    LinkKind::Score => (icon::TROPHY, tr.score),
                    LinkKind::Profile => (icon::USER, tr.profile),
                    LinkKind::Replay => (icon::REPLAY, tr.replay),
                    LinkKind::Video => (icon::VIDEO, tr.video),
                };
                ui.label(RichText::new(ic).font(theme::icon_font(16.0)).color(fade(MUTED))).on_hover_text(kind);
                ui.label(RichText::new(&it.user).family(theme::medium()).color(fade(TEXT)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(v) = origin_value(tr, &it.origin) {
                        ui.label(RichText::new(v).family(theme::medium()).color(fade(TEXT)));
                    }
                });
            });

            let short = it.url.trim_start_matches("https://");
            let link = ui.add(
                egui::Label::new(RichText::new(short).monospace().color(fade(TEXT2)))
                    .truncate()
                    .sense(Sense::click()),
            );
            if link.clicked() {
                let _ = open::that(&it.url);
            }
            link.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr.open);

            if !it.message.is_empty() {
                ui.label(RichText::new(&it.message).color(fade(TEXT2)));
            }

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let mut meta = it.received.format("%H:%M").to_string();
                if done {
                    let st = if it.status == Status::Watched { tr.watched } else { tr.skipped };
                    meta = format!("{meta} · {}", st.to_lowercase());
                }
                ui.label(RichText::new(meta).monospace().color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if done {
                        if icon_button(ui, icon::UNDO, tr.undo).clicked() {
                            action = Some(Action::Undo);
                        }
                        if icon_button(ui, icon::TRASH, tr.delete).clicked() {
                            action = Some(Action::Delete);
                        }
                    } else {
                        if watch_button(ui, tr.watched).clicked() {
                            action = Some(Action::Watched);
                        }
                        if icon_button(ui, icon::X, tr.skip).clicked() {
                            action = Some(Action::Skip);
                        }
                    }
                    if icon_button(ui, icon::COPY, tr.copy).clicked() {
                        ui.ctx().copy_text(it.url.clone());
                        action = Some(Action::Copied);
                    }
                });
            });
        });
    action
}

fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(title).family(theme::medium()).size(15.0).color(TEXT));
    ui.add_space(4.0);
    Frame::new()
        .fill(CARD)
        .stroke(theme::hairline())
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            body(ui);
        });
    ui.add_space(24.0);
}

fn field(ui: &mut Ui, label: &str, help: Option<&str>, body: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(label).color(TEXT2));
        body(ui);
        if let Some(h) = help {
            ui.label(RichText::new(h).small().color(MUTED));
        }
    });
}

fn queue_name(tr: &T, q: Queue) -> &'static str {
    match q {
        Queue::Paid => tr.q_paid,
        Queue::Points => tr.q_points,
        Queue::Free => tr.q_free,
    }
}

fn origin_value(tr: &T, o: &Origin) -> Option<String> {
    match o {
        Origin::DonationAlerts { amount, currency } | Origin::DonatePay { amount, currency } if *amount > 0.0 => {
            Some(format!("{} {}", money(*amount), currency_sign(currency)))
        }
        Origin::Bits { bits } => Some(format!("{bits} {}", tr.bits)),
        Origin::Manual => Some(tr.manual.to_string()),
        _ => None,
    }
}

fn money(v: f64) -> String {
    if v.fract().abs() < 0.005 { format!("{v:.0}") } else { format!("{v:.2}") }
}

fn currency_sign(c: &str) -> &str {
    match c {
        "RUB" => "₽",
        "USD" => "$",
        "EUR" => "€",
        "" => "",
        other => other,
    }
}

fn short_date(tr: &T, d: NaiveDate, today: NaiveDate) -> String {
    let m = tr.months[d.month0() as usize];
    if d.year() == today.year() {
        format!("{} {}", d.day(), m)
    } else {
        format!("{} {} {}", d.day(), m, d.year())
    }
}

fn long_date(tr: &T, d: NaiveDate) -> String {
    format!("{} {} {}", d.day(), tr.months[d.month0() as usize], d.year())
}

/// The message without the links, so cards don't repeat the URL.
fn strip_links(text: &str, rules: &crate::parse::Rules) -> String {
    text.split_whitespace()
        .filter(|w| find_links(w, rules).is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
