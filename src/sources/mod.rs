pub mod donatepay;
pub mod donationalerts;
pub mod twitch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use crate::model::{Conn, Event, Service};

#[derive(Clone)]
pub struct Sink {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Sink {
    pub fn new(tx: Sender<Event>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }

    pub fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }

    pub fn status(&self, s: Service, c: Conn) {
        self.send(Event::Status(s, c));
    }
}

/// A background connection. Dropping it asks the thread to stop.
pub struct Worker {
    stop: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(f: impl FnOnce(Arc<AtomicBool>) + Send + 'static) -> Worker {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::spawn(move || f(s));
    Worker { stop }
}

/// Sleeps in small steps so a stop request is noticed quickly. Returns false if stopped.
pub fn wait(stop: &AtomicBool, d: Duration) -> bool {
    let step = Duration::from_millis(200);
    let mut left = d;
    while !left.is_zero() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let s = left.min(step);
        std::thread::sleep(s);
        left -= s;
    }
    !stop.load(Ordering::Relaxed)
}

pub fn stopped(stop: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed)
}

/// Reconnect delay that grows after repeated failures, capped at a minute.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs((2u64 << attempt.min(5)).min(60))
}

#[cfg(test)]
mod live {
    //! Network checks against the real services. Run with `cargo test -- --ignored`.
    use super::*;
    use crate::model::Event;
    use std::sync::mpsc::channel;

    fn collect(secs: u64, f: impl FnOnce(Sink, Arc<AtomicBool>) + Send + 'static) -> Vec<Event> {
        let (tx, rx) = channel();
        let sink = Sink::new(tx, egui::Context::default());
        let w = spawn(move |stop| f(sink, stop));
        std::thread::sleep(Duration::from_secs(secs));
        drop(w);
        rx.try_iter().collect()
    }

    #[test]
    #[ignore]
    fn twitch_connects() {
        let ch = std::env::var("KYU_TEST_CHANNEL").unwrap_or("osulive".into());
        let ev = collect(15, move |s, stop| twitch::run(ch, s, &stop));
        let online = ev.iter().any(|e| matches!(e, Event::Status(Service::Twitch, Conn::Online)));
        let msgs = ev.iter().filter(|e| matches!(e, Event::Message(_))).count();
        println!("online={online} messages={msgs}");
        assert!(online);
    }

    #[test]
    #[ignore]
    fn donationalerts_socket_answers() {
        let ev = collect(8, |s, stop| donationalerts::run("invalid-token".into(), s, &stop));
        for e in &ev {
            if let Event::Status(_, c) = e {
                println!("{c:?}");
            }
        }
        assert!(ev.iter().any(|e| matches!(e, Event::Status(Service::DonationAlerts, Conn::Online))));
    }
}
