use anchor_lang::prelude::*;
use crate::error::SrAmmError;

pub const TICK_SPACING: i32 = 100; // Can be adjusted based on desired granularity
pub const MIN_TICK: i32 = -887272;
pub const MAX_TICK: i32 = 887272;
pub const BITMAP_WORD_SIZE: usize = 128; // Using u128 instead of u256

// New structure for per-tick data.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct TickDataEntry {
    pub tick: i32,
    pub init_timestamp: u64,
    pub borrowed_amount: u128,
}

#[account]
pub struct TickBitmap {
    pub pool: Pubkey,
    // Each word in the bitmap represents 128 ticks using u128.
    pub bitmap: Vec<u128>,
    // Map each active tick to extra data.
    pub tick_data: Vec<TickDataEntry>,
}

impl TickBitmap {
    // Convert price to tick index.
    pub fn price_to_tick(price: u128) -> Result<i32> {
        let tick = ((price as f64).ln() / 0.0001_f64.ln()) as i32;
        if tick < MIN_TICK || tick > MAX_TICK {
            return Err(SrAmmError::PriceOutOfBounds.into());
        }
        Ok(tick - (tick % TICK_SPACING))
    }

    // Convert tick index to price.
    pub fn tick_to_price(tick: i32) -> Result<u128> {
        if tick < MIN_TICK || tick > MAX_TICK {
            return Err(SrAmmError::PriceOutOfBounds.into());
        }
        let price = (1.0001_f64.powi(tick) * (1u128 << 64) as f64) as u128;
        Ok(price)
    }

    // Set a tick flag to a known value (true to set, false to clear).
    pub fn set_tick(&mut self, tick: i32, value: bool) -> Result<()> {
        let (word_pos, bit_pos) = self.position(tick)?;
        while self.bitmap.len() <= word_pos {
            self.bitmap.push(0);
        }
        if value {
            self.bitmap[word_pos] |= 1u128 << bit_pos;
        } else {
            self.bitmap[word_pos] &= !(1u128 << bit_pos);
        }
        Ok(())
    }

    // Existing function to flip the tick (toggle the bit).
    pub fn flip_tick(&mut self, tick: i32) -> Result<()> {
        let (word_pos, bit_pos) = self.position(tick)?;
        while self.bitmap.len() <= word_pos {
            self.bitmap.push(0);
        }
        self.bitmap[word_pos] ^= 1u128 << bit_pos;
        Ok(())
    }

    // Initialize a tick—set it active and record extra data.
    // If a tick already exists, add the new borrowed_amount.
    pub fn initialize_tick(&mut self, tick: i32, init_timestamp: u64, borrowed_amount: u128) -> Result<()> {
        // Ensure the tick is active.
        self.set_tick(tick, true)?;
        if let Some(entry) = self.tick_data.iter_mut().find(|entry| entry.tick == tick) {
            entry.borrowed_amount = entry.borrowed_amount
                .checked_add(borrowed_amount)
                .ok_or(SrAmmError::MathError)?;
        } else {
            self.tick_data.push(TickDataEntry {
                tick,
                init_timestamp,
                borrowed_amount,
            });
        }
        Ok(())
    }

    // Check if a tick is initialized.
    pub fn is_initialized(&self, tick: i32) -> Result<bool> {
        let (word_pos, bit_pos) = self.position(tick)?;
        if word_pos >= self.bitmap.len() {
            return Ok(false);
        }
        Ok((self.bitmap[word_pos] & (1u128 << bit_pos)) != 0)
    }

    // Get position in bitmap for a tick.
    fn position(&self, tick: i32) -> Result<(usize, u8)> {
        if tick < MIN_TICK || tick > MAX_TICK {
            return Err(SrAmmError::PriceOutOfBounds.into());
        }
        let tick = tick / TICK_SPACING;
        let word_pos = ((tick + MAX_TICK) / BITMAP_WORD_SIZE as i32) as usize;
        let bit_pos = ((tick + MAX_TICK) % BITMAP_WORD_SIZE as i32) as u8;
        Ok((word_pos, bit_pos))
    }

    // Find the next initialized tick in the given direction.
    pub fn next_initialized_tick(&self, tick: i32, positive: bool) -> Result<Option<i32>> {
        let (mut word_pos, bit_pos) = self.position(tick)?;
        if word_pos >= self.bitmap.len() {
            return Ok(None);
        }
        let mut current_word = self.bitmap[word_pos];
        if positive {
            current_word &= !((1u128 << bit_pos) - 1);
        } else {
            current_word &= (1u128 << bit_pos) - 1;
        }
        while current_word == 0 && word_pos < self.bitmap.len() {
            word_pos = if positive {
                word_pos + 1
            } else {
                word_pos.checked_sub(1).ok_or(SrAmmError::MathError)?
            };
            if word_pos >= self.bitmap.len() {
                return Ok(None);
            }
            current_word = self.bitmap[word_pos];
        }
        if current_word == 0 {
            return Ok(None);
        }
        let next_bit = if positive {
            current_word.trailing_zeros() as i32
        } else {
            (BITMAP_WORD_SIZE as i32 - 1) - current_word.leading_zeros() as i32
        };
        let tick = (word_pos as i32 * BITMAP_WORD_SIZE as i32 + next_bit - MAX_TICK) * TICK_SPACING;
        Ok(Some(tick))
    }
}