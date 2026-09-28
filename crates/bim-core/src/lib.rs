//! Renderer-agnostic building model. Names follow IFC (IfcWall, IfcSlab, ...)
//! so an IFC exporter can map 1:1 later.
pub mod model;
pub mod project;
pub use model::*;
pub use project::*;
