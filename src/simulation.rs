//! The simulated Mob and Crowd as the consoles read them: what a sum or a TopN board holds, plus
//! the simulated participants the options ask for. Only reads; the show holds the real posts
//! alone. The simulated answers come from `primetime`, deterministic per (round, question).

use crate::{
    ids::{QuestionNumber, Round},
    questions::{Question, QuestionBank},
    sbg::key::{Board, ElectionItem, Key, ParsedKey, QuestionSum},
    show::{Show, TopN},
    store::SimulationOptions,
};

/// What the SBG read path reads: `show` with the simulated participants of `options` added.
pub struct SimulatedReads<'a> {
    show: &'a Show,
    options: SimulationOptions,
    bank: &'a QuestionBank,
}

impl<'a> SimulatedReads<'a> {
    /// `bank`: the served bank, the questions the simulation answers.
    pub fn new(show: &'a Show, options: SimulationOptions, bank: &'a QuestionBank) -> Self {
        Self {
            show,
            options,
            bank,
        }
    }

    /// Asked question, else the bank's first with a warning.
    fn asked_question(&self, round: Round, question: QuestionNumber) -> &'a Question {
        let bank = self.bank;
        let asked = self.show.asked(round, question).and_then(|id| bank.get(id));
        asked.unwrap_or_else(|| {
            tracing::warn!(round = round.0, question = question.get(), "no QA stored for this question: the synthetic mob and crowd answer the bank's first question");
            bank.fallback().expect("the bank served is never empty")
        })
    }

    pub fn sum(&self, parsed: &ParsedKey) -> Option<f64> {
        let key = parsed.raw.as_str();
        if self.options.synthetic_crowd > 0
            && let Key::QuestionSum {
                round,
                question: Some(question),
                sum: QuestionSum::Crowd(choice),
                ..
            } = parsed.kind
        {
            let served = self.asked_question(round, question);
            let simulated = crate::primetime::synthetic_crowd_votes(
                round,
                question,
                served.difficulty,
                served.correct,
                self.options.synthetic_crowd,
            );
            let real = self.show.sum(key).unwrap_or(0.0);
            return Some(real + f64::from(simulated[choice.index()]));
        }
        if self.options.synthetic_mob
            && let Key::QuestionSum {
                session,
                round,
                question: Some(question),
                sum: QuestionSum::StatBreak(bucket),
            } = &parsed.kind
            && session.is_canonical()
        {
            // The simulated Mob posts to the stat-break buckets as its 100 members would; a
            // bucket nobody is in stays NOT_FOUND.
            let served = self.asked_question(*round, *question);
            let simulated = crate::primetime::synthetic_stat_break_bucket(
                *round,
                *question,
                served.difficulty,
                *bucket,
            );
            return match (self.show.sum(key), simulated) {
                (None, 0) => None,
                (real, simulated) => Some(real.unwrap_or(0.0) + f64::from(simulated)),
            };
        }
        let total = self.show.sum(key)?;
        Some(
            if self.options.synthetic_mob
                && is_canonical_election(parsed, ElectionItem::RsTotalPlayerCount)
            {
                total + 101.0
            } else {
                total
            },
        )
    }

    pub fn top_n(&self, parsed: &ParsedKey) -> Option<Vec<(f64, Vec<u8>)>> {
        let key = parsed.raw.as_str();
        if self.options.synthetic_mob
            && let Some((round, question)) = canonical_board(parsed, Board::MobChoices)
        {
            let mut entries = self.show.top_n(key).map(TopN::pairs).unwrap_or_default();
            let served = self.asked_question(round, question);
            entries.extend(crate::primetime::synthetic_mob_choices(
                round,
                question,
                served.difficulty,
                served.correct,
            ));
            entries.sort_by(|left, right| right.0.total_cmp(&left.0));
            entries.truncate(100);
            return Some(entries);
        }
        if self.options.synthetic_mob
            && let Some((round, question)) = canonical_board(parsed, Board::Brain)
        {
            // Without a Brain the title gives Trust the Brain the correct answer: the simulated
            // Mob posts one.
            let mut entries = self.show.top_n(key).map(TopN::pairs).unwrap_or_default();
            let asked = self.asked_question(round, question);
            entries.extend(crate::primetime::synthetic_brain(
                round,
                question,
                asked.difficulty,
                asked.correct,
            ));
            entries.sort_by(|left, right| right.0.total_cmp(&left.0));
            entries.truncate(10);
            return Some(entries);
        }
        let top_n = self.show.top_n(key)?;
        let mut entries = top_n.pairs();
        if self.options.synthetic_mob && is_canonical_election(parsed, ElectionItem::MobSelection) {
            entries.extend((0..100u64).map(|offset| {
                let xuid = crate::primetime::MOB_SELECTION_XUID_BASE + offset;
                (
                    0.6743096,
                    crate::xsor::table::Node::string("Metadata", format!("{xuid:016x}")).to_bytes(),
                )
            }));
        }
        entries.sort_by(|left, right| right.0.total_cmp(&left.0));
        entries.truncate(top_n.capacity());
        Some(entries)
    }
}

/// A key of the election window `item` in the canonical session (the simulation's).
pub fn is_canonical_election(parsed: &ParsedKey, item: ElectionItem) -> bool {
    matches!(&parsed.kind, Key::Election { session, item: found, .. } if *found == item && session.is_canonical())
}

/// `(round, question)` of a question's `board` in the canonical session (the simulation's).
pub(crate) fn canonical_board(parsed: &ParsedKey, board: Board) -> Option<(Round, QuestionNumber)> {
    match &parsed.kind {
        Key::QuestionBoard {
            session,
            round,
            question: Some(question),
            board: found,
        } if *found == board && session.is_canonical() => Some((*round, *question)),
        _ => None,
    }
}
