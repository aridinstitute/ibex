use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use oeuvre_app::command::{
    CreatePerson, DeletePerson, GetPerson, ListPersons, UpdatePerson,
};
use oeuvre_app::{AppState, Command, Outcome};
use oeuvre_core::{Person, PersonId};

use crate::name::NameFlags;

#[derive(Debug, Args)]
pub struct PersonCmd {
    #[command(subcommand)]
    pub action: PersonAction,
}

#[derive(Debug, Subcommand)]
pub enum PersonAction {
    /// Add a person. With no name flags, prompts interactively.
    Add(NameFlags),
    /// Fetch one person by id.
    Get { id: String },
    /// List people. With name flags, filters by that name.
    List(NameFlags),
    /// Update a person's primary name. With no name flags, prompts interactively.
    Update {
        id: String,
        #[command(flatten)]
        name: NameFlags,
    },
    /// Delete a person by id.
    Delete { id: String },
}

pub async fn run(state: &AppState, cmd: PersonCmd) -> Result<()> {
    match cmd.action {
        PersonAction::Add(name) => add(state, name).await,
        PersonAction::Get { id } => get(state, &id).await,
        PersonAction::List(name) => list(state, name).await,
        PersonAction::Update { id, name } => update(state, &id, name).await,
        PersonAction::Delete { id } => delete(state, &id).await,
    }
}

async fn add(state: &AppState, name: NameFlags) -> Result<()> {
    let name = name.resolve_for_write()?;
    let Outcome::CreatePerson(person) = state
        .execute(Command::CreatePerson(CreatePerson {
            name,
            aliases: vec![],
        }))
        .await?
    else {
        bail!("unexpected outcome from create_person");
    };
    print_person(&person);
    Ok(())
}

async fn get(state: &AppState, id: &str) -> Result<()> {
    let id = parse_id(id)?;
    let Outcome::GetPerson(person) = state
        .execute(Command::GetPerson(GetPerson { id }))
        .await?
    else {
        bail!("unexpected outcome from get_person");
    };
    print_person(&person);
    Ok(())
}

async fn list(state: &AppState, flags: NameFlags) -> Result<()> {
    let filter = flags.into_filter();
    let Outcome::ListPersons(people) = state
        .execute(Command::ListPersons(ListPersons))
        .await?
    else {
        bail!("unexpected outcome from list_persons");
    };

    let people: Vec<_> = if filter.is_empty() {
        people
    } else {
        people
            .into_iter()
            .filter(|p| filter.matches_person(p))
            .collect()
    };

    if people.is_empty() {
        println!("(none)");
        return Ok(());
    }
    for person in &people {
        print_person(person);
    }
    Ok(())
}

async fn update(state: &AppState, id: &str, name: NameFlags) -> Result<()> {
    let id = parse_id(id)?;
    let name = name.resolve_for_write()?;

    let Outcome::GetPerson(existing) = state
        .execute(Command::GetPerson(GetPerson { id }))
        .await?
    else {
        bail!("unexpected outcome from get_person");
    };

    let Outcome::UpdatePerson(person) = state
        .execute(Command::UpdatePerson(UpdatePerson {
            id,
            name,
            aliases: existing.aliases,
        }))
        .await?
    else {
        bail!("unexpected outcome from update_person");
    };
    print_person(&person);
    Ok(())
}

async fn delete(state: &AppState, id: &str) -> Result<()> {
    let id = parse_id(id)?;
    let Outcome::DeletePerson(deleted) = state
        .execute(Command::DeletePerson(DeletePerson { id }))
        .await?
    else {
        bail!("unexpected outcome from delete_person");
    };
    println!("deleted {}", deleted.id);
    Ok(())
}

fn parse_id(id: &str) -> Result<PersonId> {
    PersonId::parse(id).with_context(|| format!("invalid person id '{id}'"))
}

fn print_person(person: &Person) {
    print!("{}  {}", person.id, person.name.display());
    if !person.aliases.is_empty() {
        let aliases: Vec<_> = person.aliases.iter().map(|a| a.display()).collect();
        print!("  aliases: {}", aliases.join("; "));
    }
    println!();
}
