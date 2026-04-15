use anchor_lang::prelude::*;

declare_id!("11111111111111111111111111111115");

#[program]
pub mod ochain_identity {
    use super::*;

    // TODO: implement DID-style identity anchoring for TEE operators
    pub fn placeholder(_ctx: Context<Placeholder>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Placeholder<'info> {
    pub authority: Signer<'info>,
}
