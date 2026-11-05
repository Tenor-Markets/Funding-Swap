#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, UpdateMarketRiskArgs};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_market_risk_updated, assert_transaction_success, default_market_args, initialize_market,
    new_svm, read_market_config, relax_market_risk_instruction, send_tx, send_without_signer,
    setup_active_market, setup_governance,
};

#[test]
fn test_relax_market_risk_updates_fields_and_rejects_wrong_directions() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");
    let before = read_market_config(&svm, market).expect("read market config");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);

    // Relaxing must decrease these fields, so every case below is larger than
    // the live value.
    let wrong_decrease_cases: [(&str, UpdateMarketRiskArgs); 4] = [
        (
            "initial margin",
            UpdateMarketRiskArgs {
                initial_margin_factor_raw: Some(before.initial_margin_factor.to_raw_bits() * 2),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "maintenance margin",
            UpdateMarketRiskArgs {
                maintenance_margin_factor_raw: Some(
                    before.maintenance_margin_factor.to_raw_bits() * 2,
                ),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "rate floor",
            UpdateMarketRiskArgs {
                rate_floor_raw: Some(before.rate_floor.to_raw_bits() * 2),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "margin threshold",
            UpdateMarketRiskArgs {
                margin_time_threshold_seconds: Some(before.margin_time_threshold_seconds * 2),
                ..UpdateMarketRiskArgs::default()
            },
        ),
    ];

    // Relaxing must increase these fields, so every case below is smaller than
    // the live value.
    let wrong_increase_cases: [(&str, UpdateMarketRiskArgs); 4] = [
        (
            "closing relief",
            UpdateMarketRiskArgs {
                closing_margin_relief_bound_factor_raw: Some(
                    before.closing_margin_relief_bound_factor.to_raw_bits() / 2,
                ),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "open interest cap",
            UpdateMarketRiskArgs {
                hard_gross_open_interest_cap_atoms: Some(
                    before.hard_gross_open_interest_cap_atoms / 2,
                ),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "order rate bound",
            UpdateMarketRiskArgs {
                order_rate_bound_factor_raw: Some(before.order_rate_bound_factor.to_raw_bits() / 2),
                ..UpdateMarketRiskArgs::default()
            },
        ),
        (
            "trade mark bound",
            UpdateMarketRiskArgs {
                trade_mark_update_bound_factor_raw: Some(
                    before.trade_mark_update_bound_factor.to_raw_bits() / 2,
                ),
                ..UpdateMarketRiskArgs::default()
            },
        ),
    ];

    for (_name, args) in wrong_decrease_cases.into_iter().chain(wrong_increase_cases) {
        let ix = relax_market_risk_instruction(authority.pubkey(), market, args);

        assert_instruction_error(
            send_tx(&mut svm, authority, vec![ix], &[]),
            0,
            u32::from(StateError::RiskUpdateWrongDirection),
        );
    }

    let args = UpdateMarketRiskArgs {
        initial_margin_factor_raw: Some(before.initial_margin_factor.to_raw_bits() / 2),
        maintenance_margin_factor_raw: Some(before.maintenance_margin_factor.to_raw_bits() / 2),
        closing_margin_relief_bound_factor_raw: Some(
            before.closing_margin_relief_bound_factor.to_raw_bits() * 2,
        ),
        rate_floor_raw: Some(before.rate_floor.to_raw_bits() / 2),
        hard_gross_open_interest_cap_atoms: Some(before.hard_gross_open_interest_cap_atoms * 2),
        mark_observation_window_seconds: Some(before.mark_observation_window_seconds * 2),
        margin_time_threshold_seconds: Some(before.margin_time_threshold_seconds / 2),
        order_rate_bound_factor_raw: Some(before.order_rate_bound_factor.to_raw_bits() * 2),
        trade_mark_update_bound_factor_raw: Some(
            before.trade_mark_update_bound_factor.to_raw_bits() * 2,
        ),
    };

    let ix = relax_market_risk_instruction(authority.pubkey(), market, args.clone());

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_market_risk_updated(&svm, market, before, &args);
}

#[test]
fn test_relax_market_risk_fails_while_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");
    let before = read_market_config(&svm, market).expect("read market config");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let args = UpdateMarketRiskArgs {
        rate_floor_raw: Some(before.rate_floor.to_raw_bits() / 2),
        ..UpdateMarketRiskArgs::default()
    };

    let ix = relax_market_risk_instruction(authority.pubkey(), market, args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_relax_market_risk_fails_when_signer_is_not_recovery_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");
    let before = read_market_config(&svm, market).expect("read market config");

    let attacker = airdrop_keypair(&mut svm);
    let args = UpdateMarketRiskArgs {
        rate_floor_raw: Some(before.rate_floor.to_raw_bits() / 2),
        ..UpdateMarketRiskArgs::default()
    };

    let ix = relax_market_risk_instruction(attacker.pubkey(), market, args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_relax_market_risk_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");
    let before = read_market_config(&svm, market).expect("read market config");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let payer = airdrop_keypair(&mut svm);
    let args = UpdateMarketRiskArgs {
        rate_floor_raw: Some(before.rate_floor.to_raw_bits() / 2),
        ..UpdateMarketRiskArgs::default()
    };

    let ix = relax_market_risk_instruction(authority.pubkey(), market, args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
