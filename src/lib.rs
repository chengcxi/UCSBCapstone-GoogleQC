//! Strict graphlike DEM IR conversion to Fusion/Micro Blossom parser input.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub mod decoding;
mod hardware;
mod validation;
pub mod window;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dem {
    pub schema_version: u32,
    pub num_detectors: usize,
    pub num_observables: usize,
    pub detector_coordinates: Vec<Vec<f64>>,
    pub errors: Vec<ErrorMechanism>,
    #[serde(default)]
    pub provenance: serde_json::Value,
    #[serde(default)]
    pub window_annotations: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorMechanism {
    pub probability: f64,
    pub components: Vec<Component>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub detectors: Vec<usize>,
    pub observables: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Initializer {
    pub vertex_num: usize,
    pub weighted_edges: Vec<(usize, usize, i64)>,
    pub virtual_vertices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub i: f64,
    pub j: f64,
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeMetadata {
    pub detectors: Vec<usize>,
    pub observables: Vec<usize>,
    pub probability: f64,
    /// Original error/component indices, including merged independent mechanisms.
    pub sources: Vec<(usize, usize)>,
}

#[derive(Debug, Serialize)]
pub struct Graph {
    pub schema_version: u32,
    pub num_detectors: usize,
    pub num_observables: usize,
    /// Full coordinates are semantic metadata; positions below are visualization only.
    pub detector_coordinates: Vec<Vec<f64>>,
    pub initializer: Initializer,
    pub positions: Vec<Position>,
    /// Index-aligned with initializer.weighted_edges. XOR these observable IDs
    /// along the selected correction; upstream parser input does not store them.
    pub edge_metadata: Vec<EdgeMetadata>,
    pub weight_scale: f64,
    pub correlated_components_approximated: bool,
    pub provenance: serde_json::Value,
    pub window_annotations: serde_json::Value,
}

fn canonical(ids: &[usize], limit: usize) -> Result<Vec<usize>, String> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    if sorted.iter().any(|&d| d >= limit) || sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err("IR IDs must be in range, unique, and parity-normalized".into());
    }
    Ok(sorted)
}

pub fn convert(dem: Dem, scale: f64, allow_correlated: bool) -> Result<Graph, String> {
    dem.validate()?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("weight scale must be positive and finite".into());
    }
    let mut merged: BTreeMap<Vec<usize>, EdgeMetadata> = BTreeMap::new();
    let mut approximated = false;
    for (ei, error) in dem.errors.iter().enumerate() {
        let p = error.probability;
        if p == 0.0 {
            continue;
        }
        if error.components.len() > 1 {
            if !allow_correlated {
                return Err(format!(
                    "error {ei}: correlated decomposition requires --allow-correlated"
                ));
            }
            approximated = true;
        }
        let mut within_error = std::collections::BTreeSet::new();
        for (ci, component) in error.components.iter().enumerate() {
            let ds = canonical(&component.detectors, dem.num_detectors)?;
            let obs = canonical(&component.observables, dem.num_observables)?;
            if ds.is_empty() {
                if !obs.is_empty() {
                    return Err(format!(
                        "error {ei}: undetectable logical fault cannot be an MWPM edge"
                    ));
                }
                continue;
            }
            if ds.len() > 2 {
                return Err(format!("error {ei}: hyperedge has {} detectors", ds.len()));
            }
            if !within_error.insert(ds.clone()) {
                return Err(format!(
                    "error {ei}: repeated support within correlated mechanism"
                ));
            }
            if let Some(edge) = merged.get_mut(&ds) {
                if edge.observables != obs {
                    return Err(format!(
                        "error {ei}: parallel edges with differing logical labels are unsupported"
                    ));
                }
                // Independent errors on identical support combine by odd parity.
                edge.probability = edge.probability * (1.0 - p) + p * (1.0 - edge.probability);
                edge.sources.push((ei, ci));
            } else {
                merged.insert(
                    ds.clone(),
                    EdgeMetadata {
                        detectors: ds,
                        observables: obs,
                        probability: p,
                        sources: vec![(ei, ci)],
                    },
                );
            }
        }
    }
    if merged.is_empty() {
        return Err("DEM has no nonzero graphlike error mechanisms".into());
    }
    let boundary = dem.num_detectors;
    let has_boundary = merged.keys().any(|ds| ds.len() == 1);
    let mut edges = Vec::new();
    let metadata: Vec<_> = merged.into_values().collect();
    for edge in &metadata {
        // Micro Blossom's ErrorPatternReader requires EVEN integer weights.
        // Avoid forming (1-p)/p: it overflows for valid subnormal p.
        let log_odds = (-edge.probability).ln_1p() - edge.probability.ln();
        let half = ((scale / 2.0) * log_odds).round();
        if !half.is_finite() || half > f64::from(i32::MAX / 2) {
            return Err("quantized weight exceeds supported signed 32-bit range".into());
        }
        let weight = (half as i64) * 2;
        edges.push((
            edge.detectors[0],
            *edge.detectors.get(1).unwrap_or(&boundary),
            weight,
        ));
    }
    let mut positions: Vec<_> = dem
        .detector_coordinates
        .iter()
        .map(|c| Position {
            i: c.first().copied().unwrap_or(0.0),
            j: c.get(1).copied().unwrap_or(0.0),
            t: c.get(2).copied().unwrap_or(0.0),
        })
        .collect();
    if has_boundary {
        positions.push(Position {
            i: 0.0,
            j: 0.0,
            t: 0.0,
        });
    }
    Ok(Graph {
        schema_version: 1,
        num_detectors: dem.num_detectors,
        num_observables: dem.num_observables,
        detector_coordinates: dem.detector_coordinates,
        initializer: Initializer {
            vertex_num: dem.num_detectors + usize::from(has_boundary),
            weighted_edges: edges,
            virtual_vertices: if has_boundary { vec![boundary] } else { vec![] },
        },
        positions,
        edge_metadata: metadata,
        weight_scale: scale,
        correlated_components_approximated: approximated,
        provenance: dem.provenance,
        window_annotations: dem.window_annotations,
    })
}

/// Global correction parity; residual_syndrome = input syndrome XOR correction boundary.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CorrectionParity {
    pub detectors: Vec<usize>,
    pub observables: Vec<usize>,
    pub residual_syndrome: Vec<usize>,
}

fn toggle(set: &mut BTreeSet<usize>, value: usize) {
    if !set.insert(value) {
        set.remove(&value);
    }
}

impl Graph {
    /// Apply correction edges by XOR. Repeated edges cancel; duplicate input
    /// defect IDs are rejected. This is algebra, not a decoder or a seam oracle.
    #[doc(alias = "check_correction")]
    pub fn evaluate_correction(
        &self,
        edges: &[usize],
        defects: &[usize],
    ) -> Result<CorrectionParity, String> {
        let mut residual: BTreeSet<_> = canonical(defects, self.num_detectors)?
            .into_iter()
            .collect();
        let mut detectors = BTreeSet::new();
        let mut observables = BTreeSet::new();
        for &index in edges {
            let edge = self
                .edge_metadata
                .get(index)
                .ok_or_else(|| format!("correction edge index {index} is out of range"))?;
            for &detector in &edge.detectors {
                toggle(&mut detectors, detector);
                toggle(&mut residual, detector);
            }
            for &observable in &edge.observables {
                toggle(&mut observables, observable);
            }
        }
        Ok(CorrectionParity {
            detectors: detectors.into_iter().collect(),
            observables: observables.into_iter().collect(),
            residual_syndrome: residual.into_iter().collect(),
        })
    }

    /// A graph-only Syndrome Pattern v1.0 file (no fabricated syndrome shots).
    /// Run upstream `micro_blossom parser` to derive the hardware graph metadata.
    pub fn parser_input(&self) -> Result<String, serde_json::Error> {
        Ok(format!(
            "Syndrome Pattern v1.0 microblossom-toolchain\n{}\n{}\n",
            serde_json::to_string(&self.initializer)?,
            serde_json::to_string(&self.positions)?
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dem(errors: serde_json::Value) -> Dem {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "num_detectors": 3, "num_observables": 2,
            "detector_coordinates": [[0,0,0], [1,0,1], []], "errors": errors
        }))
        .unwrap()
    }
    fn e(p: f64, ds: &[usize], obs: &[usize]) -> serde_json::Value {
        serde_json::json!({"probability": p, "components": [{"detectors": ds, "observables": obs}]})
    }
    #[test]
    fn boundary_weights_and_observables() {
        let g = convert(
            dem(serde_json::json!([
                e(0.1, &[1], &[0]),
                e(0.2, &[0, 2], &[])
            ])),
            100.0,
            false,
        )
        .unwrap();
        assert_eq!(g.initializer.virtual_vertices, vec![3]);
        assert_eq!(g.initializer.vertex_num, 4);
        assert_eq!(g.initializer.weighted_edges, vec![(0, 2, 138), (1, 3, 220)]);
        assert_eq!(g.edge_metadata[1].observables, vec![0]);
        assert_eq!(g.parser_input().unwrap().lines().count(), 3);
    }
    #[test]
    fn merges_independent_faults() {
        let g = convert(
            dem(serde_json::json!([
                e(0.1, &[0, 1], &[]),
                e(0.2, &[1, 0], &[])
            ])),
            100.0,
            false,
        )
        .unwrap();
        assert!((g.edge_metadata[0].probability - 0.26).abs() < 1e-12);
        assert!(g.initializer.virtual_vertices.is_empty());
    }
    #[test]
    fn rejects_lossy_or_invalid_inputs() {
        for errors in [
            serde_json::json!([e(0.1, &[0, 1, 2], &[])]),
            serde_json::json!([e(0.1, &[], &[0])]),
            serde_json::json!([e(0.6, &[0], &[])]),
            serde_json::json!([e(0.1, &[3], &[])]),
            serde_json::json!([e(0.1, &[0, 0], &[])]),
            serde_json::json!([e(0.1, &[0], &[2])]),
            serde_json::json!([e(0.1, &[0], &[]), e(0.1, &[0], &[1])]),
        ] {
            assert!(convert(dem(errors), 100.0, false).is_err());
        }
    }
    #[test]
    fn correlation_requires_explicit_opt_in() {
        let errors = serde_json::json!([{"probability":0.1,"components":[
            {"detectors":[0],"observables":[]}, {"detectors":[1,2],"observables":[0]}]}]);
        assert!(convert(dem(errors.clone()), 100.0, false).is_err());
        assert!(
            convert(dem(errors), 100.0, true)
                .unwrap()
                .correlated_components_approximated
        );
    }
    #[test]
    fn validates_scale_and_zero_probability() {
        assert!(convert(dem(serde_json::json!([e(0.1, &[0], &[])])), f64::NAN, false).is_err());
        let g = convert(
            dem(serde_json::json!([e(0.0, &[0], &[]), e(0.5, &[1], &[])])),
            100.0,
            false,
        )
        .unwrap();
        assert_eq!(g.initializer.weighted_edges, vec![(1, 3, 0)]);
    }
    #[test]
    fn subnormal_probability_has_finite_weight() {
        let g = convert(
            dem(serde_json::json!([e(f64::from_bits(1), &[0], &[])])),
            100.0,
            false,
        )
        .unwrap();
        let weight = g.initializer.weighted_edges[0].2;
        assert!(weight > 70_000 && weight < 75_000);
        assert_eq!(weight % 2, 0);
    }

    #[test]
    fn weight_overflow_is_rejected() {
        assert!(convert(dem(serde_json::json!([e(0.1, &[0], &[])])), f64::MAX, false).is_err());
    }

    #[test]
    fn validates_zero_probability_ids_and_empty_components() {
        for errors in [
            serde_json::json!([e(0.0, &[99], &[]), e(0.1, &[0], &[])]),
            serde_json::json!([e(0.0, &[0], &[99]), e(0.1, &[0], &[])]),
            serde_json::json!([{"probability": 0.0, "components": []}, e(0.1, &[0], &[])]),
        ] {
            assert!(convert(dem(errors), 100.0, false).is_err());
        }
    }

    #[test]
    fn preserves_full_coordinate_metadata() {
        let mut d = dem(serde_json::json!([e(0.1, &[0], &[])]));
        d.detector_coordinates[0] = vec![1.0, 2.0, 3.0, 42.0];
        let g = convert(d, 100.0, false).unwrap();
        assert_eq!(g.detector_coordinates[0], vec![1.0, 2.0, 3.0, 42.0]);
        assert_eq!((g.num_detectors, g.num_observables), (3, 2));
    }

    #[test]
    fn correction_parity_cancels_and_tracks_multiple_observables() {
        let g = convert(
            dem(serde_json::json!([
                e(0.1, &[0, 1], &[0, 1]),
                e(0.2, &[1], &[1])
            ])),
            100.0,
            false,
        )
        .unwrap();
        let c = g.evaluate_correction(&[0, 1], &[0]).unwrap();
        assert_eq!(c.detectors, vec![0]);
        assert_eq!(c.observables, vec![0]);
        assert!(c.residual_syndrome.is_empty());
        let c = g.evaluate_correction(&[0, 0], &[2]).unwrap();
        assert!(c.detectors.is_empty() && c.observables.is_empty());
        assert_eq!(c.residual_syndrome, vec![2]);
        assert!(g.evaluate_correction(&[2], &[]).is_err());
        assert!(g.evaluate_correction(&[], &[0, 0]).is_err());
        assert!(g.evaluate_correction(&[], &[3]).is_err());
    }

    fn annotated_dem() -> Dem {
        let mut d = dem(serde_json::json!([e(0.1, &[0, 1], &[0])]));
        d.window_annotations = serde_json::json!({
            "schema_version": 1, "annotation_only": true, "windows": [{
                "id": "top", "layer": 0, "commit": [0], "buffer": [1],
                "temporal_boundary": "open",
                "commit_buffer_seams": [{"error": 0, "component": 0}],
                "outer_crossings": []
            }]
        });
        d
    }

    #[test]
    fn rust_revalidates_window_annotations() {
        assert!(convert(annotated_dem(), 100.0, false).is_ok());
        for (field, value) in [
            ("layer", serde_json::json!(2)),
            ("temporal_boundary", serde_json::json!("closed")),
            ("commit", serde_json::json!([99])),
            ("buffer", serde_json::json!([0])),
            ("commit_buffer_seams", serde_json::json!([])),
            (
                "outer_crossings",
                serde_json::json!([{"error": 99, "component": 0}]),
            ),
        ] {
            let mut d = annotated_dem();
            d.window_annotations["windows"][0][field] = value;
            assert!(convert(d, 100.0, false).is_err(), "{field}");
        }
        let mut d = annotated_dem();
        let mut duplicate = d.window_annotations["windows"][0].clone();
        duplicate["id"] = serde_json::json!("overlap");
        d.window_annotations["windows"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(convert(d, 100.0, false).unwrap_err().contains("same-layer"));
    }

    #[test]
    fn ir_schema_rejects_unknown_fields() {
        let mut value = serde_json::json!({
            "schema_version": 1, "num_detectors": 1, "num_observables": 0,
            "detector_coordinates": [[]], "errors": [e(0.1, &[0], &[])], "typo": true
        });
        assert!(serde_json::from_value::<Dem>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove("typo");
        value["errors"][0]["components"][0]["typo"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Dem>(value).is_err());
    }
}
