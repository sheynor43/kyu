//! Read-only Twitch chat over IRC. Anonymous login is enough to read chat,
//! and channel point rewards that ask for text arrive as normal messages
//! carrying a `custom-reward-id` tag, so no OAuth is needed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::{backoff, stopped, wait, Sink};
use crate::model::{Conn, Event, Incoming, Origin, Service};

pub fn run(channel: String, sink: Sink, stop: &AtomicBool) {
    let channel = channel.trim().trim_start_matches('#').to_lowercase();
    let mut attempt = 0;
    while !stopped(stop) {
        sink.status(Service::Twitch, Conn::Connecting);
        match session(&channel, &sink, stop) {
            Ok(()) => return,
            Err(e) => {
                sink.status(Service::Twitch, Conn::Error(e.to_string()));
                attempt += 1;
                if !wait(stop, backoff(attempt)) {
                    return;
                }
            }
        }
    }
}

fn session(channel: &str, sink: &Sink, stop: &AtomicBool) -> std::io::Result<()> {
    let mut stream = TcpStream::connect("irc.chat.twitch.tv:6667")?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let nick = format!("justinfan{}", 10000 + std::process::id() % 80000);
    write!(stream, "CAP REQ :twitch.tv/tags twitch.tv/commands\r\nPASS SCHMOOPIIE\r\nNICK {nick}\r\nJOIN #{channel}\r\n")?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let mut idle = 0u32;
    loop {
        if stopped(stop) {
            let _ = write!(stream, "PART #{channel}\r\n");
            return Ok(());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Err(std::io::Error::new(ErrorKind::UnexpectedEof, "disconnected")),
            Ok(_) => {
                idle = 0;
                let msg = line.trim_end();
                if msg.starts_with("PING") {
                    write!(stream, "PONG{}\r\n", &msg[4..])?;
                    continue;
                }
                handle(msg, channel, sink);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                // Twitch pings every ~5 minutes. Ten minutes of silence means a dead socket.
                idle += 1;
                if idle > 1200 {
                    return Err(std::io::Error::new(ErrorKind::TimedOut, "no data from twitch"));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn handle(raw: &str, channel: &str, sink: &Sink) {
    let Some(m) = parse(raw) else { return };
    match m.command {
        "ROOMSTATE" | "001" => sink.status(Service::Twitch, Conn::Online),
        "NOTICE" if m.trailing.contains("Login") => {
            sink.status(Service::Twitch, Conn::Error(m.trailing.to_string()))
        }
        "PRIVMSG" => {
            if !m.params.eq_ignore_ascii_case(&format!("#{channel}")) {
                return;
            }
            let user = m
                .tags
                .get("display-name")
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| m.prefix_nick.to_string());
            let origin = if let Some(id) = m.tags.get("custom-reward-id") {
                sink.send(Event::RewardSeen { id: id.to_string(), sample: m.trailing.chars().take(60).collect() });
                Origin::Reward { reward_id: id.to_string() }
            } else if let Some(bits) = m.tags.get("bits").and_then(|b| b.parse().ok()) {
                Origin::Bits { bits }
            } else {
                Origin::Chat
            };
            sink.send(Event::Message(Incoming { origin, user, text: m.trailing.to_string() }));
        }
        _ => {}
    }
}

struct IrcMsg<'a> {
    tags: HashMap<&'a str, String>,
    prefix_nick: &'a str,
    command: &'a str,
    params: &'a str,
    trailing: &'a str,
}

fn parse(mut s: &str) -> Option<IrcMsg<'_>> {
    let mut tags = HashMap::new();
    if let Some(rest) = s.strip_prefix('@') {
        let (t, r) = rest.split_once(' ')?;
        for kv in t.split(';') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            tags.insert(k, unescape(v));
        }
        s = r;
    }
    let mut prefix_nick = "";
    if let Some(rest) = s.strip_prefix(':') {
        let (p, r) = rest.split_once(' ')?;
        prefix_nick = p.split('!').next().unwrap_or("");
        s = r;
    }
    let (head, trailing) = match s.split_once(" :") {
        Some((h, t)) => (h, t),
        None => (s, ""),
    };
    let (command, params) = head.split_once(' ').unwrap_or((head, ""));
    Some(IrcMsg { tags, prefix_nick, command, params: params.trim(), trailing })
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('s') => out.push(' '),
                Some(':') => out.push(';'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reward_message() {
        let raw = "@badge-info=;custom-reward-id=abc-123;display-name=Cookiezi;id=1 :cookiezi!cookiezi@cookiezi.tmi.twitch.tv PRIVMSG #mrekk :https://osu.ppy.sh/scores/1";
        let m = parse(raw).unwrap();
        assert_eq!(m.command, "PRIVMSG");
        assert_eq!(m.params, "#mrekk");
        assert_eq!(m.tags.get("custom-reward-id").unwrap(), "abc-123");
        assert_eq!(m.tags.get("display-name").unwrap(), "Cookiezi");
        assert_eq!(m.trailing, "https://osu.ppy.sh/scores/1");
    }

    #[test]
    fn unescapes_tags() {
        assert_eq!(unescape(r"a\sb\:c"), "a b;c");
    }
}
