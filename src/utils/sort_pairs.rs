use crate::{traits::SortedIterator, utils::KAryHeap};
use anyhow::{anyhow, Context, Result};
use core::marker::PhantomData;
use dsi_bitstream::prelude::*;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// To be able to sort a payload, we must be able to write and read it back from
/// a bitstream
pub trait SortPairsPayload: Send + Copy {
    /// write self to the bitsream and return the number of bits written
    fn to_bitstream<E: Endianness, B: WriteCodes<E>>(&self, bitstream: &mut B) -> Result<usize>;
    /// deserialize Self from the bitstream and return its
    fn from_bitstream<E: Endianness, B: ReadCodes<E>>(bitstream: &mut B) -> Result<Self>;
}

impl SortPairsPayload for () {
    #[inline(always)]
    fn to_bitstream<E: Endianness, B: WriteCodes<E>>(&self, _bitstream: &mut B) -> Result<usize> {
        Ok(0)
    }
    #[inline(always)]
    fn from_bitstream<E: Endianness, B: ReadCodes<E>>(_bitstream: &mut B) -> Result<Self> {
        Ok(())
    }
}

/// A struct that ingests paris of nodes and a generic payload and sort them
/// in chunks of `batch_size` triples, then dumps them to disk.
pub struct SortPairs<T: SortPairsPayload = ()> {
    /// The batch size
    batch_size: usize,
    /// The length of the last batch might be smaller than `batch_size`
    last_batch_len: usize,
    /// The batch of triples we are currently building
    batch: Vec<(usize, usize, T)>,
    /// were we are going to store the tmp files
    dir: PathBuf,
    /// keep track of how many batches we created
    num_batches: usize,
}

impl<T: SortPairsPayload> core::ops::Drop for SortPairs<T> {
    fn drop(&mut self) {
        let _ = self.dump();
    }
}

impl<T: SortPairsPayload> SortPairs<T> {
    /// Create a new `SortPairs` with a given batch size
    ///
    /// The `dir` must be empty, and in particular it **must not** be shared
    /// with other `SortPairs` instances.
    pub fn new<P: AsRef<Path>>(batch_size: usize, dir: P) -> Result<Self> {
        let dir = dir.as_ref();
        let mut dir_entries =
            std::fs::read_dir(dir).with_context(|| format!("Could not list {}", dir.display()))?;
        if dir_entries.next().is_some() {
            Err(anyhow!("{} is not empty", dir.display()))
        } else {
            Ok(SortPairs {
                batch_size,
                last_batch_len: 0,
                batch: Vec::with_capacity(batch_size),
                dir: dir.to_owned(),
                num_batches: 0,
            })
        }
    }

    /// Add a triple to the graph.
    pub fn push(&mut self, x: usize, y: usize, t: T) -> Result<()> {
        self.batch.push((x, y, t));
        if self.batch.len() >= self.batch_size {
            self.dump()?;
        }
        Ok(())
    }

    /// Dump the current batch to disk
    fn dump(&mut self) -> Result<()> {
        // early exit
        if self.batch.is_empty() {
            return Ok(());
        }
        // sort ignoring the payload
        self.batch.par_sort_unstable_by_key(|(x, y, _)| (*x, *y));
        // create a batch file where to dump
        let batch_name = self.dir.join(format!("{:06x}", self.num_batches));
        let file = std::io::BufWriter::with_capacity(1 << 22, std::fs::File::create(&batch_name)?);
        // createa bitstream to write to the file
        let mut stream = <BufferedBitStreamWrite<LE, _>>::new(FileBackend::new(file));
        // Dump the triples to the bitstream
        let (mut prev_src, mut prev_dst) = (0, 0);
        for &(src, dst, payload) in &self.batch {
            // write the src gap as gamma
            stream.write_gamma((src - prev_src) as _)?;
            if src != prev_src {
                // Reset prev_y
                prev_dst = 0;
            }
            // write the dst gap as gamma
            stream.write_gamma((dst - prev_dst) as _)?;
            // write the payload
            payload.to_bitstream(&mut stream)?;
            (prev_src, prev_dst) = (src, dst);
        }
        // flush the stream and reset the buffer
        stream.flush()?;
        self.last_batch_len = self.batch.len();
        self.batch.clear();
        self.num_batches += 1;
        Ok(())
    }

    /// Cancel all the files that were created
    pub fn cancel_batches(&mut self) -> Result<()> {
        for i in 0..self.num_batches {
            let batch_name = self.dir.join(format!("{:06x}", i));
            // It's OK if something is not OK here
            std::fs::remove_file(batch_name)?;
        }
        self.num_batches = 0;
        self.last_batch_len = 0;
        self.batch.clear();
        Ok(())
    }

    pub fn iter(&mut self) -> Result<KMergeIters<T, BatchIterator<T>>> {
        self.dump()?;
        Ok(KMergeIters::new((0..self.num_batches).map(|batch_idx| {
            BatchIterator::new(
                self.dir.join(format!("{:06x}", batch_idx)),
                if batch_idx == self.num_batches - 1 {
                    self.last_batch_len
                } else {
                    self.batch_size
                },
            )
            .unwrap()
        })))
    }
}

/// An iterator that can read the batch files generated by [`SortPairs`] and
/// iterate over the triples
#[derive(Debug)]
pub struct BatchIterator<T: SortPairsPayload> {
    file_path: PathBuf,
    stream: BufferedBitStreamRead<LE, u64, FileBackend<u32, std::io::BufReader<std::fs::File>>>,
    len: usize,
    current: usize,
    prev_src: usize,
    prev_dst: usize,
    marker: PhantomData<T>,
}

impl<T: SortPairsPayload> BatchIterator<T> {
    pub fn new<P: AsRef<std::path::Path>>(file_path: P, len: usize) -> Result<Self> {
        let file_path = file_path.as_ref();
        let file = std::io::BufReader::new(
            std::fs::File::open(file_path)
                .with_context(|| format!("Cannot open batch {}", file_path.to_string_lossy()))?,
        );
        let stream = <BufferedBitStreamRead<LE, u64, _>>::new(FileBackend::new(file));
        Ok(BatchIterator {
            file_path: file_path.to_owned(),
            stream,
            len,
            current: 0,
            prev_src: 0,
            prev_dst: 0,
            marker: PhantomData,
        })
    }
}

impl<T: SortPairsPayload> Clone for BatchIterator<T> {
    fn clone(&self) -> Self {
        // we can't directly clone the stream, so we need to reopen the file
        // and seek to the same position
        let file = std::io::BufReader::new(std::fs::File::open(&self.file_path).unwrap());
        let mut stream = <BufferedBitStreamRead<LE, u64, _>>::new(FileBackend::new(file));
        stream.set_pos(self.stream.get_pos()).unwrap();
        assert_eq!(stream.get_pos(), self.stream.get_pos());
        BatchIterator {
            file_path: self.file_path.clone(),
            stream,
            len: self.len,
            current: self.current,
            prev_src: self.prev_src,
            prev_dst: self.prev_dst,
            marker: PhantomData,
        }
    }
}

unsafe impl<T: SortPairsPayload> SortedIterator for BatchIterator<T> {}

impl<T: SortPairsPayload> Iterator for BatchIterator<T> {
    type Item = (usize, usize, T);
    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.len {
            return None;
        }
        let src = self.prev_src + self.stream.read_gamma().unwrap() as usize;
        if src != self.prev_src {
            // Reset prev_y
            self.prev_dst = 0;
        }
        let dst = self.prev_dst + self.stream.read_gamma().unwrap() as usize;
        let payload = T::from_bitstream(&mut self.stream).unwrap();
        self.prev_src = src;
        self.prev_dst = dst;
        self.current += 1;
        Some((src, dst, payload))
    }
}

#[derive(Clone, Debug)]
/// Private struct that can be used to sort triples based only on the nodes and
/// ignoring the payload
struct HeadTail<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> {
    head: (usize, usize),
    payload: T,
    tail: I,
}

impl<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> PartialEq for HeadTail<T, I> {
    fn eq(&self, other: &Self) -> bool {
        self.head == other.head
    }
}
impl<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> PartialOrd
    for HeadTail<T, I>
{
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.head.cmp(&other.head))
    }
}

#[derive(Clone, Debug)]
/// Merge K different sorted iterators
pub struct KMergeIters<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> {
    heap: KAryHeap<HeadTail<T, I>>,
}

impl<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> KMergeIters<T, I> {
    pub fn new(iters: impl Iterator<Item = I>) -> Self {
        let mut heap = KAryHeap::with_capacity(iters.size_hint().1.unwrap_or(10));
        for mut iter in iters {
            match iter.next() {
                None => {}
                Some((src, dst, payload)) => {
                    heap.push(HeadTail {
                        head: (src, dst),
                        payload,
                        tail: iter,
                    });
                }
            }
        }
        KMergeIters { heap }
    }
}

impl<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> Iterator
    for KMergeIters<T, I>
{
    type Item = (usize, usize, T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.heap.is_empty() {
            return None;
        }
        // Read the head of the heap
        let head_tail = self.heap.peek_mut();
        let (src, dst) = head_tail.head;
        let result = (src, dst, head_tail.payload);
        match head_tail.tail.next() {
            None => {
                // Remove the head of the heap if the iterator ended
                self.heap.pop();
            }
            Some((src, dst, payload)) => {
                // set the new values
                head_tail.head = (src, dst);
                head_tail.payload = payload;
                // fix the heap
                self.heap.bubble_down(0);
            }
        }
        Some(result)
    }
}

unsafe impl<T: Copy, I: Iterator<Item = (usize, usize, T)> + SortedIterator> SortedIterator
    for KMergeIters<T, I>
{
}

#[cfg(test)]
#[test]
pub fn test_push() -> Result<()> {
    #[derive(Clone, Copy, Debug)]
    struct MySortPairsPayload(usize);
    impl SortPairsPayload for MySortPairsPayload {
        fn from_bitstream<E: Endianness, B: ReadCodes<E>>(bitstream: &mut B) -> Result<Self> {
            bitstream
                .read_delta()
                .map(|x| MySortPairsPayload(x as usize))
        }
        fn to_bitstream<E: Endianness, B: WriteCodes<E>>(
            &self,
            bitstream: &mut B,
        ) -> Result<usize> {
            bitstream.write_delta(self.0 as u64)
        }
    }
    let dir = tempfile::tempdir()?;
    let mut sp = SortPairs::new(10, dir.into_path())?;
    let n = 25;
    for i in 0..n {
        sp.push(i, i + 1, MySortPairsPayload(i + 2))?;
    }
    let mut iter = sp.iter()?;
    let mut cloned = iter.clone();

    for _ in 0..n {
        let (x, y, p) = iter.next().unwrap();
        println!("{} {} {}", x, y, p.0);
        assert_eq!(x + 1, y);
        assert_eq!(x + 2, p.0);
    }

    for _ in 0..n {
        let (x, y, p) = cloned.next().unwrap();
        println!("{} {} {}", x, y, p.0);
        assert_eq!(x + 1, y);
        assert_eq!(x + 2, p.0);
    }
    Ok(())
}
