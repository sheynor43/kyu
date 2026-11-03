use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::model::{Day, Queue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Ru,
    En,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub twitch_channel: String,
    /// Empty means every reward with a link counts.
    pub reward_ids: Vec<String>,
    pub bits_enabled: bool,
    pub bits_min: u32,
    pub da_token: String,
    pub dp_token: String,
    pub donation_min: f64,
    pub link_servers: bool,
    pub link_cloud: bool,
    pub link_video: bool,
    /// Extra private server domains, comma separated as typed in settings.
    pub extra_servers: String,
    pub one_per_viewer: bool,
    pub ignore_duplicates: bool,
    pub day_start_hour: u32,
    pub auto_delete: bool,
    pub keep_days: u32,
    pub show_paid: bool,
    pub show_points: bool,
    pub show_free: bool,
    pub dp_cursor: u64,
    pub known_rewards: Vec<Reward>,
}

/// A channel points reward seen in chat. IRC only carries the id, so the last
/// redemption text is kept to tell rewards apart in settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reward {
    pub id: String,
    pub sample: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::Ru,
            twitch_channel: String::new(),
            reward_ids: Vec::new(),
            bits_enabled: true,
            bits_min: 100,
            da_token: String::new(),
            dp_token: String::new(),
            donation_min: 0.0,
            link_servers: true,
            link_cloud: true,
            link_video: true,
            extra_servers: String::new(),
            one_per_viewer: true,
            ignore_duplicates: true,
            day_start_hour: 6,
            auto_delete: false,
            keep_days: 30,
            show_paid: true,
            show_points: true,
            show_free: true,
            dp_cursor: 0,
            known_rewards: Vec::new(),
        }
    }
}

impl Settings {
    pub fn shows(&self, q: Queue) -> bool {
        match q {
            Queue::Paid => self.show_paid,
            Queue::Points => self.show_points,
            Queue::Free => self.show_free,
        }
    }

    pub fn link_rules(&self) -> crate::parse::Rules {
        crate::parse::Rules {
            servers: self.link_servers,
            cloud: self.link_cloud,
            video: self.link_video,
            extra_servers: crate::parse::parse_domains(&self.extra_servers),
        }
    }

    pub fn toggle(&mut self, q: Queue) {
        let v = match q {
            Queue::Paid => &mut self.show_paid,
            Queue::Points => &mut self.show_points,
            Queue::Free => &mut self.show_free,
        };
        *v = !*v;
    }
}

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open() -> Self {
        let root = std::env::var_os("KYU_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| directories::ProjectDirs::from("", "", "kyu").map(|d| d.data_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("kyu-data"));
        let _ = fs::create_dir_all(root.join("days"));
        Self { root }
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    fn day_path(&self, d: NaiveDate) -> PathBuf {
        self.root.join("days").join(format!("{}.json", d.format("%Y-%m-%d")))
    }

    pub fn load_settings(&self) -> Settings {
        let p = self.settings_path();
        match read_json(&p) {
            Some(s) => s,
            None => {
                // keep an unreadable file around instead of overwriting the tokens in it
                if p.exists() {
                    let _ = fs::copy(&p, p.with_extension("json.bak"));
                }
                Settings::default()
            }
        }
    }

    pub fn save_settings(&self, s: &Settings) {
        write_json(&self.settings_path(), s);
    }

    pub fn load_day(&self, d: NaiveDate) -> Day {
        read_json(&self.day_path(d)).unwrap_or_default()
    }

    pub fn save_day(&self, d: NaiveDate, day: &Day) {
        write_json(&self.day_path(d), day);
    }

    /// All stored days, newest first, with (pending, total) counts.
    pub fn list_days(&self) -> BTreeMap<NaiveDate, (usize, usize)> {
        let mut out = BTreeMap::new();
        let Ok(rd) = fs::read_dir(self.root.join("days")) else { return out };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".json") else { continue };
            let Ok(d) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else { continue };
            let day = self.load_day(d);
            out.insert(d, (day.pending(), day.items.len()));
        }
        out
    }

    /// Removes days older than `keep_days` relative to `today`. Returns how many were removed.
    pub fn prune(&self, today: NaiveDate, keep_days: u32) -> usize {
        let cutoff = today - chrono::Duration::days(keep_days as i64);
        let mut n = 0;
        for d in self.list_days().keys() {
            if *d < cutoff && fs::remove_file(self.day_path(*d)).is_ok() {
                n += 1;
            }
        }
        n
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> Option<T> {
    let s = fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

/// Writes through a temp file so a crash mid-write never leaves a broken day file.
fn write_json<T: Serialize>(p: &Path, v: &T) {
    let tmp = p.with_extension("json.tmp");
    if let Ok(s) = serde_json::to_string_pretty(v) {
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, p);
        }
    }
}
