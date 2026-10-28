//! Shared failures for restoration design.

/// Invalid input or an unsupported geometric configuration.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DesignError {
    /// The region contains no triangles.
    #[error("the prep region is empty")]
    EmptyRegion,
    /// All region triangles have zero area.
    #[error("the prep region has no nondegenerate triangles")]
    DegenerateRegion,
    /// A region or vertex index is invalid, or a used position is nonfinite.
    #[error("the prep region contains invalid indices or nonfinite geometry")]
    InvalidMesh,
    /// An axis is zero or nonfinite.
    #[error("the insertion axis must be finite and nonzero")]
    InvalidAxis,
    /// A numeric parameter is outside its documented range.
    #[error("invalid parameter: {0}")]
    InvalidParameter(&'static str),
    /// The margin has too few distinct projected points, overlaps or crosses itself.
    #[error("the projected margin must be a simple polygon with at least three points")]
    InvalidMargin,
    /// None of the interior grid rays intersect the prep region.
    #[error("no interior grid samples intersect the prep region")]
    NoSurfaceSamples,
    /// The requested grid exceeds the allocation or offset-work limit.
    #[error("the requested grid is too large; increase grid_step or reduce the gaps")]
    GridTooLarge,
    /// Constrained triangulation failed.
    #[error("could not triangulate the projected margin")]
    Triangulation,
}
