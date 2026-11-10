//! Players ranked by score, for the public boards.

/// Whole points, as the title shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Points(pub u64);

impl Points {
    /// `None` unless `score` is finite and not negative; fractions are dropped.
    pub fn from_score(score: f64) -> Option<Self> {
        // `as` saturates at u64::MAX; the title caps scores far below it.
        (score.is_finite() && score >= 0.0).then_some(Self(score as u64))
    }
}

/// An Xbox gamertag: up to 15 letters, digits and spaces, starting with a letter. Names come from
/// the consoles, so anything else is not published.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Gamertag(String);

impl Gamertag {
    const MAX_LEN: usize = 15;

    pub fn new(text: &str) -> Option<Self> {
        let valid = text.len() <= Self::MAX_LEN
            && text.starts_with(|c: char| c.is_ascii_alphabetic())
            && !text.ends_with(' ')
            && text.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ');
        valid.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 1-based; equal points share a rank (1, 2, 2, 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rank(pub usize);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub rank: Rank,
    pub gamertag: Gamertag,
    pub points: Points,
}

/// Most points first; equal points by gamertag.
pub fn rank(mut players: Vec<(Gamertag, Points)>) -> Vec<Standing> {
    players.sort_by(|(a_tag, a), (b_tag, b)| b.cmp(a).then_with(|| a_tag.cmp(b_tag)));
    let mut standings: Vec<Standing> = Vec::with_capacity(players.len());
    for (index, (gamertag, points)) in players.into_iter().enumerate() {
        let rank = match standings.last() {
            Some(previous) if previous.points == points => previous.rank,
            _ => Rank(index + 1),
        };
        standings.push(Standing {
            rank,
            gamertag,
            points,
        });
    }
    standings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_xbox_gamertags_are_published() {
        for valid in ["PlayerPhat", "A", "Mr Smith 2", "abcdefghijklmno"] {
            assert!(Gamertag::new(valid).is_some(), "{valid}");
        }
        for invalid in [
            "",
            "2Fast",
            " Lead",
            "Trail ",
            "abcdefghijklmnop",
            "<b>x</b>",
            "Tab\tbed",
        ] {
            assert!(Gamertag::new(invalid).is_none(), "{invalid}");
        }
    }
}
