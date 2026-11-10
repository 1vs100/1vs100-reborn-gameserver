use serde::Deserialize;

#[derive(Deserialize)]
struct Question {
    #[serde(rename = "Question")]
    text: String,
    #[serde(rename = "CorrectAnswer")]
    correct_answer: TypedInteger,
    #[serde(rename = "ChoiceList")]
    choices: Choices,
}

#[derive(Deserialize)]
struct TypedInteger {
    #[serde(rename = "@type")]
    kind: String,
    #[serde(rename = "$text")]
    value: u32,
}

#[derive(Deserialize)]
struct Choices {
    #[serde(rename = "XsorItem")]
    items: Vec<String>,
}

#[test]
fn captured_one_question_fixture_preserves_content_and_types() {
    let question: Question =
        quick_xml::de::from_str(include_str!("../fixtures/question-251317-en-us.xml"))
            .expect("question fixture must deserialize");

    assert_eq!(question.text, "Which company makes and sells the iPod?");
    assert_eq!(question.correct_answer.kind, "integer");
    assert_eq!(question.correct_answer.value, 3);
    assert_eq!(question.choices.items, ["Dell", "Nokia", "Apple"]);
}
