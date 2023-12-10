use anchor_lang::AnchorDeserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use anyhow::{Result, anyhow, Context};


#[derive(Debug, Clone)]
pub struct JLPCacheAccounts {
    pub pool: Pubkey,
    pub custody_accounts: Vec<JLPCustodyAccount>,
    pub transfer_authority: Pubkey,
    pub event_authority: Pubkey,
}

#[derive(Debug, Clone)]
pub struct JLPCustodyAccount {
    // the custody account
    pub account: Pubkey,
    pub mint: Pubkey,
    pub token_account: Pubkey,
    pub oracle_account: Pubkey,
}


impl JLPCacheAccounts {
    /// loads all the account information that we need for JLP deposits
    pub async fn load_accounts(rpc: &RpcClient, pool: Pubkey) -> Result<JLPCacheAccounts> {
        let acct_data = rpc.get_account_data(&pool).await?;
        let pool_acct = crate::Pool::deserialize(&mut &acct_data[8..])?;
        let mut custody_accounts = Vec::with_capacity(pool_acct.custodies.len());
        for custody in pool_acct.custodies {
            let acct_data = rpc.get_account_data(&custody).await?;
            let custody_acct = crate::Custody::deserialize(&mut &acct_data[8..])?;
            custody_accounts.push(JLPCustodyAccount {
                account: custody,
                mint: custody_acct.mint,
                token_account: custody_acct.token_account,
                oracle_account: custody_acct.oracle.oracle_account
            });
        }

        Ok(Self {
            pool: pool,
            custody_accounts,
            transfer_authority: Pubkey::find_program_address(
                &[&b"transfer_authority"[..]], &crate::id()
            ).0,
            event_authority: Pubkey::find_program_address(
                &[&b"__event_authority"[..]], &crate::id()
            ).0
        })
    }
    pub async fn load_pool(&self, rpc: &RpcClient) -> Result<crate::Pool> {
        let acct_data = rpc.get_account_data(&self.pool).await?;
        let acct = crate::Pool::deserialize(&mut &acct_data[..])?;
        Ok(acct)
    }
}