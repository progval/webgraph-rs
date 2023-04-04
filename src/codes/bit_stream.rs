use super::WordReader;

/// A BitStream built uppon a generic [`WordReader`] that caches the read words 
/// in a buffer
pub struct BufferedBitStreamReader<WR: WordReader> {
    /// The backend that's used to read the words to fill the buffer
    backend: WR,
    /// The current cache of bits (at most 2 words) that's used to read the 
    /// codes
    buffer: u128,
    /// Number of bits valid left in the buffer
    valid_bits: u64,
}

impl<WR: WordReader> BufferedBitStreamReader<WR> {

    /// Create a new [`BufferedBitStreamReader`] on a generic backend
    pub fn new(backend: WR) -> Result<Self, WR::Error> {

        // TODO!: Should we do early filling? 
        // This would fail if the backend has only 64 bits which, while 
        // unlikely, it should be possible.
        // 
        // ```
        // let low_word = backend.read_next_word()? as u128;
        // let high_word = backend.read_next_word()? as u128;
        // let buffer = (high_word << 64) | low_word;
        // ```

        Ok(Self {
            backend,
            buffer: 0,
            valid_bits: 0,
        })
    }

    /// Ensure that in the buffer there are at least 64 bits to read
    fn refill(&mut self) -> Result<(), WR::Error> {
        // if we have 64 valid bits, we don't have space for a new word
        // and by definition we can only read
        if self.valid_bits > 64 {
            return Ok(());
        }

        // Read a new 64-bit word and put it in the buffer
        let new_word = self.backend.read_next_word()?;
        self.buffer |= (new_word as u128) << self.valid_bits;
        self.valid_bits += 64;
        
        Ok(())
    }

    /// Read `n_bits` from the buffer and return them in the lowest bits
    /// 
    pub fn read_bits(&mut self, n_bits: u64) -> Result<u64, WR::Error> {
        // TODO: should these be errors?
        debug_assert!(n_bits <= 64);
        debug_assert!(n_bits != 0);

        if n_bits > self.valid_bits {
            self.refill()?;
        }

        // this is equivalent to `(1 << n_bits) - 1` but it doesn't overflow
        // for n_bits = 64. The alternative is to use a wrapping_shl abd
        // wrapping_sub, but I find this more readable.
        let mask = u64::MAX >> (64 - n_bits);
        // read the `n_bits` lowest bits of the buffer
        let result = self.buffer as u64 & mask;

        // remove the read bits from the buffer
        self.valid_bits -= n_bits;
        self.buffer >>= n_bits;
        
        Ok(result)
    }

    /// Read an unary code
    pub fn read_unary(&mut self) -> Result<u64, WR::Error> {
        let mut result = 0;
        loop {
            // count the zeros from the left
            let zeros = self.buffer.leading_zeros() as u64;

            // if we encountered an 1 in the valid_bits we can return            
            if zeros < self.valid_bits {
                result += zeros;
                return Ok(result);
            }

            result += self.valid_bits;
            self.valid_bits = 0;
            
            // otherwise we didn't encounter the ending 1 yet so we need to 
            // refill and iter again
            self.refill()?;
        }
    }
}
