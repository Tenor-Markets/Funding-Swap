#![cfg(feature = "test-helpers")]

use funding_swap::state::MARKET_PENDING;

mod common;
use common::{
    airdrop_keypair, assert_market_source_seeded, assert_market_status, assert_transaction_success,
    initialize_market, jupiter_seed_market_source_instruction, jupiter_sol_market_args, new_svm,
    phoenix_seed_market_source_instruction, phoenix_sol_market_args, send_tx, set_clock,
    set_jupiter_sol_custody_account, set_phoenix_source_accounts, setup_governance,
    JUPITER_FIXTURE_CHECKPOINT, JUPITER_FIXTURE_LAST_UPDATE, JUPITER_FIXTURE_SLOT,
    PHOENIX_FIXTURE_CHECKPOINT, PHOENIX_FIXTURE_LAST_UPDATE, PHOENIX_FIXTURE_SLOT,
};

/// Seeds from the captured mainnet Jupiter SOL custody fixture at slot
/// `436_948_909` (2026-08-03T09:54:17Z).
#[test]
fn test_seed_market_source_succeeds_from_mainnet_jupiter_fixture() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        initialize_market(&mut svm, &setup, jupiter_sol_market_args()).expect("initialize market");

    set_jupiter_sol_custody_account(&mut svm).expect("set Jupiter custody fixture");
    set_clock(&mut svm, JUPITER_FIXTURE_LAST_UPDATE, JUPITER_FIXTURE_SLOT);

    let payer = airdrop_keypair(&mut svm);
    let ix = jupiter_seed_market_source_instruction(market);

    assert_transaction_success(send_tx(&mut svm, &payer, vec![ix], &[]));

    assert_market_source_seeded(
        &svm,
        market,
        JUPITER_FIXTURE_CHECKPOINT,
        JUPITER_FIXTURE_LAST_UPDATE,
    );
    assert_market_status(&svm, market, MARKET_PENDING);
}

/// Seeds from the captured mainnet Phoenix/Rise accounts at slot `436_979_123`
/// (1_785_763_629, 2026-08-03T13:27:09Z).
#[test]
fn test_seed_market_source_succeeds_from_mainnet_phoenix_fixture() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        initialize_market(&mut svm, &setup, phoenix_sol_market_args()).expect("initialize market");

    set_phoenix_source_accounts(&mut svm).expect("set Phoenix source fixtures");
    set_clock(&mut svm, PHOENIX_FIXTURE_LAST_UPDATE, PHOENIX_FIXTURE_SLOT);

    let payer = airdrop_keypair(&mut svm);
    let ix = phoenix_seed_market_source_instruction(market);

    assert_transaction_success(send_tx(&mut svm, &payer, vec![ix], &[]));

    assert_market_source_seeded(
        &svm,
        market,
        PHOENIX_FIXTURE_CHECKPOINT,
        PHOENIX_FIXTURE_LAST_UPDATE,
    );
    assert_market_status(&svm, market, MARKET_PENDING);
}
