//! One handler per kind of request.

mod counters;
mod game_state;
mod store;
mod top_n;
mod user;

pub(in crate::sbg) use {counters::*, game_state::*, store::*, top_n::*, user::*};
