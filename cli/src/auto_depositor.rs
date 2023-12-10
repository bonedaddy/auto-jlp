use std::str::FromStr;
use solana_sdk::signer::Signer;
use anchor_lang::AnchorDeserialize;
use anyhow::{Result, anyhow, Context};
use config::Configuration;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

use crate::check_jlp_liquidity::POOL_ACCT;

pub async fn auto_deposit(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;

    let keypair_contents = conf.keypair.contents();
    // expect it to be pk
    let keypair = Keypair::from_base58_string(&keypair_contents);
    let owner = keypair.pubkey();    

    let rpc = conf.rpc();

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    let pool_acct = Pubkey::from_str(POOL_ACCT).unwrap();
    let jlp_account_cache = perpetuals::jlp_cacher::JLPCacheAccounts::load_accounts(&rpc, pool_acct).await?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("goodbye");
                return Ok(());
            }
            _ = ticker.tick() => {

            }
        }

        let pool = jlp_account_cache.load_pool(&rpc).await?;
        log::info!("aum {}, aum_max {}", pool.aum_usd, pool.limit.max_aum_usd);

        if pool.aum_usd >= pool.limit.max_aum_usd {
            log::debug!("max deposit cap reached");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        let room_for_deposit_usd = pool.limit.max_aum_usd - pool.aum_usd;

        log::info!("room for deposit {}", room_for_deposit_usd);
    }
}