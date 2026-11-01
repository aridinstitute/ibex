use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use ibex_app::command::{CreateTag, ListTags};
use ibex_app::{AppState, Command, Outcome};
use ibex_core::NonEmptyStr;

#[derive(Debug, Args)]
pub struct TagCmd {
    #[command(subcommand)]
    pub action: TagAction,
}

#[derive(Debug, Subcommand)]
pub enum TagAction {
    /// List tags in the library.
    List,
    /// Create a tag with unique text.
    Add { text: String },
}

pub async fn run(state: &AppState, cmd: TagCmd) -> Result<()> {
    match cmd.action {
        TagAction::List => {
            let Outcome::ListTags(tags) = state.execute(Command::ListTags(ListTags)).await? else {
                bail!("unexpected outcome from list_tags");
            };
            if tags.is_empty() {
                println!("(none)");
            }
            for tag in tags {
                println!("{}  {}", tag.id, tag.text.as_str());
            }
        }
        TagAction::Add { text } => {
            let text = NonEmptyStr::try_new(&text)?;
            let Outcome::CreateTag(tag) = state
                .execute(Command::CreateTag(CreateTag { text }))
                .await?
            else {
                bail!("unexpected outcome from create_tag");
            };
            println!("{}  {}", tag.id, tag.text.as_str());
        }
    }
    Ok(())
}
