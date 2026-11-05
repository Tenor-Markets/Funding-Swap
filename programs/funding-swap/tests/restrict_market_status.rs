#![cfg(feature = "test-helpers")]

use funding_swap::state::{
    GovernanceCategory, StateError, MARKET_ACTIVE, MARKET_CLOSE_ONLY, MARKET_ORDERS_ONLY,
    MARKET_PAUSED,
};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error, assert_market_status,
    assert_transaction_success, default_market_args, initialize_market, new_svm,
    restrict_market_status_instruction, send_tx, send_without_signer, set_market_status,
    setup_active_market, setup_governance,
};

#[test]
fn test_restrict_market_status_enforces_emergency_transitions() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let authority = setup.authority(GovernanceCategory::Emergency);

    let allowed_cases = [
        (MARKET_ACTIVE, MARKET_CLOSE_ONLY),
        (MARKET_ACTIVE, MARKET_ORDERS_ONLY),
        (MARKET_ACTIVE, MARKET_PAUSED),
        (MARKET_CLOSE_ONLY, MARKET_ORDERS_ONLY),
        (MARKET_CLOSE_ONLY, MARKET_PAUSED),
        (MARKET_ORDERS_ONLY, MARKET_PAUSED),
    ];

    for (from, to) in allowed_cases {
        set_market_status(&mut svm, market, from).expect("set market status");

        let ix = restrict_market_status_instruction(authority.pubkey(), market, to);

        svm.expire_blockhash();

        assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));
        assert_market_status(&svm, market, to);
    }

    let rejected_cases = [
        (MARKET_CLOSE_ONLY, MARKET_ACTIVE),
        (MARKET_ORDERS_ONLY, MARKET_ACTIVE),
        (MARKET_PAUSED, MARKET_ACTIVE),
        (MARKET_ORDERS_ONLY, MARKET_CLOSE_ONLY),
        (MARKET_PAUSED, MARKET_CLOSE_ONLY),
        (MARKET_PAUSED, MARKET_ORDERS_ONLY),
    ];

    for (from, to) in rejected_cases {
        set_market_status(&mut svm, market, from).expect("set market status");

        let ix = restrict_market_status_instruction(authority.pubkey(), market, to);

        svm.expire_blockhash();

        assert_instruction_error(
            send_tx(&mut svm, authority, vec![ix], &[]),
            0,
            u32::from(StateError::InvalidMarketStatusTransition),
        );
    }
}

#[test]
fn test_restrict_market_status_fails_while_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let authority = setup.authority(GovernanceCategory::Emergency);
    let ix = restrict_market_status_instruction(authority.pubkey(), market, MARKET_CLOSE_ONLY);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_restrict_market_status_fails_when_signer_is_not_emergency_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let attacker = airdrop_keypair(&mut svm);
    let ix = restrict_market_status_instruction(attacker.pubkey(), market, MARKET_CLOSE_ONLY);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_restrict_market_status_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let market =
        setup_active_market(&mut svm, &setup, default_market_args()).expect("active market");

    let authority = setup.authority(GovernanceCategory::Emergency);
    let payer = airdrop_keypair(&mut svm);
    let ix = restrict_market_status_instruction(authority.pubkey(), market, MARKET_CLOSE_ONLY);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
