#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use {
    anyhow::{anyhow, Result},
    clap::{Arg, ArgMatches, Command},
    config::Configuration,
};

mod auto_depositor;
mod check_jlp_liquidity;
mod swapper;


#[tokio::main]
pub async fn main() -> Result<()> {
    let matches = Command::new("auto-jlp")
        .arg(config_flag())
        .arg(debug_flag())
        .subcommands(vec![
            Command::new("config")
                .about("configuration management commands")
                .subcommands(vec![Command::new("new")
                    .aliases(["gen", "generate"])
                    .about("create and save a new configuration file")
                    .arg(keypair_type_flag())]),
            Command::new("check-jlp-liquidity"),
            Command::new("auto-deposit")
                .arg(
                    Arg::new("deposit-mint")
                        .long("deposit-mint")
                        .help("token mint to deposit, only usdc supported"),
                )
                .arg(
                    Arg::new("deposit-amount")
                        .long("deposit-amount")
                        .help("ui amount (ie: 1.5) to deposit")
                        .long_help("if larger than free space, 10% of free space is used")
                        .value_parser(clap::value_parser!(f64)),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("always force deposit regardless of available capacity")
                        .action(clap::ArgAction::SetTrue)
                        .required(false),
                )
                .arg(
                    Arg::new("skip-capacity-check")
                        .long("skip-capacity-check")
                        .help("skip capacity check")
                        .action(clap::ArgAction::SetTrue)
                        .required(false),
                )
                .arg(
                    Arg::new("priority-fee")
                        .long("priority-fee")
                        .help("priority fee to use (ie: 0.01)")
                        .value_parser(clap::value_parser!(f64)),
                ),
                Command::new("swap-tokens")
                .arg(
                    Arg::new("input-token")
                    .long("input-token")
                    .help("input token mint to swap")
                )
                .arg(
                    Arg::new("output-token")
                    .long("output-token")
                    .help("output token mint to use")
                )
                .arg(
                    Arg::new("swap-amount")
                    .long("swap-amount")
                    .help("ui amount of tokens to swap")
                    .value_parser(clap::value_parser!(f64))
                )
        ])
        .get_matches();

    let conf_path = matches.get_one::<String>("config").unwrap();
    let debug_log = matches.get_flag("debug");

    utils::init_logger(debug_log);

    process_matches(&matches, conf_path).await?;

    Ok(())
}

async fn process_matches(matches: &ArgMatches, conf_path: &str) -> Result<()> {
    match matches.subcommand() {
        Some(("config", c)) => match c.subcommand() {
            Some(("new", n)) => {
                let cfg = Configuration::new(n.get_one::<String>("keypair-type").unwrap());
                Ok(cfg.save(conf_path)?)
            }
            _ => Err(anyhow!("{INVALID_COMMAND}")),
        },
        Some(("check-jlp-liquidity", cjl)) => {
            Ok(check_jlp_liquidity::check_jlp_liquidity(cjl, conf_path).await?)
        }
        Some(("auto-deposit", ad)) => Ok(auto_depositor::auto_deposit(ad, conf_path).await?),
        Some(("swap-tokens", st)) => Ok(swapper::swap_tokens(st, conf_path).await?),
        _ => Err(anyhow!("{INVALID_COMMAND}")),
    }
}

fn config_flag() -> Arg {
    Arg::new("config")
        .long("config")
        .help("path to the configuration file")
        .default_value("config.yaml")
}

fn keypair_type_flag() -> Arg {
    Arg::new("keypair-type")
        .long("keypair-type")
        .help("type of keypair we are using")
        .required(true)
}

fn debug_flag() -> Arg {
    Arg::new("debug")
        .long("debug")
        .help("enable debug logging")
        .action(clap::ArgAction::SetTrue)
        .required(false)
}

const INVALID_COMMAND: &str = "invalid command, try running --help";
