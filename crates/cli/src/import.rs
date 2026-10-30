use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use oeuvre_app::command::{ImportBiblatex, ImportCslJson, PreviewBiblatex, PreviewCslJson};
use oeuvre_app::{AppState, Command, CommandError, Outcome};
use oeuvre_core::{
    AssignedTarget, CollectionId, EntryChoice, EntryProposal, ExistingIdOrName,
    IMPORT_MAX_SOURCE_BYTES, ImportAssignments, ImportChoices, ImportIssue, ImportPreview,
    ImportReport, NonEmptyStr, PersonChoice, PersonMatchPreview, StoreError, TagId, import_codes,
};

/// Bibliography source formats `ovr import` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportFormatArg {
    /// BibLaTeX / BibTeX (`.bib`).
    Biblatex,
    /// CSL-JSON item array or single item (`.json`).
    CslJson,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ConflictPolicyArg {
    #[default]
    Skip,
    Duplicate,
}

/// Import a bibliography file. Every imported or reused entry also receives
/// the given collections and tags; existing memberships are kept.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Bibliography file to import.
    pub path: PathBuf,

    /// Source format; inferred from the file extension when omitted.
    #[arg(long, value_enum)]
    pub format: Option<ImportFormatArg>,

    /// Add entries to the collection with this exact name, creating a
    /// top-level collection when none exists. Repeatable.
    #[arg(long = "collection", value_name = "NAME")]
    pub collections: Vec<String>,

    /// Add entries to the existing collection with this id. Repeatable.
    #[arg(long = "collection-id", value_name = "ID")]
    pub collection_ids: Vec<String>,

    /// Tag entries with this exact text, creating the tag when none exists.
    /// Repeatable.
    #[arg(long = "tag", value_name = "TEXT")]
    pub tags: Vec<String>,

    /// Tag entries with the existing tag with this id. Repeatable.
    #[arg(long = "tag-id", value_name = "ID")]
    pub tag_ids: Vec<String>,

    /// Preview proposed imports and conflicts without writing to the library.
    #[arg(long)]
    pub dry_run: bool,

    /// Skip conflicting entries or deliberately import publication duplicates.
    #[arg(long, value_enum, default_value = "skip")]
    pub on_conflict: ConflictPolicyArg,
}

pub async fn run(state: &AppState, args: ImportArgs) -> Result<()> {
    if args.dry_run {
        return preview(state, &args).await;
    }
    let report = execute(state, &args).await?;
    print!("{}", format_report(&report));
    if !report.failed.is_empty() {
        return Err(CommandError::Store(StoreError::new(
            "import stopped after a storage failure; entries reported as imported or reused were kept",
        ))
        .into());
    }
    Ok(())
}

async fn preview(state: &AppState, args: &ImportArgs) -> Result<()> {
    let _ = assignments(args)?;
    let format = args
        .format
        .map(Ok)
        .unwrap_or_else(|| infer_format(&args.path))?;
    let source_text = read_source(&args.path)?;
    let preview = preview_source(state, format, &source_text).await?;
    println!(
        "preview: {} entries, {} need review, {} diagnostics (no writes)",
        preview.entries.len(),
        preview.entries.iter().filter(|e| e.needs_review()).count(),
        preview.diagnostics.len()
    );
    for entry in &preview.entries {
        let action = match entry.proposal {
            EntryProposal::Create => "new",
            EntryProposal::AlreadyCommitted { .. } => "already committed",
            EntryProposal::PublicationDuplicate { .. } => "duplicate",
            EntryProposal::Skip { .. } => "skip",
        };
        println!("{action}\t{}\t{}", entry.entry_key, entry.title);
        for conflict in &entry.conflicts {
            println!("  conflict: {}: {}", conflict.code, conflict.message);
        }
    }
    for diagnostic in &preview.diagnostics {
        println!("diagnostic: {}", diagnostic.message);
    }
    Ok(())
}

async fn preview_source(
    state: &AppState,
    format: ImportFormatArg,
    source_text: &str,
) -> Result<ImportPreview> {
    let outcome = match format {
        ImportFormatArg::Biblatex => {
            state
                .execute(Command::PreviewBiblatex(PreviewBiblatex {
                    source_text: source_text.to_owned(),
                }))
                .await?
        }
        ImportFormatArg::CslJson => {
            state
                .execute(Command::PreviewCslJson(PreviewCslJson {
                    source_text: source_text.to_owned(),
                }))
                .await?
        }
    };
    match outcome {
        Outcome::PreviewBiblatex(preview) | Outcome::PreviewCslJson(preview) => Ok(preview),
        _ => bail!("unexpected outcome from import preview"),
    }
}

fn conflict_choices(preview: &ImportPreview, policy: ConflictPolicyArg) -> ImportChoices {
    let mut choices = ImportChoices::default();
    for entry in &preview.entries {
        if !entry.needs_review() {
            continue;
        }
        let decision = match (&entry.proposal, policy) {
            (EntryProposal::PublicationDuplicate { .. }, ConflictPolicyArg::Duplicate) => {
                EntryChoice::IntentionalDuplicate
            }
            _ => EntryChoice::Skip,
        };
        if policy == ConflictPolicyArg::Duplicate
            && !matches!(decision, EntryChoice::IntentionalDuplicate)
            && entry
                .conflicts
                .iter()
                .all(|c| c.code == import_codes::AMBIGUOUS_CREATOR)
        {
            // A creator ambiguity alone can be resolved by creating a new person.
        } else {
            choices.entries.insert(entry.entry_key.clone(), decision);
        }
        for person in &entry.people {
            if matches!(person.match_, PersonMatchPreview::Ambiguous { .. }) {
                choices.people.insert(
                    ImportChoices::person_key(&entry.entry_key, person.index),
                    PersonChoice::CreateNew,
                );
            }
        }
    }
    choices
}

pub async fn execute(state: &AppState, args: &ImportArgs) -> Result<ImportReport> {
    let format = match args.format {
        Some(format) => format,
        None => infer_format(&args.path)?,
    };
    let assignments = assignments(args)?;
    let source_text = read_source(&args.path)?;
    let preview = preview_source(state, format, &source_text).await?;
    let mut choices = conflict_choices(&preview, args.on_conflict);
    choices.assignments = assignments;
    let outcome = match format {
        ImportFormatArg::Biblatex => {
            state
                .execute(Command::ImportBiblatex(ImportBiblatex {
                    source_text,
                    choices,
                }))
                .await?
        }
        ImportFormatArg::CslJson => {
            state
                .execute(Command::ImportCslJson(ImportCslJson {
                    source_text,
                    choices,
                }))
                .await?
        }
    };
    match outcome {
        Outcome::ImportBiblatex(report) | Outcome::ImportCslJson(report) => Ok(report),
        _ => bail!("unexpected outcome from import"),
    }
}

fn infer_format(path: &Path) -> Result<ImportFormatArg> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("bib" | "bibtex" | "biblatex") => Ok(ImportFormatArg::Biblatex),
        Some("json" | "csljson") => Ok(ImportFormatArg::CslJson),
        _ => Err(CommandError::Invalid(format!(
            "cannot infer the format of {}; pass --format biblatex or --format csl-json",
            path.display()
        ))
        .into()),
    }
}

fn read_source(path: &Path) -> Result<String> {
    use std::io::Read as _;
    // Bounded read instead of a metadata length check: pipes and device files
    // report length 0, and a file can grow between the check and the read.
    let file = std::fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(IMPORT_MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    if bytes.len() > IMPORT_MAX_SOURCE_BYTES {
        return Err(CommandError::Invalid(format!(
            "{} is larger than the {} MiB import limit",
            path.display(),
            IMPORT_MAX_SOURCE_BYTES / (1024 * 1024)
        ))
        .into());
    }
    String::from_utf8(bytes).with_context(|| format!("read {}: not valid UTF-8", path.display()))
}

fn invalid(message: String) -> anyhow::Error {
    CommandError::Invalid(message).into()
}

fn assignments(args: &ImportArgs) -> Result<ImportAssignments> {
    let mut out = ImportAssignments::default();
    for name in &args.collections {
        let name = NonEmptyStr::try_new(name)
            .map_err(|_| invalid("--collection must not be empty".into()))?;
        out.collections.push(ExistingIdOrName::Name { name });
    }
    for id in &args.collection_ids {
        let id = CollectionId::parse(id.trim())
            .map_err(|_| invalid(format!("--collection-id {id:?} is not a collection id")))?;
        out.collections.push(ExistingIdOrName::Existing { id });
    }
    for text in &args.tags {
        let name =
            NonEmptyStr::try_new(text).map_err(|_| invalid("--tag must not be empty".into()))?;
        out.tags.push(ExistingIdOrName::Name { name });
    }
    for id in &args.tag_ids {
        let id = TagId::parse(id.trim())
            .map_err(|_| invalid(format!("--tag-id {id:?} is not a tag id")))?;
        out.tags.push(ExistingIdOrName::Existing { id });
    }
    Ok(out)
}

fn targets<Id: std::fmt::Display>(targets: &[AssignedTarget<Id>]) -> String {
    targets
        .iter()
        .map(|t| {
            if t.created {
                format!("{} ({}, new)", t.name, t.id)
            } else {
                format!("{} ({})", t.name, t.id)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn issue_line(label: &str, issue: &ImportIssue) -> String {
    let key = issue.entry_key.as_deref().unwrap_or("-");
    match issue.code.as_deref() {
        Some(code) => format!("{label}\t{key}\t{} [{code}]\n", issue.message),
        None => format!("{label}\t{key}\t{}\n", issue.message),
    }
}

/// Human report: a summary line, applied targets, then one tab-separated
/// row per entry outcome.
pub fn format_report(report: &ImportReport) -> String {
    let mut out = format!(
        "imported {}, reused {}, skipped {}, conflicted {}, failed {}, not attempted {}, warnings {}\n",
        report.imported.len(),
        report.reused.len(),
        report.skipped.len(),
        report.conflicted.len(),
        report.failed.len(),
        report.not_attempted.len(),
        report.warnings.len(),
    );
    if !report.assignments.collections.is_empty() {
        out.push_str(&format!(
            "collections: {}\n",
            targets(&report.assignments.collections)
        ));
    }
    if !report.assignments.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", targets(&report.assignments.tags)));
    }
    for entry in &report.imported {
        out.push_str(&format!(
            "imported\t{}\t{}\n",
            entry.entry_key, entry.resource_id
        ));
    }
    for entry in &report.reused {
        out.push_str(&format!(
            "reused\t{}\t{}\n",
            entry.entry_key, entry.resource_id
        ));
    }
    for (label, issues) in [
        ("conflicted", &report.conflicted),
        ("skipped", &report.skipped),
        ("failed", &report.failed),
        ("not-attempted", &report.not_attempted),
        ("warning", &report.warnings),
    ] {
        for issue in issues {
            out.push_str(&issue_line(label, issue));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_adapter_surreal::SurrealStore;
    use oeuvre_app::Surface;
    use oeuvre_app::command::{CreateCollection, GetResource};

    fn args(path: &str) -> ImportArgs {
        ImportArgs {
            path: PathBuf::from(path),
            format: None,
            collections: vec![],
            collection_ids: vec![],
            tags: vec![],
            tag_ids: vec![],
            dry_run: false,
            on_conflict: ConflictPolicyArg::Skip,
        }
    }

    fn temp_file(name: &str, body: &str) -> PathBuf {
        let unique = CollectionId::generate();
        let dir = std::env::temp_dir().join(format!("ovr-import-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    async fn state() -> AppState {
        AppState::builder(SurrealStore::open_memory().await.unwrap())
            .surface(Surface::Test)
            .build()
    }

    #[test]
    fn infers_format_from_extension() {
        assert_eq!(
            infer_format(Path::new("refs.BIB")).unwrap(),
            ImportFormatArg::Biblatex
        );
        assert_eq!(
            infer_format(Path::new("refs.json")).unwrap(),
            ImportFormatArg::CslJson
        );
        let err = infer_format(Path::new("refs.txt")).unwrap_err();
        let cmd = err.downcast_ref::<CommandError>().expect("command error");
        assert!(cmd.to_string().contains("--format"), "{cmd}");
    }

    #[test]
    fn rejects_bad_ids_and_empty_names_before_touching_the_library() {
        let mut a = args("refs.bib");
        a.collection_ids = vec!["nope".into()];
        assert!(
            assignments(&a)
                .unwrap_err()
                .to_string()
                .contains("--collection-id")
        );
        let mut a = args("refs.bib");
        a.tags = vec!["   ".into()];
        assert!(assignments(&a).unwrap_err().to_string().contains("--tag"));
    }

    #[tokio::test]
    async fn imports_bib_with_names_and_ids_and_retries_additively() {
        let state = state().await;
        let Outcome::CreateCollection(methods) = state
            .execute(Command::CreateCollection(CreateCollection {
                name: NonEmptyStr::try_new("Methods").unwrap(),
                parent_id: None,
            }))
            .await
            .unwrap()
        else {
            panic!("collection");
        };
        let path = temp_file(
            "references.bib",
            "@book{a, title={First}, author={Ada, Lovelace}}\n@book{b, title={Second}, author={Hopper, Grace}}\n",
        );
        let mut a = args(path.to_str().unwrap());
        a.collections = vec!["Research".into()];
        a.collection_ids = vec![methods.id.to_string()];
        a.tags = vec!["review".into(), "2026".into()];

        let report = execute(&state, &a).await.unwrap();
        assert_eq!(report.imported.len(), 2, "{report:?}");
        let text = format_report(&report);
        assert!(text.starts_with("imported 2, reused 0"), "{text}");
        assert!(text.contains("Research ("), "{text}");
        assert!(text.contains(", new)"), "{text}");
        let Outcome::GetResource(r) = state
            .execute(Command::GetResource(GetResource {
                id: report.imported[0].resource_id,
            }))
            .await
            .unwrap()
        else {
            panic!("get");
        };
        assert_eq!(r.tag_ids.len(), 2);
        assert_eq!(r.collection_ids.len(), 2);

        let retry = execute(&state, &a).await.unwrap();
        assert_eq!(retry.reused.len(), 2, "{retry:?}");
        assert!(retry.assignments.tags.iter().all(|t| !t.created));
        assert!(format_report(&retry).contains("reused\ta\t"));
    }

    #[tokio::test]
    async fn imports_csl_json_with_explicit_format_and_no_flags() {
        let state = state().await;
        let path = temp_file(
            "refs.data",
            r#"[{"id":"x","type":"book","title":"CSL Book","author":[{"family":"Lovelace","given":"Ada"}]}]"#,
        );
        let mut a = args(path.to_str().unwrap());
        a.format = Some(ImportFormatArg::CslJson);
        let report = execute(&state, &a).await.unwrap();
        assert_eq!(report.imported.len(), 1, "{report:?}");
        assert!(report.assignments.tags.is_empty());
        assert!(!format_report(&report).contains("tags:"));
    }

    #[tokio::test]
    async fn ambiguous_collection_name_fails_with_id_guidance() {
        let state = state().await;
        for _ in 0..2 {
            state
                .execute(Command::CreateCollection(CreateCollection {
                    name: NonEmptyStr::try_new("Dup").unwrap(),
                    parent_id: None,
                }))
                .await
                .unwrap();
        }
        let path = temp_file("r.bib", "@book{a, title={First}, author={Ada, Lovelace}}");
        let mut a = args(path.to_str().unwrap());
        a.collections = vec!["Dup".into()];
        let err = execute(&state, &a).await.unwrap_err();
        let cmd = err.downcast_ref::<CommandError>().expect("command error");
        assert!(matches!(cmd, CommandError::Invalid(_)));
        assert!(cmd.to_string().contains("select one by id"), "{cmd}");
    }

    #[test]
    fn report_lists_conflicts_and_partial_failures() {
        let report = ImportReport {
            conflicted: vec![ImportIssue::new(Some("dup".into()), "DOI exists").with_code("c")],
            failed: vec![ImportIssue::new(Some("b".into()), "disk full")],
            not_attempted: vec![ImportIssue::new(Some("c".into()), "later")],
            ..Default::default()
        };
        let text = format_report(&report);
        assert!(text.contains("conflicted\tdup\tDOI exists [c]"), "{text}");
        assert!(text.contains("failed\tb\tdisk full"), "{text}");
        assert!(text.contains("not-attempted\tc\tlater"), "{text}");
    }
}
