use anchor_lang::{AnchorDeserialize, InstructionData, ToAccountMetas};
use anyhow::{anyhow, Context, Result};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    program_pack::Pack,
    pubkey::Pubkey,
};

pub const LP_TOKEN_MINT: Pubkey = solana_sdk::pubkey!("27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4");
pub const USDC_TOKEN_MINT: Pubkey = solana_sdk::pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

#[derive(Debug, Clone)]
pub struct JLPCacheAccountKeys {
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

#[derive(Clone)]
pub struct JLPCacheAccounts {
    pub token_mint: spl_token::state::Mint,
    pub pool: crate::Pool,
    pub usdc_token_account: spl_token::state::Account,
}

impl JLPCacheAccountKeys {
    pub async fn create_lp_token_ata_ix(
        &self,
        rpc: &RpcClient,
        owner: Pubkey,
    ) -> Option<Instruction> {
        if rpc
            .get_account_data(&spl_associated_token_account::get_associated_token_address(
                &owner,
                &LP_TOKEN_MINT,
            ))
            .await
            .is_ok()
        {
            return None;
        }
        Some(
            spl_associated_token_account::instruction::create_associated_token_account(
                &owner,
                &owner,
                &LP_TOKEN_MINT,
                &spl_token::id(),
            ),
        )
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
    pub async fn load_account_keys(
        rpc: &RpcClient,
        perp: Pubkey,
        pool: Pubkey,
    ) -> Result<JLPCacheAccountKeys> {
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
                oracle_account: custody_acct.oracle.oracle_account,
            });
        }

        Ok(Self {
            pool: pool,
            perp: perp,
            custody_accounts,
            transfer_authority: Pubkey::find_program_address(
                &[&b"transfer_authority"[..]],
                &crate::id(),
            )
            .0,
            event_authority: Pubkey::find_program_address(
                &[&b"__event_authority"[..]],
                &crate::id(),
            )
            .0,
        })
    }
    pub async fn load_accounts(&self, rpc: &RpcClient, usdc_ata: Pubkey) -> Result<JLPCacheAccounts> {
        let mut accounts = rpc
            .get_multiple_accounts(&[self.pool, LP_TOKEN_MINT, usdc_ata])
            .await?;
        let pool_acct = match std::mem::take(&mut accounts[0]) {
            Some(pool_account) => crate::Pool::deserialize(&mut &pool_account.data[8..])?,
            None => return Err(anyhow!("failed to get pool account")),
        };
        let lp_mint = match std::mem::take(&mut accounts[1]) {
            Some(lp_account) => spl_token::state::Mint::unpack(&lp_account.data[..])?,
            None => return Err(anyhow!("failed to get mint account")),
        };
        let usdc_ata = match std::mem::take(&mut accounts[2]) {
            Some(usdc_ata) => {
                spl_token::state::Account::unpack(&usdc_ata.data[..])?
            }
            None => return Err(anyhow!("failed to get usdc account"))
        };
        Ok(JLPCacheAccounts {
            token_mint: lp_mint,
            pool: pool_acct,
            usdc_token_account: usdc_ata,
        })
    }
    pub fn generate_liquidity_add_ix(
        &self,
        deposit_mint: Pubkey,
        owner: Pubkey,
        deposit_amount: u64,
        min_out: u64,
    ) -> Result<Instruction> {
        let custody_info = self
            .custody_account_for_mint(deposit_mint)
            .with_context(|| "no custody account for mint")?;
        let ix_data = crate::instruction::AddLiquidity {
            _params: crate::AddLiquidityParams {
                token_amount_in: deposit_amount,
                min_lp_amount_out: min_out,
                token_amount_pre_swap: None,
            },
        };
        let mut ix_accounts = crate::accounts::AddLiquidity {
            owner,
            funding_account: spl_associated_token_account::get_associated_token_address(
                &owner,
                &deposit_mint,
            ),
            lp_token_account: spl_associated_token_account::get_associated_token_address(
                &owner,
                &LP_TOKEN_MINT,
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
            program: crate::id(),
        }
        .to_account_metas(None);

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
            data: ix_data.data(),
        })
    }
}

impl JLPCacheAccounts {
    pub fn calculate_jlp_price(&self) -> f64 {
        let supply =
            spl_token::amount_to_ui_amount(self.token_mint.supply, self.token_mint.decimals);
        (self.pool.aum_usd as f64 / supply) / 10_usize.pow(self.token_mint.decimals as u32) as f64
    }
    pub fn free_space(&self) -> bool {
        self.pool.aum_usd < self.pool.limit.max_aum_usd
    }
}
