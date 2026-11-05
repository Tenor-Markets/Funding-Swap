#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, UpdateFeesAndLiquidationArgs};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_market_fees_updated, assert_transaction_success, default_market_args, fixed_raw,
    initialize_market, new_svm, read_market_config, send_tx, send_without_signer,
    setup_active_market, setup_governance, update_fees_and_liquidation_instruction,
};

#[test]
fn test_update_fees_and_liquidation_succeeds() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");
    let before = read_market_config(&svm, market).expect("read market config");

    let authority = setup.authority(GovernanceCategory::FeesAndLiquidation);

    let args = UpdateFeesAndLiquidationArgs {
        taker_fee_rate_raw: Some(fixed_raw(2, 1_000)),
        annualized_settlement_fee_rate_raw: Some(fixed_raw(2, 100)),
        liquidation_base_incentive_raw: Some(fixed_raw(2, 100)),
        liquidation_incentive_slope_raw: Some(fixed_raw(2, 100)),
        liquidation_fee_rate_raw: Some(fixed_raw(2, 100)),
    };

    let ix = update_fees_and_liquidation_instruction(authority.pubkey(), market, args.clone());

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_market_fees_updated(&svm, market, before, &args);
}

#[test]
fn test_update_fees_and_liquidation_rejects_negative_values() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let authority = setup.authority(GovernanceCategory::FeesAndLiquidation);

    type Mutator = fn(&mut UpdateFeesAndLiquidationArgs);

    let cases: [Mutator; 5] = [
        |args| args.taker_fee_rate_raw = Some(-1),
        |args| args.annualized_settlement_fee_rate_raw = Some(-1),
        |args| args.liquidation_base_incentive_raw = Some(-1),
        |args| args.liquidation_incentive_slope_raw = Some(-1),
        |args| args.liquidation_fee_rate_raw = Some(-1),
    ];

    for mutate in cases {
        let mut args = UpdateFeesAndLiquidationArgs::default();
        mutate(&mut args);

        let ix = update_fees_and_liquidation_instruction(authority.pubkey(), market, args);

        assert_instruction_error(
            send_tx(&mut svm, authority, vec![ix], &[]),
            0,
            u32::from(StateError::InvalidMarketFactors),
        );
    }
}

#[test]
fn test_update_fees_and_liquidation_fails_while_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let authority = setup.authority(GovernanceCategory::FeesAndLiquidation);
    let args = UpdateFeesAndLiquidationArgs {
        taker_fee_rate_raw: Some(fixed_raw(2, 1_000)),
        ..UpdateFeesAndLiquidationArgs::default()
    };

    let ix = update_fees_and_liquidation_instruction(authority.pubkey(), market, args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_update_fees_and_liquidation_fails_when_signer_is_not_fee_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let attacker = airdrop_keypair(&mut svm);
    let args = UpdateFeesAndLiquidationArgs {
        taker_fee_rate_raw: Some(fixed_raw(2, 1_000)),
        ..UpdateFeesAndLiquidationArgs::default()
    };

    let ix = update_fees_and_liquidation_instruction(attacker.pubkey(), market, args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_update_fees_and_liquidation_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let authority = setup.authority(GovernanceCategory::FeesAndLiquidation);
    let payer = airdrop_keypair(&mut svm);
    let args = UpdateFeesAndLiquidationArgs {
        taker_fee_rate_raw: Some(fixed_raw(2, 1_000)),
        ..UpdateFeesAndLiquidationArgs::default()
    };

    let ix = update_fees_and_liquidation_instruction(authority.pubkey(), market, args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
