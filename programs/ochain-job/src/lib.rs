use anchor_lang::prelude::*;

declare_id!("11111111111111111111111111111114");

#[program]
pub mod ochain_job {
    use super::*;

    // TODO: implement job lifecycle (post_job, claim_job, submit_result, slash_timeout)
    pub fn placeholder(_ctx: Context<Placeholder>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Placeholder<'info> {
    pub authority: Signer<'info>,
}
