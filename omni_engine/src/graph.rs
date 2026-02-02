use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Topo;
use crate::nodes::AudioNode;
use omni_shared::MidiNoteEvent;

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
    pub fn process(&mut self, final_output: &mut [f32], sample_rate: f32, midi_events: &[MidiNoteEvent]) {
        // Zero out the final output initially? Or assume it's the accumulator?
        final_output.fill(0.0);

        let mut topo = Topo::new(&self.graph);
        
        while let Some(node_idx) = topo.next(&self.graph) {
            if let Some(node) = self.graph.node_weight_mut(node_idx) {
                node.process(final_output, sample_rate, midi_events);
            }
        }
    }
    
    pub fn node_mut(&mut self, idx: NodeIndex) -> Option<&mut Box<dyn AudioNode>> {
        self.graph.node_weight_mut(idx)
    }
}
