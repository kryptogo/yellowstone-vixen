/// Log-based Integration Tests
///
/// These tests verify parsers that extract swap events from transaction logs
/// using the full InstructionParser.parse() flow.
///
/// ## Log-based parsers include:
/// - Raydium AMM V4
/// - Raydium CLMM
/// - Raydium CPMM
/// - Meteora Pools
/// - Moonshot
/// - Orca Whirlpool
/// - Pancake
///
/// ## ix_path Navigation
///
/// For log-based parsers, the ix_path points to the **program instruction that emits the log**,
/// not its inner instructions.
///
/// Example: For a Raydium AMM V4 swap:
/// - `&[2]` = Top-level Raydium AMM instruction (CORRECT - logs are emitted here)
/// - `&[2, 0]` = Inner Token Program instruction (WRONG - Token Program doesn't emit swap logs)
///
#[path = "../common/mod.rs"]
mod common;

fn init_tracing() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init()
            .ok();
    });
}

// ============================================================================
// Raydium AMM V4 Tests
// ============================================================================
/// NOTE: Use https://raylogdecoder.vercel.app/ to decode the ray log to get the amount_in and amount_out
#[tokio::test]
async fn test_raydium_amm_v4_parser_flow() {
    init_tracing();

    // SwapBaseIn
    common::assert_raydium_amm_v4_parser_flow(
        "54MFrVcfzQEnfMCQo2KtRJErGBnr2rgJ7ShAQ8mpr61FdyiQsc8vuxBYqz8xGmM4C23sYcm1Wic3gJTjUf5u9Pkr",
        &[2],     // top-level Raydium AMM instruction
        32508133, // amount_in
        12795559, // out_amount
    )
    .await
    .expect("Raydium AMM V4 parser flow test failed");
}

// ============================================================================
// Raydium CLMM Tests
// ============================================================================
/// NOTE: See event section in Solscan to get the amount_in and amount_out
#[tokio::test]
async fn test_raydium_clmm_parser_flow() {
    init_tracing();

    common::assert_raydium_clmm_parser_flow(
        "nexzRp8Z5abE2pfaySm7bft7PqnTAQG64Y11gBHvzqdLUYspc84dTtQY9P6BiAMMDNYBTEBLhMDtbHoYYNgUvxS",
        &[4],         // top-level Raydium CLMM instruction
        650000000,    // amount_0/amount_1 (depends on zero_for_one)
        928319794967, // amount_1/amount_0 (depends on zero_for_one)
    )
    .await
    .expect("Raydium CLMM parser flow test failed");
}

// ============================================================================
// Raydium CPMM Tests
// ============================================================================
#[tokio::test]
async fn test_raydium_cpmm_parser_flow() {
    init_tracing();

    common::assert_raydium_cpmm_parser_flow(
        "4RoVbE9HB9GSQN1wyBRW7TJCq4ovvWyMfegQAM1Lvd3UgYWGGgJcW3GYruAi7j1poKboPCS2bK71J4iM5EUwxD6R",
        &[3],         // top-level Raydium CPMM instruction
        218686363204, // input_amount
        69520899,     // output_amount
    )
    .await
    .expect("Raydium CPMM parser flow test failed");
}

// ============================================================================
// Meteora Pools Tests
// ============================================================================
#[tokio::test]
async fn test_meteora_pools_parser_flow() {
    init_tracing();

    common::assert_meteora_pools_parser_flow(
        "2mHGPXMzxs6NtaHtbVqku9iKCBy1uAbohMk1yB1it6gku9xXnkQt7TaCh5seb66n7wsADf13MsYYutnYRNrkzbSX",
        &[0],         // top-level Meteora Pools instruction
        455036072,    // in_amount
        124965910713, // out_amount
    )
    .await
    .expect("Meteora Pools parser flow test failed");
}

// ============================================================================
// Moonshot Tests
// ============================================================================
#[tokio::test]
async fn test_moonshot_parser_flow() {
    init_tracing();

    common::assert_moonshot_parser_flow(
        "5UWcde33J3rxFusKri4UCihzq2YatSoYbVjEhm5PRbYxx7VGxh2DPAMixkfnZ5wVyoE4wZNhwMLeJCULkufRd5cn",
        &[2],          // top-level Moonshot instruction
        1965030,       // collateral_amount
        6551568276092, // amount
    )
    .await
    .expect("Moonshot parser flow test failed");
}

// ============================================================================
// Orca Whirlpool Tests
// ============================================================================
#[tokio::test]
async fn test_orca_whirlpool_parser_flow() {
    init_tracing();

    common::assert_orca_whirlpool_parser_flow(
        "N5qR3DcvdJfwk4kcCCDBMPgJdGmm8mVoXn32QxNrQovaDQCACWaDxJYVBaoUcP7gE342jvJGU2NPcu7mr9qFD9T",
        &[3],          // top-level Orca Whirlpool instruction
        1001000000,    // input_amount
        7640760418498, // output_amount
    )
    .await
    .expect("Orca Whirlpool parser flow test failed");
}

// ============================================================================
// Pancake Tests
// ============================================================================
#[tokio::test]
async fn test_pancake_parser_flow() {
    init_tracing();

    // Test with DFlow Aggregator tx, because we only filter out OKX and jupiter
    common::assert_pancake_parser_flow(
        "fwY3Gkn8Xbiz3xJPHhchLsJmSgRB8ehT3Cvf8PxTV4tXDaDFA7efmEspwUi5pCDQbBQB6HpU4oME1gJrYWZWPmF",
        &[3, 5],
        179190000,
        1260641743,
    )
    .await
    .expect("Pancake parser flow test failed");
}
