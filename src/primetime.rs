use std::num::NonZeroU8;

use crate::{
    ids::{QuestionNumber, Round, Xuid},
    questions::{Choice, Difficulty},
};

const MOB_SIZE: usize = 100;
const VIRTUAL_XUID_BASE: u64 = 0xE000_FEED_0000_0000;
const CHOICE_SALT: u64 = 0xA076_1D64_78BD_642F;
const ROLL_SALT: u64 = 0xE703_7ED1_A0B4_28DB;

/// Every question of a round from its first through `last`.
fn up_to(last: QuestionNumber) -> impl Iterator<Item = QuestionNumber> {
    (1..=last.get()).map(|number| QuestionNumber::new(number).expect("counting up from 1"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatorError {
    QuestionOutOfOrder,
    ConflictingReplay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobVote {
    pub xuid: Xuid,
    pub choice: Choice,
    pub correct: bool,
    pub synthetic: bool,
}

struct Member {
    xuid: Xuid,
    skill: f64,
    alive: bool,
}

struct PreviousQuestion {
    number: QuestionNumber,
    difficulty: Difficulty,
    correct_choice: Choice,
    votes: Vec<MobVote>,
}

pub struct MobSimulator {
    seed: u64,
    members: Vec<Member>,
    previous: Option<PreviousQuestion>,
}

impl MobSimulator {
    /// `one`: the One's XUID when one is known; a member it would be taken for is left out.
    pub fn new(seed: u64, one: Option<Xuid>) -> Self {
        // Cyclic skill buckets; the title mean is unobserved.
        let members = (0..MOB_SIZE)
            .map(|index| Member {
                xuid: Xuid(VIRTUAL_XUID_BASE + index as u64 + 1),
                skill: (index % 9) as f64,
                alive: true,
            })
            .filter(|member| Some(member.xuid) != one)
            .collect();
        Self {
            seed,
            members,
            previous: None,
        }
    }

    pub fn answer(
        &mut self,
        question: QuestionNumber,
        difficulty: Difficulty,
        correct_choice: Choice,
    ) -> Result<Vec<MobVote>, SimulatorError> {
        if let Some(previous) = &self.previous {
            if question == previous.number {
                return if difficulty == previous.difficulty
                    && correct_choice == previous.correct_choice
                {
                    Ok(previous.votes.clone())
                } else {
                    Err(SimulatorError::ConflictingReplay)
                };
            }
            if question.get() != previous.number.get() + 1 {
                return Err(SimulatorError::QuestionOutOfOrder);
            }
        } else if question != QuestionNumber::FIRST {
            return Err(SimulatorError::QuestionOutOfOrder);
        }

        let votes = self
            .members
            .iter_mut()
            .filter(|member| member.alive)
            .map(|member| {
                let correct = answers_correctly(self.seed, member, question, difficulty);
                let choice = member_choice(self.seed, member, question, correct, correct_choice);
                member.alive = correct;
                MobVote {
                    xuid: member.xuid,
                    choice,
                    correct,
                    synthetic: true,
                }
            })
            .collect::<Vec<_>>();
        self.previous = Some(PreviousQuestion {
            number: question,
            difficulty,
            correct_choice,
            votes: votes.clone(),
        });
        Ok(votes)
    }
}

/// Whether `member` answers `question` correctly; eliminated members keep answering as ExMob.
fn answers_correctly(
    seed: u64,
    member: &Member,
    question: QuestionNumber,
    difficulty: Difficulty,
) -> bool {
    let probability = 0.5 - 0.15 * (difficulty.level() - member.skill);
    unit_roll(seed, member.xuid, question, ROLL_SALT) < probability
}

/// Stat-break type: what each player is bucketed by.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StatBreak {
    /// `roundQuestionsCorrect`, 5 buckets.
    NumCorrectAnswers,
    /// `roundCurrentStreak`, 4 buckets.
    Streak,
}

impl StatBreak {
    pub fn buckets(self) -> usize {
        match self {
            Self::NumCorrectAnswers => 5,
            Self::Streak => 4,
        }
    }

    /// The `number`th bucket of this stat; `None` when the stat has fewer.
    pub fn bucket(self, number: u8) -> Option<Bucket> {
        NonZeroU8::new(number)
            .filter(|number| usize::from(number.get()) <= self.buckets())
            .map(|number| Bucket { stat: self, number })
    }
}

/// One bucket of a stat-break: bucket 1 holds the lowest stat. Made only by
/// [`StatBreak::bucket`], so its number is one its stat has.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Bucket {
    stat: StatBreak,
    number: NonZeroU8,
}

impl Bucket {
    pub fn stat(self) -> StatBreak {
        self.stat
    }

    /// The bucket's place in a list of its stat's buckets, counted from 0.
    pub fn index(self) -> usize {
        usize::from(self.number.get()) - 1
    }
}

/// Lowest stat per bucket after `question` (`statbreak.lua`).
pub fn stat_break_thresholds(question: QuestionNumber, buckets: usize) -> Vec<u32> {
    let q = f64::from(question.get());
    let n = buckets as f64;
    (1..=buckets)
        .map(|i| match i {
            1 => 0,
            _ => (q - q * (1.0 - (i as f64 - 1.0) / (n - 1.0))).ceil() as u32,
        })
        .collect()
}

/// Simulated Mob members in `bucket` after `question` of `round`.
pub fn synthetic_stat_break_bucket(
    round: Round,
    question: QuestionNumber,
    difficulty: Difficulty,
    bucket: Bucket,
) -> u32 {
    synthetic_stat_break(round, question, difficulty, bucket.stat())[bucket.index()]
}

/// Simulated Mob members per stat-break bucket after `question` of `round`, stat included.
pub fn synthetic_stat_break(
    round: Round,
    question: QuestionNumber,
    difficulty: Difficulty,
    stat: StatBreak,
) -> Vec<u32> {
    let mob = MobSimulator::new(0x1_5100_0000 ^ u64::from(round.0), None);
    let thresholds = stat_break_thresholds(question, stat.buckets());
    let mut counts = vec![0; stat.buckets()];
    for member in &mob.members {
        let (mut correct, mut streak) = (0, 0);
        for number in up_to(question) {
            let level = if number == question {
                difficulty
            } else {
                Difficulty::from_level(number.get().min(5))
            };
            if answers_correctly(mob.seed, member, number, level) {
                correct += 1;
                streak += 1;
            } else {
                streak = 0;
            }
        }
        let value = match stat {
            StatBreak::NumCorrectAnswers => correct,
            StatBreak::Streak => streak,
        };
        let bucket = thresholds
            .iter()
            .rposition(|lowest| value >= *lowest)
            // The first threshold is 0, so some threshold always matches.
            .unwrap_or(0);
        counts[bucket] += 1;
    }
    counts
}

/// The choice `member` gives on `question`: the correct one, or one of the two others.
fn member_choice(
    seed: u64,
    member: &Member,
    question: QuestionNumber,
    correct: bool,
    correct_choice: Choice,
) -> Choice {
    if correct {
        correct_choice
    } else {
        wrong_choice(
            correct_choice,
            random_word(seed, member.xuid, question, CHOICE_SALT) as usize & 1,
        )
    }
}

/// Points per correct answer on the simulated Brain board.
const BRAIN_POINTS_PER_CORRECT_ANSWER: f64 = 700.0;

/// Simulated Brain board for `question` of `round`, in client shape.
pub fn synthetic_brain(
    round: Round,
    question: QuestionNumber,
    difficulty: Difficulty,
    correct_choice: Choice,
) -> Vec<(f64, Vec<u8>)> {
    let mob = MobSimulator::new(0x1_5100_0000 ^ u64::from(round.0), None);
    let mut board: Vec<(f64, Vec<u8>)> = mob
        .members
        .iter()
        .map(|member| {
            let earlier = up_to(question)
                .take_while(|number| *number != question)
                .filter(|number| {
                    answers_correctly(
                        mob.seed,
                        member,
                        *number,
                        Difficulty::from_level(number.get().min(5)),
                    )
                })
                .count();
            let score = if question == QuestionNumber::FIRST {
                member.skill
            } else {
                BRAIN_POINTS_PER_CORRECT_ANSWER * earlier as f64 + member.skill
            };
            let correct = answers_correctly(mob.seed, member, question, difficulty);
            let choice = member_choice(mob.seed, member, question, correct, correct_choice);
            let xuid = MOB_SELECTION_XUID_BASE + (member.xuid.0 - VIRTUAL_XUID_BASE - 1);
            (
                score,
                format!(
                    "<Metadata type=\"table\"><xuid>{xuid}</xuid><choice type=\"float\">{choice}</choice></Metadata>"
                )
                .into_bytes(),
            )
        })
        .collect();
    // Stable: equal scores keep member order, as a TopN keeps first arrival.
    board.sort_by(|left, right| right.0.total_cmp(&left.0));
    board.truncate(10);
    board
}

/// First XUID of the MobSelection slate; votes share these so the title matches them.
pub(crate) const MOB_SELECTION_XUID_BASE: u64 = 0xF000_0000_0000_0001;

/// MobChoices entries for `question` of `round`; replays 1..=`question` so eliminations carry over.
pub fn synthetic_mob_choices(
    round: Round,
    question: QuestionNumber,
    difficulty: Difficulty,
    correct_choice: Choice,
) -> Vec<(f64, Vec<u8>)> {
    let mut mob = MobSimulator::new(0x1_5100_0000 ^ u64::from(round.0), None);
    let mut votes = Vec::new();
    for number in up_to(question) {
        let (level, correct) = if number == question {
            (difficulty, correct_choice)
        } else {
            (Difficulty::from_level(number.get().min(5)), Choice::C)
        };
        match mob.answer(number, level, correct) {
            Ok(result) => votes = result,
            Err(_) => return Vec::new(),
        }
    }
    votes
        .into_iter()
        .map(|vote| {
            let xuid = MOB_SELECTION_XUID_BASE + (vote.xuid.0 - VIRTUAL_XUID_BASE - 1);
            (
                f64::from(vote.choice.get()),
                format!(
                    "<Metadata><xuid>{xuid}</xuid><correct type=\"boolean\">{}</correct></Metadata>",
                    u8::from(vote.correct)
                )
                .into_bytes(),
            )
        })
        .collect()
}

/// One of the two choices that are not `correct`, picked by `index` (0 or 1).
fn wrong_choice(correct: Choice, index: usize) -> Choice {
    match correct {
        Choice::A => [Choice::B, Choice::C][index],
        Choice::B => [Choice::A, Choice::C][index],
        Choice::C => [Choice::A, Choice::B][index],
    }
}

fn unit_roll(seed: u64, xuid: Xuid, question: QuestionNumber, salt: u64) -> f64 {
    unit(random_word(seed, xuid, question, salt))
}

fn random_word(seed: u64, xuid: Xuid, question: QuestionNumber, salt: u64) -> u64 {
    mix64(
        seed ^ xuid.0.rotate_left(23) ^ u64::from(question.get()).wrapping_mul(GOLDEN_GAMMA) ^ salt,
    )
}

/// The 64-bit golden ratio, SplitMix64's increment.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64's finaliser: the one mixer of the simulation (the Mob, its Brain and the Crowd).
fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

/// A mixed word as a fraction in [0, 1), from its top 53 bits.
fn unit(word: u64) -> f64 {
    (word >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{MOB_SIZE, MobSimulator};
    use crate::{
        ids::{QuestionNumber, Xuid},
        questions::{Choice, Difficulty},
    };

    const ONE_XUID: Xuid = Xuid(0xE000_07FF_484D_4B37);

    fn question(number: u32) -> QuestionNumber {
        QuestionNumber::new(number).expect("a question number is not 0")
    }

    fn controlled_mob() -> MobSimulator {
        let mut mob = MobSimulator::new(7, Some(ONE_XUID));
        mob.members.truncate(2);
        mob.members[0].skill = 8.0;
        mob.members[1].skill = -10.0;
        mob
    }

    #[test]
    fn fixed_seed_replays_same_synthetic_mob_answers() {
        let mut first = MobSimulator::new(7, Some(ONE_XUID));
        let mut replay = MobSimulator::new(7, Some(ONE_XUID));

        assert_eq!(
            first
                .answer(question(1), Difficulty::Medium, Choice::B)
                .unwrap(),
            replay
                .answer(question(1), Difficulty::Medium, Choice::B)
                .unwrap()
        );
    }

    #[test]
    fn roster_has_100_unique_synthetic_xuids_distinct_from_the_one() {
        let mut mob = MobSimulator::new(7, Some(ONE_XUID));
        let votes = mob
            .answer(question(1), Difficulty::Easy, Choice::B)
            .unwrap();
        let xuids: HashSet<_> = votes.iter().map(|vote| vote.xuid).collect();

        assert_eq!(votes.len(), 100);
        assert_eq!(xuids.len(), 100);
        assert!(!xuids.contains(&ONE_XUID));
        assert!(votes.iter().all(|vote| vote.synthetic));
        assert!(votes.iter().all(|vote| vote.xuid.0 >> 32 == 0xE000_FEED));
    }

    #[test]
    fn correctness_metadata_matches_choice_and_source_probability_rule() {
        let mut mob = controlled_mob();
        let votes = mob
            .answer(question(1), Difficulty::VeryEasy, Choice::B)
            .unwrap();

        assert_eq!(votes.len(), 2);
        assert!(votes[0].correct);
        assert_eq!(votes[0].choice, Choice::B);
        assert!(!votes[1].correct);
        assert_ne!(votes[1].choice, Choice::B);
    }

    #[test]
    fn only_survivors_vote_on_the_next_question() {
        let mut mob = controlled_mob();
        let first = mob
            .answer(question(1), Difficulty::VeryEasy, Choice::B)
            .unwrap();
        let second = mob
            .answer(question(2), Difficulty::VeryHard, Choice::A)
            .unwrap();

        let survivors: HashSet<_> = first
            .iter()
            .filter(|vote| vote.correct)
            .map(|vote| vote.xuid)
            .collect();
        assert_eq!(survivors.len(), 1);
        assert_eq!(second.len(), 1);
        assert!(survivors.contains(&second[0].xuid));
    }

    #[test]
    fn repeated_question_fetch_is_idempotent() {
        let mut mob = controlled_mob();
        let first = mob
            .answer(question(1), Difficulty::VeryEasy, Choice::B)
            .unwrap();
        let replay = mob
            .answer(question(1), Difficulty::VeryEasy, Choice::B)
            .unwrap();

        assert_eq!(first, replay);
        assert_eq!(
            mob.answer(question(2), Difficulty::VeryHard, Choice::A)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn no_survivors_produce_no_votes() {
        let mut mob = controlled_mob();
        for member in &mut mob.members {
            member.alive = false;
        }

        assert!(
            mob.answer(question(1), Difficulty::Medium, Choice::B)
                .unwrap()
                .is_empty()
        );
        assert!(
            mob.answer(question(2), Difficulty::Medium, Choice::B)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn all_correct_answers_preserve_the_entire_mob() {
        let mut mob = controlled_mob();
        for member in &mut mob.members {
            member.skill = 8.0;
        }

        let votes = mob
            .answer(question(1), Difficulty::VeryEasy, Choice::C)
            .unwrap();
        assert_eq!(votes.len(), 2);
        assert!(
            votes
                .iter()
                .all(|vote| vote.correct && vote.choice == Choice::C)
        );
    }

    #[test]
    fn a_one_inside_the_synthetic_namespace_is_not_simulated() {
        let one = Xuid(0xE000_FEED_0000_0001);
        let mut mob = MobSimulator::new(7, Some(one));
        let votes = mob
            .answer(question(1), Difficulty::Easy, Choice::B)
            .unwrap();

        assert_eq!(votes.len(), MOB_SIZE - 1);
        assert!(votes.iter().all(|vote| vote.xuid != one));
    }
}

/// Share of simulated crowd members that answer correctly.
fn crowd_accuracy(difficulty: Difficulty) -> f64 {
    match difficulty {
        Difficulty::VeryEasy => 0.85,
        Difficulty::Easy => 0.75,
        Difficulty::Medium => 0.62,
        Difficulty::Hard => 0.5,
        Difficulty::VeryHard => 0.4,
    }
}

/// Crowd votes for one question; deterministic per console. Never drives progression.
pub fn synthetic_crowd_votes(
    round: Round,
    question: QuestionNumber,
    difficulty: Difficulty,
    correct: Choice,
    size: u32,
) -> [u32; 3] {
    let mut votes = [0u32; 3];
    let accuracy = crowd_accuracy(difficulty);
    let seed = (u64::from(round.0) << 40) ^ (u64::from(question.get()) << 20) ^ 0xC20D;
    for member in 0..size {
        let h =
            mix64((seed ^ u64::from(member).wrapping_mul(0x1000_0001)).wrapping_add(GOLDEN_GAMMA));
        let roll = unit(h);
        let choice = if roll < accuracy {
            correct
        } else {
            // one of the two wrong choices, chosen by another bit of the hash
            wrong_choice(correct, (h & 1) as usize)
        };
        votes[choice.index()] += 1;
    }
    votes
}

#[cfg(test)]
mod crowd_tests {
    use super::synthetic_crowd_votes;
    use crate::{
        ids::{QuestionNumber, Round},
        questions::{Choice, Difficulty},
    };

    fn question(number: u32) -> QuestionNumber {
        QuestionNumber::new(number).expect("a question number is not 0")
    }

    #[test]
    fn crowd_is_deterministic_and_leans_to_the_right_answer() {
        let a = synthetic_crowd_votes(
            Round(1),
            question(1),
            Difficulty::from_level(1),
            Choice::C,
            100,
        );
        assert_eq!(
            a,
            synthetic_crowd_votes(
                Round(1),
                question(1),
                Difficulty::from_level(1),
                Choice::C,
                100
            )
        );
        assert_eq!(a.iter().sum::<u32>(), 100);
        assert!(a[2] > a[0] && a[2] > a[1], "{a:?}");
        // harder questions are answered worse
        assert!(
            synthetic_crowd_votes(
                Round(1),
                question(3),
                Difficulty::from_level(5),
                Choice::B,
                1000
            )[1] < synthetic_crowd_votes(
                Round(1),
                question(3),
                Difficulty::from_level(1),
                Choice::B,
                1000
            )[1]
        );
    }
}

#[cfg(test)]
mod stat_break_tests {
    use super::{
        MOB_SIZE, StatBreak, stat_break_thresholds, synthetic_mob_choices, synthetic_stat_break,
    };
    use crate::{
        ids::{QuestionNumber, Round},
        questions::{Choice, Difficulty},
    };

    fn question(number: u32) -> QuestionNumber {
        QuestionNumber::new(number).expect("a question number is not 0")
    }

    #[test]
    fn a_bucket_is_numbered_from_one_up_to_its_stats_count() {
        assert_eq!(StatBreak::NumCorrectAnswers.bucket(0), None);
        assert_eq!(StatBreak::NumCorrectAnswers.bucket(5).unwrap().index(), 4);
        assert_eq!(StatBreak::NumCorrectAnswers.bucket(6), None);
        assert_eq!(StatBreak::Streak.bucket(4).unwrap().index(), 3);
        assert_eq!(StatBreak::Streak.bucket(5), None);
    }

    #[test]
    fn thresholds_are_the_titles_and_match_exact_arithmetic() {
        assert_eq!(stat_break_thresholds(question(1), 5), [0, 1, 1, 1, 1]);
        assert_eq!(stat_break_thresholds(question(2), 5), [0, 1, 1, 2, 2]);
        assert_eq!(stat_break_thresholds(question(3), 5), [0, 1, 2, 3, 3]);
        assert_eq!(stat_break_thresholds(question(9), 4), [0, 3, 6, 9]);
        // The title computes in doubles; no question of a show rounds differently from integers.
        for number in 1..=40u32 {
            for buckets in [4usize, 5] {
                let exact: Vec<u32> = (1..=buckets as u32)
                    .map(|i| (number * (i - 1)).div_ceil(buckets as u32 - 1))
                    .collect();
                assert_eq!(
                    stat_break_thresholds(question(number), buckets),
                    exact,
                    "q{number} n{buckets}"
                );
            }
        }
    }

    #[test]
    fn every_simulated_member_is_in_exactly_one_bucket() {
        for number in 1..=6 {
            for stat in [StatBreak::NumCorrectAnswers, StatBreak::Streak] {
                let counts = synthetic_stat_break(
                    Round(1),
                    question(number),
                    Difficulty::from_level(2),
                    stat,
                );
                assert_eq!(counts.len(), stat.buckets());
                assert_eq!(
                    counts.iter().sum::<u32>() as usize,
                    MOB_SIZE,
                    "q{number} {stat:?}"
                );
            }
        }
        let (round, question, difficulty) = (Round(2), question(4), Difficulty::from_level(3));
        assert_eq!(
            synthetic_stat_break(round, question, difficulty, StatBreak::Streak),
            synthetic_stat_break(round, question, difficulty, StatBreak::Streak)
        );
    }

    #[test]
    fn after_question_one_the_top_bucket_holds_the_mob_that_answered_correctly() {
        let correct_votes =
            synthetic_mob_choices(Round(1), question(1), Difficulty::from_level(1), Choice::C)
                .iter()
                .filter(|(_, metadata)| metadata.ends_with(b">1</correct></Metadata>"))
                .count() as u32;
        let counts = synthetic_stat_break(
            Round(1),
            question(1),
            Difficulty::from_level(1),
            StatBreak::NumCorrectAnswers,
        );
        assert_eq!(counts[4], correct_votes);
        assert_eq!(counts[0], MOB_SIZE as u32 - correct_votes);
    }
}

#[cfg(test)]
mod brain_tests {
    use super::{synthetic_brain, synthetic_mob_choices};
    use crate::{
        ids::{QuestionNumber, Round},
        questions::{Choice, Difficulty},
    };

    fn question(number: u32) -> QuestionNumber {
        QuestionNumber::new(number).expect("a question number is not 0")
    }

    /// The choices in a board or a MobChoices reply, by member xuid.
    fn choices(entries: &[(f64, Vec<u8>)], field: &str) -> Vec<(String, u8)> {
        entries
            .iter()
            .map(|(score, metadata)| {
                let xml = String::from_utf8(metadata.clone()).unwrap();
                let xuid = xml
                    .split("<xuid>")
                    .nth(1)
                    .unwrap()
                    .split('<')
                    .next()
                    .unwrap();
                let choice = match field {
                    "choice" => xml.split("\">").nth(2).unwrap()[..1].parse().unwrap(),
                    _ => *score as u8,
                };
                (xuid.to_owned(), choice)
            })
            .collect()
    }

    #[test]
    fn the_board_is_the_ten_highest_in_the_clients_shape() {
        let board = synthetic_brain(Round(1), question(3), Difficulty::from_level(2), Choice::B);
        assert_eq!(board.len(), 10);
        assert!(board.windows(2).all(|pair| pair[0].0 >= pair[1].0));
        let xml = String::from_utf8(board[0].1.clone()).unwrap();
        assert!(xml.starts_with("<Metadata type=\"table\"><xuid>"), "{xml}");
        assert!(xml.ends_with("</choice></Metadata>"), "{xml}");
        assert_eq!(
            board,
            synthetic_brain(Round(1), question(3), Difficulty::from_level(2), Choice::B)
        );
    }

    #[test]
    fn a_member_still_in_the_mob_answers_as_in_mob_choices() {
        let votes = choices(
            &synthetic_mob_choices(Round(1), question(2), Difficulty::from_level(2), Choice::C),
            "score",
        );
        let board = choices(
            &synthetic_brain(Round(1), question(2), Difficulty::from_level(2), Choice::C),
            "choice",
        );
        let mut compared = 0;
        for (xuid, choice) in &board {
            if let Some((_, voted)) = votes.iter().find(|(member, _)| member == xuid) {
                assert_eq!(choice, voted, "{xuid}");
                compared += 1;
            }
        }
        assert!(compared > 0, "the board holds members still in the Mob");
    }

    #[test]
    fn on_an_easy_question_the_brain_mostly_knows_the_answer() {
        for correct in Choice::ALL {
            let board = choices(
                &synthetic_brain(Round(1), question(1), Difficulty::from_level(1), correct),
                "choice",
            );
            let right = board
                .iter()
                .filter(|(_, choice)| *choice == correct.get())
                .count();
            assert!(right > board.len() / 2, "{right} of {}", board.len());
        }
    }
}
