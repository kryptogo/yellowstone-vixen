use std::time::Duration;

use tracing::{error, info};
use yellowstone_vixen::{vixen_core::Parser, Pipeline, Runtime};
use yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcSource;

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_test_config, run_integration_test_with_event_completion,
    test_handlers::{JupiterTestHandler, OkxTestHandler},
};

// Initialize tracing once for all tests
fn init_tracing() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init()
            .ok(); // Ignore if already initialized
    });
}

// Import parsers
use kryptogo_vixen_okx_dex_parser::instructions_parser::InstructionParser as OkxInstructionParser;
use yellowstone_vixen_jupiter_swap_parser::instructions_parser::InstructionParser as JupiterInstructionParser;

/// Integration test
///
/// Configuration:
/// - Use --config path/to/config.toml to specify configuration file
/// - Falls back to environment variables for backward compatibility:
///   - GRPC_URL: gRPC service address
///   - GRPC_AUTH_TOKEN: authentication token
///   - GRPC_TIMEOUT: timeout in seconds
#[tokio::test]
#[ignore]
async fn test_jupiter_parser() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    info!("Starting Jupiter-only integration test");

    let config = create_test_config().map_err(|e| {
        error!("Failed to load configuration: {}", e);
        e
    })?;
    let (jupiter_handler, shutdown_rx) = JupiterTestHandler::new();
    let jupiter_parser = JupiterInstructionParser;

    info!("Jupiter Parser ID: {}", Parser::id(&jupiter_parser));

    let vixen_runtime = Runtime::<YellowstoneGrpcSource>::builder()
        .instruction(Pipeline::new(jupiter_parser, [jupiter_handler.clone()]))
        .build(config);

    info!("Starting Jupiter parser runtime...");

    let max_duration = Duration::from_secs(30);

    let result = run_integration_test_with_event_completion(
        || async { vixen_runtime.try_run_async().await.map_err(|e| e.into()) },
        shutdown_rx,
        max_duration,
    )
    .await;

    let stats = jupiter_handler.get_stats();
    info!("Jupiter Parser Statistics:");
    info!("  - Swap events: {}", stats.swap_count);
    info!("  - Route events: {}", stats.route_count);
    info!("  - Total volume: {}", stats.total_volume);

    result
}

#[tokio::test]
#[ignore]
async fn test_okx_parser() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    info!("Starting OKX-only integration test");

    let config = create_test_config().map_err(|e| {
        error!("Failed to load configuration: {}", e);
        e
    })?;
    let (okx_handler, shutdown_rx) = OkxTestHandler::new();
    let okx_parser = OkxInstructionParser;

    info!("OKX Parser ID: {}", Parser::id(&okx_parser));

    let vixen_runtime = Runtime::<YellowstoneGrpcSource>::builder()
        .instruction(Pipeline::new(okx_parser, [okx_handler.clone()]))
        .build(config);

    info!("Starting OKX parser runtime...");

    let max_duration = Duration::from_secs(30);

    let result = run_integration_test_with_event_completion(
        || async { vixen_runtime.try_run_async().await.map_err(|e| e.into()) },
        shutdown_rx,
        max_duration,
    )
    .await;

    let stats = okx_handler.get_stats();
    info!("OKX Parser Statistics:");
    info!("  - Swap events: {}", stats.swap_count);
    info!("  - Aggregation events: {}", stats.aggregation_count);
    info!("  - Total volume: {}", stats.total_volume);

    result
}

/// Test OKX DEX v2 parser flow with full InstructionParser.parse()
///
/// These tests use the full parser flow instead of directly parsing CPI events.
/// The parser handles instruction discriminator matching, account parsing, and CPI event extraction.
#[tokio::test]
async fn test_okx_dex_v2_parser_flow() {
    init_tracing();

    // Swap instruction
    common::assert_okx_v2_parser_flow(
        "4XfXNQABC7igdCgtux9dXDb6Dj8VzxBQb5JzgpNdy3ajKdnMbRfiZbywfbuoQTvQ3XCHdBvPBSCCqzDKaenHETVY",
        &[3], // top-level OKX instruction
        2000500000,
        295045121,
    )
    .await
    .expect("Swap parser flow test failed");

    // SwapTob instruction
    common::assert_okx_v2_parser_flow(
        "3Rrgt5ABbfUNoqerVQNCjfQYwafnSm3VNgmtB31aZ4y11Rc4FSHjdMzrXSkyquNnFVp8NAjrU1fAk6ero1cbw59q",
        &[6], // top-level OKX instruction
        10000000,
        14918710783,
    )
    .await
    .expect("SwapTob parser flow test failed");

    // SwapTobEnhanced instruction
    common::assert_okx_v2_parser_flow(
        "2wpzTEZzyWgC9ZTHMmppcdVwKDdCE1owBby1cFPNKB2S6XWW4sc4w3mxgDq4N1Z5bhzAGhLQqk6qMDCrVEi5RVhc",
        &[6], // top-level OKX instruction
        1000000,
        5699503,
    )
    .await
    .expect("SwapTobEnhanced parser flow test failed");

    // SwapTobWithReceiver instruction (called via aggregator)
    common::assert_okx_v2_parser_flow(
        "5H5SLPoNyvKjSQfUfiu3PxMKiqfejMh6wuge2TmteRJc6jGxW77XzbiQsvcd9y5zGrfkQ8E7cATepgTHkTu19shp",
        &[3, 2], // top-level #3 → inner OKX instruction
        4675790000,
        115187775,
    )
    .await
    .expect("SwapTobWithReceiver parser flow test failed");

    // SwapToc instruction
    common::assert_okx_v2_parser_flow(
        "X41pjVYMdoZd15v1AnHpqV9sGspTEBfzhJ6uk95X2tdthxnQCiGDz5iLfdkhhPfV6cNX14Jpqivq5wmonDudDMi",
        &[4], // top-level OKX instruction
        1191877137296814,
        7968827164,
    )
    .await
    .expect("SwapToc parser flow test failed");

    // SwapTocV2 instruction
    common::assert_okx_v2_parser_flow(
        "37DzX3osK9x5jKsCZnZHtkLopf3xmEekHDubpUBd9dVxPy9yCF9TWzvy5rLNSFnM9FyqnE9LeYyGDRvs4hdXmajc",
        &[7], // top-level OKX instruction
        1986400000,
        224645346850,
    )
    .await
    .expect("SwapTocV2 parser flow test failed");
}

/// Test PumpSwap Buy/Sell parser flow with full InstructionParser.parse()
#[tokio::test]
async fn test_pump_swaps_parser_flow() {
    init_tracing();

    // Buy instruction (called via aggregator)
    // Note: values updated to match actual parsed event from full parser flow
    common::assert_pumpswap_buy_parser_flow(
        "3V41y1wkTjYDQ4UAz6gaLT8h7v75VKEURKn6shgipHuobtM9xdTbjzy2oGbLCW4hiYgJzCZ4hoMQ2TXTJxWkw9sG",
        &[8],          // top-level PumpSwap instruction
        8783039791744, // quote_amount_in (SOL spent)
        7426425826,    // base_amount_out (tokens received)
    )
    .await
    .expect("PumpSwaps Buy parser flow test failed");

    // Sell instruction (called via aggregator)
    common::assert_pumpswap_sell_parser_flow(
        "3V41y1wkTjYDQ4UAz6gaLT8h7v75VKEURKn6shgipHuobtM9xdTbjzy2oGbLCW4hiYgJzCZ4hoMQ2TXTJxWkw9sG",
        &[5],          // top-level PumpSwap instruction
        7621520530,    // base_amount_in (tokens spent)
        9016142101046, // quote_amount_out (SOL received)
    )
    .await
    .expect("PumpSwaps Sell parser flow test failed");

    // 202505 data - Buy instruction with base event fields only
    common::assert_pumpswap_buy_parser_flow(
        "MyZn74cbZJfethB6Ps9MtgcS19h7euFRvSZA4eefjEvwUK1YAnfhJzbXwwWhxqeu3ooXgPjgJUuREMRHB5fH29z",
        &[5, 0],
        29611164,
        3950276478,
    )
    .await
    .expect("PumpSwaps Buy (May 2025) parser flow test failed");

    // IDL expects 25 bytes (including track_volume: OptionBool) but on-chain data is 24 bytes
    // This is an IDL version mismatch - the parser needs to be regenerated with correct IDL
    common::assert_pumpswap_buy_parser_flow(
        "4toJQMzqWiCNJpTHKdyBXNwrxThVbiAntihtJmZd19Pf2uxqe56W313ZxoGLmXW1wfUEKaW4aiTrygFJksFEDMDD",
        &[3, 0],      // top-level #3 → inner PumpSwap instruction
        247500000,    // quote_amount_in
        165156835142, // base_amount_out
    )
    .await
    .expect("PumpSwaps BuyExactQuoteIn parser flow test failed");
}

/// Test Jupiter parser flow with full InstructionParser.parse()
#[tokio::test]
async fn test_jupiter_swap_events_parser_flow() {
    init_tracing();

    // Jupiter Route instruction with SwapEvent
    common::assert_jupiter_parser_flow(
        "vRYNRDqsLW7Kk6GHPzxYytqxHDzDMTGfD2SD3fYsUZgA7o7yhDp97orn9uVoZKjWXYYoNMnGb4jzz2GxZuD2UV1",
        &[2, 0],    // top-level Jupiter instruction
        0,          // event_index: first SwapEvent
        2092119022, // input_amount
        472821137,  // output_amount
    )
    .await
    .expect("Jupiter SwapEvent parser flow test failed");
}

/// Test Meteora DLMM parser flow with full InstructionParser.parse()
#[tokio::test]
async fn test_meteora_dlmm_swap_events_parser_flow() {
    init_tracing();

    // Meteora DLMM Swap instruction (called via aggregator)
    common::assert_meteora_dlmm_parser_flow(
        "2DfsmTYvMqKwXDBEicEtqLeFfyJ43LLPeVbg8NSjzsQZuhzKzUmZP9XeQLm8C9z8pu3z5paHdJKcnQrw3PA8s4hs",
        &[1],      // top-level #1 Meteora instruction
        116033029, // amount_in
        521092597, // amount_out
    )
    .await
    .expect("Meteora DLMM SwapEvent parser flow test failed");
}

/// Test PumpFun parser flow with full InstructionParser.parse()
#[tokio::test]
async fn test_pumpfun_trade_events_parser_flow() {
    init_tracing();

    // PumpFun Buy instruction (called via aggregator)
    // NOTE: For buy, source = sol_amount, dest = token_amount
    common::assert_pumpfun_parser_flow(
        "22K6ixTV6Hk9mk9dBqbTcixYw2LXNYEDyiENzLMTs4S8z9i3WRjYLpXDM2mE75nP36moUZ5MeH1ahTvUvYP9L8jH",
        &[4, 0],       // top-level #4 → inner PumpFun instruction
        246875000,     // sol_amount (source for buy)
        4087530976228, // token_amount (dest for buy)
    )
    .await
    .expect("PumpFun TradeEvent parser flow test failed");
}
