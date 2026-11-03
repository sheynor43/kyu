//! DonationAlerts widget socket. It speaks Socket.IO v2 (Engine.IO 3) over a
//! websocket and takes the secret token from the DonationAlerts dashboard.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

use super::{backoff, stopped, wait, Sink};
use crate::model::{Conn, Event, Incoming, Origin, Service};

const URL: &str = "wss://socket.donationalerts.com/socket.io/?EIO=3&transport=websocket";

pub fn run(token: String, sink: Sink, stop: &AtomicBool) {
    let mut attempt = 0;
    while !stopped(stop) {
        sink.status(Service::DonationAlerts, Conn::Connecting);
        match session(token.trim(), &sink, stop) {
            Ok(()) => return,
            Err(e) => {
                sink.status(Service::DonationAlerts, Conn::Error(e));
                attempt += 1;
                if !wait(stop, backoff(attempt)) {
                    return;
                }
            }
        }
    }
}

fn session(token: &str, sink: &Sink, stop: &AtomicBool) -> Result<(), String> {
    let (mut ws, _) = tungstenite::connect(URL).map_err(|e| e.to_string())?;
    let timeout = Some(Duration::from_millis(500));
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(timeout),
        MaybeTlsStream::Rustls(s) => s.get_mut().set_read_timeout(timeout),
        _ => Ok(()),
    }
    .map_err(|e| e.to_string())?;

    let mut ping_every = Duration::from_secs(25);
    let mut last_ping = Instant::now();
    loop {
        if stopped(stop) {
            let _ = ws.close(None);
            return Ok(());
        }
        if last_ping.elapsed() >= ping_every {
            ws.send(Message::text("2")).map_err(|e| e.to_string())?;
            last_ping = Instant::now();
        }
        let text = match ws.read() {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) => return Err("closed by server".into()),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                continue
            }
            Err(e) => return Err(e.to_string()),
        };

        if let Some(open) = text.strip_prefix('0') {
            if let Ok(v) = serde_json::from_str::<Value>(open) {
                if let Some(ms) = v["pingInterval"].as_u64() {
                    ping_every = Duration::from_millis(ms);
                }
            }
        } else if text == "40" {
            let hello = serde_json::json!(["add-user", {"token": token, "type": "minor"}]);
            ws.send(Message::text(format!("42{hello}"))).map_err(|e| e.to_string())?;
            sink.status(Service::DonationAlerts, Conn::Online);
        } else if let Some(ev) = text.strip_prefix("42") {
            if let Some(inc) = parse_event(ev) {
                sink.send(Event::Message(inc));
            }
        }
    }
}

fn parse_event(ev: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(ev).ok()?;
    let arr = v.as_array()?;
    if arr.first()?.as_str()? != "donation" {
        return None;
    }
    // The payload is usually a JSON string inside the JSON array.
    let payload = match arr.get(1)? {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        other => other.clone(),
    };
    if let Some(t) = payload.get("alert_type") {
        if t.as_str().map(|s| s != "1").unwrap_or(false) || t.as_i64().map(|n| n != 1).unwrap_or(false) {
            return None;
        }
    }
    let amount = num(&payload["amount_main"]).or_else(|| num(&payload["amount"])).unwrap_or(0.0);
    let currency = if payload.get("amount_main").is_some() {
        payload["currency_main"].as_str().or(payload["currency"].as_str())
    } else {
        payload["currency"].as_str()
    }
    .unwrap_or("RUB")
    .to_string();
    Some(Incoming {
        origin: Origin::DonationAlerts { amount, currency },
        user: payload["username"].as_str().unwrap_or("anonymous").to_string(),
        text: payload["message"].as_str().unwrap_or("").to_string(),
    })
}

pub fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_payload() {
        let ev = r#"["donation","{\"id\":1,\"alert_type\":\"1\",\"username\":\"whitecat\",\"message\":\"https://osu.ppy.sh/scores/5\",\"amount\":\"150\",\"currency\":\"RUB\"}"]"#;
        let i = parse_event(ev).unwrap();
        assert_eq!(i.user, "whitecat");
        assert_eq!(i.origin, Origin::DonationAlerts { amount: 150.0, currency: "RUB".into() });
    }

    #[test]
    fn skips_non_donations() {
        let ev = r#"["donation","{\"alert_type\":\"4\",\"username\":\"a\"}"]"#;
        assert!(parse_event(ev).is_none());
    }
}
