//! Title state lengths and substate timings, from component sums, not literals.

use std::time::Duration;

/// `PTTimingsTable.RoundStartTable` sub-timings.
pub const DETERMINE_ONE_TIME: Duration = Duration::from_secs(24);
pub const FILLER_ONE_TIME: Duration = Duration::from_secs(11);
pub const MOB_WON_AWARD_TIME: Duration = Duration::from_secs(10);
pub const ELECT_HOST_TIME: Duration = Duration::from_secs(6);
pub const OPEN_DOORS_TIME: Duration = Duration::from_secs(5);
pub const BUMPER_TIME: Duration = Duration::from_secs(10);
pub const LIVE_HOST_INTRO_TIME: Duration = Duration::from_secs(7);
pub const SHOW_INTRO_TIME: Duration = Duration::from_secs(6);
pub const MOB_INTRO_TIME: Duration = Duration::from_secs(5);
pub const CROWD_INTRO_TIME: Duration = Duration::from_secs(8);
pub const HOST_INTRO_HANDOFF_TIME: Duration = Duration::from_secs(6);
pub const ONE_INTRODUCTION_TIME: Duration = Duration::from_secs(8);
pub const INITIAL_EXPLANATION_TIME: Duration = Duration::from_secs(5);

/// `PTTimingsTable.QALoopTable` QA-base components.
/// `readAnswersTime` (0 in the table, ~4 at runtime) is added separately.
pub const ONE_VS_XX_TIME: Duration = Duration::from_secs(3);
pub const CROWD_REACTION_TIME: Duration = Duration::from_secs(2);
pub const SHOW_VALUE_TIME: Duration = Duration::from_secs(1);
pub const READ_QUESTION_TIME: Duration = Duration::from_secs(4);
pub const SUBMIT_ANSWERS_TIME: Duration = Duration::from_secs(4);
pub const ANSWER_OR_HELP_BUFFER_TIME: Duration = Duration::from_secs(6);
/// Runtime answer-presentation buffers, 4 s total.
pub const PRE_BUFFER_TIME: Duration = Duration::from_millis(750);
pub const BETWEEN_CHOICES_1_2_TIME: Duration = Duration::from_millis(750);
pub const BETWEEN_CHOICES_2_3_TIME: Duration = Duration::from_millis(750);
pub const POST_BUFFER_TIME: Duration = Duration::from_millis(1_750);
pub const READ_ANSWERS_RUNTIME: Duration = PRE_BUFFER_TIME
    .saturating_add(BETWEEN_CHOICES_1_2_TIME)
    .saturating_add(BETWEEN_CHOICES_2_3_TIME)
    .saturating_add(POST_BUFFER_TIME);
/// MoneyOrMob components as summed; staged states assume elimination time 5 s.
pub const MOM_ELIMINATE_TIME: Duration = Duration::from_secs(5);
pub const MOM_RESULTS_TIME: Duration = Duration::from_secs(4);
pub const MOM_OFFER_TIME: Duration = Duration::from_secs(5);
pub const MOM_HARD_STOP_TIME: Duration = Duration::from_secs(12);
pub const MOM_BUFFER_TIME: Duration = Duration::from_secs(6);
/// MoneyOrMob length for a staged state.
pub const MOM: Duration = MOM_ELIMINATE_TIME
    .saturating_add(MOM_RESULTS_TIME)
    .saturating_add(MOM_BUFFER_TIME)
    .saturating_add(MOM_OFFER_TIME);

/// Ad break lengths per `AdTimingResetPT` entry.
pub const AD_BREAK: [Duration; 4] = [
    Duration::from_secs(86),
    Duration::from_secs(171),
    Duration::from_secs(101),
    Duration::from_secs(101),
];
/// Served `QALoopStatTime`, overwriting the table's 60.
pub const QA_LOOP_STAT_TIME: Duration = Duration::from_secs(10);
/// Stat break length: intro + stat + outro.
const STAT_BREAK_PADDING: Duration = Duration::from_secs(5);
pub const STAT_BREAK: Duration = STAT_BREAK_PADDING
    .saturating_add(QA_LOOP_STAT_TIME)
    .saturating_add(STAT_BREAK_PADDING);

/// `OnesAnswer` without the answer reveal.
pub const ONES_ANSWER_BASE: Duration = Duration::from_secs(4 + 2 + 4);
/// Answer reveal by Mob tier, posted as `AnswerRevealTime`.
pub const ANSWER_REVEAL: [Duration; 2] = [Duration::from_secs(6), Duration::from_secs(7)];

/// Whether MoneyOrMob ends in the hard stop instead of the offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardStop {
    Yes,
    No,
}

/// Whether QAMobElimination skips its breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuddenStop {
    Yes,
    No,
}

/// MoneyOrMob length for elimination time `elim`; the hard stop replaces the offer.
pub fn money_or_mob(elim: Duration, hard_stop: HardStop, ad: Duration, stat: Duration) -> Duration {
    let tail = match hard_stop {
        HardStop::Yes => MOM_HARD_STOP_TIME,
        HardStop::No => MOM_OFFER_TIME + ad + stat,
    };
    elim + MOM_RESULTS_TIME + MOM_BUFFER_TIME + tail
}

/// Same for QAMobElimination, without the offer; a sudden stop skips the breaks.
pub fn qa_mob_elimination(
    elim: Duration,
    sudden_stop: SuddenStop,
    ad: Duration,
    stat: Duration,
) -> Duration {
    let breaks = match sudden_stop {
        SuddenStop::Yes => Duration::ZERO,
        SuddenStop::No => ad + stat,
    };
    elim + MOM_RESULTS_TIME + breaks
}

/// `PTTimingsTable.JoinInProgressTable`.
pub const JIP_PRE_BUMPER_TIME: Duration = Duration::from_secs(3);
pub const JIP_BUMPER_TIME: Duration = Duration::from_secs(10);

/// `PTTimingsTable.ServerStateTimeLength` - computed exactly as the Lua
/// sums them.
pub const ELECT_HOST: Duration = DETERMINE_ONE_TIME
    .saturating_add(FILLER_ONE_TIME)
    .saturating_add(MOB_WON_AWARD_TIME)
    .saturating_add(ELECT_HOST_TIME);
pub const ROUND_START: Duration = DETERMINE_ONE_TIME.saturating_add(FILLER_ONE_TIME);
pub const LIVE_HOST_INTRO: Duration = OPEN_DOORS_TIME
    .saturating_add(LIVE_HOST_INTRO_TIME)
    .saturating_add(BUMPER_TIME);
pub const INTRODUCTIONS: Duration = SHOW_INTRO_TIME
    .saturating_add(MOB_INTRO_TIME)
    .saturating_add(CROWD_INTRO_TIME)
    .saturating_add(HOST_INTRO_HANDOFF_TIME)
    .saturating_add(ONE_INTRODUCTION_TIME)
    .saturating_add(INITIAL_EXPLANATION_TIME);
pub const QA_BASE: Duration = ONE_VS_XX_TIME
    .saturating_add(CROWD_REACTION_TIME)
    .saturating_add(SHOW_VALUE_TIME)
    .saturating_add(READ_QUESTION_TIME)
    .saturating_add(SUBMIT_ANSWERS_TIME)
    .saturating_add(ANSWER_OR_HELP_BUFFER_TIME);
/// QA as published: base + runtime `readAnswersTime`.
pub const QA: Duration = QA_BASE.saturating_add(READ_ANSWERS_RUNTIME);

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn mob_elimination_lengths_follow_the_title_formula() {
        assert_eq!(money_or_mob(secs(5), HardStop::No, secs(0), secs(0)), MOM);
        assert_eq!(
            money_or_mob(secs(5), HardStop::Yes, secs(0), secs(0)),
            secs(27)
        );
        assert_eq!(
            money_or_mob(secs(7), HardStop::No, AD_BREAK[0], secs(0)),
            secs(108)
        );
        // live-observed: QAMobElimination 11 s = 7 + 4; with the first ad (elim 8): 8 + 4 + 86
        assert_eq!(
            qa_mob_elimination(secs(7), SuddenStop::No, secs(0), secs(0)),
            secs(11)
        );
        assert_eq!(
            qa_mob_elimination(secs(8), SuddenStop::No, AD_BREAK[0], secs(0)),
            secs(98)
        );
        assert_eq!(
            qa_mob_elimination(secs(8), SuddenStop::Yes, AD_BREAK[0], STAT_BREAK),
            secs(12)
        );
    }
}
