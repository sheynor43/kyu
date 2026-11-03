use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Score,
    Profile,
    /// A replay file or a cloud folder with one.
    Replay,
    Video,
}

/// Which of the three queues an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Queue {
    Paid,
    Points,
    Free,
}

impl Queue {
    pub const ALL: [Queue; 3] = [Queue::Paid, Queue::Points, Queue::Free];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    Chat,
    Reward { reward_id: String },
    Bits { bits: u32 },
    DonationAlerts { amount: f64, currency: String },
    DonatePay { amount: f64, currency: String },
    Manual,
}

impl Origin {
    pub fn queue(&self) -> Queue {
        match self {
            Origin::Chat | Origin::Manual => Queue::Free,
            Origin::Reward { .. } => Queue::Points,
            Origin::Bits { .. } | Origin::DonationAlerts { .. } | Origin::DonatePay { .. } => Queue::Paid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Watched,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub received: DateTime<Local>,
    pub queue: Queue,
    pub origin: Origin,
    pub user: String,
    pub kind: LinkKind,
    pub url: String,
    #[serde(default)]
    pub message: String,
    pub status: Status,
    #[serde(default)]
    pub done_at: Option<DateTime<Local>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Day {
    pub items: Vec<Item>,
}

impl Day {
    pub fn pending(&self) -> usize {
        self.items.iter().filter(|i| i.status == Status::Pending).count()
    }
}

/// Raw message from any source, before links are extracted.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub origin: Origin,
    pub user: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Twitch,
    DonationAlerts,
    DonatePay,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Conn {
    Off,
    Connecting,
    Online,
    Error(String),
}

pub enum Event {
    Message(Incoming),
    Status(Service, Conn),
    /// A channel points reward id seen in chat, so it can be picked in settings.
    RewardSeen { id: String, sample: String },
    /// Last processed DonatePay transaction id, persisted so a restart catches up.
    DonatePayCursor(u64),
}

/// The streaming "day" starts at a configurable hour, so a stream that runs past
/// midnight stays in one day.
pub fn day_of(t: DateTime<Local>, day_start_hour: u32) -> NaiveDate {
    (t - chrono::Duration::hours(day_start_hour as i64)).date_naive()
}
