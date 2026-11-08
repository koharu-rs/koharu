//! Reads for tools. Answers serialize the project's own values, so a tool
//! sees a layer as the file stores it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{Background, History, Layer, LayerId, Page, PageId, ProjectId, Result, Session, Size};

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// List the pages in order, with their sizes and backgrounds.
    GetProject,
    /// Get one page with its layers, each holding its children, masks among
    /// them.
    GetPage { page: PageId },
    /// Get one layer with everything inside it.
    GetLayer { layer: LayerId },
    /// Get the undo and redo labels.
    GetHistory,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(untagged)]
pub enum Answer {
    Project(ProjectAnswer),
    Page(Page),
    Layer(Box<Layer>),
    History(History),
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ProjectAnswer {
    pub id: ProjectId,
    /// The format version the file holds.
    pub version: String,
    pub pages: Vec<PageSummary>,
}

/// A page without its layers, which `get_page` answers.
#[derive(Clone, Debug, Serialize, Type)]
pub struct PageSummary {
    pub id: PageId,
    pub size: Size,
    pub background: Option<Background>,
}

impl Session {
    pub fn query(&self, query: Query) -> Result<Answer> {
        let project = self.project();
        Ok(match query {
            Query::GetProject => Answer::Project(ProjectAnswer {
                id: project.id(),
                version: self.version()?,
                pages: project
                    .pages()
                    .map(|page| PageSummary {
                        id: page.id,
                        size: page.size,
                        background: page.background,
                    })
                    .collect(),
            }),
            Query::GetPage { page } => Answer::Page(project.page(page)?.clone()),
            Query::GetLayer { layer } => Answer::Layer(Box::new(project.layer(layer)?.clone())),
            Query::GetHistory => Answer::History(self.history()),
        })
    }
}
