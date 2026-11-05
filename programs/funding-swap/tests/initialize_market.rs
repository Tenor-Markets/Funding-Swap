#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, MAX_ORDERS};
use funding_swap::CreateMarketArgs;
use funding_swap_math::rate::MAX_DECIMALS;
use funding_swap_math::MAX_TICK_STEP;
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_market_initialized, create_uninitialized_market_account, default_market_args, fixed_raw,
    initialize_market, initialize_market_instruction, new_svm, read_governance,
    send_initialize_market, send_tx, send_without_signer, setup_governance,
};

#[test]
fn test_initialize_market_succeeds_for_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let expected_market_id = read_governance(&svm, setup.governance)
        .expect("read governance")
        .next_market_id;

    let args = default_market_args();

    let market = initialize_market(&mut svm, &setup, args.clone()).expect("initialize market");

    assert_market_initialized(&svm, setup.governance, market, &args, expected_market_id);
}

#[test]
fn test_initialize_market_validates_config() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let base = default_market_args();

    type Mutator = fn(&mut CreateMarketArgs);

    let cases: [(&str, Mutator, StateError); 15] = [
        (
            "tick scale",
            |args| args.tick_scale = MAX_DECIMALS + 1,
            StateError::InvalidAssetDecimals,
        ),
        (
            "underlying decimals",
            |args| args.underlying_decimals = MAX_DECIMALS + 1,
            StateError::InvalidAssetDecimals,
        ),
        (
            "negative start",
            |args| args.start_timestamp = -1,
            StateError::InvalidMarketTiming,
        ),
        (
            "nonpositive maturity",
            |args| args.maturity_timestamp = 0,
            StateError::InvalidMarketTiming,
        ),
        (
            "start at maturity",
            |args| args.start_timestamp = args.maturity_timestamp,
            StateError::InvalidMarketTiming,
        ),
        (
            "zero open interest cap",
            |args| args.hard_gross_open_interest_cap_atoms = 0,
            StateError::InvalidMarketLimits,
        ),
        (
            "zero mark window",
            |args| args.mark_observation_window_seconds = 0,
            StateError::InvalidMarketLimits,
        ),
        (
            "zero margin threshold",
            |args| args.margin_time_threshold_seconds = 0,
            StateError::InvalidMarketLimits,
        ),
        (
            "negative initial margin",
            |args| args.initial_margin_factor_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "negative maintenance margin",
            |args| args.maintenance_margin_factor_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "negative rate floor",
            |args| args.rate_floor_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "negative order rate bound",
            |args| args.order_rate_bound_factor_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "negative taker fee",
            |args| args.taker_fee_rate_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "negative liquidation fee",
            |args| args.liquidation_fee_rate_raw = -1,
            StateError::InvalidMarketFactors,
        ),
        (
            "initial margin below maintenance",
            |args| {
                args.initial_margin_factor_raw = fixed_raw(1, 10);
                args.maintenance_margin_factor_raw = fixed_raw(1, 5);
            },
            StateError::InvalidMarketFactors,
        ),
    ];

    for (_name, mutate, expected_error) in cases {
        let mut args = base.clone();
        mutate(&mut args);

        let market = create_uninitialized_market_account(&mut svm).expect("market account");
        let result = send_initialize_market(&mut svm, &setup, market, args);

        assert_instruction_error(result, 0, u32::from(expected_error));
    }

    let tick_step_cases: [Mutator; 2] = [
        |args| args.tick_step = 0,
        |args| args.tick_step = MAX_TICK_STEP + 1,
    ];

    for mutate in tick_step_cases {
        let mut args = base.clone();
        mutate(&mut args);

        let market = create_uninitialized_market_account(&mut svm).expect("market account");
        let result = send_initialize_market(&mut svm, &setup, market, args);

        assert_instruction_error(result, 0, u32::from(StateError::InvalidTickStep));
    }

    let order_book_limit_cases: [Mutator; 4] = [
        |args| args.minimum_order_size_atoms = 0,
        |args| {
            args.minimum_order_size_atoms = 2;
            args.maximum_order_size_atoms = 1;
        },
        |args| args.maximum_open_orders_per_subaccount = 0,
        |args| args.maximum_open_orders_per_subaccount = MAX_ORDERS as u8 + 1,
    ];

    for mutate in order_book_limit_cases {
        let mut args = base.clone();
        mutate(&mut args);

        let market = create_uninitialized_market_account(&mut svm).expect("market account");
        let result = send_initialize_market(&mut svm, &setup, market, args);

        assert_instruction_error(result, 0, u32::from(StateError::InvalidOrderBookLimits));
    }
}

#[test]
fn test_initialize_market_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    setup_governance(&mut svm).expect("governance setup");

    let attacker = airdrop_keypair(&mut svm);
    let market = create_uninitialized_market_account(&mut svm).expect("market account");

    let ix = initialize_market_instruction(attacker.pubkey(), market, default_market_args());

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_initialize_market_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);
    let market = create_uninitialized_market_account(&mut svm).expect("market account");

    let ix = initialize_market_instruction(authority.pubkey(), market, default_market_args());

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
