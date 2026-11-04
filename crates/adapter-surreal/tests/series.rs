use ibex_adapter_surreal::SurrealStore;
use ibex_core::{NewSeries, NonEmptyStr, Store};

#[tokio::test]
async fn lists_empty_and_created_series_in_title_order() {
    let store = SurrealStore::open_memory().await.unwrap();
    assert!(store.list_series().await.unwrap().is_empty());
    let mut ids = Vec::new();
    for title in ["Zoology", "Classics"] {
        let series = store
            .create_series(NewSeries {
                title: NonEmptyStr::try_new(title).unwrap(),
            })
            .await
            .unwrap();
        ids.push(series.id);
    }
    let series = store.list_series().await.unwrap();
    assert_eq!(
        series.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
        ["Classics", "Zoology"]
    );
    assert_eq!(
        series.iter().map(|s| s.id).collect::<Vec<_>>(),
        [ids[1], ids[0]]
    );
}
