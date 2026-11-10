use onevs100_gameserver::{
    questions::ShowSeed,
    sbg::key::ParsedKey,
    store::{GetError, Store},
};

const SETUP_PREFIX: &str =
    "v3.Setup.MGS.1vs100.Store.QuestionBank.00000000-0000-0000-0000-000000000000.Locale";
const SESSION_PREFIX: &str = "v3.Session.MGS.1vs100.Store.00000000-0000-0000-0000-000000000001.PRIMETIME.QuestionBank.Difficulty";
/// The show the bank keys above belong to.
fn show() -> ShowSeed {
    ShowSeed::of("00000000-0000-0000-0000-000000000000")
}
const CONFIG: &str = "v3.Setup.MGS.1vs100.Store.Config";
const QUESTION: &[u8] = include_bytes!("../fixtures/question-251317-en-us.xml");

/// Every question-bank key the title reads, for each difficulty.
fn bank_keys(difficulty: u32) -> [String; 4] {
    [
        format!("{SETUP_PREFIX}.en-US.Difficulty.{difficulty}.ResetID"),
        format!("{SETUP_PREFIX}.en-us.Difficulty.{difficulty}.ResetID"),
        format!("{SESSION_PREFIX}.{difficulty}.Available"),
        format!("{SESSION_PREFIX}.{difficulty}.Used"),
    ]
}

#[test]
fn generated_documents_are_not_seeded_into_the_store() {
    // SBG answers these keys from the generators (`question_list_get`, `question_get`, the per-request Config),
    // so a stored copy would only show the admin a document the consoles never receive.
    let store = Store::bootstrap();
    let mut keys: Vec<String> = (1..=5).flat_map(bank_keys).collect();
    keys.push(format!("{SETUP_PREFIX}.en-US.Difficulty.1.ID.251317"));
    keys.push(CONFIG.to_owned());
    for key in keys {
        assert_eq!(store.get(&key), Err(GetError::NotFound), "{key}");
    }
}

#[test]
fn question_bank_keys_are_answered_from_the_served_bank() {
    let store = Store::bootstrap();
    for difficulty in 1..=5 {
        for key in bank_keys(difficulty) {
            let document = store.question_list_get(&ParsedKey::parse(&key), None, show());
            assert!(document.is_some(), "{key}");
        }
        let available = store
            .question_list_get(
                &ParsedKey::parse(&format!("{SESSION_PREFIX}.{difficulty}.Available")),
                None,
                show(),
            )
            .unwrap();
        assert!(
            String::from_utf8(available).unwrap().contains("<XsorItem"),
            "difficulty {difficulty} has questions"
        );
    }
    for key in [
        format!("{SETUP_PREFIX}.en-US.Difficulty.1.ID.251317"),
        format!("{SETUP_PREFIX}.en-us.Difficulty.1.ID.251317"),
    ] {
        assert_eq!(
            store.question_get(&ParsedKey::parse(&key)).as_deref(),
            Some(QUESTION),
            "{key}"
        );
    }
    assert_eq!(
        store.question_get(&ParsedKey::parse(&format!(
            "{SETUP_PREFIX}.en-US.Difficulty.1.ID.999999"
        ))),
        None
    );
}

#[test]
fn fixture_preserves_trace_backed_question_types_and_choices() {
    let xml = std::str::from_utf8(QUESTION).unwrap();

    assert!(xml.contains("<Question>Which company makes and sells the iPod?</Question>"));
    assert!(xml.contains("<CorrectAnswer type=\"integer\">3</CorrectAnswer>"));
    assert_eq!(xml.matches("<XsorItem>").count(), 3);
    assert!(xml.contains("<XsorItem>Dell</XsorItem>"));
    assert!(xml.contains("<XsorItem>Nokia</XsorItem>"));
    assert!(xml.contains("<XsorItem>Apple</XsorItem>"));
}

#[test]
fn the_titles_own_question_lists_are_read_back() {
    // The title writes Available and Used back as it picks questions; it is answered what it
    // wrote, in the form the bank's lists are served in.
    let mut store = Store::bootstrap();
    let available = format!("{SESSION_PREFIX}.1.Available");
    let used = format!("{SESSION_PREFIX}.1.Used");
    let get = |store: &Store, key: &str| {
        String::from_utf8(
            store
                .question_list_get(&ParsedKey::parse(key), None, show())
                .unwrap(),
        )
        .unwrap()
    };
    let bank = get(&store, &available);
    assert!(
        bank.contains(">251317<") && bank.contains(">251402<"),
        "{bank}"
    );
    assert_eq!(get(&store, &used), "<XsorObject />");

    // 1149 posts the lists as `xsor:object`, 1317 as `XsorObject` (both from the recordings).
    store.set(
        available.clone(),
        br#"<xsor:object xmlns:xsor="http://schemas.xbox.com/xsor" name="SBG.Actions" type="table"><xsor:item type="float">251402</xsor:item></xsor:object>"#.to_vec(),
    );
    store.set(
        used.clone(),
        br#"<XsorObject><XsorItem type="float">251317</XsorItem><XsorItem type="float">251401</XsorItem></XsorObject>"#.to_vec(),
    );
    assert_eq!(
        get(&store, &available),
        r#"<XsorObject><XsorItem type="float">251402</XsorItem></XsorObject>"#
    );
    assert_eq!(
        get(&store, &used),
        r#"<XsorObject><XsorItem type="float">251317</XsorItem><XsorItem type="float">251401</XsorItem></XsorObject>"#
    );
    store.set(used.clone(), b"<XsorObject />".to_vec());
    assert_eq!(get(&store, &used), "<XsorObject />", "emptied by the title");
    assert!(
        get(&store, &format!("{SESSION_PREFIX}.2.Available")).contains("<XsorItem"),
        "other difficulties: still the bank"
    );

    store.set(available.clone(), b"not a list".to_vec());
    assert_eq!(get(&store, &available), bank, "unreadable: the bank's list");

    store.set(
        available.clone(),
        br#"<XsorObject><XsorItem type="float">251402</XsorItem></XsorObject>"#.to_vec(),
    );
    store.reset_session();
    assert_eq!(
        get(&store, &available),
        bank,
        "a new show starts from the bank"
    );
}
