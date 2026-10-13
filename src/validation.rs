//! Validation shared by bridge-produced IR and direct Rust imports.
use crate::{Dem, canonical};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotations {
    schema_version: u32,
    annotation_only: bool,
    windows: Vec<Window>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    id: String,
    layer: u8,
    commit: Vec<usize>,
    buffer: Vec<usize>,
    temporal_boundary: String,
    commit_buffer_seams: Vec<Reference>,
    outer_crossings: Vec<Reference>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Reference {
    error: usize,
    component: usize,
}

impl Dem {
    /// Validate all IDs, including zero-probability mechanisms, before conversion.
    /// Annotations are checked against the DEM, not trusted as executable semantics.
    #[doc(alias = "validate_ir")]
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported IR schema version".into());
        }
        if self.num_detectors == 0 || self.detector_coordinates.len() != self.num_detectors {
            return Err("need at least one detector and coordinates entry per detector".into());
        }
        if self
            .detector_coordinates
            .iter()
            .flatten()
            .any(|c| !c.is_finite())
        {
            return Err("coordinates must be finite".into());
        }
        for (ei, error) in self.errors.iter().enumerate() {
            if !error.probability.is_finite() || !(0.0..=0.5).contains(&error.probability) {
                return Err(format!(
                    "error {ei}: only probabilities in [0, 0.5] are supported"
                ));
            }
            if error.components.is_empty() {
                return Err(format!("error {ei}: missing components"));
            }
            for (ci, c) in error.components.iter().enumerate() {
                canonical(&c.detectors, self.num_detectors)
                    .and_then(|_| canonical(&c.observables, self.num_observables))
                    .map_err(|e| format!("error {ei} component {ci}: {e}"))?;
            }
        }
        if !self.window_annotations.is_null() {
            self.validate_annotations()?;
        }
        Ok(())
    }

    fn validate_annotations(&self) -> Result<(), String> {
        let annotations: Annotations = serde_json::from_value(self.window_annotations.clone())
            .map_err(|e| format!("invalid window annotations: {e}"))?;
        if annotations.schema_version != 1 || !annotations.annotation_only {
            return Err(
                "window annotations require schema_version 1 and annotation_only true".into(),
            );
        }
        if annotations.windows.is_empty() {
            return Err("window annotations must contain at least one window".into());
        }
        let mut ids = BTreeSet::new();
        let mut committed = [BTreeSet::new(), BTreeSet::new()];
        for window in annotations.windows {
            let fail = |reason: &str| format!("window '{}': {reason}", window.id);
            if window.id.trim().is_empty() || !ids.insert(window.id.clone()) {
                return Err(fail("IDs must be unique nonempty strings"));
            }
            let boundary = match window.layer {
                0 => "open",
                1 => "closed",
                _ => return Err(fail("layer must be 0 or 1")),
            };
            if window.temporal_boundary != boundary {
                return Err(fail("temporal boundary conflicts with layer"));
            }
            let commit: BTreeSet<_> = canonical(&window.commit, self.num_detectors)
                .map_err(|e| fail(&e))?
                .into_iter()
                .collect();
            let buffer: BTreeSet<_> = canonical(&window.buffer, self.num_detectors)
                .map_err(|e| fail(&e))?
                .into_iter()
                .collect();
            if commit.is_empty() || !commit.is_disjoint(&buffer) {
                return Err(fail("commit must be nonempty and disjoint from buffer"));
            }
            if window.layer == 1 && !buffer.is_empty() {
                return Err(fail("layer 1 does not have a buffer"));
            }
            let owned = &mut committed[usize::from(window.layer)];
            if !owned.is_disjoint(&commit) {
                return Err(fail("same-layer commit regions overlap"));
            }
            owned.extend(&commit);
            let members: BTreeSet<_> = commit.union(&buffer).copied().collect();
            let mut seams = Vec::new();
            let mut cuts = Vec::new();
            for (ei, error) in self.errors.iter().enumerate() {
                for (ci, c) in error.components.iter().enumerate() {
                    let ds: BTreeSet<_> = c.detectors.iter().copied().collect();
                    if !ds.is_disjoint(&commit) && !ds.is_disjoint(&buffer) {
                        seams.push(Reference {
                            error: ei,
                            component: ci,
                        });
                    }
                    if !ds.is_disjoint(&members) && !ds.is_subset(&members) {
                        cuts.push(Reference {
                            error: ei,
                            component: ci,
                        });
                    }
                }
            }
            if seams != window.commit_buffer_seams || cuts != window.outer_crossings {
                return Err(fail("seam/crossing references do not match the DEM"));
            }
        }
        Ok(())
    }
}
