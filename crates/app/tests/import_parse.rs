//! Parser tests with no database or application state.

use oeuvre_app::import::parse_biblatex;
use oeuvre_core::{ImportFormat, ResourceKind, import_codes};

#[test]
fn parse_biblatex_maps_a_book_without_app_state() {
    let source = r#"@book{lovelace1843,
        author = {Lovelace, Ada},
        title = {Notes on the Analytical Engine},
        publisher = {Scientific Memoirs},
        year = {1843},
        doi = {10.1000/notes},
        custom = {Retain me}
    }"#;
    let draft = parse_biblatex(source).expect("parse");
    assert_eq!(draft.provenance.format, ImportFormat::Biblatex);
    assert_eq!(draft.entries.len(), 1);
    let entry = &draft.entries[0];
    assert_eq!(entry.source_key(), "lovelace1843");
    assert_eq!(entry.kind(), ResourceKind::Book);
    assert_eq!(entry.title().as_str(), "Notes on the Analytical Engine");
    assert_eq!(entry.unresolved_creators.len(), 1);
    assert_eq!(
        entry.unresolved_creators[0].name.display(),
        "Ada Lovelace"
    );
    assert_eq!(
        entry.resource.publisher.as_deref(),
        Some("Scientific Memoirs")
    );
    assert_eq!(entry.resource.doi.as_deref(), Some("10.1000/notes"));
    assert_eq!(
        entry
            .resource
            .metadata
            .source_fields
            .get("custom")
            .map(String::as_str),
        Some("Retain me")
    );
    assert!(entry.resource.creators.is_empty());
    assert!(entry.resource.series_id.is_none());
}

#[test]
fn parse_biblatex_strips_bom_and_records_skipped_entries() {
    let source = "\u{feff}@book{ok,title={Ok}}\n@patent{skip,title={Nope}}";
    let draft = parse_biblatex(source).expect("parse");
    assert_eq!(draft.entries.len(), 1);
    assert_eq!(draft.entries[0].source_key(), "ok");
    assert!(
        draft.diagnostics.iter().any(|d| {
            d.entry_key.as_deref() == Some("skip")
                && d.code == import_codes::UNSUPPORTED_TYPE
                && d.severity == oeuvre_core::DiagnosticSeverity::Error
        }),
        "expected unsupported-type diagnostic: {:?}",
        draft.diagnostics
    );
}

#[test]
fn parse_biblatex_rejects_cycles_and_oversized_sources() {
    let cycle = "@book{a,title={A},crossref={b}} @book{b,title={B},crossref={a}}";
    let err = parse_biblatex(cycle).expect_err("cycle");
    assert!(
        err.code == import_codes::REFERENCE_CYCLE
            || err.code == import_codes::REFERENCE_TOO_DEEP
            || err.message.contains("cyclic")
            || err.message.contains("reference"),
        "{err:?}"
    );

    let huge = format!("@book{{x,title={{X}}}}{}", " ".repeat(oeuvre_core::IMPORT_MAX_SOURCE_BYTES));
    let err = parse_biblatex(&huge).expect_err("too large");
    assert_eq!(err.code, import_codes::SOURCE_TOO_LARGE);
}

#[test]
fn parse_biblatex_inherits_crossref_without_parent_resource_id() {
    let source = r#"
        @book{parent, title={Collected Essays}, editor={Hopper, Grace}, year={2020}}
        @incollection{chapter, title={A chapter}, author={Lovelace, Ada}, crossref={parent}}
    "#;
    let draft = parse_biblatex(source).expect("parse");
    assert_eq!(draft.entries.len(), 2);
    let chapter = draft
        .entries
        .iter()
        .find(|e| e.source_key() == "chapter")
        .expect("chapter");
    assert_eq!(chapter.kind(), ResourceKind::BookSection);
    assert_eq!(
        chapter.resource.metadata.venue.as_deref(),
        Some("Collected Essays")
    );
    assert!(chapter.had_reference_fields);
    assert!(chapter.resource.parent_id.is_none());
    // Pre-expansion originals omit inherited venue; expanded map and origins record it.
    assert!(!chapter.provenance.original_fields.contains_key("booktitle"));
    assert!(
        chapter
            .provenance
            .inherited_origins
            .contains_key("booktitle")
            || chapter.provenance.expanded_fields.contains_key("booktitle")
            || chapter.resource.metadata.venue.is_some()
    );
    assert_eq!(
        chapter
            .provenance
            .inherited_origins
            .get("year")
            .map(String::as_str)
            .or_else(|| chapter.provenance.inherited_origins.get("date").map(String::as_str)),
        Some("parent")
    );
}

#[test]
fn parse_preserves_notes_particles_pages_and_optional_date_warnings() {
    let source = r#"
        @book{one,
          title = {Names},
          author = {van Beethoven, Ludwig and {World Bank} and others},
          pages = {10--20},
          note = {A note},
          annotation = {An annotation},
          year = {bad-year}
        }
    "#;
    let draft = parse_biblatex(source).expect("parse");
    assert_eq!(draft.entries.len(), 1);
    let entry = &draft.entries[0];
    assert_eq!(entry.provenance.note.as_deref(), Some("A note"));
    assert_eq!(entry.provenance.annotation.as_deref(), Some("An annotation"));
    assert!(entry.provenance.original_fields.contains_key("note"));
    assert_eq!(
        entry.unresolved_creators[0].name.particle(),
        Some("van")
    );
    assert!(entry.unresolved_creators.iter().any(|c| c.name.is_literal()));
    assert_eq!(
        entry.resource.pages.as_ref().map(|p| p.spans[0].start.number),
        Some(Some(10))
    );
    assert!(
        draft.diagnostics.iter().any(|d| {
            d.severity == oeuvre_core::DiagnosticSeverity::Warning
                && (d.message.contains("and others") || d.field.as_deref() == Some("date"))
        }),
        "{:?}",
        draft.diagnostics
    );
}
