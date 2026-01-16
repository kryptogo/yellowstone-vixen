pub mod test_handlers;

use std::{path::PathBuf, time::Duration};

use tokio::sync::broadcast;
use yellowstone_vixen::config::{BufferConfig, VixenConfig};
use yellowstone_vixen_mock::{
    create_mock_transaction_update_with_cache, parse_instructions_from_txn_update,
};
use yellowstone_vixen_yellowstone_grpc_source::YellowstoneGrpcConfig;

/// Trait for CPI events that can be parsed from instruction data and have token changes
pub trait CpiEventParseable {
    fn from_inner_instruction_data(data: &[u8]) -> Option<Self>
    where
        Self: Sized;
    fn source_token_change(&self) -> u64;
    fn destination_token_change(&self) -> u64;
}

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

/// Asserts that a CPI event at the specified instruction indices has the expected token changes.
///
/// # Arguments
/// * `signature` - Transaction signature to fetch and parse
/// * `ix_path` - Path of instruction indices: [top_level, inner, nested_inner, ...]
/// * `expected_source_token_change` - Expected source token change value
/// * `expected_destination_token_change` - Expected destination token change value
///
/// # Example
/// ```ignore
/// // 2-level: top-level #6 → inner #5
/// assert_okx_dex_v2_cpi_event_token_changes::<SwapCpiEvent2>(sig, &[6, 5], 100, 200).await?;
///
/// // 3-level: top-level #3 → inner #2 → nested #8
/// assert_okx_dex_v2_cpi_event_token_changes::<SwapCpiEvent2>(sig, &[3, 2, 8], 100, 200).await?;
/// ```
pub async fn assert_okx_dex_v2_cpi_event_token_changes<T: CpiEventParseable>(
    signature: &str,
    ix_path: &[usize],
    expected_source_token_change: u64,
    expected_destination_token_change: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert!(
        ix_path.len() >= 2,
        "ix_path must have at least 2 indices (top-level and inner)"
    );

    let txn_update = create_mock_transaction_update_with_cache(signature)
        .await
        .expect("Failed to fetch transaction");

    let instruction_updates =
        parse_instructions_from_txn_update(&txn_update).expect("Failed to parse instructions");

    let top_level_ix = instruction_updates
        .get(ix_path[0])
        .ok_or_else(|| format!("Top-level instruction index {} not found", ix_path[0]))?;

    // Navigate through the inner instruction path
    let mut current_inner = top_level_ix
        .inner
        .get(ix_path[1])
        .ok_or_else(|| format!("Inner instruction index {} not found", ix_path[1]))?;

    for (depth, &idx) in ix_path.iter().enumerate().skip(2) {
        current_inner = current_inner.inner.get(idx).ok_or_else(|| {
            format!(
                "Nested instruction index {} at depth {} not found",
                idx, depth
            )
        })?;
    }

    let event = T::from_inner_instruction_data(&current_inner.data)
        .ok_or("Failed to parse CPI event from instruction data")?;

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

// Macro to implement CpiEventParseable for multiple types
macro_rules! impl_cpi_event_parseable {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl CpiEventParseable for $ty {
                fn from_inner_instruction_data(data: &[u8]) -> Option<Self> {
                    Self::from_inner_instruction_data(data)
                }
                fn source_token_change(&self) -> u64 {
                    self.source_token_change
                }
                fn destination_token_change(&self) -> u64 {
                    self.destination_token_change
                }
            }
        )+
    };
}

impl_cpi_event_parseable!(
    yellowstone_vixen_okx_dex_v2_parser::types::SwapCpiEvent2,
    yellowstone_vixen_okx_dex_v2_parser::types::SwapWithFeesCpiEvent2,
    yellowstone_vixen_okx_dex_v2_parser::types::SwapWithFeesCpiEventEnhanced,
    yellowstone_vixen_okx_dex_v2_parser::types::SwapTobV2CpiEvent2,
    yellowstone_vixen_okx_dex_v2_parser::types::SwapTocV2CpiEvent2,
);
