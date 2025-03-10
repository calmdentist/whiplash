use anchor_lang::prelude::*;

declare_id!("9PNrWgYx3Gh8NqQyNUkUEamwy8T2gGJLbKbD4EotTn17");

#[program]
pub mod whiplash {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
