//! Classification rules for every collection in one window: keyword rules
//! and the auto-classify opt-in. Each row saves on its own; keywords commit
//! on Enter or when the field loses focus.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use oeuvre_core::{Collection, CollectionId, parse_keywords};

use crate::db::DbWorker;

/// A row was saved; the library refreshes its collections from the store.
pub struct ClassificationRulesSaved;

struct RuleRow {
    /// The last saved state; drafts live in `keywords`.
    collection: Collection,
    keywords: Entity<InputState>,
    saving: bool,
    error: Option<String>,
    _subscription: Subscription,
}

pub struct ClassificationRules {
    worker: DbWorker,
    rows: Vec<RuleRow>,
}

impl EventEmitter<ClassificationRulesSaved> for ClassificationRules {}

impl ClassificationRules {
    pub fn new(
        worker: DbWorker,
        collections: Vec<Collection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            worker,
            rows: Vec::new(),
        };
        this.set_collections(collections, window, cx);
        this
    }

    /// Reconcile with the library's collections, keeping drafts and errors
    /// of rows that still exist. A row's field is only rewritten when its
    /// saved rules changed elsewhere and it holds no unsaved draft.
    pub fn set_collections(
        &mut self,
        collections: Vec<Collection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut previous = std::mem::take(&mut self.rows);
        for collection in collections {
            if let Some(ix) = previous
                .iter()
                .position(|row| row.collection.id == collection.id)
            {
                let mut row = previous.swap_remove(ix);
                let clean =
                    row.keywords.read(cx).value().as_ref() == row.collection.keywords.join(", ");
                if clean && row.collection.keywords != collection.keywords && !row.saving {
                    let text = collection.keywords.join(", ");
                    row.keywords
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
                row.collection = collection;
                self.rows.push(row);
            } else {
                self.rows.push(self.new_row(collection, window, cx));
            }
        }
        cx.notify();
    }

    fn new_row(
        &self,
        collection: Collection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RuleRow {
        let text = collection.keywords.join(", ");
        let keywords = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder("e.g. citation networks, bibliometrics");
            state.set_value(text, window, cx);
            state
        });
        let id = collection.id;
        let subscription = cx.subscribe_in(
            &keywords,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_keywords(id, window, cx);
                }
            },
        );
        RuleRow {
            collection,
            keywords,
            saving: false,
            error: None,
            _subscription: subscription,
        }
    }

    fn commit_keywords(&mut self, id: CollectionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter_mut().find(|row| row.collection.id == id) else {
            return;
        };
        if row.saving {
            return;
        }
        match parse_keywords(row.keywords.read(cx).value().as_ref()) {
            Err(error) => {
                row.error = Some(error);
                cx.notify();
            }
            Ok(keywords) if keywords == row.collection.keywords => {
                // Only spacing or duplicates changed; show the saved form.
                row.error = None;
                let text = keywords.join(", ");
                if row.keywords.read(cx).value().as_ref() != text {
                    row.keywords
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
                cx.notify();
            }
            Ok(keywords) => {
                let include = row.collection.jev_enabled;
                self.save(id, keywords, include, true, window, cx);
            }
        }
    }

    fn set_included(
        &mut self,
        id: CollectionId,
        include: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter().find(|row| row.collection.id == id) else {
            return;
        };
        if row.saving || row.collection.jev_enabled == include {
            return;
        }
        // Saved keywords, not the draft: the checkbox never commits typing.
        let keywords = row.collection.keywords.clone();
        self.save(id, keywords, include, false, window, cx);
    }

    fn save(
        &mut self,
        id: CollectionId,
        keywords: Vec<String>,
        include: bool,
        rewrite_field: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.rows.iter_mut().find(|row| row.collection.id == id) else {
            return;
        };
        row.saving = true;
        // A checkbox save leaves the draft untouched, so a validation error
        // for that draft must survive; only a stale save failure is obsolete.
        row.error = if rewrite_field {
            None
        } else {
            draft_error(row, cx)
        };
        let worker = self.worker.clone();
        cx.spawn_in(window, async move |this, window| {
            let result = worker
                .set_collection_classification(id, keywords, include)
                .await;
            _ = this.update_in(window, |this, window, cx| {
                let Some(row) = this.rows.iter_mut().find(|row| row.collection.id == id) else {
                    return;
                };
                row.saving = false;
                match result {
                    Ok(collection) => {
                        if rewrite_field {
                            let text = collection.keywords.join(", ");
                            row.keywords
                                .update(cx, |state, cx| state.set_value(text, window, cx));
                        }
                        row.collection = collection;
                        row.error = if rewrite_field {
                            None
                        } else {
                            draft_error(row, cx)
                        };
                        cx.emit(ClassificationRulesSaved);
                    }
                    Err(error) => row.error = Some(format!("Could not save: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    #[cfg(test)]
    pub fn row_state(&self, ix: usize) -> Option<(&Collection, &Entity<InputState>, Option<&str>)> {
        self.rows
            .get(ix)
            .map(|row| (&row.collection, &row.keywords, row.error.as_deref()))
    }

    #[cfg(test)]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

fn draft_error(row: &RuleRow, cx: &App) -> Option<String> {
    parse_keywords(row.keywords.read(cx).value().as_ref()).err()
}

const NAME_WIDTH: f32 = 200.;
const INCLUDE_WIDTH: f32 = 110.;

impl Render for ClassificationRules {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let border = cx.theme().border;
        v_flex()
            .id("classification-rules")
            .test_support()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    // Clears the traffic lights in the transparent title bar.
                    .pt_10()
                    .px_4()
                    .pb_3()
                    .gap_1()
                    .child(div().text_xl().child("Classification rules"))
                    .child(div().text_sm().text_color(muted).child(
                        "A collection is suggested when a resource's title, abstract, or \
                         keywords contain one of its keywords. Auto-classify (⌘J) considers \
                         only the collections included here. Nothing is added without your \
                         confirmation.",
                    )),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_1()
                    .gap_3()
                    .border_b_1()
                    .border_color(border)
                    .text_sm()
                    .text_color(muted)
                    .child(div().w(px(NAME_WIDTH)).child("Collection"))
                    .child(div().flex_1().child("Keywords (comma-separated)"))
                    .child(div().w(px(INCLUDE_WIDTH)).child("Auto-classify")),
            )
            .child(
                v_flex()
                    .id("classification-rules-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.rows.is_empty(), |list| {
                        list.child(
                            div()
                                .px_4()
                                .py_3()
                                .text_color(muted)
                                .child("No collections yet. Create one in the library sidebar."),
                        )
                    })
                    .children(self.rows.iter().enumerate().map(|(ix, row)| {
                        let id = row.collection.id;
                        let name = row.collection.name.as_str().to_owned();
                        v_flex()
                            .id(("rule-row", ix))
                            .test_support()
                            .px_4()
                            .py_2()
                            .gap_1()
                            .border_b_1()
                            .border_color(border)
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(div().w(px(NAME_WIDTH)).truncate().child(name.clone()))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(Input::new(&row.keywords).disabled(row.saving)),
                                    )
                                    .child(
                                        div().w(px(INCLUDE_WIDTH)).child(
                                            Checkbox::new(("rule-auto", ix))
                                                .checked(row.collection.jev_enabled)
                                                .disabled(row.saving)
                                                .accessibility_label(format!(
                                                    "Include {name} in auto-classify"
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, checked: &bool, window, cx| {
                                                        this.set_included(id, *checked, window, cx)
                                                    },
                                                )),
                                        ),
                                    ),
                            )
                            .when(row.saving, |el| {
                                el.child(div().text_sm().text_color(muted).child("Saving…"))
                            })
                            .when_some(row.error.clone(), |el, error| {
                                el.child(
                                    div()
                                        .id(("rule-error", ix))
                                        .test_support()
                                        .text_sm()
                                        .text_color(danger)
                                        .child(error),
                                )
                            })
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::ClassificationRules;
    use crate::db::DbWorker;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
    use oeuvre_core::{NonEmptyStr, parse_keywords};

    fn settle(cx: &mut TestAppContext, worker: &DbWorker) {
        for _ in 0..4 {
            cx.run_until_parked();
            worker.list_page_blocking(None, 1).unwrap();
        }
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn checkbox_save_keeps_invalid_keyword_error_until_valid_commit(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let worker = DbWorker::spawn(
            oeuvre_app::AppState::builder(oeuvre_app::testing::MemoryStore::new())
                .surface(oeuvre_app::Surface::Test)
                .build(),
        );
        let rt = tokio::runtime::Runtime::new().unwrap();
        let collection = rt.block_on(async {
            let collection = worker
                .create_collection(NonEmptyStr::try_new("Networks").unwrap(), None)
                .await
                .unwrap();
            worker
                .set_collection_classification(collection.id, vec!["saved".into()], false)
                .await
                .unwrap()
        });
        let (handle, rules) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| ClassificationRules::new(worker.clone(), vec![collection], window, cx))
            })
            .unwrap()
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let input = rules.read(cx).rows[0].keywords.clone();
            input.update(cx, |state, cx| state.set_value("a,,b", window, cx));
            input.focus_handle(cx).focus(window, cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(rules.read(cx).rows[0].error.is_some());
            assert!(window.try_find(("rule-error", 0usize)).is_some());
            window.click(("rule-auto", 0usize), cx);
        })
        .unwrap();
        settle(cx, &worker);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let row = &rules.read(cx).rows[0];
            assert!(row.collection.jev_enabled);
            assert_eq!(row.collection.keywords, vec!["saved"]);
            assert_eq!(row.keywords.read(cx).value().as_ref(), "a,,b");
            assert_eq!(
                row.error.as_deref(),
                parse_keywords("a,,b").err().as_deref(),
                "checkbox success must not hide an invalid unsaved keyword draft"
            );
            assert!(window.try_find(("rule-error", 0usize)).is_some());

            let input = row.keywords.clone();
            input.update(cx, |state, cx| {
                state.set_value("valid, keywords", window, cx)
            });
            input.focus_handle(cx).focus(window, cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, &worker);
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let row = &rules.read(cx).rows[0];
            assert!(row.error.is_none());
            assert_eq!(row.collection.keywords, vec!["valid", "keywords"]);
            assert!(row.collection.jev_enabled);
            assert!(window.try_find(("rule-error", 0usize)).is_none());
        })
        .unwrap();
        let stored = rt.block_on(worker.list_collection_forest()).unwrap();
        assert_eq!(stored[0].collection.keywords, vec!["valid", "keywords"]);
        assert!(stored[0].collection.jev_enabled);
    }
}
