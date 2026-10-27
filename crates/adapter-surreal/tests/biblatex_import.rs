use oeuvre_adapter_surreal::SurrealStore;
use oeuvre_app::command::{
    GetResourceDetail, ImportBiblatex, ListResources, ListResourcesQueryPage,
};
use oeuvre_app::{AppState, Command, Outcome, Surface};
use oeuvre_core::{ResourceQuery, ResourceSort, SortDirection};

#[tokio::test]
async fn imported_metadata_is_visible_in_library_rows_and_details() {
    let state = AppState::builder(SurrealStore::open_memory().await.unwrap())
        .surface(Surface::Test)
        .build();
    let Outcome::ImportBiblatex(report) = state.execute(Command::ImportBiblatex(ImportBiblatex {
        source_text: "@article{paper,title={An imported paper},author={Hopper, Grace},journaltitle={Example Journal},doi={10.1000/paper},keywords={history, computing},date={2024-03-15}}".into(),
    })).await.unwrap() else { panic!("import"); };
    assert_eq!(report.imported.len(), 1);
    let Outcome::GetResourceDetail(detail) = state
        .execute(Command::GetResourceDetail(GetResourceDetail {
            id: report.imported[0].resource_id,
        }))
        .await
        .unwrap()
    else {
        panic!("detail");
    };
    assert_eq!(detail.doi.as_deref(), Some("10.1000/paper"));
    assert_eq!(detail.keywords, ["history", "computing"]);
    assert_eq!(
        detail.container_title.as_ref().map(|s| s.as_str()),
        Some("Example Journal")
    );
    let Outcome::ListResources(rows) = state
        .execute(Command::ListResources(ListResources))
        .await
        .unwrap()
    else {
        panic!("rows");
    };
    assert_eq!(
        rows[0].container_title.as_ref().map(|s| s.as_str()),
        Some("Example Journal")
    );
    for sort in [
        ResourceSort::Title,
        ResourceSort::Published,
        ResourceSort::Publisher,
        ResourceSort::PrimaryCreator,
        ResourceSort::DateAdded,
        ResourceSort::DateEdited,
    ] {
        let Outcome::ListResourcesQueryPage(page) = state
            .execute(Command::ListResourcesQueryPage(ListResourcesQueryPage {
                query: ResourceQuery::sorted(sort, SortDirection::Ascending),
                offset: 0,
                limit: 10,
            }))
            .await
            .unwrap()
        else {
            panic!("page");
        };
        assert_eq!(
            page.rows[0].container_title.as_ref().map(|s| s.as_str()),
            Some("Example Journal")
        );
    }
}
