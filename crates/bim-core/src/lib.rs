//! Renderer-agnostic building model. Names follow IFC (IfcWall, IfcSlab, ...)
//! so an IFC exporter can map 1:1 later.
pub mod index;
pub mod model;
pub mod plan;
pub mod project;
pub use index::*;
pub use model::*;
pub use project::*;
