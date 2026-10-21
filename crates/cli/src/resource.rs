use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use oeuvre_app::command::ListResources;
use oeuvre_app::{AppState, Command, Outcome};
use oeuvre_core::ResourceSummary;

#[derive(Debug, Args)]
pub struct ResourceCmd {
    #[command(subcommand)]
    pub action: ResourceAction,
}

#[derive(Debug, Subcommand)]
pub enum ResourceAction {
    /// List resources as summary rows, newest first.
    List,
}

pub async fn run(state: &AppState, cmd: ResourceCmd) -> Result<()> {
    match cmd.action {
        ResourceAction::List => list(state).await,
    }
}

async fn list(state: &AppState) -> Result<()> {
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await?
    else {
        bail!("unexpected outcome from list_resources");
    };

    if rows.is_empty() {
        println!("(none)");
        return Ok(());
    }
    for row in &rows {
        println!("{}", format_summary_row(row));
    }
    Ok(())
}

/// One line per summary: `id  kind  title  creators  (year)  in container`,
/// omitting fields the row does not carry. Publisher stays out of the row
/// to keep lines readable; `resource get` will surface it later.
fn format_summary_row(row: &ResourceSummary) -> String {
    let mut out = format!("{}  {}  {}", row.id, row.kind.as_str(), row.title.as_str());
    if !row.creators_label.is_empty() {
        out.push_str("  ");
        out.push_str(&row.creators_label);
    }
    if let Some(year) = &row.published_label {
        out.push_str("  (");
        out.push_str(year.as_str());
        out.push(')');
    }
    if let Some(container) = &row.container_title {
        out.push_str("  in ");
        out.push_str(container.as_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::{NonEmptyStr, ResourceId, ResourceKind};

    fn summary(
        title: &str,
        creators: &str,
        published: Option<&str>,
        container: Option<&str>,
    ) -> ResourceSummary {
        ResourceSummary {
            id: ResourceId::generate(),
            kind: ResourceKind::JournalArticle,
            title: NonEmptyStr::try_new(title).expect("title non-empty"),
            creators_label: creators.to_owned(),
            published_label: published
                .map(|p| NonEmptyStr::try_new(p).expect("published non-empty")),
            publisher: None,
            container_title: container
                .map(|c| NonEmptyStr::try_new(c).expect("container non-empty")),
        }
    }

    #[test]
    fn formats_full_row() {
        let row = summary(
            "Quantum bibliometrics",
            "Ada; Grace",
            Some("2024"),
            Some("Oeuvre Journal"),
        );
        assert_eq!(
            format_summary_row(&row),
            format!(
                "{}  journalArticle  Quantum bibliometrics  Ada; Grace  (2024)  in Oeuvre Journal",
                row.id
            )
        );
    }

    #[test]
    fn omits_absent_fields() {
        let row = summary("A Lone Title", "", None, None);
        assert_eq!(
            format_summary_row(&row),
            format!("{}  journalArticle  A Lone Title", row.id)
        );
    }

    #[test]
    fn omits_empty_creators_but_keeps_year() {
        let row = summary("Solo Work", "", Some("2020"), None);
        assert_eq!(
            format_summary_row(&row),
            format!("{}  journalArticle  Solo Work  (2020)", row.id)
        );
    }
}
