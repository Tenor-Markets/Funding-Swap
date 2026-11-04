#![cfg(feature = "test-helpers")]

use funding_swap::state::{
    AuthorityUpdate, GovernanceCategory, StateError, UpdateAuthoritiesArgs, GOVERNANCE_CATEGORIES,
};
use solana_sdk::{signature::Keypair, signer::Signer};

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_governance_updated,
    assert_instruction_error, assert_transaction_success, authority_update, new_svm,
    read_governance, send_tx, send_without_signer, setup_governance,
    update_authorities_instruction,
};

#[test]
fn test_update_authorities_succeeds_for_authority_administrator() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let governance = setup.governance;
    let authority = setup.authority(GovernanceCategory::AuthorityAdministration);
    let before = read_governance(&svm, governance).expect("read governance");

    let new_risk_authority = Keypair::new().pubkey();
    let new_fee_destination = Keypair::new().pubkey();

    let args = UpdateAuthoritiesArgs {
        authority_update: Some(authority_update(
            GovernanceCategory::RiskTightening,
            new_risk_authority,
        )),
        subaccount_creation_fee_lamports: Some(77),
        subaccount_creation_fee_destination: Some(new_fee_destination),
    };

    let ix = update_authorities_instruction(authority.pubkey(), args.clone());

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_governance_updated(&svm, governance, before, &args);
}

#[test]
fn test_update_authorities_fails_when_signer_is_not_authority_administrator() {
    let mut svm = new_svm().expect("svm");

    setup_governance(&mut svm).expect("governance setup");

    let attacker = airdrop_keypair(&mut svm);

    let args = UpdateAuthoritiesArgs {
        authority_update: Some(authority_update(
            GovernanceCategory::Deleveraging,
            attacker.pubkey(),
        )),
        subaccount_creation_fee_lamports: None,
        subaccount_creation_fee_destination: None,
    };

    let ix = update_authorities_instruction(attacker.pubkey(), args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_update_authorities_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let authority = setup.authority(GovernanceCategory::AuthorityAdministration);
    let payer = airdrop_keypair(&mut svm);

    let args = UpdateAuthoritiesArgs {
        authority_update: Some(authority_update(
            GovernanceCategory::FeesAndLiquidation,
            Keypair::new().pubkey(),
        )),
        subaccount_creation_fee_lamports: None,
        subaccount_creation_fee_destination: None,
    };

    let ix = update_authorities_instruction(authority.pubkey(), args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

#[test]
fn test_update_authorities_rotates_authority_administrator_and_revokes_old_signer() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let governance = setup.governance;
    let authority = setup.authority(GovernanceCategory::AuthorityAdministration);
    let before = read_governance(&svm, governance).expect("read governance");

    let new_administrator = Keypair::new().pubkey();

    let args = UpdateAuthoritiesArgs {
        authority_update: Some(authority_update(
            GovernanceCategory::AuthorityAdministration,
            new_administrator,
        )),
        subaccount_creation_fee_lamports: None,
        subaccount_creation_fee_destination: None,
    };

    let ix = update_authorities_instruction(authority.pubkey(), args.clone());

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_governance_updated(&svm, governance, before, &args);

    let stale_args = UpdateAuthoritiesArgs {
        authority_update: Some(authority_update(
            GovernanceCategory::Treasury,
            Keypair::new().pubkey(),
        )),
        subaccount_creation_fee_lamports: None,
        subaccount_creation_fee_destination: None,
    };

    let stale_ix = update_authorities_instruction(authority.pubkey(), stale_args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![stale_ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_update_authorities_fails_for_unknown_category() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let authority = setup.authority(GovernanceCategory::AuthorityAdministration);

    let args = UpdateAuthoritiesArgs {
        authority_update: Some(AuthorityUpdate {
            category: GOVERNANCE_CATEGORIES as u8,
            authority: Keypair::new().pubkey(),
        }),
        subaccount_creation_fee_lamports: None,
        subaccount_creation_fee_destination: None,
    };

    let ix = update_authorities_instruction(authority.pubkey(), args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidGovernanceCategory),
    );
}
