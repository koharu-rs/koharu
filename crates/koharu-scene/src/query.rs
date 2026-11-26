//! Reads for tools. Answers serialize the project's own values, so a tool
//! sees a layer as the file stores it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    Background, History, Layer, LayerId, Meta, Page, PageId, ProjectId, Result, Session, Size,
};

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema)]
#[serde(tag = "query", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// List the pages in order, with their sizes, backgrounds, and how many
    /// of their blocks are translated, and what the project holds beyond
    /// them, such as the faces it sets.
    GetProject,
    /// Get one page with its layers, each holding its children, masks among
    /// them.
    GetPage { page: PageId },
    /// Get one layer with everything inside it.
    GetLayer { layer: LayerId },
    /// List the texts a reader sees on a page, in reading order: each block
    /// and text layer that shows.
    GetStory { page: PageId },
    /// Get the undo and redo labels.
    GetHistory,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(untagged)]
pub enum Answer {
    Project(ProjectAnswer),
    Page(Page),
    Layer(Box<Layer>),
    Story(Vec<LayerId>),
    History(History),
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ProjectAnswer {
    pub id: ProjectId,
    /// The format version the file holds.
    pub version: String,
    pub pages: Vec<PageSummary>,
    pub meta: Meta,
}

/// A page without its layers, which `get_page` answers, and how far its
/// translation is, as a CAT tool's file list shows each file's progress,
/// Trados Studio's Files view among them.
#[derive(Clone, Debug, Serialize, Type)]
pub struct PageSummary {
    pub id: PageId,
    pub size: Size,
    pub background: Option<Background>,
    /// Its blocks, hidden ones too.
    pub texts: usize,
    /// Its blocks whose translation isn't blank. A cleared translation keeps
    /// its style, and styling an untranslated block makes an empty one, so
    /// neither counts.
    pub translated: usize,
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
                    .map(|page| {
                        let blocks = project.blocks(page.id)?;
                        Ok(PageSummary {
                            id: page.id,
                            size: page.size,
                            background: page.background,
                            texts: blocks.len(),
                            translated: blocks
                                .iter()
                                .filter_map(|(_, block)| block.translation.as_ref())
                                .filter(|text| !text.content.text().trim().is_empty())
                                .count(),
                        })
                    })
                    .collect::<Result<_>>()?,
                meta: project.meta().clone(),
            }),
            Query::GetPage { page } => Answer::Page(project.page(page)?.clone()),
            Query::GetLayer { layer } => Answer::Layer(Box::new(project.layer(layer)?.clone())),
            Query::GetStory { page } => Answer::Story(project.story(page)?),
            Query::GetHistory => Answer::History(self.history()),
        })
    }
}
