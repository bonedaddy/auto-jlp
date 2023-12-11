//! A proxy-style authentication program for integrating SSI with phoenix dex

#![deny(missing_docs)]
#![forbid(unsafe_code)]

/// entrypoint for on-chain instructions
mod entrypoint;

/// instruction processor which gets invoked by the entrypoint and is
/// responsible for routing instructions to the correct handler
pub mod processor;

/// instruction definitions
pub mod instructions;

solana_program::declare_id!("PrxYykBoXUrH8LEFoxExZQCHyZqziVfWbmdrrTtkwaB");
