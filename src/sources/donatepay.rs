//! DonatePay has no public push API for third parties, so this polls the
//! transactions endpoint. The API allows one request every 20 seconds.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;

use super::donationalerts::num;
use super::{stopped, wait, Sink};
use crate::model::{Conn, Event, Incoming, Origin, Service};

const POLL: Duration = Duration::from_secs(20);

/// `cursor` is the last transaction id already handled. Zero means first run:
/// old donations are skipped and only new ones after start are queued.
pub fn run(token: String, mut cursor: u64, sink: Sink, stop: &AtomicBool) {
    sink.status(Service::DonatePay, Conn::Connecting);
    while !stopped(stop) {
        match fetch(token.trim()) {
            Ok(list) => {
                sink.status(Service::DonatePay, Conn::Online);
                let newest = list.iter().map(|(id, _)| *id).max().unwrap_or(cursor);
                if cursor != 0 {
                    let mut fresh: Vec<_> = list.into_iter().filter(|(id, _)| *id > cursor).collect();
                    fresh.sort_by_key(|(id, _)| *id);
                    for (_, inc) in fresh {
                        sink.send(Event::Message(inc));
                    }
                }
                if newest > cursor || cursor == 0 {
                    cursor = newest.max(1);
                    sink.send(Event::DonatePayCursor(cursor));
                }
            }
            Err(e) => sink.status(Service::DonatePay, Conn::Error(e)),
        }
        if !wait(stop, POLL) {
            return;
        }
    }
}

fn fetch(token: &str) -> Result<Vec<(u64, Incoming)>, String> {
    let url = format!(
        "https://donatepay.ru/api/v1/transactions?access_token={token}&type=donation&status=success&limit=25&order=DESC"
    );
    let body = ureq::get(&url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse(&body)
}

fn parse(body: &str) -> Result<Vec<(u64, Incoming)>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if v["status"].as_str() != Some("success") {
        return Err(v["message"].as_str().unwrap_or("bad response").to_string());
    }
    let mut out = Vec::new();
    for t in v["data"].as_array().into_iter().flatten() {
        let Some(id) = t["id"].as_u64().or_else(|| t["id"].as_str().and_then(|s| s.parse().ok())) else {
            continue;
        };
        let user = t["what"].as_str().or(t["vars"]["name"].as_str()).unwrap_or("anonymous").to_string();
        let text = t["comment"].as_str().or(t["vars"]["comment"].as_str()).unwrap_or("").to_string();
        let amount = num(&t["sum"]).unwrap_or(0.0);
        let currency = t["currency"].as_str().unwrap_or("RUB").to_string();
        out.push((id, Incoming { origin: Origin::DonatePay { amount, currency }, user, text }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transactions() {
        let body = r#"{"status":"success","data":[{"id":77,"what":"Mrekk","sum":"300.00","comment":"osu.ppy.sh/users/7562902","type":"donation","status":"success"}]}"#;
        let l = parse(body).unwrap();
        assert_eq!(l[0].0, 77);
        assert_eq!(l[0].1.user, "Mrekk");
        assert_eq!(l[0].1.origin, Origin::DonatePay { amount: 300.0, currency: "RUB".into() });
    }

    #[test]
    fn reports_errors() {
        assert!(parse(r#"{"status":"error","message":"Wrong access token"}"#).is_err());
    }
}
