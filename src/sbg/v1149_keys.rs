//! v2.0.1149.0 store keys translated to the namespace the store works on.

/// (v2.0.1149.0 template, v2.0.1317.0 template). Placeholders in the same order on both sides.
/// More specific templates must come before ones they would also match (`.GameState`).
const KEYS: &[(&str, &str)] = &[
    // Setup
    (
        "v1.Setup.MGS.1vs100.Config",
        "v3.Setup.MGS.1vs100.Store.Config",
    ),
    (
        "v1.Setup.MGS.1vs100.Config.Week~N",
        "v3.Setup.MGS.1vs100.Store.Config.Week~N",
    ),
    (
        "v1.Setup.MGS.1vs100.Sessions.~P.MergedLaunchData",
        "v3.Setup.MGS.1vs100.Store.Sessions.~P.MergedLaunchData",
    ),
    // 1149 writes this one with a lower-case `setup`.
    (
        "v1.setup.MGS.1vs100.~E.AvailableLanguages",
        "v3.Setup.MGS.1vs100.Store.~E.AvailableLanguages",
    ),
    (
        "v1.Setup.MGS.1vs100.QuestionBank.~E.Locale.~L.Difficulty.~DL.ID.~Q",
        "v3.Setup.MGS.1vs100.Store.QuestionBank.~E.Locale.~L.Difficulty.~DL.ID.~Q",
    ),
    (
        "v1.Setup.MGS.1vs100.QuestionBank.~E.Locale.~L.Difficulty.~DL.ResetID",
        "v3.Setup.MGS.1vs100.Store.QuestionBank.~E.Locale.~L.Difficulty.~DL.ResetID",
    ),
    // User documents and logs
    (
        "v1.Session.MGS.1vs100.UserStorage",
        "v3.Season.MGS.1vs100.UserStore.UserStorage",
    ),
    (
        "v1.Session.MGS.1vs100.UserStats",
        "v3.Season.MGS.1vs100.UserStore.UserStats",
    ),
    (
        "v1.Log.MGS.1vs100.UserError.~EC",
        "v3.Session.MGS.1vs100.Log.UserError.~EC",
    ),
    // Round / question documents (Season.Store data collection first: they share the
    // `.GameState` tail with the session GameState key below).
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.GameState",
        "v3.Season.MGS.1vs100.Store.~P.~GE.Round.~R.Question.~Q.GameState",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.AdBreak",
        "v3.Season.MGS.1vs100.Store.~P.~GE.Round.~R.Question.~Q.AdBreak",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.AutoHelp",
        "v3.Season.MGS.1vs100.Store.~P.~GE.Round.~R.Question.~Q.AutoHelp",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Stop",
        "v3.Season.MGS.1vs100.Store.~P.~GE.Stop",
    ),
    (
        "v1.Session.MGS.1vs100.~P.StaleState.~X",
        "v3.Season.MGS.1vs100.Store.~P.StaleState.~X",
    ),
    (
        "v1.Session.MGS.1vs100.~P.GameState",
        "v3.Session.MGS.1vs100.Store.~P.GameState",
    ),
    // Role determination
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.TheOne",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.~K.TheOne",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.MobSelection",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.~K.MobSelection",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.TheMob",
        "v3.Session.MGS.1vs100.Store.~P.~GE.~K.TheMob",
    ),
    (
        // HOSTTOOL_LOBBYPLAYERS; `Server_LobbyPlayers` appends the polling bucket on both builds.
        "v1.Session.MGS.1vs100.~P.~GE.LobbyPlayerCount.~N",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.LobbyPlayerCount.~N",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.RSTotalPlayerCount",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.~K.RSTotalPlayerCount",
    ),
    // Lua appends the bucket number to the RealAvatars keys (`...Bucket13`).
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.RealAvatars.Bucket~N",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.~K.RealAvatars.Bucket~N",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.RealAvatars.Bucket~N",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.RealAvatars.Bucket~N",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.~K.RealAvatars.Bucket",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.~K.RealAvatars.Bucket",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.RealAvatars.Bucket",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.RealAvatars.Bucket",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.RealAvatars.EPChosenPlayers",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.RealAvatars.EPChosenPlayers",
    ),
    // QOS (the title appends a suffix to the trailing-dot keys)
    (
        "v1.Session.MGS.1vs100.Program.~P.QOS.~K.Qualified",
        "v3.Session.MGS.1vs100.Sum.Program.~P.QOS.~K.Qualified",
    ),
    (
        "v1.Session.MGS.1vs100.Program.~P.QOS.~K.Disqualified",
        "v3.Session.MGS.1vs100.Sum.Program.~P.QOS.~K.Disqualified",
    ),
    (
        "v1.Session.MGS.1vs100.Program.~P.QOS.~K.RTT.~N",
        "v3.Session.MGS.1vs100.Sum.Program.~P.QOS.~K.RTT.~N",
    ),
    (
        "v1.Session.MGS.1vs100.Program.~P.QOS.~K.BandwidthUpstream.~N",
        "v3.Session.MGS.1vs100.Sum.Program.~P.QOS.~K.BandwidthUpstream.~N",
    ),
    (
        "v1.Session.MGS.1vs100.Program.~P.QOS.~K.BandwidthDownstream.~N",
        "v3.Session.MGS.1vs100.Sum.Program.~P.QOS.~K.BandwidthDownstream.~N",
    ),
    // Primetime question
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.MobChoices",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.Round.~R.Question.~Q.MobChoices",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.Brain",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.Round.~R.Question.~Q.Brain",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.Crowd.~C",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.Round.~R.Question.~Q.Crowd.~C",
    ),
    // Lua appends the bucket number (`...Bucket4`).
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.StatBreaks.~S.Bucket~N",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.Round.~R.Question.~Q.StatBreaks.~S.Bucket~N",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.StatBreaks.~S.Bucket",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.Round.~R.Question.~Q.StatBreaks.~S.Bucket",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.TotalPlayerCount",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.Round.~R.Question.~Q.TotalPlayerCount",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.RoundScores",
        "v3.Session.MGS.1vs100.TopN.~P.~GE.Round.~R.Question.~Q.RoundScores",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.MatchMake~M",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.Round.~R.Question.~Q.MatchMake~M",
    ),
    // Episode (per-question, no round)
    (
        "v1.Session.MGS.1vs100.~P.~GE.Question.~Q.TotalCorrect",
        "v3.Season.MGS.1vs100.Sum.~P.~GE.Question.~Q.TotalCorrect",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Question.~Q.TotalPlayerCount",
        "v3.Season.MGS.1vs100.Sum.~P.~GE.Question.~Q.TotalPlayerCount",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Question.~Q.RoundScores",
        "v3.Season.MGS.1vs100.TopN.~P.~GE.Question.~Q.RoundScores",
    ),
    // Lua appends the bucket number (`...Bucket4`).
    (
        "v1.Session.MGS.1vs100.~P.~GE.Question.~Q.StatBreaks.~S.Bucket~N",
        "v3.Season.MGS.1vs100.Sum.~P.~GE.Question.~Q.StatBreaks.~S.Bucket~N",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.Question.~Q.StatBreaks.~S.Bucket",
        "v3.Season.MGS.1vs100.Sum.~P.~GE.Question.~Q.StatBreaks.~S.Bucket",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.EPShowEndTopScores",
        "v3.Season.MGS.1vs100.TopN.~P.~GE.EPShowEndTopScores",
    ),
    // Question bank bookkeeping
    (
        "v1.Session.MGS.1vs100.~P.~GE.QuestionBank.Difficulty.~DL.Available",
        "v3.Session.MGS.1vs100.Store.~P.~GE.QuestionBank.Difficulty.~DL.Available",
    ),
    (
        "v1.Session.MGS.1vs100.~P.~GE.QuestionBank.Difficulty.~DL.Used",
        "v3.Session.MGS.1vs100.Store.~P.~GE.QuestionBank.Difficulty.~DL.Used",
    ),
    // Host tool / stats
    (
        "v1.Session.MGS.1vs100.~P.~GE.LobbyPlayerCount",
        "v3.Session.MGS.1vs100.Sum.~P.~GE.LobbyPlayerCount",
    ),
    (
        "v1.Stats.MGS.1vs100.~P.~GE.Round.~R.Question.~Q.HostTool.TopMobScore",
        "v3.Stats.MGS.1vs100.TopN.~P.~GE.Round.~R.Question.~Q.HostTool.TopMobScore",
    ),
    (
        "v1.Stats.MGS.1vs100.~P.~GE.HostTool.~H",
        "v3.Stats.MGS.1vs100.Sum.~P.~GE.HostTool.~H",
    ),
    (
        "v1.Stats.MGS.1vs100.StatsUpdate.~T.~Q",
        "v3.Stats.MGS.1vs100.Sum.StatsUpdate.~T.~Q",
    ),
    // Prizing
    (
        "v1.Season.MGS.1vs100.Prizing.Session.~P.Round.~R.RoundEndTopScores",
        "v3.Season.MGS.1vs100.TopN.Prizing.Session.~P.Round.~R.RoundEndTopScores",
    ),
    (
        "v1.Season.MGS.1vs100.Prizing.Session.~P.Xuid.~X.Round.~R",
        "v3.Season.MGS.1vs100.Store.Prizing.Session.~P.Xuid.~X.Round.~R",
    ),
];

/// The 1317 key for a 1149 key. Keys outside the table keep their namespace and get the `Store`
/// class (`v1.<ns>.MGS.1vs100.<rest>` -> `v3.<ns>.MGS.1vs100.Store.<rest>`); anything else is
/// returned unchanged.
pub fn to_store_key(key: &str) -> String {
    for (from, to) in KEYS {
        let Some(holes) = match_template(from, key) else {
            continue;
        };
        match fill_template(to, &holes) {
            Some(filled) => return filled,
            None => tracing::warn!(key, from, to, "SBG v1149 key template hole count mismatch"),
        }
    }
    if let Some((namespace, rest)) = key
        .strip_prefix("v1.")
        .and_then(|k| k.split_once(".MGS.1vs100."))
    {
        tracing::debug!(key, "SBG v1149 key not in the translation table");
        return format!("v3.{namespace}.MGS.1vs100.Store.{rest}");
    }
    key.to_owned()
}

/// A template split into literal text and placeholders (`~` + upper-case letters).
fn parts(template: &str) -> Vec<Part<'_>> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('~') {
        if start > 0 {
            out.push(Part::Lit(&rest[..start]));
        }
        rest = &rest[start..];
        let name_len = rest[1..]
            .find(|c: char| !c.is_ascii_uppercase())
            .unwrap_or(rest.len() - 1);
        out.push(Part::Hole);
        rest = &rest[1 + name_len..];
    }
    if !rest.is_empty() {
        out.push(Part::Lit(rest));
    }
    out
}

#[derive(Clone, Copy)]
enum Part<'a> {
    Lit(&'a str),
    Hole,
}

/// The placeholder values when `key` matches `template` as a whole. A placeholder takes at least
/// one character and may span dots (backtracking).
fn match_template<'k>(template: &str, key: &'k str) -> Option<Vec<&'k str>> {
    fn go<'k>(parts: &[Part<'_>], key: &'k str, holes: &mut Vec<&'k str>) -> bool {
        match parts.split_first() {
            None => key.is_empty(),
            Some((Part::Lit(lit), rest)) => key
                .strip_prefix(lit)
                .is_some_and(|tail| go(rest, tail, holes)),
            Some((Part::Hole, rest)) => {
                for end in key
                    .char_indices()
                    .map(|(i, c)| i + c.len_utf8())
                    .collect::<Vec<_>>()
                {
                    holes.push(&key[..end]);
                    if go(rest, &key[end..], holes) {
                        return true;
                    }
                    holes.pop();
                }
                false
            }
        }
    }
    let mut holes = Vec::new();
    go(&parts(template), key, &mut holes).then_some(holes)
}

/// `template` with each of its holes replaced from `holes`; `None` when the two disagree on how
/// many holes there are.
fn fill_template(template: &str, holes: &[&str]) -> Option<String> {
    let split = parts(template);
    if split
        .iter()
        .filter(|part| matches!(part, Part::Hole))
        .count()
        != holes.len()
    {
        return None;
    }
    let mut holes = holes.iter();
    split
        .into_iter()
        .map(|part| match part {
            Part::Lit(lit) => Some(lit),
            Part::Hole => holes.next().copied(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "89cb983b-7148-4ec1-ae87-2849e2ba9a7b";

    #[test]
    fn captured_startup_keys() {
        // Keys from the 2026-10-18 v2.0.1149.0 capture (fixtures/sbg-1149-capture-20261018.txt).
        assert_eq!(
            to_store_key("v1.Setup.MGS.1vs100.Config"),
            "v3.Setup.MGS.1vs100.Store.Config"
        );
        assert_eq!(
            to_store_key("v1.Log.MGS.1vs100.UserError.2159149064"),
            "v3.Session.MGS.1vs100.Log.UserError.2159149064"
        );
        // Native ad-engine config: no 1317 counterpart, falls back to the Store class.
        assert_eq!(
            to_store_key("v1.Setup.MGS.1vs100.Pangolin.Config"),
            "v3.Setup.MGS.1vs100.Store.Pangolin.Config"
        );
    }

    #[test]
    fn lobby_player_count_buckets_are_sums() {
        // Captured 2026-10-19 (EP lobby); `Server_LobbyPlayers` appends the polling bucket.
        let pid = "655a39c6-1a68-5db7-b04f-ad76f3b20018";
        for mode in ["EXTENDED_PLAY", "PRIMETIME"] {
            assert_eq!(
                to_store_key(&format!(
                    "v1.Session.MGS.1vs100.{pid}.{mode}.LobbyPlayerCount.60"
                )),
                format!("v3.Session.MGS.1vs100.Sum.{pid}.{mode}.LobbyPlayerCount.60")
            );
        }
    }

    #[test]
    fn real_avatar_buckets_keep_their_number_and_are_top_n() {
        // Captured 2026-10-19: Lua appends the bucket number to `...RealAvatars.Bucket`.
        let pid = "655a39c6-1a68-5db7-b04f-ad76f3b20018";
        assert_eq!(
            to_store_key(&format!(
                "v1.Session.MGS.1vs100.{pid}.EXTENDED_PLAY.RealAvatars.Bucket13"
            )),
            format!("v3.Session.MGS.1vs100.TopN.{pid}.EXTENDED_PLAY.RealAvatars.Bucket13")
        );
        assert_eq!(
            to_store_key(&format!(
                "v1.Session.MGS.1vs100.{pid}.PRIMETIME.81.RealAvatars.Bucket10"
            )),
            format!("v3.Session.MGS.1vs100.TopN.{pid}.PRIMETIME.81.RealAvatars.Bucket10")
        );
    }

    #[test]
    fn stat_break_buckets_keep_their_number_and_are_sums() {
        // Captured 2026-10-19 (EP); PT has the same form with `Round.<r>.`.
        let pid = "655a39c6-1a68-5db7-b04f-ad76f3b20018";
        for stat in ["NumCorrectAnswers", "Streak"] {
            assert_eq!(
                to_store_key(&format!(
                    "v1.Session.MGS.1vs100.{pid}.EXTENDED_PLAY.Question.3.StatBreaks.{stat}.Bucket4"
                )),
                format!(
                    "v3.Season.MGS.1vs100.Sum.{pid}.EXTENDED_PLAY.Question.3.StatBreaks.{stat}.Bucket4"
                )
            );
            assert_eq!(
                to_store_key(&format!(
                    "v1.Session.MGS.1vs100.{pid}.PRIMETIME.Round.1.Question.3.StatBreaks.{stat}.Bucket4"
                )),
                format!(
                    "v3.Session.MGS.1vs100.Sum.{pid}.PRIMETIME.Round.1.Question.3.StatBreaks.{stat}.Bucket4"
                )
            );
        }
    }

    #[test]
    fn lua_message_keys() {
        for (v1149, v1317) in [
            (
                "v1.Setup.MGS.1vs100.Config.Week1".to_owned(),
                "v3.Setup.MGS.1vs100.Store.Config.Week1".to_owned(),
            ),
            (
                "v1.Session.MGS.1vs100.UserStorage".to_owned(),
                "v3.Season.MGS.1vs100.UserStore.UserStorage".to_owned(),
            ),
            (
                format!("v1.Session.MGS.1vs100.{SESSION}.GameState"),
                format!("v3.Session.MGS.1vs100.Store.{SESSION}.GameState"),
            ),
            (
                format!("v1.Session.MGS.1vs100.{SESSION}.PRIMETIME.Round.2.Question.3.GameState"),
                format!(
                    "v3.Season.MGS.1vs100.Store.{SESSION}.PRIMETIME.Round.2.Question.3.GameState"
                ),
            ),
            (
                format!("v1.Session.MGS.1vs100.{SESSION}.PRIMETIME.Round.1.Question.1.MobChoices"),
                format!(
                    "v3.Session.MGS.1vs100.TopN.{SESSION}.PRIMETIME.Round.1.Question.1.MobChoices"
                ),
            ),
            (
                format!(
                    "v1.Session.MGS.1vs100.{SESSION}.PRIMETIME.Round.1.Question.1.Crowd.Choice2"
                ),
                format!(
                    "v3.Session.MGS.1vs100.Sum.{SESSION}.PRIMETIME.Round.1.Question.1.Crowd.Choice2"
                ),
            ),
            (
                format!("v1.Session.MGS.1vs100.{SESSION}.PRIMETIME.R1.TheMob"),
                format!("v3.Session.MGS.1vs100.Store.{SESSION}.PRIMETIME.R1.TheMob"),
            ),
            (
                "v1.setup.MGS.1vs100.00000000-0000-0000-0000-000000000000.AvailableLanguages"
                    .to_owned(),
                "v3.Setup.MGS.1vs100.Store.00000000-0000-0000-0000-000000000000.AvailableLanguages"
                    .to_owned(),
            ),
            (
                format!("v1.Session.MGS.1vs100.Program.{SESSION}.QOS.K.RTT.150"),
                format!("v3.Session.MGS.1vs100.Sum.Program.{SESSION}.QOS.K.RTT.150"),
            ),
            (
                "v1.Stats.MGS.1vs100.StatsUpdate.TotalSeen.251317".to_owned(),
                "v3.Stats.MGS.1vs100.Sum.StatsUpdate.TotalSeen.251317".to_owned(),
            ),
            (
                format!("v1.Session.MGS.1vs100.{SESSION}.TotalLoggers"),
                format!("v3.Session.MGS.1vs100.Store.{SESSION}.TotalLoggers"),
            ),
        ] {
            assert_eq!(to_store_key(&v1149), v1317, "{v1149}");
        }
    }

    #[test]
    fn every_translation_fills_all_of_its_holes() {
        let hole_count = |template: &str| {
            parts(template)
                .iter()
                .filter(|part| matches!(part, Part::Hole))
                .count()
        };
        for (from, to) in KEYS {
            assert_eq!(hole_count(from), hole_count(to), "{from} -> {to}");
        }
    }

    #[test]
    fn a_template_with_another_hole_count_fills_no_key() {
        assert_eq!(fill_template("a.~P.b", &["1"]), Some("a.1.b".to_owned()));
        assert_eq!(fill_template("a.~P.b", &[]), None);
        assert_eq!(fill_template("a.b", &["1"]), None);
    }

    #[test]
    fn foreign_keys_pass_through() {
        for key in ["v3.Setup.MGS.1vs100.Store.Config", "garbage", ""] {
            assert_eq!(to_store_key(key), key);
        }
    }
}
