#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, MARKET_ACTIVE};
use solana_sdk::signer::Signer;

mod common;
use common::{
    activate_market, activate_market_instruction, airdrop_keypair, assert_account_not_signer,
    assert_instruction_error, assert_market_status, default_market_args, initialize_market,
    new_svm, send_tx, send_without_signer, set_clock, set_market_source_seeded, setup_governance,
};

#[test]
fn test_activate_market_succeeds_and_rejects_repeat() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let args = default_market_args();

    let market = initialize_market(&mut svm, &setup, args).expect("initialize market");

    set_market_source_seeded(&mut svm, market, true).expect("seed market source");

    assert_market_status(&svm, market, funding_swap::state::MARKET_PENDING);

    let (_, _, seeded) = common::read_market_source(&svm, market).expect("read market source");
    assert!(seeded, "source_seeded flag was not patched");

    activate_market(&mut svm, &setup, market).expect("activate market");

    assert_market_status(&svm, market, MARKET_ACTIVE);

    let authority = setup.authority(GovernanceCategory::Resources);
    let repeat = activate_market_instruction(authority.pubkey(), market);

    svm.expire_blockhash();

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![repeat], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_market_fails_when_source_is_not_seeded() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = activate_market_instruction(authority.pubkey(), market);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_market_fails_when_start_has_passed() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let args = default_market_args();
    let start_timestamp = args.start_timestamp;

    let market = initialize_market(&mut svm, &setup, args).expect("initialize market");

    set_market_source_seeded(&mut svm, market, true).expect("seed market source");
    set_clock(&mut svm, start_timestamp, 0);

    let authority = setup.authority(GovernanceCategory::Resources);
    let ix = activate_market_instruction(authority.pubkey(), market);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::MarketAlreadyStarted),
    );
}

#[test]
fn test_activate_market_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    set_market_source_seeded(&mut svm, market, true).expect("seed market source");

    let attacker = airdrop_keypair(&mut svm);
    let ix = activate_market_instruction(attacker.pubkey(), market);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_activate_market_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let market =
        initialize_market(&mut svm, &setup, default_market_args()).expect("initialize market");

    set_market_source_seeded(&mut svm, market, true).expect("seed market source");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);

    let ix = activate_market_instruction(authority.pubkey(), market);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
