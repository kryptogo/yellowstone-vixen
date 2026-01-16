# PumpSwap Parser Upgrade Guide

## Upgrade Prerequisite

Before applying the modifications below, complete these steps:

1. **Update IDL**: Ensure the IDL file in `parser-generator/pumpswap` folder is updated with the latest version
2. **Run code generation**: Execute `script.cjs` to generate the base parser files

   ```bash
   cd parser-generator/pumpswap
   node script.cjs
   ```

3. **Verify no errors**: Confirm the generated code compiles without errors
4. **Apply modifications**: Once the base code is generated successfully, apply the modifications documented below

---

## Checklist

Use this checklist to track progress when applying modifications:

- [ ] 1. Forward Compatible Parsing (event + accounts)
- [ ] 2. Self CPI Parse
- [ ] 3. IX Index as Shared Data Feature
- [ ] 4. Adding New Buy/Sell Instructions
- [ ] 5. Fix Panic on Short Data

**Order of Application**: These changes are non-conflicting and can be applied in any order.

**Verification**: After each change, run the following tests to verify correctness:

```bash
# Unit tests
cargo test -p pump-swaps-parser

# Integration tests (specific signatures)
cargo test test_pump_swaps_specific_signatures --features integration-test
```

---

## Detailed Changes

### 1. Forward Compatible Parsing

refer to commit `4c03f87`, `0a23b55`

**Problem**: Failed to parse old version Pump AMM events

**Modified Files**:

- `src/generated_parser/proto_helpers.rs`
- `src/generated_sdk/types/buy_event.rs`
- `src/generated_sdk/types/sell_event.rs`

**Design Approach: Fixed-Prefix Parsing (Forward Compatible)**

The fix uses a "fixed-prefix parsing" approach - only parse the first N bytes that we care about, ignoring any additional data appended by future versions.

#### 1.1 Core Concept

```
Event Data Layout:
┌─────────────────────────────────────┬─────────────────────┬─────────────────┐
│  First 304 bytes (base fields)      │  Extra fields       │  Future fields  │
│  timestamp, amounts, pubkeys...     │  coin_creator...    │  ???            │
└─────────────────────────────────────┴─────────────────────┴─────────────────┘
                 ↑
                 │
    We ONLY parse this part, regardless of total length
```

**Key Insight**: As long as the first 304 bytes (base fields) remain stable across versions, we can safely parse them without worrying about what comes after.

#### 1.2 Implementation

**Step 1: Define minimal struct with only the fields we need (304 bytes)**

```rust
// BuyEventBaseVersion only contains the first 304 bytes of data
pub struct BuyEventBaseVersion {
    pub timestamp: i64,                              // 8 bytes
    pub base_amount_out: u64,                        // 8 bytes
    pub max_quote_amount_in: u64,                    // 8 bytes
    pub user_base_token_reserves: u64,               // 8 bytes
    pub user_quote_token_reserves: u64,              // 8 bytes
    pub pool_base_token_reserves: u64,               // 8 bytes
    pub pool_quote_token_reserves: u64,              // 8 bytes
    pub quote_amount_in: u64,                        // 8 bytes
    pub lp_fee_basis_points: u64,                    // 8 bytes
    pub lp_fee: u64,                                 // 8 bytes
    pub protocol_fee_basis_points: u64,              // 8 bytes
    pub protocol_fee: u64,                           // 8 bytes
    pub quote_amount_in_with_lp_fee: u64,            // 8 bytes
    pub user_quote_amount_in: u64,                   // 8 bytes
    pub pool: Pubkey,                                // 32 bytes
    pub user: Pubkey,                                // 32 bytes
    pub user_base_token_account: Pubkey,             // 32 bytes
    pub user_quote_token_account: Pubkey,            // 32 bytes
    pub protocol_fee_recipient: Pubkey,              // 32 bytes
    pub protocol_fee_recipient_token_account: Pubkey, // 32 bytes
    // Extra fields (coin_creator, etc.) are ignored - we only parse base fields
}
// Total: 304 bytes
```

**Step 2: Slice data to fixed length before parsing**

```rust
// Only parse first 304 bytes, ignoring anything after
if buy_event_data.len() >= 304 {
    if let Ok(event) = BuyEventBaseVersion::try_from_slice(&buy_event_data[..304]) {
        return Some(event);
    }
}
```

This works for ANY version:

| Data Source | Total Length | Parsed Length | Result |
|-------------|--------------|---------------|--------|
| Old event   | 304 bytes    | 304 bytes     | OK - exact match |
| New event   | 352 bytes    | 304 bytes     | OK - extra 48 bytes ignored |
| Future event| 400+ bytes   | 304 bytes     | OK - extra bytes ignored |

#### 1.3 Proto Output Handling

Since proto schema expects all fields including `coin_creator`, use default values for fields not in base version:

```rust
proto_def::BuyEvent {
    // Base fields - use actual values
    timestamp: event.timestamp,
    base_amount_out: event.base_amount_out,
    // ... other base fields ...

    // Extra fields - use default values (not parsed from base version)
    coin_creator: "11111111111111111111111111111111".to_string(),
    coin_creator_fee_basis_points: 0,
    coin_creator_fee: 0,
}
```

#### 1.4 Same Pattern for Instruction Accounts

The same "fixed-prefix" approach applies to instruction accounts parsing:

```
Accounts Layout:
┌─────────────────────────────────────┬─────────────────────────────────────┬─────────────────┐
│  First 17 accounts (required)       │  Accounts 18-19 (optional)          │  Future accounts│
│  pool, user, global_config...       │  coin_creator_vault_ata/authority   │  ???            │
└─────────────────────────────────────┴─────────────────────────────────────┴─────────────────┘
                 ↑
                 │
    Only REQUIRE first 17, treat rest as optional
```

**Implementation:**

```rust
// Only require minimum 17 accounts
let expected_accounts_len = 17;
check_min_accounts_req(accounts_len, expected_accounts_len)?;

// Parse first 17 required accounts
let pool = next_account(accounts)?;
let user = next_account(accounts)?;
// ... accounts 3-17 ...
let program = next_account(accounts)?;

// Optional accounts 18-19 (use default if not present)
let coin_creator_vault_ata = if accounts_len >= 18 {
    next_account(accounts)?
} else {
    solana_pubkey::Pubkey::default()
};
let coin_creator_vault_authority = if accounts_len >= 19 {
    next_account(accounts)?
} else {
    solana_pubkey::Pubkey::default()
};
// Any accounts 20+ are silently ignored
```

This works for ANY version:

| Tx Version | Account Count | Result |
|------------|---------------|--------|
| V1 (old)   | 17 accounts   | OK - all required accounts present |
| V2 (new)   | 19 accounts   | OK - optional accounts parsed |
| V3 (future)| 20+ accounts  | OK - extra accounts ignored |

#### 1.5 Why This Design Works

| Benefit | Explanation |
|---------|-------------|
| **Forward Compatible** | Future versions can add fields/accounts at the end without breaking parser |
| **No Version Detection** | Don't need to detect V1/V2/V3 - just parse what we need |
| **Borsh Compatible** | Borsh deserializes exactly the bytes needed by struct definition |
| **Fail-Safe** | Only requirement: minimum data length or account count |

#### 1.6 Limitation

If Pump AMM changes the **order or type** of existing fields/accounts, this approach breaks. However, this is extremely unlikely as it would break backward compatibility for all existing integrations.

---

### 2. Self CPI Parse

refer to commit `996cde7`, `b623a6f`

**Problem**: Self CPI log was being treated as invalid instruction discriminator

**Modified Files**:

- `src/generated_parser/instructions_parser.rs`

**Specific Changes**: Added handler for self CPI log discriminator, returning `ParseError::Filtered`:

```rust
// self cpi log
[0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d] => {
    Err(yellowstone_vixen_core::ParseError::Filtered)
},
```

---

### 3. IX Index as Shared Data Feature

refer to commit `ca594fa`

**Modified Files**:

- `src/generated_parser/instructions_parser.rs`

**Specific Changes**: Added `ix_index` field under `shared-data` feature:

```rust
#[cfg(feature = "shared-data")]
let ix_index = ix.ix_index;

// ...

ix.map(|ix| InstructionUpdateOutput {
    parsed_ix: ix,
    shared_data,
    ix_index,  // new field
})
```

---

### 4. Adding New Buy/Sell Instructions

refer to commit `5e40992`, `dfc30d9`, `9c6b739`

**When to apply**: These modifications should be applied when adding new buy/sell instruction variants (e.g., `buy_exact_quote_in`, `sell_exact_base_in`, etc.).

**Modified Files**:

- `Cargo.toml`
- `src/generated_parser/instructions_parser.rs`
- `src/generated_parser/proto_helpers.rs`
- `src/generated_sdk/types/buy_event.rs`
- `src/generated_sdk/types/sell_event.rs`

#### 4.1 Use deserialize_checked_swap

**Problem**: Swap parsing errors were being logged unnecessarily, making it hard to identify real issues.

**Solution**: Use `deserialize_checked_swap` instead of `deserialize_checked`:

```rust
// Before
let de_ix_data: BuyIxData = deserialize_checked(ix_data, &ix_discriminator)?;

// After
let de_ix_data: BuyIxData = yellowstone_vixen_core::deserialize_checked_swap(
    ix_data,
    &ix_discriminator,
    "Buy",
    deserialize_checked,
)?;
```

#### 4.2 Parse DEX Swap Events

1. **Modified PumpAmmProgramIx enum**: Added `Option<BuyEvent>` / `Option<SellEvent>` parameters to Buy/Sell:

   ```rust
   pub enum PumpAmmProgramIx {
       Buy(BuyIxAccounts, BuyIxData, Option<BuyEvent>),  // added event
       Sell(SellIxAccounts, SellIxData, Option<SellEvent>),  // added event
       // ...
   }
   ```

2. **Filter Jupiter/OKX aggregator**: Return `ParseError::Filtered` when parent program is known aggregator:

   ```rust
   // Helper function to check if program is a known aggregator
   fn is_known_aggregator(program: &solana_pubkey::Pubkey) -> bool {
       const JUPITER_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
       const OKX_DEX: &str = "6m2CDdhRgxpH4WjvdzxAYbGxwdGUz5MziiL5jek2kBma";
       const OKX_DEX_V2: &str = "okxDvRDQcyPsz4D6xwAMCjYSxpkLABsYYHUqoVqVsuq";

       let program_str = program.to_string();
       program_str == JUPITER_V6 || program_str == OKX_DEX || program_str == OKX_DEX_V2
   }

   // In parse function
   if ix.parent_program.as_ref().is_some_and(is_known_aggregator) {
       return Err(yellowstone_vixen_core::ParseError::Filtered);
   }
   ```

3. **Parse BuyEvent/SellEvent from inner instructions**:

   ```rust
   let buy_event = ix
       .inner
       .iter()
       .find_map(|inner_ix| BuyEvent::from_inner_instruction_data(&inner_ix.data));
   ```

4. **Define BuyEventBaseVersion/SellEventBaseVersion struct**: Only parse base fields (304 bytes), ignoring any extra fields added in future versions. See [Section 1.2](#12-implementation) for full struct definition.

   ```rust
   // In src/generated_sdk/types/buy_event.rs
   pub struct BuyEventBaseVersion {
       pub timestamp: i64,
       pub base_amount_out: u64,
       // ... (304 bytes total, see Section 1.2 for complete struct)
   }
   ```

5. **Added CPI log prefix and discriminator constants**:

   ```rust
   pub const DISCRIMINATOR: [u8; 8] = [0x67, 0xf4, 0x52, 0x1f, 0x2c, 0xf5, 0x77, 0x77];
   pub const CPI_LOG_PREFIX: [u8; 8] = [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d];
   ```

6. **Added `from_inner_instruction_data` method**: Parses event data from CPI logs

   ```rust
   // In src/generated_sdk/types/buy_event.rs
   impl BuyEventBaseVersion {
       pub fn from_inner_instruction_data(data: &[u8]) -> Option<Self> {
           // Check for CPI log prefix
           if data.len() < 8 || &data[..8] != &CPI_LOG_PREFIX {
               return None;
           }
           let event_data = &data[8..];

           // Check for event discriminator
           if event_data.len() < 8 || &event_data[..8] != &DISCRIMINATOR {
               return None;
           }
           let buy_event_data = &event_data[8..];

           // Parse base version (first 304 bytes)
           if buy_event_data.len() >= 304 {
               if let Ok(event) = BuyEventBaseVersion::try_from_slice(&buy_event_data[..304]) {
                   return Some(event);
               }
           }
           None
       }
   }
   ```

7. **Proto helpers support**: Implement `IntoProto` for base version with default values for extra fields

   ```rust
   // In src/generated_parser/proto_helpers.rs
   impl IntoProto<proto_def::BuyEvent> for BuyEventBaseVersion {
       fn into_proto(self) -> proto_def::BuyEvent {
           proto_def::BuyEvent {
               // Base fields - use actual values
               timestamp: self.timestamp,
               base_amount_out: self.base_amount_out,
               max_quote_amount_in: self.max_quote_amount_in,
               // ... other base fields ...

               // Extra fields - use default values
               coin_creator: "11111111111111111111111111111111".to_string(),
               coin_creator_fee_basis_points: 0,
               coin_creator_fee: 0,
           }
       }
   }
   ```

---

### 5. Fix Panic on Short Data

refer to commit `9f7f68f`, `4805c8d`

**Problem**: Panic when instruction data length is less than 8 bytes

**Modified Files**:

- `src/generated_parser/instructions_parser.rs`

**Specific Changes**: Added length check before parsing discriminator:

```rust
if ix.data.len() < 8 {
    return Err(yellowstone_vixen_core::ParseError::from(
        "Instruction data too short".to_owned(),
    ));
}
```
