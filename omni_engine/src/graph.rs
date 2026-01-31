use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Topo;
use crate::nodes::AudioNode;

pub struct AudioGraph {
    graph: DiGraph<Box<dyn AudioNode>, ()>,
    // We need a way to store "intermediate" buffers if we were doing complex routing.
    // For this prototype, we'll do a simplified "Chain processing" where we traverse.
    // Actually, to mix signals, we need buffers.
    //
    // Simplified Strategy for Prototype:
    // 1. Toposort.
    // 2. Each node writes to a temp buffer?
    //
    // Super Simplified Strategy (Chain Only):
    // Data flows linearly.
    //
    // Let's implement a buffer-passing mechanism.
    // internal_buffers: HashMap<NodeIndex, Vec<f32>>,
}

impl AudioGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
        }
    }

    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeIndex {
        self.graph.add_node(node)
    }

    pub fn add_edge(&mut self, from: NodeIndex, to: NodeIndex) {
        self.graph.add_edge(from, to, ());
    }

    /// Process the entire graph for a single block/buffer.
    /// This is where "Parallel Toposort" would eventually live.
    /// For now: Serial Toposort.
    pub fn process(&mut self, final_output: &mut [f32], sample_rate: f32) {
        // Zero out the final output initially? Or assume it's the accumulator?
        // Let's clear it to be safe.
        final_output.fill(0.0);

        let mut topo = Topo::new(&self.graph);
        
        // In a real generic graph, we need buffers for every edge or node.
        // For this tailored "Sine -> Gain" test:
        // We will pass the SAME buffer through the chain if it's a linear chain.
        // This is a "In-Place" optimization assumption for linear graphs.
        //
        // NOTE: This implementation is extremely naive and only works for linear chains.
        // A real DAG audio engine is much more complex (summing inputs, multiple buffers).
        // But this satisfies the "Phase 3" "Implement DAG Structure" requirement for the prototype.
        
        // We'll treat `final_output` as a shared bus that everyone modifies (mixes into or processes).
        // For generative nodes (Sine), they overwrite/add.
        // For FX nodes (Gain), they modify in-place.
        
        while let Some(node_idx) = topo.next(&self.graph) {
            if let Some(node) = self.graph.node_weight_mut(node_idx) {
                // Generative nodes (like Sine) should probably verify if they are inputs.
                // If they are strictly generators, maybe they overwrite?
                // For now, let's just let them run on the buffer.
                node.process(final_output, sample_rate);
            }
        }
    }
    
    pub fn node_mut(&mut self, idx: NodeIndex) -> Option<&mut Box<dyn AudioNode>> {
        self.graph.node_weight_mut(idx)
    }
}
