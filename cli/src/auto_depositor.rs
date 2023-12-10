use std::str::FromStr;
use solana_sdk::{signer::Signer, program_pack::Pack, compute_budget::ComputeBudgetInstruction, transaction::Transaction};
use anchor_lang::AnchorDeserialize;
use anyhow::{Result, anyhow, Context};
use config::Configuration;
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use tokio::signal::unix::{Signal, SignalKind};

use crate::check_jlp_liquidity::{POOL_ACCT, PERPETUALS_ACCT};

pub async fn auto_deposit(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;
    let rpc = conf.rpc();
    let keypair_contents = conf.keypair.contents();
    // expect it to be pk
    let keypair = Keypair::from_base58_string(&keypair_contents);
    let owner = keypair.pubkey();    

    let force = matches.get_flag("force");

    let priority_fee = matches.get_one::<f64>("priority-fee").unwrap();
    let priority_fee = spl_token::ui_amount_to_amount(*priority_fee, 9);

    let deposit_amount = matches.get_one::<f64>("deposit-amount").unwrap();

    let deposit_mint = matches.get_one::<String>("deposit-mint").unwrap();
    let deposit_mint = Pubkey::from_str(deposit_mint).unwrap();

    let deposit_mint_dollar_value = matches.get_one::<u128>("deposit-mint-dollar-value").unwrap();

    log::info!("deposit_mint {}, deposit_amount {}", deposit_mint, deposit_amount);


    let acct_data = rpc.get_account_data(&deposit_mint).await?;
    let deposit_mint_acct = spl_token::state::Mint::unpack(&acct_data[..])?;
    let ui_deposit_amount = spl_token::ui_amount_to_amount(*deposit_amount, deposit_mint_acct.decimals);

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    let pool_acct = Pubkey::from_str(POOL_ACCT).unwrap();
    let perp_acct = Pubkey::from_str(PERPETUALS_ACCT).unwrap();
    let jlp_account_cache = perpetuals::jlp_cacher::JLPCacheAccounts::load_accounts(&rpc, perp_acct,pool_acct).await?;

    let unit_price_ix = ComputeBudgetInstruction::set_compute_unit_price(priority_fee);
    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(400_000);

    // create the ata account if needed

    if let Some(ix) = jlp_account_cache.create_lp_token_ata_ix(
        &rpc,
        owner
    ).await {
        let mut tx = Transaction::new_with_payer(&[ix], Some(&owner));
        tx.sign(&vec![&keypair], rpc.get_latest_blockhash().await?);
        let sig = rpc.send_and_confirm_transaction(&tx).await?;
        log::info!("sent create ata tx {}", sig);
    }

    let mut sig_int = tokio::signal::unix::signal(SignalKind::interrupt())?;
    let mut sig_quit = tokio::signal::unix::signal(SignalKind::quit())?;
    let mut sig_term = tokio::signal::unix::signal(SignalKind::terminate())?;
    loop {
        tokio::select! {
            biased;
            _ = sig_int.recv() => {
                log::info!("goodbye");
                return Ok(());
            }
            _ = sig_quit.recv() => {
                log::info!("goodbye");
                return Ok(());
            }
            _ = sig_term.recv() => {
                log::info!("goodbye");
                return Ok(());
            }
            _ = ticker.tick() => {

            }
        }

        let pool = jlp_account_cache.load_pool(&rpc).await?;
        log::info!("aum {}, aum_max {}", pool.aum_usd, pool.limit.max_aum_usd);

        if pool.aum_usd >= pool.limit.max_aum_usd  && !force {
            log::debug!("max deposit cap reached");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        //let room_for_deposit_usd = pool.limit.max_aum_usd - pool.aum_usd;

        //let deposit_token_capacity = *deposit_mint_dollar_value / room_for_deposit_usd;

        let add_liq_ix = jlp_account_cache.generate_liquidity_add_ix(
            deposit_mint,
            owner,
            ui_deposit_amount
        )?;

        let mut tx = Transaction::new_with_payer(
            &[unit_price_ix.clone(), cu_ix.clone(), add_liq_ix],
            Some(&owner)
        );
        tx.sign(&vec![&keypair], rpc.get_latest_blockhash().await?);

        let sig = rpc.send_transaction_with_config(
            &tx,
            solana_client::rpc_config::RpcSendTransactionConfig { 
                skip_preflight: true, 
                ..Default::default()
            }       
        ).await?;
        log::info!("sent add liquidity {}", sig);
    }
}