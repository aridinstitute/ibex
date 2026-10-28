//! Retained sibling pages. Visible positions are snapshots, never resource identity.
use oeuvre_core::{ResourceGridPage, ResourceGridRow, ResourceId, ResourceSummary};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct ChildPage {
    pub rows: Vec<ResourceGridRow>,
    pub next: Option<u64>,
    pub loaded: bool,
    pub request: Option<u64>,
    pub error: Option<String>,
}

pub(super) struct ResourceGrid {
    roots: Vec<ResourceGridRow>,
    pub children: HashMap<ResourceId, ChildPage>,
    pub expanded: HashSet<ResourceId>,
    serial: u64,
    pub positions: HashMap<ResourceId, (usize, Option<ResourceId>, bool)>,
}

impl ResourceGrid {
    pub fn append_roots(&mut self, page: ResourceGridPage) {
        self.roots.extend(page.rows);
    }

    pub fn parent(&self, id: ResourceId) -> Option<ResourceId> {
        self.positions.get(&id).and_then(|(_, parent, _)| *parent)
    }

    pub fn collapse(&mut self, id: ResourceId) {
        let mut branch = vec![id];
        while let Some(id) = branch.pop() {
            self.expanded.remove(&id);
            if let Some(page) = self.children.get_mut(&id) {
                // A reply for a closed branch may not reopen or populate it.
                page.request = None;
                branch.extend(page.rows.iter().map(|row| row.summary.id));
            }
        }
    }

    pub fn begin(&mut self, id: ResourceId) -> Option<(u64, u64)> {
        if !self.expanded.contains(&id) {
            return None;
        }
        let page = self.children.entry(id).or_default();
        if page.request.is_some() {
            return None;
        }
        let offset = if page.loaded { page.next? } else { 0 };
        self.serial += 1;
        page.request = Some(self.serial);
        page.error = None;
        Some((self.serial, offset))
    }

    pub fn finish(
        &mut self,
        id: ResourceId,
        token: u64,
        reply: Result<ResourceGridPage, String>,
    ) -> bool {
        if !self.expanded.contains(&id) {
            return false;
        }
        let Some(page) = self.children.get_mut(&id) else {
            return false;
        };
        if page.request != Some(token) {
            return false;
        }
        page.request = None;
        match reply {
            Ok(reply) => {
                page.rows.extend(reply.rows);
                page.next = reply.next_offset;
                page.loaded = true;
            }
            Err(error) => page.error = Some(error),
        }
        true
    }
    pub fn new(page: ResourceGridPage) -> Self {
        Self {
            roots: page.rows,
            children: HashMap::new(),
            expanded: HashSet::new(),
            serial: 0,
            positions: HashMap::new(),
        }
    }

    pub fn visible(&mut self) -> Vec<ResourceSummary> {
        fn walk(
            grid: &ResourceGrid,
            siblings: &[ResourceGridRow],
            depth: usize,
            parent: Option<ResourceId>,
            rows: &mut Vec<ResourceSummary>,
            positions: &mut HashMap<ResourceId, (usize, Option<ResourceId>, bool)>,
        ) {
            for row in siblings {
                let id = row.summary.id;
                if positions.contains_key(&id) {
                    continue;
                }
                positions.insert(id, (depth, parent, row.has_children));
                rows.push(row.summary.clone());
                if grid.expanded.contains(&id) {
                    if let Some(children) = grid.children.get(&id) {
                        walk(grid, &children.rows, depth + 1, Some(id), rows, positions);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        let mut positions = HashMap::new();
        walk(self, &self.roots, 0, None, &mut rows, &mut positions);
        self.positions = positions;
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oeuvre_core::{NonEmptyStr, ResourceKind, UtcMillis};

    fn row(title: &str, has_children: bool) -> ResourceGridRow {
        ResourceGridRow {
            summary: ResourceSummary {
                id: ResourceId::generate(),
                kind: ResourceKind::Book,
                title: NonEmptyStr::try_new(title).unwrap(),
                creators_label: String::new(),
                published_label: None,
                publisher: None,
                container_title: None,
                date_added: UtcMillis::from_unix_ms(0),
                date_edited: UtcMillis::from_unix_ms(0),
            },
            has_children,
        }
    }

    #[test]
    fn expansion_flattens_siblings_without_promoting_descendants_to_roots() {
        let book = row("Book", true);
        let chapter = row("Chapter", false);
        let other = row("Other book", false);
        let mut grid = ResourceGrid::new(ResourceGridPage {
            rows: vec![book.clone(), other],
            next_offset: None,
        });
        grid.children.insert(
            book.summary.id,
            ChildPage {
                rows: vec![chapter.clone()],
                loaded: true,
                ..Default::default()
            },
        );
        assert_eq!(grid.visible().len(), 2);
        grid.expanded.insert(book.summary.id);
        let titles: Vec<_> = grid
            .visible()
            .iter()
            .map(|row| row.title.as_str().to_owned())
            .collect();
        assert_eq!(titles, ["Book", "Chapter", "Other book"]);
        assert_eq!(
            grid.positions[&chapter.summary.id],
            (1, Some(book.summary.id), false)
        );
    }

    #[test]
    fn collapse_and_reexpand_rejects_old_reply_and_retries_failures_at_same_offset() {
        let book = row("Book", true);
        let id = book.summary.id;
        let mut grid = ResourceGrid::new(ResourceGridPage {
            rows: vec![book],
            next_offset: None,
        });
        grid.visible();
        grid.expanded.insert(id);
        let (old, offset) = grid.begin(id).unwrap();
        assert_eq!(offset, 0);
        assert!(grid.begin(id).is_none());
        grid.collapse(id);
        grid.expanded.insert(id);
        let (current, _) = grid.begin(id).unwrap();
        assert!(!grid.finish(
            id,
            old,
            Ok(ResourceGridPage {
                rows: vec![row("Stale", false)],
                next_offset: None
            })
        ));
        assert!(grid.finish(id, current, Err("Offline".into())));
        assert_eq!(grid.children[&id].error.as_deref(), Some("Offline"));
        let (retry, offset) = grid.begin(id).unwrap();
        assert_eq!(offset, 0);
        grid.finish(
            id,
            retry,
            Ok(ResourceGridPage {
                rows: vec![row("First chapter", false)],
                next_offset: Some(1),
            }),
        );
        let (more, offset) = grid.begin(id).unwrap();
        assert_eq!(offset, 1);
        grid.finish(id, more, Err("Offline again".into()));
        assert_eq!(grid.visible().len(), 2);
        let (_, offset) = grid.begin(id).unwrap();
        assert_eq!(offset, 1);
    }

    #[test]
    fn journal_issue_article_depth_and_root_pagination_are_independent() {
        let journal = row("Journal", true);
        let issue = row("Issue", true);
        let article = row("Article", false);
        let mut grid = ResourceGrid::new(ResourceGridPage {
            rows: vec![journal.clone()],
            next_offset: Some(1),
        });
        grid.visible();
        grid.expanded.insert(journal.summary.id);
        let (request, _) = grid.begin(journal.summary.id).unwrap();
        grid.finish(
            journal.summary.id,
            request,
            Ok(ResourceGridPage {
                rows: vec![issue.clone()],
                next_offset: None,
            }),
        );
        grid.visible();
        grid.expanded.insert(issue.summary.id);
        let (request, _) = grid.begin(issue.summary.id).unwrap();
        grid.finish(
            issue.summary.id,
            request,
            Ok(ResourceGridPage {
                rows: vec![article.clone()],
                next_offset: None,
            }),
        );
        grid.append_roots(ResourceGridPage {
            rows: vec![row("Other root", false)],
            next_offset: None,
        });
        let titles: Vec<_> = grid
            .visible()
            .iter()
            .map(|row| row.title.as_str().to_owned())
            .collect();
        assert_eq!(titles, ["Journal", "Issue", "Article", "Other root"]);
        assert_eq!(
            grid.positions[&article.summary.id],
            (2, Some(issue.summary.id), false)
        );
        grid.collapse(journal.summary.id);
        assert_eq!(grid.visible().len(), 2);
        assert!(grid.expanded.is_empty());
    }
}
