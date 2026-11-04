#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError};
use solana_sdk::{pubkey::Pubkey, signer::Signer};

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_governance_initialized,
    assert_instruction_error, assert_transaction_success, governance_args,
    initialize_governance_instruction, new_svm, send_tx, send_without_signer,
    setup_upgrade_authority,
};

#[test]
fn test_initialize_governance_succeeds_for_program_upgrade_authority() {
    let mut svm = new_svm().expect("svm");

    let authority = setup_upgrade_authority(&mut svm).expect("upgrade authority");

    let args = governance_args(authority.pubkey());
    let ix = initialize_governance_instruction(authority.pubkey(), args.clone());

    assert_transaction_success(send_tx(&mut svm, &authority, vec![ix], &[]));

    assert_governance_initialized(&svm, &args);
}

#[test]
fn test_initialize_governance_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let authority = setup_upgrade_authority(&mut svm).expect("upgrade authority");
    let payer = airdrop_keypair(&mut svm);

    let ix =
        initialize_governance_instruction(authority.pubkey(), governance_args(authority.pubkey()));

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

#[test]
fn test_initialize_governance_fails_when_signer_is_not_program_upgrade_authority() {
    let mut svm = new_svm().expect("svm");

    setup_upgrade_authority(&mut svm).expect("upgrade authority");

    let attacker = airdrop_keypair(&mut svm);

    let ix =
        initialize_governance_instruction(attacker.pubkey(), governance_args(attacker.pubkey()));

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_initialize_governance_fails_when_args_name_a_different_approved_authority() {
    let mut svm = new_svm().expect("svm");

    let authority = setup_upgrade_authority(&mut svm).expect("upgrade authority");

    let mut args = governance_args(authority.pubkey());
    args.authorities[GovernanceCategory::AuthorityAdministration.index()] = Pubkey::new_unique();

    let ix = initialize_governance_instruction(authority.pubkey(), args);

    assert_instruction_error(
        send_tx(&mut svm, &authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}
