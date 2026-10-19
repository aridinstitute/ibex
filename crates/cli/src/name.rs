use anyhow::Result;
use clap::Args;
use dialoguer::Input;
use oeuvre_core::{Name, Person};

#[derive(Debug, Clone, Args)]
pub struct NameFlags {
    /// Given / first name.
    #[arg(long)]
    pub first: Option<String>,

    /// Family / last name (mononyms go here).
    #[arg(long)]
    pub last: Option<String>,

    /// Generational or honorific suffix (Jr., III, …).
    #[arg(long)]
    pub suffix: Option<String>,
}

/// Optional normalized name parts for list filtering. Empty filter = no constraint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamePartFilter {
    first: Option<String>,
    last: Option<String>,
    suffix: Option<String>,
}

impl NameFlags {
    pub fn any_set(&self) -> bool {
        self.first.is_some() || self.last.is_some() || self.suffix.is_some()
    }

    pub fn into_name(self) -> Result<Name> {
        Name::try_new(
            self.first.as_deref(),
            self.last.as_deref(),
            self.suffix.as_deref(),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Flags if any were passed; otherwise prompt interactively. Add/update only.
    pub fn resolve_for_write(self) -> Result<Name> {
        if self.any_set() {
            self.into_name()
        } else {
            prompt_name()
        }
    }

    /// List only. Never prompts. Empty / whitespace parts drop out like [`Name`].
    pub fn into_filter(self) -> NamePartFilter {
        NamePartFilter {
            first: normalize_flag(self.first),
            last: normalize_flag(self.last),
            suffix: normalize_flag(self.suffix),
        }
    }
}

impl NamePartFilter {
    pub fn is_empty(&self) -> bool {
        self.first.is_none() && self.last.is_none() && self.suffix.is_none()
    }

    fn matches_name(&self, name: &Name) -> bool {
        part_eq(self.first.as_deref(), name.first())
            && part_eq(self.last.as_deref(), name.last())
            && part_eq(self.suffix.as_deref(), name.suffix())
    }

    /// AND of set parts, case-insensitive; primary or any alias.
    pub fn matches_person(&self, person: &Person) -> bool {
        if self.is_empty() {
            return true;
        }
        person.indexed_names().any(|n| self.matches_name(n))
    }
}

pub fn prompt_name() -> Result<Name> {
    let first: String = Input::new()
        .with_prompt("First")
        .allow_empty(true)
        .interact_text()?;
    let last: String = Input::new()
        .with_prompt("Last")
        .allow_empty(true)
        .interact_text()?;
    let suffix: String = Input::new()
        .with_prompt("Suffix")
        .allow_empty(true)
        .interact_text()?;

    Name::try_new(
        empty_to_none(&first),
        empty_to_none(&last),
        empty_to_none(&suffix),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn normalize_flag(part: Option<String>) -> Option<String> {
    part.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_owned())
        }
    })
}

fn empty_to_none(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn part_eq(want: Option<&str>, got: Option<&str>) -> bool {
    match want {
        None => true,
        Some(w) => got.is_some_and(|g| g.eq_ignore_ascii_case(w)),
    }
}
