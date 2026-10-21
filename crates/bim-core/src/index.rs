//! Element and door-opening lookups. `Project` answers them with linear scans (fine for one
//! element); `ProjectIndex` hashes them once so whole-project passes stay O(n log n).
use crate::{model::*, project::*};
use std::borrow::Cow;
use std::collections::HashMap;

/// (offset, width, height) of a door opening along its host wall.
pub type Opening = (f32, f32, f32);

pub trait Lookup {
    /// First element with this id.
    fn get(&self, id: ElementId) -> Result<&Element, ProjectError>;
    /// Doors hosted by `wall`, sorted by offset (see `Project::openings_in`).
    fn openings(&self, wall: ElementId) -> Cow<'_, [Opening]>;
}

impl Lookup for Project {
    fn get(&self, id: ElementId) -> Result<&Element, ProjectError> {
        self.element(id)
    }
    fn openings(&self, wall: ElementId) -> Cow<'_, [Opening]> {
        Cow::Owned(self.openings_in(wall))
    }
}

/// Hashed lookups over a borrowed project. Same answers as `Project`'s scans
/// (first element wins on a duplicate id; openings in element order, stably sorted).
pub struct ProjectIndex<'a> {
    pub project: &'a Project,
    by_id: HashMap<ElementId, usize>,
    openings: HashMap<ElementId, Vec<Opening>>,
}

impl<'a> ProjectIndex<'a> {
    pub fn new(project: &'a Project) -> Self {
        let mut by_id = HashMap::with_capacity(project.elements.len());
        let mut openings: HashMap<ElementId, Vec<Opening>> = HashMap::new();
        for (i, e) in project.elements.iter().enumerate() {
            by_id.entry(e.id).or_insert(i);
            if let ElementKind::Door {
                host,
                offset,
                width,
                height,
                ..
            } = e.kind
            {
                openings
                    .entry(host)
                    .or_default()
                    .push((offset, width, height));
            }
        }
        for v in openings.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        Self {
            project,
            by_id,
            openings,
        }
    }
    /// Position of the element in `project.elements`.
    pub fn position(&self, id: ElementId) -> Option<usize> {
        self.by_id.get(&id).copied()
    }
}

impl Lookup for ProjectIndex<'_> {
    fn get(&self, id: ElementId) -> Result<&Element, ProjectError> {
        self.position(id)
            .map(|i| &self.project.elements[i])
            .ok_or(ProjectError::MissingElement(id))
    }
    fn openings(&self, wall: ElementId) -> Cow<'_, [Opening]> {
        Cow::Borrowed(self.openings.get(&wall).map_or(&[][..], |v| v.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_matches_scans() {
        let p = Project::sample();
        let ix = ProjectIndex::new(&p);
        for e in &p.elements {
            assert_eq!(ix.get(e.id).unwrap(), p.get(e.id).unwrap());
            assert_eq!(ix.openings(e.id), p.openings(e.id));
        }
        assert!(ix.get(ElementId::new()).is_err());
    }
}
