use anchor_lang::AnchorDeserialize;
use anyhow::{anyhow, Context, Result};
use config::Configuration;
use jupiter_api::swapper::Swapper;
use perpetuals::jlp_cacher::{LP_TOKEN_MINT, JLPCacheAccounts, JLPCacheAccountKeys, USDC_TOKEN_MINT};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction, program_pack::Pack, signer::Signer,
    transaction::Transaction,
};
use solana_sdk::{pubkey::Pubkey, signature::Keypair};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::signal::unix::{Signal, SignalKind};

use crate::check_jlp_liquidity::{PERPETUALS_ACCT, POOL_ACCT};

const LP_MINT_STR: &str = "27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4";
pub const USDC_MINT_STR: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

pub async fn auto_deposit(matches: &clap::ArgMatches, conf_path: &str) -> Result<()> {
    let conf = Configuration::load(conf_path)?;
    let rpc = conf.rpc();
    let keypair_contents = conf.keypair.contents();
    // expect it to be pk
    let keypair = Keypair::from_base58_string(&keypair_contents);
    let owner = keypair.pubkey();

    let swapper = Arc::new(Swapper::new(
        Arc::new(rpc),
        keypair
    ));
    let keypair = swapper.keypair();
    let force = matches.get_flag("force");
    let skip_capacity_check = matches.get_flag("skip-capacity-check");

    let priority_fee = matches.get_one::<f64>("priority-fee").unwrap();
    let priority_fee = spl_token::ui_amount_to_amount(*priority_fee, 9);

    let deposit_amount = matches.get_one::<f64>("deposit-amount").unwrap();

    let deposit_mint = matches.get_one::<String>("deposit-mint").unwrap();
    let deposit_mint = Pubkey::from_str(deposit_mint).unwrap();

    log::info!(
        "deposit_mint {}, deposit_amount {}",
        deposit_mint,
        deposit_amount
    );

    let acct_data = swapper.rpc.get_account_data(&deposit_mint).await?;
    let deposit_mint_acct = spl_token::state::Mint::unpack(&acct_data[..])?;
    let ui_deposit_amount =
        spl_token::ui_amount_to_amount(*deposit_amount, deposit_mint_acct.decimals);

    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    let pool_acct = Pubkey::from_str(POOL_ACCT).unwrap();
    let perp_acct = Pubkey::from_str(PERPETUALS_ACCT).unwrap();
    let jlp_account_cache =
        Arc::new(perpetuals::jlp_cacher::JLPCacheAccountKeys::load_account_keys(&swapper.rpc, perp_acct, pool_acct)
            .await?);

    let unit_price_ix = ComputeBudgetInstruction::set_compute_unit_price(priority_fee);
    let cu_ix = ComputeBudgetInstruction::set_compute_unit_limit(400_000);

    // create the ata account if needed

    if let Some(ix) = jlp_account_cache.create_lp_token_ata_ix(&swapper.rpc, owner).await {
        let mut tx = Transaction::new_with_payer(&[ix], Some(&owner));
        tx.sign(&vec![&keypair], swapper.rpc.get_latest_blockhash().await?);
        let sig = swapper.rpc.send_and_confirm_transaction(&tx).await?;
        log::info!("sent create ata tx {}", sig);
    }

    let mut sig_int = tokio::signal::unix::signal(SignalKind::interrupt())?;
    let mut sig_quit = tokio::signal::unix::signal(SignalKind::quit())?;
    let mut sig_term = tokio::signal::unix::signal(SignalKind::terminate())?;

    let (swap_tx, swap_rx) = tokio::sync::mpsc::channel::<()>(128);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();

    let usdc_ata = spl_associated_token_account::get_associated_token_address(
        &keypair.pubkey(),
        &deposit_mint
    );

    {
        let swapper = swapper.clone();
        let jlp_cache = jlp_account_cache.clone();
        tokio::task::spawn(async move {
            if let Err(err) = swapi_boi(swapper, swap_rx, exit_rx, jlp_cache).await {
                log::error!("{err:#?}");
            }
        });
    }
    loop {
        tokio::select! {
            biased;
            _ = sig_int.recv() => {
                log::info!("goodbye");
                let _ = exit_tx.send(());
                return Ok(());
            }
            _ = sig_quit.recv() => {
                log::info!("goodbye");
                let _ = exit_tx.send(());
                return Ok(());
            }
            _ = sig_term.recv() => {
                log::info!("goodbye");
                let _ = exit_tx.send(());
                return Ok(());
            }
            _ = ticker.tick() => {

            }
        }
        let jlp_accounts = jlp_account_cache.load_accounts(&swapper.rpc, usdc_ata).await?;
        let jlp_price = jlp_accounts.calculate_jlp_price();

        // compute the min jlp token per usdc
        let min_jlp_per_usdc = 1.0 / jlp_price;

        log::info!(
            "aum {}, aum_max {}, jlp_price {jlp_price}, jlp_per_usdc {min_jlp_per_usdc}",
            jlp_accounts.pool.aum_usd,
            jlp_accounts.pool.limit.max_aum_usd
        );

        if jlp_accounts.pool.aum_usd >= jlp_accounts.pool.limit.max_aum_usd && !skip_capacity_check {
            log::debug!("max deposit cap reached");
            continue;
        }

        let (deposit_amount, min_out) = if force {
            (
                ui_deposit_amount,
                spl_token::ui_amount_to_amount(
                    (*deposit_amount * min_jlp_per_usdc) * 0.99, // 1% slip
                    deposit_mint_acct.decimals,
                ),
            )
        } else {
            let room_for_deposit_usd =
                jlp_accounts.pool.limit.max_aum_usd - jlp_accounts.pool.aum_usd;
            let deposit_amount = if (room_for_deposit_usd as f64) < (*deposit_amount) {
                // available capacity less than deposit amount so override deposit amount with capacity
                room_for_deposit_usd as f64
            } else {
                // use deposit
                *deposit_amount
            };
            let cur_bal = spl_token::amount_to_ui_amount(jlp_accounts.usdc_token_account.amount, deposit_mint_acct.decimals);
            // if the available room is more than our current balance, overwrite with our balance
            let deposit_amount = if deposit_amount > cur_bal {
                cur_bal
            } else {
                deposit_amount
            };
            (
                spl_token::ui_amount_to_amount(deposit_amount, deposit_mint_acct.decimals),
                spl_token::ui_amount_to_amount(
                    (deposit_amount * min_jlp_per_usdc) * 0.99, // 1% slip
                    deposit_mint_acct.decimals,
                ),
            )
        };

        log::info!(
            "depositing {} usdc for expected {} jlp",
            deposit_amount,
            min_out
        );

        let add_liq_ix = jlp_account_cache.generate_liquidity_add_ix(
            deposit_mint,
            owner,
            deposit_amount,
            min_out,
        )?;

        let mut tx = Transaction::new_with_payer(
            &[unit_price_ix.clone(), cu_ix.clone(), add_liq_ix],
            Some(&owner),
        );
        tx.sign(&vec![&keypair], swapper.rpc.get_latest_blockhash().await?);
        for _ in 0..3 {
            match swapper.rpc
            .send_transaction_with_config(
                &tx,
                solana_client::rpc_config::RpcSendTransactionConfig {
                    skip_preflight: false,
                    ..Default::default()
                },
            )
            .await {
                Ok(sig) => {
                    log::info!("sent add liquidity {}", sig);
                    if let Err(err) = swap_tx.send(()).await {
                        log::error!("failed to send swap notification {err:#?}");
                    }
                    break;
                }
                Err(err) => {
                    log::error!("failed to send add liq tx {err:#?}, retrying...");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
}


async fn swapi_boi(
    swapper: Arc<Swapper>, 
    mut swap_trigger: tokio::sync::mpsc::Receiver<()>,
    mut exit_rx: tokio::sync::oneshot::Receiver<()>,
    jlp_account_cache: Arc<JLPCacheAccountKeys>,
) -> anyhow::Result<()> {
    let keypair = swapper.keypair();
    let owner = keypair.pubkey();
    let owner_str = owner.to_string();
    let jlp_ata = spl_associated_token_account::get_associated_token_address(
        &owner,
        &LP_TOKEN_MINT
    );
    let usdc_ata = spl_associated_token_account::get_associated_token_address(
        &owner,
        &USDC_TOKEN_MINT
    );
    let swap_api = Arc::new(jupiter_api::client::Client::new()?);

    let jlp_pool_rate = Arc::new(AtomicF64::new(0.0));
    let jlp_swap_rate = Arc::new(AtomicF64::new(0.0));

    {
        let jlp_pool_rate = jlp_pool_rate.clone();
        let jlp_swap_rate = jlp_swap_rate.clone();
        let swapper = swapper.clone();
        let swap_api = swap_api.clone();
        tokio::task::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                ticker.tick().await;
                match jlp_account_cache.load_accounts(&swapper.rpc, usdc_ata).await {
                    Ok(jlp_accts) => {
                        let pool_jlp_price = jlp_accts.calculate_jlp_price();
                        log::info!("swapi_boi::pricer::pool_jlp_price({pool_jlp_price})");
                        jlp_pool_rate.store(pool_jlp_price, Ordering::SeqCst);
                        match swap_api.price_query(LP_MINT_STR, USDC_MINT_STR).await {
                            Ok(price_query) => {
                                let keys = price_query.data.keys().collect::<Vec<_>>()[0];
                                let price_data = price_query.data.get(keys).unwrap();
                                jlp_swap_rate.store(price_data.price, Ordering::SeqCst);
                                log::info!("swapi_boi::pricer::swap_jlp_price({})", price_data.price);
                            }
                            Err(err) => {
                                log::error!("failed to query jlp swap price {err:#?}");
                            }
                        }
                    }
                    Err(err) => {
                        log::error!("failed to load jlp accounts {err:#}");
                    }
                }
            }
        });
    }


    loop {
        log::info!("waiting for swap requests");
        tokio::select! {
            biased;
            _ = swap_trigger.recv() => {},
            _ = &mut exit_rx => {
                log::info!("swapi_boi goodbye");
                return Ok(());
            }
        }
        log::info!("swapping...");
        let jlp_tkn_acct = match swapper.rpc.get_account_data(&jlp_ata).await {
            Ok(jlp_ata_acct_data) => {
                match spl_token::state::Account::unpack(&jlp_ata_acct_data) {
                    Ok(tkn_acct) => {
                        tkn_acct
                    }
                    Err(err) => {
                        log::error!("failed to unpack jupiter ata {err:#?}");
                        continue;
                    }
                }
            }
            Err(err) => {
                log::error!("failed to fetch jupiter ata {err:#?}");
                continue;
            }
        };

        let jlp_pool_price = jlp_pool_rate.load(Ordering::SeqCst);
        let jlp_swap_price = jlp_swap_rate.load(Ordering::SeqCst);
        log::info!("swapi_boi::swapper::pool_jlp_price({jlp_pool_price})");
        log::info!("swapi_boi::pricer::swap_jlp_price({jlp_swap_price})");
        // only swap if 99% of the swap price is greater than pool price
        if jlp_swap_price *0.99 < jlp_pool_price {
            log::info!("99% of swap price {jlp_swap_price} is less than pool price {jlp_pool_price}, skipping swap");
            continue;
        }
        for _ in 0..3 {
            match swap_api.new_quote(
                LP_MINT_STR,
                USDC_MINT_STR,
                jlp_tkn_acct.amount,
                &[]
            ).await {
                Ok(quote_response) => {
                    match swap_api
                    .new_swap(quote_response, &owner_str, true).await {
                        Ok(swap_response) => {
                            match swapper.new_swap(swap_response, false, 5).await {
                                Ok(sig) => {
                                    log::info!("sent swap tx {}", sig);
                                    continue;
                                }
                                Err(err) => {
                                    log::error!("failed to execute swap {err:#?}, retrying...");
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                }
                            }
                        }
                        Err(err) => {
                            log::error!("failed to fetch swap data {err:#?}, retrying...");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                    }
                }
                Err(err) => {
                    log::error!("failed to fetch quote {err:#?}, retrying... ");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
        log::error!("quote retry failed");

    }
}

struct AtomicF64 {
    inner: AtomicU64,
}

impl AtomicF64 {
    fn new(value: f64) -> AtomicF64 {
        AtomicF64 {
            inner: AtomicU64::new(value.to_bits()),
        }
    }

    fn load(&self, order: Ordering) -> f64 {
        f64::from_bits(self.inner.load(order))
    }

    fn store(&self, value: f64, order: Ordering) {
        self.inner.store(value.to_bits(), order)
    }

    // Add more methods as needed
}