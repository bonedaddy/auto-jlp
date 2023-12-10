use anyhow::{Result, anyhow, Context};
use config::Configuration;
use anchor_lang::prelude::*;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

const PERPETUALS_ACCT: &str = "H4ND9aYttUVLFmNypZqLjZ52FYiGvdEB45GmwNoKEjTj";
const POOL_ACCT: &str = "5BUwFW4nRbftYTDMbgxykoFWqWHPzahFSNAaaaJtVKsq";

pub async fn check_jlp_liquidity(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;
    let rpc = conf.rpc();
    let acct_data = rpc.get_account_data(&Pubkey::from_str(POOL_ACCT).unwrap()).await?;
    let perp_pools = perpetuals::Pool::deserialize(&mut &acct_data[8..])?;
    println!("{}", perp_pools.log());
    Ok(())
}

