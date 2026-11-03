use regex::Regex;
use std::sync::OnceLock;

use crate::model::LinkKind;

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub kind: LinkKind,
    pub url: String,
}

fn score_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:https?://)?(?:www\.)?osu\.ppy\.sh/scores/(?:(osu|taiko|fruits|mania)/)?(\d+)")
            .unwrap()
    })
}

fn profile_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:https?://)?(?:www\.)?osu\.ppy\.sh/(?:users|u)/([^\s/?#]+)(?:/(osu|taiko|fruits|mania))?")
            .unwrap()
    })
}

/// Finds osu! score and profile links in a chat message or donation text.
/// Links are normalised so the same score sent twice compares equal.
pub fn find_links(text: &str) -> Vec<Link> {
    let mut found: Vec<(usize, Link)> = Vec::new();

    for c in score_re().captures_iter(text) {
        let start = c.get(0).unwrap().start();
        let id = &c[2];
        let url = match c.get(1) {
            Some(mode) => format!("https://osu.ppy.sh/scores/{}/{}", mode.as_str().to_lowercase(), id),
            None => format!("https://osu.ppy.sh/scores/{id}"),
        };
        found.push((start, Link { kind: LinkKind::Score, url }));
    }

    for c in profile_re().captures_iter(text) {
        let start = c.get(0).unwrap().start();
        let user = c[1].trim_end_matches(|ch: char| ",.!)]}>\"'".contains(ch));
        if user.is_empty() {
            continue;
        }
        let url = match c.get(2) {
            Some(mode) => format!("https://osu.ppy.sh/users/{}/{}", user, mode.as_str().to_lowercase()),
            None => format!("https://osu.ppy.sh/users/{user}"),
        };
        found.push((start, Link { kind: LinkKind::Profile, url }));
    }

    found.sort_by_key(|(start, _)| *start);
    let mut out: Vec<Link> = Vec::new();
    for (_, link) in found {
        if !out.iter().any(|l| l.url.eq_ignore_ascii_case(&link.url)) {
            out.push(link);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores() {
        let l = find_links("глянь https://osu.ppy.sh/scores/1234567890 пж");
        assert_eq!(l, vec![Link { kind: LinkKind::Score, url: "https://osu.ppy.sh/scores/1234567890".into() }]);
        let l = find_links("osu.ppy.sh/scores/osu/4567");
        assert_eq!(l[0].url, "https://osu.ppy.sh/scores/osu/4567");
    }

    #[test]
    fn profiles() {
        let l = find_links("my profile: https://osu.ppy.sh/users/2/osu, and https://osu.ppy.sh/u/peppy!");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].url, "https://osu.ppy.sh/users/2/osu");
        assert_eq!(l[1].url, "https://osu.ppy.sh/users/peppy");
        assert!(l.iter().all(|x| x.kind == LinkKind::Profile));
    }

    #[test]
    fn mixed_and_dupes() {
        let l = find_links("https://osu.ppy.sh/users/123 https://osu.ppy.sh/scores/9 https://osu.ppy.sh/users/123");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].kind, LinkKind::Profile);
        assert_eq!(l[1].kind, LinkKind::Score);
    }

    #[test]
    fn ignores_other_links() {
        assert!(find_links("https://osu.ppy.sh/beatmapsets/1#osu/2 https://youtube.com").is_empty());
    }
}
