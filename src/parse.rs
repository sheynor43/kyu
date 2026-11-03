use regex::Regex;
use std::sync::OnceLock;

use crate::model::LinkKind;

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub kind: LinkKind,
    pub url: String,
}

/// Which link families to accept besides osu.ppy.sh. Built from settings.
#[derive(Debug, Clone)]
pub struct Rules {
    pub servers: bool,
    pub cloud: bool,
    pub video: bool,
    pub extra_servers: Vec<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Self { servers: true, cloud: true, video: true, extra_servers: Vec::new() }
    }
}

/// Private osu! servers. Only their profile and score pages are picked up.
pub const SERVERS: &[&str] = &[
    "akatsuki.gg", "akatsuki.pw", "gatari.pw", "ripple.moe", "ppy.sb", "ez-pp.farm", "kawata.pw", "ussr.pl",
];

/// File hosts where people put replays.
const CLOUD: &[&str] = &[
    "mediafire.com", "drive.google.com", "disk.yandex.ru", "disk.yandex.com", "disk.yandex.kz", "yadi.sk",
    "dropbox.com", "mega.nz", "mega.io", "cdn.discordapp.com", "media.discordapp.net", "catbox.moe",
    "gofile.io",
];

const VIDEO: &[&str] = &["youtube.com", "youtu.be", "streamable.com", "clips.twitch.tv", "medal.tv"];

fn score_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^osu\.ppy\.sh/scores/(?:(osu|taiko|fruits|mania)/)?(\d+)").unwrap())
}

fn profile_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^osu\.ppy\.sh/(?:users|u)/([^\s/?#]+)(?:/(osu|taiko|fruits|mania))?").unwrap()
    })
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(?:https?://)?(?:www\.)?((?:[a-z0-9-]+\.)+[a-z]{2,})(/[^\s]*)?").unwrap())
}

fn on_domain(host: &str, list: &[impl AsRef<str>]) -> bool {
    list.iter().any(|d| {
        let d = d.as_ref();
        host == d || host.ends_with(&format!(".{d}"))
    })
}

/// Finds osu! score and profile links, plus private server, replay and video
/// links allowed by `rules`. Links are normalised so the same link sent twice
/// compares equal.
pub fn find_links(text: &str, rules: &Rules) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    for c in url_re().captures_iter(text) {
        // skip matches glued to a longer word, like the domain part of an email
        let start = c.get(0).unwrap().start();
        if start > 0 && text[..start].ends_with(['@', '.']) {
            continue;
        }
        let host = c[1].to_lowercase();
        let path = c.get(2).map(|m| m.as_str()).unwrap_or("");
        let path = path.trim_end_matches(|ch: char| ",.!)]}>\"'".contains(ch));
        if let Some(link) = classify(&host, path, rules) {
            if !out.iter().any(|l| l.url.eq_ignore_ascii_case(&link.url)) {
                out.push(link);
            }
        }
    }
    out
}

fn classify(host: &str, path: &str, rules: &Rules) -> Option<Link> {
    let url = format!("https://{host}{path}");
    let lower = path.to_lowercase();

    if host == "osu.ppy.sh" {
        let hp = format!("{host}{path}");
        if let Some(c) = score_re().captures(&hp) {
            let url = match c.get(1) {
                Some(mode) => format!("https://osu.ppy.sh/scores/{}/{}", mode.as_str().to_lowercase(), &c[2]),
                None => format!("https://osu.ppy.sh/scores/{}", &c[2]),
            };
            return Some(Link { kind: LinkKind::Score, url });
        }
        if let Some(c) = profile_re().captures(&hp) {
            let url = match c.get(2) {
                Some(mode) => format!("https://osu.ppy.sh/users/{}/{}", &c[1], mode.as_str().to_lowercase()),
                None => format!("https://osu.ppy.sh/users/{}", &c[1]),
            };
            return Some(Link { kind: LinkKind::Profile, url });
        }
        return None;
    }

    if rules.servers && (on_domain(host, SERVERS) || on_domain(host, &rules.extra_servers)) {
        let kind = if lower.contains("/score") || lower.contains("/web/replays") {
            LinkKind::Score
        } else if lower.starts_with("/u/") || lower.starts_with("/users/") || lower.starts_with("/user/") {
            LinkKind::Profile
        } else {
            return None;
        };
        return Some(Link { kind, url });
    }

    // a direct .osr file anywhere is a replay
    if rules.cloud && (lower.ends_with(".osr") || (on_domain(host, CLOUD) && path.len() > 1)) {
        return Some(Link { kind: LinkKind::Replay, url });
    }

    if rules.video && path.len() > 1 {
        let twitch_clip = on_domain(host, &["twitch.tv"]) && lower.contains("/clip/");
        if on_domain(host, VIDEO) || twitch_clip {
            return Some(Link { kind: LinkKind::Video, url });
        }
    }
    None
}

/// Splits a comma or space separated list of domains from settings.
pub fn parse_domains(s: &str) -> Vec<String> {
    s.split([',', ' ', '\n', ';'])
        .map(|d| {
            d.trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.")
                .trim_end_matches('/')
                .to_lowercase()
        })
        .filter(|d| d.contains('.'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(t: &str) -> Vec<Link> {
        find_links(t, &Rules::default())
    }

    #[test]
    fn scores() {
        let l = find("глянь https://osu.ppy.sh/scores/1234567890 пж");
        assert_eq!(l, vec![Link { kind: LinkKind::Score, url: "https://osu.ppy.sh/scores/1234567890".into() }]);
        let l = find("osu.ppy.sh/scores/osu/4567");
        assert_eq!(l[0].url, "https://osu.ppy.sh/scores/osu/4567");
    }

    #[test]
    fn profiles() {
        let l = find("my profile: https://osu.ppy.sh/users/2/osu, and https://osu.ppy.sh/u/peppy!");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].url, "https://osu.ppy.sh/users/2/osu");
        assert_eq!(l[1].url, "https://osu.ppy.sh/users/peppy");
        assert!(l.iter().all(|x| x.kind == LinkKind::Profile));
    }

    #[test]
    fn mixed_and_dupes() {
        let l = find("https://osu.ppy.sh/users/123 https://osu.ppy.sh/scores/9 https://osu.ppy.sh/users/123");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].kind, LinkKind::Profile);
        assert_eq!(l[1].kind, LinkKind::Score);
    }

    #[test]
    fn ignores_other_links() {
        assert!(find("https://osu.ppy.sh/beatmapsets/1#osu/2 https://google.com/search dt 98% 7.5* me@mail.ru").is_empty());
    }

    #[test]
    fn private_servers() {
        let l = find("akatsuki.gg/u/1001 https://osu.gatari.pw/u/2002 https://akatsuki.gg/leaderboard");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0], Link { kind: LinkKind::Profile, url: "https://akatsuki.gg/u/1001".into() });
        assert_eq!(l[1].url, "https://osu.gatari.pw/u/2002");
        let custom = Rules { extra_servers: parse_domains("https://my-server.xyz/, other.net"), ..Rules::default() };
        assert_eq!(find_links("my-server.xyz/scores/55", &custom)[0].kind, LinkKind::Score);
    }

    #[test]
    fn replays_and_videos() {
        let l = find("реплей https://www.mediafire.com/file/abc/play.osr/file и видео https://youtu.be/dQw4w9WgXcQ.");
        assert_eq!(l[0].kind, LinkKind::Replay);
        assert_eq!(l[1], Link { kind: LinkKind::Video, url: "https://youtu.be/dQw4w9WgXcQ".into() });
        assert_eq!(find("https://disk.yandex.ru/d/AbC123")[0].kind, LinkKind::Replay);
        assert_eq!(find("https://files.example.org/top.osr")[0].kind, LinkKind::Replay);
        assert_eq!(find("https://www.twitch.tv/someone/clip/FunnyClip-abc")[0].kind, LinkKind::Video);
        assert!(find("https://www.twitch.tv/someone https://mega.nz").is_empty());
    }

    #[test]
    fn rules_switch_families_off() {
        let only_osu = Rules { servers: false, cloud: false, video: false, extra_servers: vec![] };
        let t = "https://akatsuki.gg/u/1 https://mega.nz/file/x https://youtube.com/watch?v=1 https://osu.ppy.sh/u/2";
        let l = find_links(t, &only_osu);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, LinkKind::Profile);
    }
}
