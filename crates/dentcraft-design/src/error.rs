//! Shared failures for restoration design.

/// Invalid input or an unsupported geometric configuration.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DesignError {
    /// Crown input must be a connected, oriented disk with its complete boundary.
    #[error("invalid crown surface or cervical boundary")]
    InvalidCrownSurface,
    /// Polar matching requires a simple outline star-shaped about the margin centroid.
    #[error("crown outline is not star-shaped about its placement origin")]
    UnsupportedCrownOutline,
    /// A deformation folded a face or the surfaces intersected.
    #[error("crown deformation folds or intersects a surface")]
    CrownIntersection,
    /// The locked cervical edge or bounded correction could not satisfy thickness.
    #[error("crown thickness correction did not converge (worst deficit {deficit:.6} mm)")]
    CrownThickness { deficit: f64 },
    /// The projected scan lacks a sufficiently symmetric complete arch.
    #[error("partial arch: forward cannot be determined from this scan")]
    PartialArch,
    /// Geometry does not determine an unambiguous jaw orientation.
    #[error("jaw orientation is ambiguous")]
    OrientationAmbiguous,
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
    /// The margin outline turns too sharply or too noisily for the requested
    /// cervical ledge: the offset loop inverts or loses the ledge clearance.
    #[error("the margin outline cannot carry a {thickness:.3} mm cervical ledge")]
    UnsupportedMarginOffset { thickness: f64 },
    /// None of the interior grid rays intersect the prep region.
    #[error("no interior grid samples intersect the prep region")]
    NoSurfaceSamples,
    /// The requested grid exceeds the allocation or offset-work limit.
    #[error("the requested grid is too large; increase grid_step or reduce the gaps")]
    GridTooLarge,
    /// Constrained triangulation failed.
    #[error("could not triangulate the projected margin")]
    Triangulation,
    /// Geometry above the margin extends outside its projected outline.
    #[error("margin undercut: worst horizontal overhang {depth:.6} mm")]
    MarginUndercut { depth: f64 },
    /// Adaptive sampling or clearance correction exceeded its bounded work budget.
    #[error("inner surface refinement or clearance correction did not converge")]
    RefinementLimit,
}
