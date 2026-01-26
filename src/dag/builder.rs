//! DAG builder for constructing dependency graphs from phases.
//!
//! The builder takes a list of phases with their dependencies and constructs
//! a directed acyclic graph (DAG) that can be used for scheduling.

use crate::phase::Phase;
use anyhow::{Result, bail};
use std::collections::{HashMap, HashSet};

/// Index into the phase list.
pub type PhaseIndex = usize;

/// Directed graph edge: (from, to) meaning "from" must complete before "to".
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Edge {
    pub from: PhaseIndex,
    pub to: PhaseIndex,
}

/// A directed acyclic graph of phases.
#[derive(Debug)]
pub struct PhaseGraph {
    /// Phases indexed by their position
    phases: Vec<Phase>,
    /// Map from phase number to index
    index_map: HashMap<String, PhaseIndex>,
    /// Forward edges (dependencies): index -> list of phases that depend on it
    forward_edges: Vec<Vec<PhaseIndex>>,
    /// Reverse edges: index -> list of phases it depends on
    reverse_edges: Vec<Vec<PhaseIndex>>,
}

impl PhaseGraph {
    /// Get the number of phases in the graph.
    pub fn len(&self) -> usize {
        self.phases.len()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.phases.is_empty()
    }

    /// Get a phase by its index.
    pub fn get_phase(&self, index: PhaseIndex) -> Option<&Phase> {
        self.phases.get(index)
    }

    /// Get a phase by its number.
    pub fn get_phase_by_number(&self, number: &str) -> Option<&Phase> {
        self.index_map.get(number).and_then(|&i| self.phases.get(i))
    }

    /// Get the index for a phase number.
    pub fn get_index(&self, number: &str) -> Option<PhaseIndex> {
        self.index_map.get(number).copied()
    }

    /// Get all phases.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Get phases that depend on the given phase (forward edges).
    pub fn dependents(&self, index: PhaseIndex) -> &[PhaseIndex] {
        self.forward_edges.get(index).map_or(&[], |v| v.as_slice())
    }

    /// Get phases that the given phase depends on (reverse edges).
    pub fn dependencies(&self, index: PhaseIndex) -> &[PhaseIndex] {
        self.reverse_edges.get(index).map_or(&[], |v| v.as_slice())
    }

    /// Get phases with no dependencies (entry points).
    pub fn root_phases(&self) -> Vec<PhaseIndex> {
        self.reverse_edges
            .iter()
            .enumerate()
            .filter(|(_, deps)| deps.is_empty())
            .map(|(i, _)| i)
            .collect()
    }

    /// Get phases that no other phase depends on (exit points).
    pub fn leaf_phases(&self) -> Vec<PhaseIndex> {
        self.forward_edges
            .iter()
            .enumerate()
            .filter(|(_, deps)| deps.is_empty())
            .map(|(i, _)| i)
            .collect()
    }

    /// Check if all dependencies of a phase are satisfied.
    pub fn dependencies_satisfied(
        &self,
        index: PhaseIndex,
        completed: &HashSet<PhaseIndex>,
    ) -> bool {
        self.dependencies(index)
            .iter()
            .all(|dep| completed.contains(dep))
    }
}

/// Builder for constructing phase graphs.
pub struct DagBuilder {
    phases: Vec<Phase>,
}

impl DagBuilder {
    /// Create a new builder with the given phases.
    pub fn new(phases: Vec<Phase>) -> Self {
        Self { phases }
    }

    /// Build the phase graph.
    ///
    /// This validates the graph structure:
    /// - All dependencies must reference existing phases
    /// - No cycles are allowed
    pub fn build(self) -> Result<PhaseGraph> {
        if self.phases.is_empty() {
            return Ok(PhaseGraph {
                phases: Vec::new(),
                index_map: HashMap::new(),
                forward_edges: Vec::new(),
                reverse_edges: Vec::new(),
            });
        }

        // Build index map
        let mut index_map = HashMap::new();
        for (i, phase) in self.phases.iter().enumerate() {
            if index_map.contains_key(&phase.number) {
                bail!("Duplicate phase number: {}", phase.number);
            }
            index_map.insert(phase.number.clone(), i);
        }

        // Validate dependencies and build edges
        let mut forward_edges: Vec<Vec<PhaseIndex>> = vec![Vec::new(); self.phases.len()];
        let mut reverse_edges: Vec<Vec<PhaseIndex>> = vec![Vec::new(); self.phases.len()];

        for (to_idx, phase) in self.phases.iter().enumerate() {
            for dep in &phase.depends_on {
                let from_idx = *index_map.get(dep).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown dependency '{}' in phase '{}': no phase with that number exists",
                        dep,
                        phase.number
                    )
                })?;

                // Add edge: from_idx -> to_idx (from must complete before to)
                forward_edges[from_idx].push(to_idx);
                reverse_edges[to_idx].push(from_idx);
            }
        }

        let graph = PhaseGraph {
            phases: self.phases,
            index_map,
            forward_edges,
            reverse_edges,
        };

        // Validate no cycles using topological sort
        Self::validate_no_cycles(&graph)?;

        Ok(graph)
    }

    /// Validate that the graph has no cycles using Kahn's algorithm.
    fn validate_no_cycles(graph: &PhaseGraph) -> Result<()> {
        let mut in_degree: Vec<usize> = graph.reverse_edges.iter().map(|deps| deps.len()).collect();

        let mut queue: Vec<PhaseIndex> = in_degree
            .iter()
            .enumerate()
            .filter(|&(_, deg)| *deg == 0)
            .map(|(i, _)| i)
            .collect();

        let mut processed = 0;

        while let Some(node) = queue.pop() {
            processed += 1;

            for &dependent in graph.dependents(node) {
                in_degree[dependent] -= 1;
                if in_degree[dependent] == 0 {
                    queue.push(dependent);
                }
            }
        }

        if processed != graph.len() {
            // Find phases involved in cycles for better error message
            let cycle_phases: Vec<&str> = in_degree
                .iter()
                .enumerate()
                .filter(|&(_, deg)| *deg > 0)
                .filter_map(|(i, _)| graph.get_phase(i).map(|p| p.number.as_str()))
                .collect();

            bail!(
                "Cycle detected in phase dependencies. Involved phases: {:?}",
                cycle_phases
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(number: &str, deps: Vec<&str>) -> Phase {
        Phase::new(
            number,
            &format!("Phase {}", number),
            &format!("{} DONE", number),
            5,
            "test",
            deps.into_iter().map(String::from).collect(),
        )
    }

    #[test]
    fn test_build_simple_graph() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["01"]),
            phase("04", vec!["02", "03"]),
        ];

        let graph = DagBuilder::new(phases).build().unwrap();

        assert_eq!(graph.len(), 4);
        assert_eq!(graph.root_phases(), vec![0]); // Phase 01
        assert_eq!(graph.leaf_phases(), vec![3]); // Phase 04
    }

    #[test]
    fn test_dependencies_and_dependents() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["01"]),
        ];

        let graph = DagBuilder::new(phases).build().unwrap();

        // Phase 01 has no dependencies
        assert!(graph.dependencies(0).is_empty());
        // Phases 02 and 03 depend on 01
        assert_eq!(graph.dependencies(1), &[0]);
        assert_eq!(graph.dependencies(2), &[0]);
        // Phase 01 has 02 and 03 as dependents
        let dependents = graph.dependents(0);
        assert!(dependents.contains(&1));
        assert!(dependents.contains(&2));
    }

    #[test]
    fn test_cycle_detection() {
        let phases = vec![
            phase("01", vec!["03"]),
            phase("02", vec!["01"]),
            phase("03", vec!["02"]),
        ];

        let result = DagBuilder::new(phases).build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cycle"));
    }

    #[test]
    fn test_missing_dependency() {
        let phases = vec![phase("01", vec!["nonexistent"])];

        let result = DagBuilder::new(phases).build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn test_duplicate_phase_number() {
        let phases = vec![phase("01", vec![]), phase("01", vec![])];

        let result = DagBuilder::new(phases).build();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_empty_graph() {
        let graph = DagBuilder::new(vec![]).build().unwrap();
        assert!(graph.is_empty());
    }

    #[test]
    fn test_dependencies_satisfied() {
        let phases = vec![
            phase("01", vec![]),
            phase("02", vec!["01"]),
            phase("03", vec!["01", "02"]),
        ];

        let graph = DagBuilder::new(phases).build().unwrap();
        let mut completed = HashSet::new();

        // Phase 01 can run (no deps)
        assert!(graph.dependencies_satisfied(0, &completed));
        // Phase 02 cannot run yet
        assert!(!graph.dependencies_satisfied(1, &completed));

        // Complete phase 01
        completed.insert(0);
        // Phase 02 can now run
        assert!(graph.dependencies_satisfied(1, &completed));
        // Phase 03 still cannot (needs 02)
        assert!(!graph.dependencies_satisfied(2, &completed));

        // Complete phase 02
        completed.insert(1);
        // Phase 03 can now run
        assert!(graph.dependencies_satisfied(2, &completed));
    }
}
