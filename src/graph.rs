use cargo_metadata::Metadata;
use petgraph::{
    algo::all_simple_paths,
    stable_graph::{NodeIndex, StableDiGraph},
    Direction,
};

use crate::{cli::Config, dep_info::DepInfo, package::Package};

mod builder;

use builder::DepGraphBuilder;

pub type DepGraph = StableDiGraph<Package, DepInfo, u16>;

pub fn get_dep_graph(metadata: Metadata, config: &Config) -> anyhow::Result<DepGraph> {
    let mut builder = DepGraphBuilder::new(metadata)?;
    builder.add_workspace_members()?;
    builder.add_dependencies(config)?;

    Ok(builder.graph)
}

pub fn update_dep_info(graph: &mut DepGraph) {
    for idx in graph.node_indices().collect::<Vec<_>>() {
        // We're only mutating nodes, not adding or deleting them, so we can safely use the indices
        // that were collected at the start thoughout to visit each node once (or more than once,
        // in case we recurse inside update_node).
        update_node(graph, idx);
    }
}

fn update_node(graph: &mut DepGraph, idx: NodeIndex<u16>) {
    // Special case for workspace members
    if graph[idx].dep_info.is_none() {
        let mut outgoing = graph.neighbors_directed(idx, Direction::Outgoing).detach();
        while let Some(edge_idx) = outgoing.next_edge(graph) {
            graph[edge_idx].visited = true;
        }

        return;
    }

    let mut incoming = graph.neighbors_directed(idx, Direction::Incoming).detach();
    let mut node_info: Option<DepInfo> = None;
    while let Some((edge_idx, node_idx)) = incoming.next(graph) {
        if !graph[edge_idx].visited {
            update_node(graph, node_idx);
        }

        let edge_info = graph[edge_idx];
        assert!(edge_info.visited);

        if let Some(i) = &mut node_info {
            i.is_target_dep &= edge_info.is_target_dep;
            i.is_optional &= edge_info.is_optional;
            i.kind.combine_incoming(edge_info.kind);
        } else {
            node_info = Some(edge_info);
        }
    }

    let node_info = node_info.expect("non-workspace members to have at least one incoming edge");
    graph[idx].dep_info = Some(node_info);

    let mut outgoing = graph.neighbors_directed(idx, Direction::Outgoing).detach();
    while let Some(edge_idx) = outgoing.next_edge(graph) {
        let edge_info = &mut graph[edge_idx];

        // it's unclear to me why this happens... maybe a bug in petgraph?
        if edge_info.visited {
            continue;
        }

        edge_info.visited = true;
        edge_info.is_target_dep |= node_info.is_target_dep;
        edge_info.is_optional |= node_info.is_optional;
        edge_info.kind.update_outgoing(node_info.kind);
    }
}

pub fn dedup_transitive_deps(graph: &mut DepGraph) {
    // this can probably be optimized.
    // maybe it would make sense to make this less conservative about what to remove.

    for idx in graph.node_indices().collect::<Vec<_>>() {
        // We're only removing nodes, not adding new ones, so we can use the node indices collected
        // at the start as long as we check that they're still valid within the current graph.
        if !graph.contains_node(idx) {
            continue;
        }

        let mut outgoing = graph.neighbors_directed(idx, Direction::Outgoing).detach();
        while let Some((edge_idx, node_idx)) = outgoing.next(graph) {
            let any_paths =
                all_simple_paths::<Vec<_>, _>(&*graph, idx, node_idx, 1, None).next().is_some();

            if any_paths {
                graph.remove_edge(edge_idx);
            }

            // Previous more conversative and also buggy version
            /*if graph.neighbors_directed(node_idx, Direction::Incoming).count() < 2 {
                // graph[idx] is the only node that depends on graph[node_idx], do nothing
                break;
            }

            let node_kind = graph[idx].dep_kind();
            let paths: Vec<_> =
                all_simple_paths::<Vec<_>, _>(&*graph, idx, node_idx, 1, None).collect();
            if paths.iter().any(|path| path.iter().all(|&i| graph[i].dep_kind() == node_kind)) {
                graph.remove_edge(edge_idx);
            }*/
        }
    }
}
