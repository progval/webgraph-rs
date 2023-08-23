use crate::traits::*;
use anyhow::{bail, Result};
use dsi_progress_logger::ProgressLogger;
use log::info;
use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rayon::prelude::*;
use rayon::slice::ParallelSliceMut;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Mutex;

/// Write the permutation computed by the LLP algorithm inside `perm`,
/// and return the labels of said permutation.
///
/// # References
/// [Layered Label Propagation: A MultiResolution Coordinate-Free Ordering for Compressing Social Networks](https://arxiv.org/pdf/1011.5425.pdf>)
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn layered_label_propagation<G>(
    graph: &G,
    perm: &mut [usize],
    gamma: f64,
    num_cpus: Option<usize>,
    max_iters: usize,
    chunk_size: usize,
    granularity: usize,
    seed: u64,
) -> Result<Box<[usize]>>
where
    G: RandomAccessGraph + Sync,
{
    let num_cpus = num_cpus.unwrap_or_else(num_cpus::get);
    let num_nodes = graph.num_nodes();

    if perm.len() != num_nodes {
        bail!(
            "The permutation slice is long {} but we expect it to be {}.",
            perm.len(),
            num_nodes
        );
    }
    // init the permutation with the indices
    perm.iter_mut().enumerate().for_each(|(i, x)| *x = i);

    let mut can_change = Vec::with_capacity(num_nodes as _);
    can_change.extend((0..num_nodes).map(|_| AtomicBool::new(true)));
    let label_store = LabelStore::new(num_nodes as _);

    // build a thread_pool so we avoid having to re-create the threads
    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus)
        .build()?;

    // init the progress logger
    let mut glob_pr = ProgressLogger::default().display_memory();
    glob_pr.item_name = "update";
    glob_pr.start("Starting updates...");

    let seed = AtomicU64::new(seed);
    for _ in 0..max_iters {
        thread_pool.install(|| {
            // parallel shuffle using the num_cpus
            perm.par_chunks_mut(chunk_size).for_each(|chunk| {
                let seed = seed.fetch_add(1, Ordering::Relaxed);
                let mut rand = SmallRng::seed_from_u64(seed);
                chunk.shuffle(&mut rand);
            });
        });
        let mut pr = ProgressLogger::default();
        pr.item_name = "node";
        pr.local_speed = true;
        pr.expected_updates = Some(num_nodes);
        pr.start("Updating...");
        let prlock = Mutex::new(&mut pr);

        // If this iteration modified anything (early stop)
        let modified = AtomicUsize::new(0);
        let delta = Mutex::new(0.0);
        let pos = AtomicUsize::new(0);

        // in parallel run the computation
        thread_pool.scope(|scope| {
            for _ in 0..num_cpus {
                scope.spawn(|_s| {
                    let mut local_delta = 0.0;
                    let mut map = HashMap::new();
                    let mut rand = SmallRng::seed_from_u64(seed.fetch_add(1, Ordering::Relaxed));

                    loop {
                        let next_pos = pos.fetch_add(granularity, Ordering::Relaxed);
                        if next_pos >= num_nodes {
                            let mut delta = delta.lock().unwrap();
                            *delta += local_delta;
                            break;
                        }
                        let end_pos = (next_pos + granularity).min(perm.len());

                        let chunk = &perm[next_pos..end_pos];

                        for &node in chunk {
                            // if the node can't change we can skip it; if it can then
                            // we mark it as un-changeable for now and we'll unset later
                            // it if it can
                            if !can_change[node].fetch_and(false, Ordering::Relaxed) {
                                continue;
                            }

                            let successors = graph.successors(node);
                            if successors.len() == 0 {
                                continue;
                            }

                            // get the label of this node
                            let curr_label = label_store.label(node as _);
                            // get the count of how many times a
                            // label appears in the successors
                            map.clear();
                            for succ in successors {
                                map.entry(label_store.label(succ))
                                    .and_modify(|counter| *counter += 1)
                                    .or_insert(1);
                            }

                            let mut max = f64::MIN;
                            let mut old = 0.0;
                            let mut majorities = vec![];
                            // compute the most entropic label
                            for (&label, &count) in map.iter() {
                                let volume = label_store.volume(label);
                                let val =
                                    (1.0 + gamma) * count as f64 - gamma * (volume + 1) as f64;

                                if max == val {
                                    majorities.push(label);
                                }

                                if max < val {
                                    majorities.clear();
                                    max = val;
                                    majorities.push(label);
                                }

                                if label == curr_label {
                                    old = val;
                                }
                            }
                            // randomly break ties
                            let next_label = *majorities.choose(&mut rand).unwrap();
                            // if the label changed we need to update the label store
                            // and signal that this could change the neighbour nodes
                            if next_label != curr_label {
                                modified.fetch_add(1, Ordering::Relaxed);
                                for succ in graph.successors(node) {
                                    can_change[succ].store(true, Ordering::Relaxed);
                                }

                                label_store.set(node as _, next_label);
                            }

                            local_delta += max - old;
                        }
                        // update the progress logger with how many nodes we processed
                        prlock.lock().unwrap().update_with_count(perm.len());
                    }
                })
            }
        });

        pr.done_with_count(num_nodes as _);
        info!(
            "Modified: {} Delta: {}",
            modified.load(Ordering::Relaxed),
            delta.lock().unwrap()
        );
        glob_pr.update_and_display();
        if modified.load(Ordering::Relaxed) == 0 {
            break;
        }
    }

    glob_pr.done();

    // create sorted clusters by contiguous labels
    perm.par_sort_unstable_by(|&a, &b| label_store.label(a as _).cmp(&label_store.label(b as _)));

    let labels =
        unsafe { std::mem::transmute::<Box<[AtomicUsize]>, Box<[usize]>>(label_store.labels) };

    Ok(labels)
}

struct LabelStore {
    labels: Box<[AtomicUsize]>,
    volumes: Box<[AtomicUsize]>,
}

impl LabelStore {
    fn new(n: usize) -> Self {
        let mut labels = Vec::with_capacity(n);
        let mut volumes = Vec::with_capacity(n);
        for l in 0..n {
            labels.push(AtomicUsize::new(l));
            volumes.push(AtomicUsize::new(1));
        }
        Self {
            labels: labels.into_boxed_slice(),
            volumes: volumes.into_boxed_slice(),
        }
    }

    fn set(&self, node: usize, new_label: usize) {
        let old_label = self.labels[node].swap(new_label, Ordering::Relaxed);
        self.volumes[old_label].fetch_sub(1, Ordering::Relaxed);
        self.volumes[new_label].fetch_add(1, Ordering::Relaxed);
    }

    fn label(&self, node: usize) -> usize {
        self.labels[node].load(Ordering::Relaxed)
    }

    fn volume(&self, label: usize) -> usize {
        self.volumes[label].load(Ordering::Relaxed)
    }
}

unsafe impl Send for LabelStore {}
unsafe impl Sync for LabelStore {}
