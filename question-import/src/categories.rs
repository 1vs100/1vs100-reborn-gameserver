//! Our question categories: every source maps its own categories onto these, so questions
//! of several sources fill one category. A special names one of these ids. The list grows with
//! the sources: a source category nobody mapped yet gets one of its own (`otdb-33`) instead.

/// (id, name), in the order a bank lists them.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("general-knowledge", "General Knowledge"),
    ("books", "Books"),
    ("film", "Film"),
    ("music", "Music"),
    ("musicals-theatre", "Musicals & Theatre"),
    ("television", "Television"),
    ("video-games", "Video Games"),
    ("board-games", "Board Games"),
    ("science-nature", "Science & Nature"),
    ("computers", "Computers"),
    ("mathematics", "Mathematics"),
    ("mythology", "Mythology"),
    ("sports", "Sports"),
    ("geography", "Geography"),
    ("history", "History"),
    ("politics", "Politics"),
    ("art", "Art"),
    ("celebrities", "Celebrities"),
    ("animals", "Animals"),
    ("vehicles", "Vehicles"),
    ("comics", "Comics"),
    ("gadgets", "Gadgets"),
    ("anime-manga", "Anime & Manga"),
    ("cartoons-animation", "Cartoons & Animation"),
];

/// The name of our category `id`.
pub fn name(id: &str) -> Option<&'static str> {
    CATEGORIES
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_lowercase_slugs() {
        let mut seen = std::collections::HashSet::new();
        for (id, name) in CATEGORIES {
            assert!(seen.insert(*id), "{id} twice");
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{id}"
            );
            assert!(!name.is_empty());
        }
        assert_eq!(name("vehicles"), Some("Vehicles"));
        assert_eq!(name("otdb-33"), None);
    }
}
