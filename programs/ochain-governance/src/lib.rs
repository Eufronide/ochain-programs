use anchor_lang::prelude::*;

declare_id!("11111111111111111111111111111116");

#[program]
pub mod ochain_governance {
    use super::*;

    // TODO: implement governance (proposals, voting, execution of slashes / parameter changes)
    pub fn placeholder(_ctx: Context<Placeholder>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Placeholder<'info> {
    pub authority: Signer<'info>,
}
