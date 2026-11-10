//! Query strings read into the documented `*Params` structs.

use std::collections::BTreeMap;

use super::Reply;

pub(super) type Query = BTreeMap<String, String>;

/// Optional `pt|ep` query value.
pub(super) fn mode(
    value: Option<&str>,
    name: &str,
) -> Result<Option<crate::schedule::Mode>, Reply> {
    value
        .map(|value| {
            crate::schedule::Mode::parse(value)
                .ok_or_else(|| Reply::error(400, format!("{name} must be pt or ep")))
        })
        .transpose()
}

/// The query read into `T`, one of the `*Params` structs below, whose `IntoParams` documents
/// the same fields: a missing required field is "missing `name`", a value that does not parse
/// "invalid `name`", and names `T` does not have are ignored.
pub(super) fn query_as<T: serde::de::DeserializeOwned>(query: &Query) -> Result<T, Reply> {
    read(query).map_err(|error| Reply::error(400, error.to_string()))
}

/// [`query_as`] before the error becomes the reply it is answered with.
fn read<T: serde::de::DeserializeOwned>(query: &Query) -> Result<T, QueryError> {
    let fields = query.iter().map(|(name, value)| {
        (
            name.as_str(),
            QueryValue {
                name: name.as_str(),
                value: value.as_str(),
            },
        )
    });
    T::deserialize(serde::de::value::MapDeserializer::<_, QueryError>::new(
        fields,
    ))
}

/// `required`'s rule for a field `query_as` read: an empty value is a missing one.
pub(super) fn non_empty(value: String, name: &str) -> Result<String, Reply> {
    if value.is_empty() {
        Err(Reply::error(400, format!("missing `{name}`")))
    } else {
        Ok(value)
    }
}

/// Why a query does not read into its `*Params` struct.
#[derive(Debug, PartialEq, Eq)]
enum QueryError {
    /// The query holds no value for a field the struct requires.
    Missing { name: String },
    /// The value is not what the field's type parses.
    Invalid { name: String },
    /// serde's own message, which its `Error` defaults build their errors from.
    Custom { message: String },
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Missing { name } => write!(f, "missing `{name}`"),
            QueryError::Invalid { name } => write!(f, "invalid `{name}`"),
            QueryError::Custom { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for QueryError {}

impl serde::de::Error for QueryError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        QueryError::Custom {
            message: message.to_string(),
        }
    }

    fn missing_field(field: &'static str) -> Self {
        QueryError::Missing {
            name: field.to_owned(),
        }
    }
}

/// One decoded query value; numbers are parsed with `FromStr`, as the handlers did by hand.
#[derive(Clone, Copy)]
struct QueryValue<'q> {
    name: &'q str,
    value: &'q str,
}

impl<'q> QueryValue<'q> {
    fn parse<T: std::str::FromStr>(self) -> Result<T, QueryError> {
        self.value.parse().map_err(|_| QueryError::Invalid {
            name: self.name.to_owned(),
        })
    }
}

impl<'de, 'q> serde::de::IntoDeserializer<'de, QueryError> for QueryValue<'q> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

macro_rules! parse_number {
    ($($method:ident => $visit:ident),* $(,)?) => {
        $(fn $method<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, QueryError> {
            visitor.$visit(self.parse()?)
        })*
    };
}

impl<'de, 'q> serde::Deserializer<'de> for QueryValue<'q> {
    type Error = QueryError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, QueryError> {
        visitor.visit_str(self.value)
    }

    /// A value that is there is `Some`, even an empty one.
    fn deserialize_option<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, QueryError> {
        visitor.visit_some(self)
    }

    /// A unit variant named by the value; any other value is invalid.
    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, QueryError> {
        use serde::de::IntoDeserializer;
        visitor
            .visit_enum(IntoDeserializer::<QueryError>::into_deserializer(
                self.value,
            ))
            .map_err(|_| QueryError::Invalid {
                name: self.name.to_owned(),
            })
    }

    parse_number! {
        deserialize_bool => visit_bool,
        deserialize_u8 => visit_u8,
        deserialize_u16 => visit_u16,
        deserialize_u32 => visit_u32,
        deserialize_u64 => visit_u64,
        deserialize_i8 => visit_i8,
        deserialize_i16 => visit_i16,
        deserialize_i32 => visit_i32,
        deserialize_i64 => visit_i64,
        deserialize_f32 => visit_f32,
        deserialize_f64 => visit_f64,
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
        map struct identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct Params {
        limit: Option<u32>,
        xuid: String,
    }

    fn query(pairs: &[(&str, &str)]) -> Query {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn read_error(pairs: &[(&str, &str)]) -> QueryError {
        read::<Params>(&query(pairs)).unwrap_err()
    }

    #[test]
    fn a_value_that_does_not_parse_is_told_apart_from_a_missing_one() {
        let params = read::<Params>(&query(&[("limit", "3"), ("xuid", "e0")])).unwrap();
        assert_eq!((params.limit, params.xuid.as_str()), (Some(3), "e0"));
        assert_eq!(
            read_error(&[("limit", "many"), ("xuid", "e0")]),
            QueryError::Invalid {
                name: "limit".into()
            }
        );
        assert_eq!(
            read_error(&[("limit", "3")]),
            QueryError::Missing {
                name: "xuid".into()
            }
        );
    }

    #[test]
    fn a_unit_variant_is_read_by_name() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Level {
            Warn,
        }
        #[derive(Debug, serde::Deserialize)]
        struct Params {
            level: Option<Level>,
        }
        let read_level = |value| read::<Params>(&query(&[("level", value)])).map(|p| p.level);
        assert_eq!(read_level("warn"), Ok(Some(Level::Warn)));
        assert_eq!(
            read_level("loud"),
            Err(QueryError::Invalid {
                name: "level".into()
            })
        );
    }
}
