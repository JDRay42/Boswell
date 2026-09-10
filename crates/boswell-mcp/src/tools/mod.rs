//! MCP tool implementations

mod assert;
mod forget;
pub(crate) mod goals;
mod learn;
pub(crate) mod procedures;
mod query;
mod search;

pub use assert::{handle_assert, AssertParams};
pub use forget::{handle_forget, ForgetParams};
pub use goals::{
    handle_expand_goal, handle_get_goal, handle_query_goals, ExpandGoalParams, GetGoalParams,
    QueryGoalsParams,
};
pub use learn::{handle_learn, LearnParams};
pub use procedures::{
    handle_get_procedure, handle_query_procedures, handle_report_outcome, GetProcedureParams,
    QueryProceduresParams, ReportOutcomeParams,
};
pub use query::{handle_query, QueryParams};
pub use search::{handle_search, SearchParams};
