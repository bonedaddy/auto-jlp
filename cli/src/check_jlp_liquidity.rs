use anchor_lang::prelude::*;
use anyhow::{anyhow, Context, Result};
use config::Configuration;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub const PERPETUALS_ACCT: &str = "H4ND9aYttUVLFmNypZqLjZ52FYiGvdEB45GmwNoKEjTj";
pub const POOL_ACCT: &str = "5BUwFW4nRbftYTDMbgxykoFWqWHPzahFSNAaaaJtVKsq";

pub async fn check_jlp_liquidity(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;
    let rpc = conf.rpc();

    let pool = Pubkey::from_str(POOL_ACCT).unwrap();
    let perp = Pubkey::from_str(PERPETUALS_ACCT).unwrap();
    let jlp_cache_accounts =
        perpetuals::jlp_cacher::JLPCacheAccountKeys::load_account_keys(&rpc, perp, pool).await?;

    log::info!("{:#?}", jlp_cache_accounts);

    Ok(())
}
