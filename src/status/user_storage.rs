//! The two fields of the title's `UserStorage` document that the public leaderboard reads
//! (`UserStorageTypes` in the title's `messages.lua`).

use super::ranking::Points;
use crate::xsor::table::{self, Node};

/// `PS`, chosen in the title's privacy menu (`PrivacyMenu:GetSettingsTable`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivacySetting {
    Public,
    Private,
    /// The setting of a child account: like public, but never named on screen.
    Child,
}

impl PrivacySetting {
    /// Whether the player may be named in public (the title's `Callout` eligibility).
    pub fn allows_callout(self) -> bool {
        self == Self::Public
    }

    /// v2.0.1317.0 stores the setting by name.
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "Public" => Some(Self::Public),
            "Private" => Some(Self::Private),
            "Child" => Some(Self::Child),
            _ => None,
        }
    }

    /// v2.0.1149.0 stores the settings table itself; an unconfirmed (`bValid` false) one is
    /// `None`, as the title then treats the player as private.
    fn from_table(node: &Node) -> Option<Self> {
        let flag = |name: &str| match node.child(name)?.text_value()?.as_ref() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        if !flag("bValid")? {
            return None;
        }
        let option = node.child("optionNum")?.text_value()?.parse::<f64>().ok()?;
        match (option, flag("Callout")?) {
            (1.0, true) => Some(Self::Public),
            (1.0, false) => Some(Self::Child),
            (2.0, false) => Some(Self::Private),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserStorage {
    /// `LS`, the player's aggregated lifetime score; the title caps it at `SeasonScoreCap`.
    pub lifetime_score: Option<Points>,
    pub privacy: Option<PrivacySetting>,
}

/// `None` unless `xml` is an XSOR document.
pub fn parse(xml: &[u8]) -> Option<UserStorage> {
    let root = table::parse(xml)?;
    let lifetime_score = root
        .child("LS")
        .and_then(Node::text_value)
        .and_then(|text| text.parse::<f64>().ok())
        .and_then(Points::from_score);
    let privacy = root.child("PS").and_then(|node| match node.text_value() {
        Some(name) => PrivacySetting::from_name(&name),
        None => PrivacySetting::from_table(node),
    });
    Some(UserStorage {
        lifetime_score,
        privacy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_documents_parse() {
        let parsed = [
            include_bytes!("../../fixtures/user-storage-1149.xml").as_slice(),
            include_bytes!("../../fixtures/user-storage-1317.xml"),
        ]
        .map(|xml| parse(xml).unwrap());
        insta::assert_debug_snapshot!(parsed);
    }

    #[test]
    fn privacy_tables_map_to_the_title_settings() {
        let table = |valid: u8, option: u8, callout: u8| {
            let xml = format!(
                "<XsorObject><PS type=\"table\"><bValid type=\"boolean\">{valid}</bValid>\
                 <optionNum type=\"float\">{option}</optionNum>\
                 <Callout type=\"boolean\">{callout}</Callout></PS></XsorObject>"
            );
            parse(xml.as_bytes()).unwrap().privacy
        };
        assert_eq!(table(1, 1, 1), Some(PrivacySetting::Public));
        assert_eq!(table(1, 1, 0), Some(PrivacySetting::Child));
        assert_eq!(table(1, 2, 0), Some(PrivacySetting::Private));
        assert_eq!(table(0, 1, 1), None, "not confirmed in the privacy menu");
        assert_eq!(table(1, 2, 1), None, "no setting of the title");
    }

    #[test]
    fn the_served_default_document_is_private() {
        let default = parse(&crate::xsor::user_storage_default_fixture()).unwrap();
        assert_eq!(default.privacy, None);
    }

    #[test]
    fn unusable_scores_are_absent() {
        for text in ["-1", "NaN", "inf", "", "many"] {
            let xml = format!("<XsorObject><LS type=\"float\">{text}</LS></XsorObject>");
            assert_eq!(
                parse(xml.as_bytes()).unwrap().lifetime_score,
                None,
                "{text}"
            );
        }
    }
}
