use crate::traits::RandomAccessGraph;
use bitvec::prelude::*;
use dsi_progress_logger::ProgressLogger;
use std::collections::VecDeque;

/// Iterator on all nodes of the graph in a BFS order
pub struct BfsOrder<'a, G: RandomAccessGraph> {
    graph: &'a G,
    pl: ProgressLogger<'static>,
    visited: Vec<bool>,
    queue: VecDeque<usize>,
    /// If the queue is empty, resume the BFS from that node.
    ///
    /// This allows initializing the BFS from all orphan nodes without reading
    /// the reverse graph.
    start: usize,
}

impl<'a, G: RandomAccessGraph> BfsOrder<'a, G> {
    pub fn new(graph: &G) -> BfsOrder<G> {
        let num_nodes = graph.num_nodes();
        let mut pl = ProgressLogger::default().display_memory();
        pl.item_name = "node";
        pl.local_speed = true;
        pl.expected_updates = Some(num_nodes);
        pl.start("Visiting graph in BFS order...");
        BfsOrder {
            graph,
            pl,
            visited: vec![false; num_nodes],
            queue: VecDeque::new(),
            start: 0,
        }
    }
}

impl<'a, G: RandomAccessGraph> Iterator for BfsOrder<'a, G> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        self.pl.light_update();
        let current_node = match self.queue.pop_front() {
            None => {
                while unsafe { *self.visited.get_unchecked(self.start) } {
                    self.start += 1;
                    if self.start >= self.graph.num_nodes() {
                        self.pl.done();
                        return None;
                    }
                }
                unsafe { *self.visited.get_unchecked_mut(self.start) = true };
                self.start
            }
            Some(node) => node,
        };

        for succ in self.graph.successors(current_node) {
            if unsafe { !*self.visited.get_unchecked(succ) } {
                self.queue.push_back(succ);
                unsafe { *self.visited.get_unchecked_mut(succ) = true };
            }
        }

        Some(current_node)
    }
}

impl<'a, G: RandomAccessGraph> ExactSizeIterator for BfsOrder<'a, G> {
    fn len(&self) -> usize {
        self.graph.num_nodes()
    }
}
