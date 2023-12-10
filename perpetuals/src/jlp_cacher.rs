use anchor_lang::{AnchorDeserialize, ToAccountMetas, InstructionData};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, instruction::{Instruction, AccountMeta}};
use anyhow::{Result, anyhow, Context};

const LP_TOKEN_MINT: Pubkey = solana_sdk::pubkey!("27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4");

#[derive(Debug, Clone)]
pub struct JLPCacheAccounts {
    pub pool: Pubkey,
    pub perp: Pubkey,
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
    pub async fn create_lp_token_ata_ix(&self, rpc: &RpcClient ,owner: Pubkey) -> Option<Instruction> {
        if rpc.get_account_data(
            &spl_associated_token_account::get_associated_token_address(
                &owner,
                &LP_TOKEN_MINT
            )
        ).await.is_ok() {
            return None;
        }
        Some(spl_associated_token_account::instruction::create_associated_token_account(
            &owner,
            &owner,
            &LP_TOKEN_MINT,
            &spl_token::id()
        ))
    }
    pub fn custody_account_for_mint(&self, mint: Pubkey) -> Option<JLPCustodyAccount> {
        for custody in &self.custody_accounts {
            if custody.mint.eq(&mint) {
                return Some(custody.clone());
            }
        }
        None
    }
    /// loads all the account information that we need for JLP deposits
    pub async fn load_accounts(rpc: &RpcClient, perp: Pubkey, pool: Pubkey) -> Result<JLPCacheAccounts> {
        let acct_data = rpc.get_account_data(&pool).await?;
        let pool_acct = crate::Pool::deserialize(&mut &acct_data[8..])?;
        let acct_data = rpc.get_account_data(&perp).await?;
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
            perp: perp,
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
        let acct = crate::Pool::deserialize(&mut &acct_data[8..])?;
        Ok(acct)
    }
    pub fn generate_liquidity_add_ix(&self, deposit_mint: Pubkey, owner: Pubkey, deposit_amount: u64) -> Result<Instruction> {
        let custody_info = self.custody_account_for_mint(deposit_mint).with_context(|| "no custody account for mint")?;
        let ix_data = crate::instruction::AddLiquidity {
            _params: crate::AddLiquidityParams {
                token_amount_in: deposit_amount,
                min_lp_amount_out: 1,
                token_amount_pre_swap: None
            }
        };
        let mut ix_accounts = crate::accounts::AddLiquidity {
            owner,
            funding_account: spl_associated_token_account::get_associated_token_address(
                &owner,
                &deposit_mint
            ),
            lp_token_account: spl_associated_token_account::get_associated_token_address(
                &owner,
                &LP_TOKEN_MINT
            ),
            transfer_authority: self.transfer_authority,
            perpetuals: self.perp,
            pool: self.pool,
            custody: custody_info.account,
            custody_oracle_account: custody_info.oracle_account,
            custody_token_account: custody_info.token_account,
            lp_token_mint: LP_TOKEN_MINT,
            token_program: spl_token::id(),
            event_authority: self.event_authority,
            program: crate::id()
        }.to_account_metas(None);

        for custody in &self.custody_accounts {
            if custody.mint.eq(&deposit_mint) {
                ix_accounts.push(AccountMeta::new(custody.account, false));
            } else {
                ix_accounts.push(AccountMeta::new_readonly(custody.account, false));
            }
        }
        for custody in &self.custody_accounts {
            ix_accounts.push(AccountMeta::new_readonly(custody.oracle_account, false));
        }

        Ok(Instruction {
            program_id: crate::id(),
            accounts: ix_accounts,
            data: ix_data.data()
        })
    }
}