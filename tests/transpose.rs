/*
 * SPDX-FileCopyrightText: 2023 Inria
 * SPDX-FileCopyrightText: 2023 Sebastiano Vigna
 *
 * SPDX-License-Identifier: Apache-2.0 OR LGPL-2.1-or-later
 */

use anyhow::Result;
use webgraph::graph::arc_list_graph;
use webgraph::prelude::*;
#[test]
fn test_transpose() -> Result<()> {
    const TRANSPOSED_PATH: &str = "tests/data/cnr-2000-transposed";
    const RE_TRANSPOSED_PATH: &str = "tests/data/cnr-2000-transposed-transposed";
    const BATCH_SIZE: usize = 100_000;

    stderrlog::new()
        .verbosity(2)
        .timestamp(stderrlog::Timestamp::Second)
        .init()
        .unwrap();

    let compression_flags = CompFlags::default();

    // load cnr-2000
    let graph = webgraph::graph::bvgraph::load("tests/data/cnr-2000")?;
    let num_nodes = graph.num_nodes();
    // transpose and par compress]
    let transposed = webgraph::algorithms::transpose(&graph, BATCH_SIZE)?;

    parallel_compress_sequential_iter::<
        arc_list_graph::NodeIterator<
            std::iter::Map<KMergeIters<BatchIterator>, fn((usize, usize, ())) -> (usize, usize)>,
        >,
    >(
        TRANSPOSED_PATH,
        transposed.iter(),
        transposed.num_nodes(),
        compression_flags,
        rayon::current_num_threads(),
    )?;
    // check it
    // TODO assert_eq!(transposed.iter_nodes().len(), num_nodes);
    let transposed_graph = webgraph::graph::bvgraph::load_seq(TRANSPOSED_PATH)?;
    assert_eq!(transposed_graph.num_nodes(), num_nodes);

    log::info!("Checking that the transposed graph is correct...");
    let mut iter = transposed_graph.iter();
    while let Some((node, succ)) = iter.next() {
        for succ_node in succ {
            assert!(graph.has_arc(succ_node, node));
        }
    }
    // re-transpose and par-compress
    let retransposed = webgraph::algorithms::transpose(&transposed_graph, BATCH_SIZE)?;

    parallel_compress_sequential_iter::<
        arc_list_graph::NodeIterator<
            std::iter::Map<
                KMergeIters<BatchIterator<DummyBitSerDes>, ()>,
                fn((usize, usize, ())) -> (usize, usize),
            >,
        >,
    >(
        RE_TRANSPOSED_PATH,
        retransposed.iter(),
        retransposed.num_nodes(),
        compression_flags,
        rayon::current_num_threads(),
    )?;
    // check it
    // TODO assert_eq!(retransposed.iter_nodes().len(), num_nodes);
    let retransposed_graph = webgraph::graph::bvgraph::load_seq(RE_TRANSPOSED_PATH)?;
    assert_eq!(retransposed_graph.num_nodes(), num_nodes);

    log::info!("Checking that the re-transposed graph is as the original one...");
    let mut true_iter = graph.iter();
    let mut retransposed_iter = retransposed_graph.iter();
    for i in 0..num_nodes {
        let (node, true_succ) = true_iter.next().unwrap();
        let (retransposed_node, retransposed_succ) = retransposed_iter.next().unwrap();
        assert_eq!(node, i);
        assert_eq!(node, retransposed_node);
        assert_eq!(
            true_succ.collect::<Vec<_>>(),
            retransposed_succ.collect::<Vec<_>>(),
            "The first differing node is: {}",
            i,
        );
    }

    std::fs::remove_file(format!("{}.graph", TRANSPOSED_PATH))?;
    std::fs::remove_file(format!("{}.properties", TRANSPOSED_PATH))?;
    std::fs::remove_file(format!("{}.graph", RE_TRANSPOSED_PATH))?;
    std::fs::remove_file(format!("{}.properties", RE_TRANSPOSED_PATH))?;
    Ok(())
}
