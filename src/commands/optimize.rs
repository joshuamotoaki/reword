//! `reword optimize`: fit FSRS parameters to the user's own history.

use super::Ctx;
use crate::clock::format_ts;
use crate::error::{Error, Result};
use crate::memory::Model;
use crate::out;
use crate::text::plural;

/// Below this, fitted parameters are noise. The command says so.
const MIN_ITEMS: usize = 400;

pub fn run(ctx: &Ctx) -> Result<i32> {
    let store = &ctx.store;
    store.require()?;
    let names = store.list_decks()?;
    let decks = store.load_all(&names)?;
    ctx.report_warnings(&decks);
    let settings = ctx.settings()?;
    let current = ctx.model(&settings)?;

    let ledgers: Vec<_> = decks.iter().map(|d| &d.ledger).collect();
    let items = Model::training_items(&ledgers, &ctx.clock);
    if items.len() < MIN_ITEMS {
        return Err(Error::new(format!(
            "not enough history to fit parameters: {} across-day {}, need {MIN_ITEMS}",
            items.len(),
            if items.len() == 1 {
                "review"
            } else {
                "reviews"
            }
        ))
        .hint("Keep reviewing with the defaults; they fit most people well."));
    }

    let before = current.evaluate(items.clone())?;
    out::note(
        &ctx.term,
        &format!(
            "Fitting FSRS to {} across-day reviews. This can take a minute.",
            items.len()
        ),
    );
    let params = Model::optimize(items.clone())?;
    let fitted = Model::new(Some(&params), settings.desired_retention)?;
    let after = fitted.evaluate(items.clone())?;

    let improved = after.log_loss < before.log_loss;
    if ctx.json {
        out::json(&serde_json::json!({
            "items": items.len(),
            "before": { "log_loss": before.log_loss, "rmse_bins": before.rmse_bins, "source": if current.custom { "params.toml" } else { "default" } },
            "after": { "log_loss": after.log_loss, "rmse_bins": after.rmse_bins },
            "written": improved,
            "parameters": params,
        }));
    } else {
        out::println(&format!(
            "  {:<12} log loss {:.4}   RMSE {:.4}",
            if current.custom {
                "params.toml"
            } else {
                "default"
            },
            before.log_loss,
            before.rmse_bins
        ));
        out::println(&format!(
            "  {:<12} log loss {:.4}   RMSE {:.4}",
            "fitted", after.log_loss, after.rmse_bins
        ));
    }

    if !improved {
        out::note(
            &ctx.term,
            "The fitted parameters are not better than the current ones; nothing written.",
        );
        return Ok(0);
    }

    let body = format!(
        "# FSRS parameters fitted by `reword optimize` on {} from {}.\n\
         # Delete this file to return to the defaults.\n\
         parameters = [{}]\n",
        format_ts(ctx.clock.now()),
        plural(items.len(), "across-day review"),
        params
            .iter()
            .map(|p| format!("{p:.4}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    std::fs::write(store.params_path(), body)?;
    out::note(&ctx.term, &format!("Wrote {}/params.toml", store.display()));
    Ok(0)
}
