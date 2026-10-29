//! Structural checks for the pinned Micro Blossom graph interface.
//! These do not prove growth-bound correctness, offloader safety, or RTL timing.
use crate::Graph;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tree {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    p: Option<usize>,
    l: Option<usize>,
    r: Option<usize>,
}

fn tree(value: &Value, leaves: usize, field: &str) -> Result<(), String> {
    let fail = |reason: &str| format!("invalid upstream {field}: {reason}");
    let tree: Tree = serde_json::from_value(value.clone()).map_err(|e| fail(&e.to_string()))?;
    let count = leaves
        .checked_mul(2)
        .and_then(|v| v.checked_sub(1))
        .ok_or_else(|| fail("invalid leaf count"))?;
    if tree.nodes.len() != count {
        return Err(fail("expected 2 * leaves - 1 nodes"));
    }
    for (i, node) in tree.nodes.iter().enumerate() {
        if i == count - 1 {
            if node.p.is_some() {
                return Err(fail("root must not have a parent"));
            }
        } else {
            let parent = node
                .p
                .filter(|&p| p > i && p < count)
                .ok_or_else(|| fail("parent must be a later node in the tree"))?;
            let parent = &tree.nodes[parent];
            if parent.l != Some(i) && parent.r != Some(i) {
                return Err(fail("parent/child references disagree"));
            }
        }
        if i < leaves {
            if node.l.is_some() || node.r.is_some() {
                return Err(fail("leaf must not have children"));
            }
        } else {
            let (Some(l), Some(r)) = (node.l, node.r) else {
                return Err(fail("internal node needs both children"));
            };
            if l == r || l >= i || r >= i {
                return Err(fail("children must be distinct earlier nodes"));
            }
            if tree.nodes[l].p != Some(i) || tree.nodes[r].p != Some(i) {
                return Err(fail("child/parent references disagree"));
            }
        }
    }
    Ok(())
}

impl Graph {
    /// Validate identity and structural invariants before publishing upstream output.
    #[doc(alias = "validate_hardware_graph")]
    pub fn verify_hardware_graph(&self, hardware: &Value) -> Result<(), String> {
        let expected_edges: Vec<_> = self
            .initializer
            .weighted_edges
            .iter()
            .map(|&(l, r, w)| serde_json::json!({"l": l, "r": r, "w": w}))
            .collect();
        if hardware.get("vertex_num") != Some(&serde_json::json!(self.initializer.vertex_num))
            || hardware.get("virtual_vertices")
                != Some(&serde_json::json!(self.initializer.virtual_vertices))
            || hardware.get("weighted_edges") != Some(&serde_json::json!(expected_edges))
        {
            return Err("upstream graph changed vertex/edge indices, weights or boundaries; observable sidecar would be invalid".into());
        }
        for field in [
            "positions",
            "vertex_binary_tree",
            "edge_binary_tree",
            "vertex_edge_binary_tree",
            "vertex_max_growth",
            "offloading",
        ] {
            if hardware.get(field).is_none_or(Value::is_null) {
                return Err(format!("upstream hardware graph is missing {field}"));
            }
        }
        if hardware["positions"]
            != serde_json::to_value(&self.positions).map_err(|e| e.to_string())?
        {
            return Err("upstream positions differ from exported visualization coordinates".into());
        }
        let n = self.initializer.vertex_num;
        let m = self.initializer.weighted_edges.len();
        tree(&hardware["vertex_binary_tree"], n, "vertex_binary_tree")?;
        tree(&hardware["edge_binary_tree"], m, "edge_binary_tree")?;
        tree(
            &hardware["vertex_edge_binary_tree"],
            n.checked_add(m)
                .ok_or("upstream tree leaf count overflow")?,
            "vertex_edge_binary_tree",
        )?;
        let growth = hardware["vertex_max_growth"]
            .as_array()
            .ok_or("upstream vertex_max_growth must be an array")?;
        if growth.len() != n || growth.iter().any(|v| v.as_i64().is_none_or(|w| w < 0)) {
            return Err(
                "upstream vertex_max_growth must have one nonnegative integer per vertex".into(),
            );
        }
        let offloading = hardware["offloading"]
            .as_array()
            .ok_or("upstream offloading must be an array")?;
        for item in offloading {
            let record = item
                .as_object()
                .filter(|o| o.len() == 1)
                .ok_or("invalid upstream offloading record")?;
            let (kind, payload) = record.iter().next().ok_or("empty offloading record")?;
            let edge = payload
                .get("e")
                .and_then(Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| self.initializer.weighted_edges.get(i))
                .ok_or("invalid upstream offloading edge index")?;
            match kind.as_str() {
                "dm" => {}
                "vm" | "fm" => {
                    let field = if kind == "vm" { "v" } else { "c" };
                    let vertex = payload
                        .get(field)
                        .and_then(Value::as_u64)
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|&i| i < n)
                        .ok_or("invalid upstream offloading vertex index")?;
                    if kind == "vm"
                        && (!self.initializer.virtual_vertices.contains(&vertex)
                            || (vertex != edge.0 && vertex != edge.1))
                    {
                        return Err(
                            "virtual-match offloader must refer to its edge's physical boundary"
                                .into(),
                        );
                    }
                }
                _ => return Err("unknown upstream offloading kind".into()),
            }
        }
        Ok(())
    }
}
