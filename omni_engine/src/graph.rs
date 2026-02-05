use petgraph::graph::{DiGraph, NodeIndex};
// use petgraph::visit::Topo;
use crate::nodes::AudioNode;
use omni_shared::MidiNoteEvent;

pub struct AudioGraph {
    graph: DiGraph<Box<dyn AudioNode>, ()>,
    // Processing Chains (Track paths)
    // Each chain is a sequence of nodes: [Source, FX1, FX2...]
    chains: Vec<Vec<NodeIndex>>,
    // Buffers for parallel processing
    buffers: Vec<Vec<f32>>,
}

use rayon::prelude::*;

impl AudioGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            chains: Vec::new(),
            buffers: Vec::new(),
        }
    }

    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeIndex {
        let idx = self.graph.add_node(node);
        // Invalidate chains
        self.chains.clear();
        idx
    }

    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex) {
        self.graph.add_edge(from, to, ());
        self.chains.clear();
    }

    pub fn update_schedule(&mut self) {
        if !self.chains.is_empty() { return; }

        // Simple Chain Discovery: 
        // Find all nodes with 0 incoming edges (Sources)
        // DFS/Walk until leaf or join.
        // For this prototype, we assume independent linear chains (Tracks) that don't merge until Master (which we might handle separately or just sum buffers).
        
        let sources: Vec<NodeIndex> = self.graph.node_indices()
            .filter(|&idx| self.graph.neighbors_directed(idx, petgraph::Direction::Incoming).count() == 0)
            .collect();

        self.chains = sources.into_iter().map(|start| {
            let mut chain = Vec::new();
            let mut current = Some(start);
            while let Some(idx) = current {
                chain.push(idx);
                // Assume single output for now (Linear Chain)
                let mut neighbors = self.graph.neighbors(idx);
                current = neighbors.next();
            }
            chain
        }).collect();
        
        // Resize buffers
        // We need one buffer per chain
        let buffer_size = omni_shared::BUFFER_SIZE * omni_shared::CHANNEL_COUNT; // 512 * 2 = 1024
        self.buffers.resize(self.chains.len(), vec![0.0; buffer_size]);
        
        println!("[AudioGraph] Schedule updated. Chains: {}", self.chains.len());
    }

    /// Parallel processing of specific nodes with provided buffers and events.
    /// This allows the Engine to manage routing/mixing while leveraging the Graph for parallel node execution.
    pub fn process_overlay(&mut self, nodes: &[NodeIndex], buffers: &mut [Vec<f32>], events: &[Vec<MidiNoteEvent>], param_events: &[Vec<omni_shared::ParameterEvent>], sample_rate: f32) {
        let graph_ptr = &mut self.graph as *mut DiGraph<Box<dyn AudioNode>, ()>;
        let ptr_int = graph_ptr as usize;
        
        // Safety: The caller must ensure that the NodeIndices in work_items are distinct.
        // In AudioEngine, track_node_indices are distinct plugins.
        
        buffers.par_iter_mut()
            .enumerate()
            .for_each(move |(i, buffer)| {
                let graph_ref = unsafe { &mut *(ptr_int as *mut DiGraph<Box<dyn AudioNode>, ()>) };
                // Bounds check should be unnecessary if caller ensures lengths match, but let's be safe(r) or just index.
                if i < nodes.len() && i < events.len() && i < param_events.len() {
                     let node_idx = nodes[i];
                     let event_slice = &events[i];
                     let param_event_slice = &param_events[i];
                     if let Some(node) = graph_ref.node_weight_mut(node_idx) {
                         node.process(buffer, sample_rate, event_slice, param_event_slice);
                     }
                }
            });
    }
    
    pub fn node_mut(&mut self, idx: NodeIndex) -> Option<&mut Box<dyn AudioNode>> {
        self.graph.node_weight_mut(idx)
    }
    
    pub fn reset(&mut self) {
        self.graph.clear();
        self.chains.clear();
    }
}

// struct GraphWrapper(*mut DiGraph<Box<dyn AudioNode>, ()>);
// unsafe impl Send for GraphWrapper {}
// unsafe impl Sync for GraphWrapper {}
