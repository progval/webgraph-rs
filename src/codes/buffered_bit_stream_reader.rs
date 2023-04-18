use super::{
    WordRead, WordStream,
    BitSeek, BitRead,
    BitOrder, M2L, L2M,
    unary_tables,
};
//use crate::utils::get_lowest_bits;
use crate::{Word, CastableInto};
use anyhow::{Result, bail, Context};

/// A BitStream built uppon a generic [`WordRead`] that caches the read words 
/// in a buffer
pub struct BufferedBitStreamRead<E: BitOrder, BW: Word, WR: WordRead> {
    /// The backend that's used to read the words to fill the buffer
    backend: WR,
    /// The current cache of bits (at most 2 words) that's used to read the 
    /// codes. The bits are read FROM MSB TO LSB
    buffer:BW,
    /// Number of bits valid left in the buffer
    valid_bits: usize,
    /// Just needed to specify the BitOrder
    _marker: core::marker::PhantomData<E>,
}

impl<E: BitOrder, BW: Word, WR: WordRead> BufferedBitStreamRead<E, BW, WR> {
    /// Create a new [`BufferedBitStreamRead`] on a generic backend
    /// 
    /// ### Example
    /// ```
    /// use webgraph::codes::*;
    /// use webgraph::utils::*;
    /// let words = [0x0043b59fccf16077];
    /// let word_reader = MemWordRead::new(&words);
    /// let mut bitstream = <BufferedBitStreamRead<M2L, _>>::new(word_reader);
    /// ```
    #[must_use]
    pub fn new(backend: WR) -> Self {
        Self {
            backend,
            buffer: BW::ZERO,
            valid_bits: 0,
            _marker: core::marker::PhantomData::default(),
        }
    }
}

impl<BW: Word, WR: WordRead> BufferedBitStreamRead<M2L, BW, WR>
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW>,
{
    /// Ensure that in the buffer there are at least 64 bits to read
    #[inline(always)]
    fn refill(&mut self) -> Result<()> {
        // if we have 64 valid bits, we don't have space for a new word
        // and by definition we can only read
        if self.valid_bits > WR::Word::BITS {
            return Ok(());
        }
        // TODO!:
        // Read a new 64-bit word and put it in the buffer
        let new_word = self.backend.read_next_word()
            .with_context(|| "Error while reflling BufferedBitStreamRead")?.to_be();
        self.valid_bits += WR::Word::BITS;
        self.buffer |= new_word.cast() << (BW::BITS - self.valid_bits).cast();
        Ok(())
    }
}

impl<BW: Word, WR: WordRead + WordStream> BitSeek 
    for BufferedBitStreamRead<M2L, BW, WR> 
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW>,
{
    #[inline]
    fn get_position(&self) -> usize {
        self.backend.get_position() * WR::Word::BITS - self.valid_bits as usize
    }

    #[inline]
    fn seek_bit(&mut self, bit_index: usize) -> Result<()> {
        self.backend.set_position(bit_index / WR::Word::BITS)
            .with_context(|| "BufferedBitStreamRead was seeking_bit")?;
        let bit_offset = bit_index % WR::Word::BITS;
        self.buffer = BW::ZERO;
        self.valid_bits = 0;
        if bit_offset != 0 {
            let new_word: BW = self.backend.read_next_word()?.to_be().cast();
            self.valid_bits = WR::Word::BITS - bit_offset;
            self.buffer = new_word << (BW::BITS - self.valid_bits).cast();
        }
        Ok(())
    }
}

impl<BW: Word, WR: WordRead> BitRead<M2L> 
    for BufferedBitStreamRead<M2L, BW, WR> 
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW> + CastableInto<u64>,
    u64: CastableInto<BW> + CastableInto<WR::Word>,
{
    type PeekType = WR::Word;

    #[inline]
    fn skip_bits(&mut self, mut n_bits: usize) -> Result<()> {
        // happy case, just shift the buffer
        if n_bits as usize <= self.valid_bits {
            self.valid_bits -= n_bits as usize;
            self.buffer <<= n_bits.cast();
            return Ok(());
        }

        // clean the buffer data
        n_bits -= self.valid_bits;
        self.valid_bits = 0;
        // skip words as needed
        while n_bits > WR::Word::BITS {
            let _ = self.backend.read_next_word()?;
            n_bits -= WR::Word::BITS;
        }
        // read the new word and clear the final bits
        self.refill()?;
        self.valid_bits -= n_bits;
        self.buffer <<= n_bits.cast();

        Ok(())
    }

    #[inline]
    fn read_bits(&mut self, mut n_bits: usize) -> Result<u64> {
        if n_bits > 64 {
            bail!("The n of bits to peek has to be in [0, 64] and {} is not.", n_bits);
        }
        if n_bits == 0 {
            return Ok(0);
        }

        // most common path, we just read the buffer        
        if n_bits < self.valid_bits {
            let result: u64 = (
                self.buffer >> (BW::BITS - n_bits).cast()
            ).cast();
            self.valid_bits -= n_bits as usize;
            self.buffer <<= n_bits.cast();
            return Ok(result);
        }

        let mut result: u64 = (
            self.buffer >> (BW::BITS - self.valid_bits).cast()
        ).cast();

        // Directly read to the result without updating the buffer
        while n_bits as usize > WR::Word::BITS {
            let new_word: u64 = self.backend.read_next_word()?.to_be().cast();
            result = (result << WR::Word::BITS) | new_word;
            n_bits -= WR::Word::BITS;
        }

        // get the final word
        let new_word: BW = self.backend.read_next_word()?.to_be().cast();
        self.valid_bits = WR::Word::BITS - n_bits;
        // compose the remaining bits
        let final_bits: u64 = (new_word >> (BW::BITS - n_bits).cast()).cast();
        result = (result << n_bits) | final_bits;
        // and put the rest in the buffer
        self.buffer = new_word << (BW::BITS - self.valid_bits).cast();

        Ok(result)
    }

    #[inline]
    fn peek_bits(&mut self, n_bits: usize) -> Result<Self::PeekType> {
        if n_bits > WR::Word::BITS {
            bail!("The n of bits to peek has to be in [0, {}] and {} is not.", WR::Word::BITS, n_bits);
        }
        if n_bits == 0 {
            return Ok(WR::Word::ZERO);
        }
        // a peek can do at most one refill, otherwise we might loose data
        if n_bits as usize > self.valid_bits {
            self.refill()?;  
        }

        // read the `n_bits` highest bits of the buffer and shift them to
        // be the lowest
        Ok((
            self.buffer >> (BW::BITS - n_bits).cast()
        ).cast())
    }

    #[inline]
    fn read_unary<const USE_TABLE: bool>(&mut self) -> Result<u64> {
        if USE_TABLE {
            if let Some(res) = unary_tables::read_table_m2l(self)? {
                return Ok(res)
            }
        }
        let mut result: u64 = 0;
        loop {
            // count the zeros from the left
            let zeros: usize = self.buffer.leading_zeros().cast();

            // if we encountered an 1 in the valid_bits we can return            
            if zeros < self.valid_bits {
                result += zeros as u64;
                self.buffer <<= (zeros + 1).cast();
                self.valid_bits -= zeros + 1;
                return Ok(result);
            }

            result += self.valid_bits as u64;
            
            // otherwise we didn't encounter the ending 1 yet so we need to 
            // refill and iter again
            let new_word: BW = self.backend.read_next_word()?.to_be().cast();
            self.valid_bits = WR::Word::BITS;
            self.buffer = new_word << (BW::BITS - WR::Word::BITS).cast();
        }
    }
}


impl<BW: Word, WR: WordRead> BufferedBitStreamRead<L2M, BW, WR>
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW>,
{
    /// Ensure that in the buffer there are at least 64 bits to read
    #[inline(always)]
    fn refill(&mut self) -> Result<()> {
        // if we have 64 valid bits, we don't have space for a new word
        // and by definition we can only read
        if self.valid_bits > WR::Word::BITS {
            return Ok(());
        }
        // TODO!:
        // Read a new 64-bit word and put it in the buffer
        let new_word = self.backend.read_next_word()
            .with_context(|| "Error while reflling BufferedBitStreamRead")?.to_le();
        self.buffer |= new_word.cast() << self.valid_bits.cast();
        self.valid_bits += WR::Word::BITS;
        Ok(())
    }
}

impl<BW: Word, WR: WordRead + WordStream> BitSeek 
    for BufferedBitStreamRead<L2M, BW, WR> 
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW>,
{
    #[inline]
    fn get_position(&self) -> usize {
        self.backend.get_position() * WR::Word::BITS - self.valid_bits as usize
    }

    #[inline]
    fn seek_bit(&mut self, bit_index: usize) -> Result<()> {
        self.backend.set_position(bit_index / WR::Word::BITS)
            .with_context(|| "BufferedBitStreamRead was seeking_bit")?;
        let bit_offset = bit_index % WR::Word::BITS;
        self.buffer = BW::ZERO;
        self.valid_bits = 0;
        if bit_offset != 0 {
            let new_word: BW = self.backend.read_next_word()?.to_le().cast();
            self.valid_bits = WR::Word::BITS - bit_offset;
            self.buffer = new_word >> self.valid_bits.cast();
        }
        Ok(())
    }
}

impl<BW: Word, WR: WordRead> BitRead<L2M> 
    for BufferedBitStreamRead<L2M, BW, WR> 
where
    BW: CastableInto<WR::Word>,
    WR::Word: CastableInto<BW> + CastableInto<u64>,
    u64: CastableInto<BW> + CastableInto<WR::Word>,
{
    type PeekType = WR::Word;

    #[inline]
    fn skip_bits(&mut self, mut n_bits: usize) -> Result<()> {
        // happy case, just shift the buffer
        if n_bits as usize <= self.valid_bits {
            self.valid_bits -= n_bits as usize;
            self.buffer >>= n_bits.cast();
            return Ok(());
        }

        // clean the buffer data
        n_bits -= self.valid_bits;
        self.valid_bits = 0;
        // skip words as needed
        while n_bits > WR::Word::BITS {
            let _ = self.backend.read_next_word()?;
            n_bits -= WR::Word::BITS;
        }
        // read the new word and clear the final bits
        self.refill()?;
        self.valid_bits -= n_bits;
        self.buffer >>= n_bits.cast();

        Ok(())
    }

    #[inline]
    fn read_bits(&mut self, mut n_bits: usize) -> Result<u64> {
        if n_bits > 64 {
            bail!("The n of bits to peek has to be in [0, 64] and {} is not.", n_bits);
        }
        if n_bits == 0 {
            return Ok(0);
        }

        // most common path, we just read the buffer        
        if n_bits < self.valid_bits {
            let shamt = (BW::BITS - n_bits).cast();
            let result: u64 = ((self.buffer << shamt) >> shamt).cast(); 
            self.valid_bits -= n_bits as usize;
            self.buffer >>= n_bits.cast();
            return Ok(result);
        }

        let mut result: u64 = self.buffer.cast();

        // Directly read to the result without updating the buffer
        while n_bits as usize > WR::Word::BITS {
            let new_word: u64 = self.backend.read_next_word()?.to_le().cast();
            result = (result << WR::Word::BITS) | new_word;
            n_bits -= WR::Word::BITS;
        }

        // get the final word
        let new_word: BW = self.backend.read_next_word()?.to_le().cast();
        self.valid_bits = WR::Word::BITS - n_bits;
        // compose the remaining bits
        let shamt = (BW::BITS - n_bits).cast();
        let final_bits: u64 = ((new_word << shamt) >> shamt).cast();
        result = (result << n_bits) | final_bits;
        // and put the rest in the buffer
        self.buffer = new_word >> n_bits.cast();

        Ok(result)
    }

    #[inline]
    fn peek_bits(&mut self, n_bits: usize) -> Result<Self::PeekType> {
        if n_bits > WR::Word::BITS {
            bail!("The n of bits to peek has to be in [0, {}] and {} is not.", WR::Word::BITS, n_bits);
        }
        if n_bits == 0 {
            return Ok(WR::Word::ZERO);
        }
        // a peek can do at most one refill, otherwise we might loose data
        if n_bits as usize > self.valid_bits {
            self.refill()?;  
        }

        // read the `n_bits` highest bits of the buffer and shift them to
        // be the lowest
        let shamt =  (BW::BITS - n_bits).cast();
        Ok(((self.buffer << shamt) >> shamt).cast())
    }

    #[inline]
    fn read_unary<const USE_TABLE: bool>(&mut self) -> Result<u64> {
        if USE_TABLE {
            if let Some(res) = unary_tables::read_table_l2m(self)? {
                return Ok(res)
            }
        }
        let mut result: u64 = 0;
        loop {
            // count the zeros from the left
            let zeros: usize = self.buffer.trailing_zeros().cast();

            // if we encountered an 1 in the valid_bits we can return            
            if zeros < self.valid_bits {
                result += zeros as u64;
                self.buffer >>= (zeros + 1).cast();
                self.valid_bits -= zeros + 1;
                return Ok(result);
            }

            result += self.valid_bits as u64;
            
            // otherwise we didn't encounter the ending 1 yet so we need to 
            // refill and iter again
            let new_word: BW = self.backend.read_next_word()?.to_le().cast();
            self.valid_bits = WR::Word::BITS;
            self.buffer = new_word;
        }
    }
}