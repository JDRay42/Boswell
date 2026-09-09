//! Procedure retrieval and outcome reporting (design 15 §3.3).
//!
//! Retrieval carries an obligation. Every procedure handed out comes with an
//! execution receipt, and the principal it was issued to must answer it before
//! it expires — an unanswered receipt counts as `unknown` against the
//! procedure, because silence is not success. The commands here print the
//! receipts they create so an operator cannot take on an obligation without
//! seeing it.

use crate::cli::ProcedureAction;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use boswell_sdk::{BoswellClient, OutcomeReportSpec, ProcedureQuerySpec};

/// Execute the procedure command.
pub async fn execute_procedure(
    action: ProcedureAction,
    client: &mut BoswellClient,
    formatter: &Formatter,
    namespace_scope: Option<String>,
    default_principal: &str,
) -> Result<()> {
    match action {
        ProcedureAction::List {
            goal,
            namespace,
            intent_contains,
            include_superseded,
            limit,
            as_principal,
            task_id,
            session_id,
        } => {
            let spec = ProcedureQuerySpec {
                issued_to: as_principal.unwrap_or_else(|| default_principal.to_string()),
                namespace: namespace.or(namespace_scope),
                goal,
                intent_contains,
                include_superseded,
                limit,
                task_id,
                session_id,
            };
            let issued = client.query_procedures(spec).await?;
            println!("{}", formatter.format_issued_procedures(&issued)?);
        }

        ProcedureAction::Show {
            id,
            as_principal,
            task_id,
            session_id,
        } => {
            let issued_to = as_principal.unwrap_or_else(|| default_principal.to_string());
            let issued = client
                .get_procedure(&id, &issued_to, namespace_scope, task_id, session_id)
                .await?;
            match issued {
                Some(d) => println!(
                    "{}",
                    formatter.format_issued_procedures(std::slice::from_ref(&d))?
                ),
                None => {
                    return Err(CliError::InvalidInput(format!(
                        "No procedure with id {}",
                        id
                    )));
                }
            }
        }

        ProcedureAction::Report {
            receipt_id,
            outcome,
            failure_mode,
            failed_step,
            executor_confidence,
            cost,
            notes,
        } => {
            // Refused here rather than at the instance so the operator gets a
            // clear message: a failure attribution on a success would silently
            // mis-file the outcome.
            if failure_mode.is_some() && !matches!(outcome, crate::cli::OutcomeArg::Failure) {
                return Err(CliError::InvalidInput(
                    "--failure-mode is only valid with --outcome failure".to_string(),
                ));
            }
            if let Some(c) = executor_confidence {
                if !(0.0..=1.0).contains(&c) {
                    return Err(CliError::InvalidInput(
                        "--executor-confidence must be between 0.0 and 1.0".to_string(),
                    ));
                }
            }

            let spec = OutcomeReportSpec {
                receipt_id: receipt_id.clone(),
                outcome: outcome.as_str().to_string(),
                failure_mode: failure_mode.map(|f| f.as_str().to_string()),
                failed_step,
                executor_confidence,
                cost,
                notes,
            };

            let resp = client.report_outcome(spec).await?;
            if !resp.accepted && !resp.already_final {
                return Err(CliError::InvalidInput(format!(
                    "No outstanding receipt with id {}",
                    receipt_id
                )));
            }
            println!("{}", formatter.format_report_outcome(&resp)?);
        }
    }

    Ok(())
}
