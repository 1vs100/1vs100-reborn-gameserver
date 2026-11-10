//! Player, connection and show-position identities: a XUID cannot be passed where a connection id
//! goes, nor a round where a question number does.

use std::{fmt, num::NonZeroU32};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A player's Xbox user id.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = u64)]
pub struct Xuid(pub u64);

impl Xuid {
    pub fn to_be_bytes(self) -> [u8; 8] {
        self.0.to_be_bytes()
    }
}

/// The 16-digit hex form the dump, the database and the admin's hex fields write.
impl fmt::LowerHex for Xuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The number of one connection, apart from every other connection's.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = u64)]
pub struct ConnId(pub u64);

impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The round of a show. Keys and GameStates number the round before the first one 0, so this type
/// admits it.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = u32)]
pub struct Round(pub u32);

impl fmt::Display for Round {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The question a show is on. The title writes 0 for the states before a round's first question,
/// which is no question at all and so not a number this type can hold.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = u32)]
pub struct QuestionNumber(pub NonZeroU32);

impl QuestionNumber {
    /// The first question of a round.
    pub const FIRST: Self = Self(NonZeroU32::MIN);

    /// `number` as a question number; `None` for 0, which names no question.
    pub fn new(number: u32) -> Option<Self> {
        NonZeroU32::new(number).map(Self)
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl fmt::Display for QuestionNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The id a question carries in the bank and in the title's `QuestionInfo`.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = u32)]
pub struct QuestionId(pub u32);

impl fmt::Display for QuestionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use utoipa::PartialSchema;

    use super::*;

    #[test]
    fn identities_are_numbers_in_json_and_in_the_api_document() {
        assert_eq!(serde_json::to_string(&Xuid(7)).unwrap(), "7");
        assert_eq!(serde_json::to_string(&ConnId(3)).unwrap(), "3");
        assert_eq!(serde_json::to_string(&Round(2)).unwrap(), "2");
        assert_eq!(serde_json::to_string(&QuestionNumber::FIRST).unwrap(), "1");
        assert_eq!(
            serde_json::to_string(&QuestionId(251_317)).unwrap(),
            "251317"
        );
        assert_eq!(
            serde_json::to_string(&Some(QuestionNumber::FIRST)).unwrap(),
            "1"
        );
        for schema in [
            serde_json::to_value(<Xuid as PartialSchema>::schema()).unwrap(),
            serde_json::to_value(<ConnId as PartialSchema>::schema()).unwrap(),
            serde_json::to_value(<Round as PartialSchema>::schema()).unwrap(),
            serde_json::to_value(<QuestionNumber as PartialSchema>::schema()).unwrap(),
            serde_json::to_value(<QuestionId as PartialSchema>::schema()).unwrap(),
        ] {
            assert_eq!(schema["type"], "integer", "{schema}");
        }
    }

    #[test]
    fn a_question_number_is_never_zero() {
        assert_eq!(QuestionNumber::new(0), None);
        assert_eq!(QuestionNumber::new(1), Some(QuestionNumber::FIRST));
        assert!(serde_json::from_str::<QuestionNumber>("0").is_err());
        assert!(serde_json::from_str::<QuestionNumber>("7").is_ok());
    }
}
