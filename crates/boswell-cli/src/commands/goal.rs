//! Goal-traversal commands (design 15 §3.2, §4.1).
//!
//! Traversal is stateless and agent-driven: the caller holds the cursor. `list`
//! finds an entry goal, `expand` surfaces one level, and the operator re-runs
//! `expand` on whichever child they pick until a candidate is a procedure.
//!
//! None of this issues an execution receipt — only fetching a leaf procedure
//! for execution does (see `commands::procedure`).

use crate::cli::GoalAction;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::{BoswellClient, GoalQuerySpec};

/// Execute the goal command.
pub async fn execute_goal(
    action: GoalAction,
    client: &mut BoswellClient,
    formatter: &Formatter,
    namespace_scope: Option<String>,
) -> Result<()> {
    match action {
        GoalAction::List {
            namespace,
            intent_contains,
            limit,
        } => {
            let spec = GoalQuerySpec {
                namespace: namespace.or(namespace_scope),
                intent_contains,
                limit,
            };
            let goals = client.query_goals(spec).await?;
            println!("{}", formatter.format_goals(&goals)?);
        }

        GoalAction::Show { id } => {
            let goal = client.get_goal(&id, namespace_scope).await?;
            match goal {
                Some(g) => println!("{}", formatter.format_goals(std::slice::from_ref(&g))?),
                None => {
                    return Err(CliError::InvalidInput(format!("No goal with id {}", id)));
                }
            }
        }

        GoalAction::Expand { id, context } => {
            let result = client.expand(&id, context, namespace_scope).await?;
            match result {
                Some(r) => println!("{}", formatter.format_expansion(&id, &r)?),
                None => {
                    return Err(CliError::InvalidInput(format!("No goal with id {}", id)));
                }
            }
        }
    }

    Ok(())
}
