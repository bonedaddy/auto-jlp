use anchor_lang::AnchorDeserialize;
use anyhow::{anyhow, Context, Result};
use config::Configuration;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction, program_pack::Pack, signer::Signer,
    transaction::Transaction,
};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::str::FromStr;
use std::sync::Arc;
use tokio::signal::unix::{Signal, SignalKind};

use crate::check_jlp_liquidity::{PERPETUALS_ACCT, POOL_ACCT};

pub async fn swap_tokens(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;
    let rpc = conf.rpc();
    let keypair_contents = conf.keypair.contents();
    // expect it to be pk
    let keypair = Keypair::from_base58_string(&keypair_contents);
    let owner = keypair.pubkey();
    let input_mint = matches.get_one::<String>("input-token").unwrap();
    let output_mint = matches.get_one::<String>("output-token").unwrap();
    let swap_amount = matches.get_one::<f64>("swap-amount").unwrap();

    let acct_data = rpc.get_account_data(&Pubkey::from_str(input_mint)?).await?;
    let input_mint_acct = spl_token::state::Mint::unpack(&acct_data)?;
    let swap_amount = spl_token::ui_amount_to_amount(*swap_amount, input_mint_acct.decimals);

    let api_client = jupiter_api::client::Client::new()?;
    let swap_client =  Arc::new(jupiter_api::swapper::Swapper::new(
        Arc::new(rpc),
        keypair
    ));


    let quote = api_client.new_quote(input_mint, output_mint, swap_amount, &[]).await?;
    let swap = api_client.new_swap(quote, &owner.to_string(), true).await?;
    let sig = swap_client.new_swap(swap, false, 5).await?;
    log::info!("sent swap {}", sig);
    Ok(())

}
