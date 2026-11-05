#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError};
use solana_sdk::signer::Signer;

mod common;
use common::{
    activate_market, activate_market_membership, activate_market_membership_instruction,
    airdrop_keypair, assert_account_exists, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_counts, create_margin_group, default_market_args, fixed_raw,
    initialize_market, market_group_membership_address, new_svm, send_tx, send_without_signer,
    set_clock, set_market_source_seeded, setup_active_margin_group, setup_governance,
};

#[test]
fn test_activate_market_group_membership_succeeds() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let membership = activate_market_membership(&mut svm, &setup, margin_group, market)
        .expect("activate membership");

    assert_account_exists(&svm, membership);
    assert_margin_group_counts(&svm, margin_group, 1, 0);
}

#[test]
fn test_activate_market_group_membership_fails_when_group_is_not_active() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let authority = setup.authority(GovernanceCategory::Resources);
    let membership = market_group_membership_address(margin_group, market);
    let ix = activate_market_membership_instruction(
        authority.pubkey(),
        margin_group,
        market,
        membership,
    );

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_market_group_membership_fails_when_market_is_not_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    set_market_source_seeded(&mut svm, market, true).expect("seed market source");

    activate_market(&mut svm, &setup, market).expect("activate market");

    let authority = setup.authority(GovernanceCategory::Resources);
    let membership = market_group_membership_address(margin_group, market);
    let ix = activate_market_membership_instruction(
        authority.pubkey(),
        margin_group,
        market,
        membership,
    );

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_market_group_membership_fails_when_start_has_passed() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 4, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let args = default_market_args();
    let start_timestamp = args.start_timestamp;
    let market = initialize_market(&mut svm, &setup, args).expect("initialize market");

    set_clock(&mut svm, start_timestamp, 0);

    let authority = setup.authority(GovernanceCategory::Resources);
    let membership = market_group_membership_address(margin_group, market);
    let ix = activate_market_membership_instruction(
        authority.pubkey(),
        margin_group,
        market,
        membership,
    );

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::MarketAlreadyStarted),
    );
}

#[test]
fn test_activate_market_group_membership_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 5, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let attacker = airdrop_keypair(&mut svm);

    let membership = market_group_membership_address(margin_group, market);

    let ix =
        activate_market_membership_instruction(attacker.pubkey(), margin_group, market, membership);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_activate_market_group_membership_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 6, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);
    let membership = market_group_membership_address(margin_group, market);
    let ix = activate_market_membership_instruction(
        authority.pubkey(),
        margin_group,
        market,
        membership,
    );

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
