#![allow(dead_code)]

pub mod test_handlers;

use std::{path::PathBuf, time::Duration};

use tokio::sync::broadcast;
use yellowstone_vixen::{
    config::{BufferConfig, VixenConfig},
    vixen_core::{instruction::InstructionUpdate, Parser},
};
use yellowstone_vixen_mock::{
    create_mock_transaction_update_with_cache, parse_instructions_from_txn_update,
};
use yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcConfig;

/// Command line options for integration tests
#[derive(clap::Parser, Debug)]
#[command(version, author, about = "Yellowstone Vixen Integration Tests")]
pub struct TestOpts {
    /// Path to the configuration file
    #[arg(long, short)]
    pub config: Option<PathBuf>,
}

/// Create test configuration with priority: CLI config > environment variables > default
pub fn create_test_config(
) -> Result<VixenConfig<YellowstoneGrpcConfig>, Box<dyn std::error::Error + Send + Sync>> {
    // Try to parse command line arguments for config file path
    let config_from_file = try_load_config_from_file();

    // If config file loading succeeds, use it
    if let Ok(config) = config_from_file {
        return Ok(config);
    }

    // Fall back to environment variables (backward compatibility)
    let config_from_env = try_load_config_from_env();
    if let Ok(config) = config_from_env {
        return Ok(config);
    }

    // If both fail, return error
    Err(
        "No valid configuration found. Please provide either a config file via --config or set \
         environment variables GRPC_URL"
            .into(),
    )
}

/// Try to load configuration from TOML file
fn try_load_config_from_file(
) -> Result<VixenConfig<YellowstoneGrpcConfig>, Box<dyn std::error::Error + Send + Sync>> {
    // Debug: print current working directory
    if let Ok(cwd) = std::env::current_dir() {
        tracing::debug!("Current working directory: {}", cwd.display());
    }
    // Check if config file path is provided via arguments or use default
    let config_path = std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|window| window[0] == "--config")
        .map(|window| PathBuf::from(&window[1]))
        .unwrap_or_else(|| {
            // Try multiple possible locations for the config file
            let possible_paths = [
                "tests/Vixen.test.toml",
                "./tests/Vixen.test.toml",
                "../tests/Vixen.test.toml",
            ];

            for path in &possible_paths {
                let p = PathBuf::from(path);
                tracing::debug!("Checking config path: {}", p.display());
                if p.exists() {
                    tracing::debug!("Found config file at: {}", p.display());
                    return p;
                }
            }

            // Fallback to the first path
            PathBuf::from("tests/Vixen.test.toml")
        });

    if !config_path.exists() {
        return Err(format!("Config file not found: {}", config_path.display()).into());
    }

    let config_content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Error reading config file {}: {}", config_path.display(), e))?;

    let config: VixenConfig<YellowstoneGrpcConfig> = toml::from_str(&config_content)
        .map_err(|e| format!("Error parsing config file {}: {}", config_path.display(), e))?;

    tracing::info!("Loaded configuration from: {}", config_path.display());
    tracing::info!("Using endpoint: {}", config.source.endpoint);
    tracing::info!(
        "Auth token: {}",
        if config.source.x_token.is_some() {
            "Set"
        } else {
            "Not set"
        }
    );

    Ok(config)
}

/// Try to load configuration from environment variables (backward compatibility)
fn try_load_config_from_env(
) -> Result<VixenConfig<YellowstoneGrpcConfig>, Box<dyn std::error::Error + Send + Sync>> {
    let grpc_url =
        std::env::var("GRPC_URL").map_err(|_| "GRPC_URL environment variable not set")?;
    let grpc_auth_token = std::env::var("GRPC_AUTH_TOKEN").ok();
    let grpc_timeout = std::env::var("GRPC_TIMEOUT")
        .unwrap_or_else(|_| "30".to_string())
        .parse::<u64>()
        .unwrap_or(30);

    // Ensure URL has proper HTTP/HTTPS prefix
    let processed_url = if !grpc_url.starts_with("http://") && !grpc_url.starts_with("https://") {
        format!("http://{grpc_url}")
    } else {
        grpc_url
    };

    tracing::info!("Loaded configuration from environment variables");
    tracing::info!("Using endpoint: {}", processed_url);
    tracing::info!(
        "Auth token: {}",
        if grpc_auth_token.is_some() {
            "Set"
        } else {
            "Not set"
        }
    );

    Ok(VixenConfig {
        source: YellowstoneGrpcConfig {
            endpoint: processed_url,
            x_token: grpc_auth_token,
            timeout: grpc_timeout,
            commitment_level: None,
            from_slot: None,
            max_decoding_message_size: None,
            accept_compression: None,
        },
        buffer: BufferConfig {
            jobs: None,
            sources_channel_size: 100,
        },
    })
}

/// Helper function to run integration test with event-based completion
pub async fn run_integration_test_with_event_completion<F, Fut>(
    test_fn: F,
    mut shutdown_rx: broadcast::Receiver<()>,
    max_duration: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    let test_future = test_fn();

    tokio::select! {
        test_result = test_future => {
            match test_result {
                Ok(()) => {
                    tracing::info!("Integration test runtime completed");
                    Ok(())
                }
                Err(e) => {
                    tracing::error!("Integration test failed: {:?}", e);
                    Err(e)
                }
            }
        }
        _ = shutdown_rx.recv() => {
            tracing::info!("Integration test stopped after receiving at least one event");
            Ok(())
        }
        _ = tokio::time::sleep(max_duration) => {
            tracing::warn!("Integration test timed out after {:?}, but this may be normal for live data tests", max_duration);
            Ok(())
        }
    }
}

// ============================================================================
// Navigation Helper
// ============================================================================

/// Navigate to an instruction using ix_path.
///
/// # Arguments
/// * `instructions` - List of top-level instructions
/// * `ix_path` - Path to the target instruction (e.g., &[3] for top-level #3, &[2, 0] for inner)
///
/// # Example
/// ```ignore
/// // Top-level instruction #3
/// navigate_to_instruction(&instructions, &[3])
///
/// // Top-level #2 → inner #0
/// navigate_to_instruction(&instructions, &[2, 0])
///
/// // Top-level #2 → inner #0 → inner #1
/// navigate_to_instruction(&instructions, &[2, 0, 1])
/// ```
fn navigate_to_instruction<'a>(
    instructions: &'a [InstructionUpdate],
    ix_path: &[usize],
) -> Result<&'a InstructionUpdate, Box<dyn std::error::Error + Send + Sync>> {
    if ix_path.is_empty() {
        return Err("ix_path cannot be empty".into());
    }

    let mut current = instructions
        .get(ix_path[0])
        .ok_or_else(|| format!("Instruction index {} not found", ix_path[0]))?;

    for (depth, &idx) in ix_path.iter().enumerate().skip(1) {
        current = current.inner.get(idx).ok_or_else(|| {
            format!(
                "Inner instruction index {} at depth {} not found",
                idx, depth
            )
        })?;
    }

    Ok(current)
}

// ============================================================================
// CPI-based Parser Helpers
// ============================================================================

/// Assert OKX DEX v2 parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the OKX instruction (e.g., &[3] for top-level)
/// * `expected_source_token_change` - Expected input amount
/// * `expected_destination_token_change` - Expected output amount
pub async fn assert_okx_v2_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_okx_dex_v2_parser::{
        instructions_parser::{InstructionParser as OkxV2Parser, OnChainLabsDexRouter2ProgramIx},
        types::{CpiEventWithFallback, SwapEventData},
    };

    let parser = OkxV2Parser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    // Extract CPI event from parsed enum
    let event: &CpiEventWithFallback = match &parsed {
        OnChainLabsDexRouter2ProgramIx::Swap(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::ProxySwap(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapTob(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapTobEnhanced(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapTobV2(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapTobWithReceiver(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapToc(_, _, Some(e)) => e,
        OnChainLabsDexRouter2ProgramIx::SwapTocV2(_, _, Some(e)) => e,
        _ => return Err("No CPI event found in parsed instruction".into()),
    };

    assert_eq!(
        event.source_token_change(),
        expected_source_token_change,
        "source_token_change mismatch"
    );
    assert_eq!(
        event.destination_token_change(),
        expected_destination_token_change,
        "destination_token_change mismatch"
    );
    Ok(())
}

/// Assert PumpSwap Buy parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the PumpSwap instruction
/// * `expected_quote_amount_in` - Expected SOL spent
/// * `expected_base_amount_out` - Expected tokens received
pub async fn assert_pumpswap_buy_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_quote_amount_in: u64,
    expected_base_amount_out: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_pump_swaps_parser::instructions_parser::{
        InstructionParser as PumpSwapsParser, PumpAmmProgramIx,
    };

    let parser = PumpSwapsParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        PumpAmmProgramIx::Buy(_, _, Some(e)) => e,
        PumpAmmProgramIx::BuyExactQuoteIn(_, _, Some(e)) => e,
        _ => return Err("Expected Buy instruction with event".into()),
    };

    assert_eq!(
        event.quote_amount_in, expected_quote_amount_in,
        "quote_amount_in mismatch"
    );
    assert_eq!(
        event.base_amount_out, expected_base_amount_out,
        "base_amount_out mismatch"
    );
    Ok(())
}

/// Assert PumpSwap Sell parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the PumpSwap instruction
/// * `expected_base_amount_in` - Expected tokens spent
/// * `expected_quote_amount_out` - Expected SOL received
pub async fn assert_pumpswap_sell_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_base_amount_in: u64,
    expected_quote_amount_out: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_pump_swaps_parser::instructions_parser::{
        InstructionParser as PumpSwapsParser, PumpAmmProgramIx,
    };

    let parser = PumpSwapsParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        PumpAmmProgramIx::Sell(_, _, Some(e)) => e,
        _ => return Err("Expected Sell instruction with event".into()),
    };

    assert_eq!(
        event.base_amount_in, expected_base_amount_in,
        "base_amount_in mismatch"
    );
    assert_eq!(
        event.quote_amount_out, expected_quote_amount_out,
        "quote_amount_out mismatch"
    );
    Ok(())
}

/// Assert Jupiter parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the Jupiter instruction
/// * `event_index` - Index into Vec<(SwapEvent, u16)> to select which event to verify
/// * `expected_source_token_change` - Expected input_amount
/// * `expected_destination_token_change` - Expected output_amount
pub async fn assert_jupiter_parser_flow(
    signature: &str,
    ix_path: &[usize],
    event_index: usize,
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_jupiter_swap_parser::{
        instructions_parser::{InstructionParser as JupiterParser, JupiterProgramIx},
        types::SwapEvent as JupiterSwapEvent,
    };

    let parser = JupiterParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    // Jupiter V1 Route variants return Vec<(SwapEvent, u16)>
    let events: &Vec<(JupiterSwapEvent, u16)> = match &parsed {
        JupiterProgramIx::Route(_, _, events) => events,
        JupiterProgramIx::ExactOutRoute(_, _, events) => events,
        JupiterProgramIx::RouteWithTokenLedger(_, _, events) => events,
        JupiterProgramIx::SharedAccountsRoute(_, _, events) => events,
        JupiterProgramIx::SharedAccountsExactOutRoute(_, _, events) => events,
        JupiterProgramIx::SharedAccountsRouteWithTokenLedger(_, _, events) => events,
        // V2 variants need different handling
        _ => return Err("Unsupported Jupiter instruction variant".into()),
    };

    let (event, _) = events.get(event_index).ok_or_else(|| {
        format!(
            "Event index {} out of bounds (len={})",
            event_index,
            events.len()
        )
    })?;

    assert_eq!(
        event.input_amount, expected_source_token_change,
        "input_amount mismatch"
    );
    assert_eq!(
        event.output_amount, expected_destination_token_change,
        "output_amount mismatch"
    );
    Ok(())
}

/// Assert Meteora DLMM parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the Meteora DLMM instruction
/// * `expected_source_token_change` - Expected amount_in
/// * `expected_destination_token_change` - Expected amount_out
pub async fn assert_meteora_dlmm_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_meteora_parser::instructions_parser::{
        InstructionParser as MeteoraDlmmParser, LbClmmProgramIx,
    };

    let parser = MeteoraDlmmParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        LbClmmProgramIx::Swap(_, _, Some(e)) => e,
        LbClmmProgramIx::SwapExactOut(_, _, Some(e)) => e,
        LbClmmProgramIx::SwapWithPriceImpact(_, _, Some(e)) => e,
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    assert_eq!(
        event.amount_in, expected_source_token_change,
        "amount_in mismatch"
    );
    assert_eq!(
        event.amount_out, expected_destination_token_change,
        "amount_out mismatch"
    );
    Ok(())
}

/// Assert PumpFun parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the PumpFun instruction
/// * `expected_source_token_change` - Expected source amount (sol_amount if buy, token_amount if sell)
/// * `expected_destination_token_change` - Expected dest amount (token_amount if buy, sol_amount if sell)
pub async fn assert_pumpfun_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_pumpfun_parser::{
        instructions_parser::{InstructionParser as PumpFunParser, PumpProgramIx},
        types::TradeEvent,
    };

    let parser = PumpFunParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        PumpProgramIx::Buy(_, _, Some(e)) => e,
        PumpProgramIx::Sell(_, _, Some(e)) => e,
        _ => return Err("No trade event found in parsed instruction".into()),
    };

    let (source, dest) = match event {
        TradeEvent::V1(v) => {
            if v.is_buy {
                (v.sol_amount, v.token_amount)
            } else {
                (v.token_amount, v.sol_amount)
            }
        },
        TradeEvent::V2(v) => {
            if v.is_buy {
                (v.sol_amount, v.token_amount)
            } else {
                (v.token_amount, v.sol_amount)
            }
        },
    };

    assert_eq!(source, expected_source_token_change, "source mismatch");
    assert_eq!(dest, expected_destination_token_change, "dest mismatch");
    Ok(())
}

// ============================================================================
// Log-based Parser Helpers
// ============================================================================

/// Assert Raydium AMM V4 parser flow with expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature
/// * `ix_path` - Path to the instruction
/// * `expected_source_token_change` - Expected amount_in (BaseIn) or direct_in (BaseOut)
/// * `expected_destination_token_change` - Expected out_amount (BaseIn) or amount_out (BaseOut)
pub async fn assert_raydium_amm_v4_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_raydium_amm_v4_parser::{
        instructions_parser::{InstructionParser as RaydiumAmmV4Parser, RaydiumAmmV4ProgramIx},
        types::SwapEvent as RaydiumAmmV4SwapEvent,
    };

    let parser = RaydiumAmmV4Parser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed.parsed_ix {
        RaydiumAmmV4ProgramIx::SwapBaseIn(_, _, Some(e)) => e,
        RaydiumAmmV4ProgramIx::SwapBaseOut(_, _, Some(e)) => e,
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    let (source, dest) = match event {
        RaydiumAmmV4SwapEvent::BaseIn(e) => (e.amount_in, e.out_amount),
        RaydiumAmmV4SwapEvent::BaseOut(e) => (e.direct_in, e.amount_out),
    };

    assert_eq!(
        source, expected_source_token_change,
        "source_token_change mismatch"
    );
    assert_eq!(
        dest, expected_destination_token_change,
        "destination_token_change mismatch"
    );
    Ok(())
}

/// Assert Raydium CLMM parser flow with expected token changes.
pub async fn assert_raydium_clmm_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_raydium_clmm_parser::instructions_parser::{
        AmmV3ProgramIx, InstructionParser as RaydiumClmmParser,
    };

    let parser = RaydiumClmmParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        AmmV3ProgramIx::Swap(_, _, Some(e)) => e,
        AmmV3ProgramIx::SwapV2(_, _, Some(e)) => e,
        // SwapRouterBaseIn doesn't have an event field
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    // zero_for_one determines direction: true = token0 -> token1, false = token1 -> token0
    let (source, dest) = if event.zero_for_one {
        (event.amount_0, event.amount_1)
    } else {
        (event.amount_1, event.amount_0)
    };

    assert_eq!(
        source, expected_source_token_change,
        "source_token_change mismatch"
    );
    assert_eq!(
        dest, expected_destination_token_change,
        "destination_token_change mismatch"
    );
    Ok(())
}

/// Assert Raydium CPMM parser flow with expected token changes.
pub async fn assert_raydium_cpmm_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_raydium_cpmm_parser::{
        instructions_parser::{InstructionParser as RaydiumCpmmParser, RaydiumCpSwapProgramIx},
        types::SwapEvent as RaydiumCpmmSwapEvent,
    };

    let parser = RaydiumCpmmParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        RaydiumCpSwapProgramIx::SwapBaseInput(_, _, Some(e)) => e,
        RaydiumCpSwapProgramIx::SwapBaseOutput(_, _, Some(e)) => e,
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    let (source, dest) = match event {
        RaydiumCpmmSwapEvent::V1(e) => (e.input_amount, e.output_amount),
        RaydiumCpmmSwapEvent::V2(e) => (e.input_amount, e.output_amount),
    };

    assert_eq!(
        source, expected_source_token_change,
        "source_token_change mismatch"
    );
    assert_eq!(
        dest, expected_destination_token_change,
        "destination_token_change mismatch"
    );
    Ok(())
}

/// Assert Meteora Pools parser flow with expected token changes.
pub async fn assert_meteora_pools_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_meteora_pools_parser::instructions_parser::{
        AmmProgramIx, InstructionParser as MeteoraPoolsParser,
    };

    let parser = MeteoraPoolsParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        AmmProgramIx::Swap(_, _, Some(e)) => e,
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    assert_eq!(
        event.in_amount, expected_source_token_change,
        "in_amount mismatch"
    );
    assert_eq!(
        event.out_amount, expected_destination_token_change,
        "out_amount mismatch"
    );
    Ok(())
}

/// Assert Moonshot parser flow with expected token changes.
pub async fn assert_moonshot_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_moonshot_parser::instructions_parser::{
        InstructionParser as MoonshotParser, TokenLaunchpadProgramIx,
    };

    let parser = MoonshotParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    let event = match &parsed {
        TokenLaunchpadProgramIx::Buy(_, _, Some(e)) => e,
        TokenLaunchpadProgramIx::Sell(_, _, Some(e)) => e,
        _ => return Err("No trade event found in parsed instruction".into()),
    };

    assert_eq!(
        event.collateral_amount, expected_source_token_change,
        "collateral_amount mismatch"
    );
    assert_eq!(
        event.amount, expected_destination_token_change,
        "amount mismatch"
    );
    Ok(())
}

/// Assert Orca Whirlpool parser flow with expected token changes.
pub async fn assert_orca_whirlpool_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_orca_whirlpool_parser::instructions_parser::{
        InstructionParser as OrcaWhirlpoolParser, WhirlpoolProgramIx,
    };

    let parser = OrcaWhirlpoolParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    // Extract TradedEvent - Swap/SwapV2 return Option, TwoHopSwap/TwoHopSwapV2 return Vec
    let event = match &parsed {
        WhirlpoolProgramIx::Swap(_, _, Some(e)) => e,
        WhirlpoolProgramIx::SwapV2(_, _, Some(e)) => e,
        WhirlpoolProgramIx::TwoHopSwap(_, _, events) if !events.is_empty() => {
            events.first().unwrap()
        },
        WhirlpoolProgramIx::TwoHopSwapV2(_, _, events) if !events.is_empty() => {
            events.first().unwrap()
        },
        _ => return Err("No traded event found in parsed instruction".into()),
    };

    assert_eq!(
        event.input_amount, expected_source_token_change,
        "input_amount mismatch"
    );
    assert_eq!(
        event.output_amount, expected_destination_token_change,
        "output_amount mismatch"
    );
    Ok(())
}

/// Assert Pancake parser flow with expected token changes.
pub async fn assert_pancake_parser_flow(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use yellowstone_vixen_pancake_parser::instructions_parser::{
        AmmV3ProgramIx, InstructionParser as PancakeParser,
    };

    let parser = PancakeParser;
    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .map_err(|e| format!("{e}"))?;
    let instructions =
        parse_instructions_from_txn_update(&txn_update).map_err(|e| format!("{e}"))?;
    let target_ix = navigate_to_instruction(&instructions, ix_path)?;

    let parsed = parser
        .parse(target_ix)
        .await
        .map_err(|e| format!("{e:?}"))?;

    // Pancake SwapRouterBaseIn returns Vec<SwapEvent>
    let event = match &parsed {
        AmmV3ProgramIx::Swap(_, _, Some(e)) => e,
        AmmV3ProgramIx::SwapV2(_, _, Some(e)) => e,
        AmmV3ProgramIx::SwapRouterBaseIn(_, _, events) if !events.is_empty() => {
            events.first().unwrap()
        },
        _ => return Err("No swap event found in parsed instruction".into()),
    };

    // zero_for_one determines direction: true = token0 -> token1, false = token1 -> token0
    let (source, dest) = if event.zero_for_one {
        (event.amount0, event.amount1)
    } else {
        (event.amount1, event.amount0)
    };

    assert_eq!(
        source, expected_source_token_change,
        "source_token_change mismatch"
    );
    assert_eq!(
        dest, expected_destination_token_change,
        "destination_token_change mismatch"
    );
    Ok(())
}
