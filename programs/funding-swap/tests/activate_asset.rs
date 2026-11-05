#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, ASSET_ACTIVE};
use solana_sdk::signer::Signer;

mod common;
use common::{
    activate_asset, activate_asset_instruction, airdrop_keypair, assert_account_not_signer,
    assert_asset_status, assert_instruction_error, create_asset, create_asset_args, new_svm,
    send_tx, send_without_signer, setup_governance, ASSET_PENDING,
};

#[test]
fn test_activate_asset_succeeds_and_rejects_repeat() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let asset = create_asset(&mut svm, &setup, create_asset_args()).expect("create asset");

    assert_asset_status(&svm, asset, ASSET_PENDING);

    activate_asset(&mut svm, &setup, asset).expect("activate asset");

    assert_asset_status(&svm, asset, ASSET_ACTIVE);

    let authority = setup.authority(GovernanceCategory::Resources);
    let repeat = activate_asset_instruction(authority.pubkey(), asset);

    svm.expire_blockhash();

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![repeat], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_asset_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let asset = create_asset(&mut svm, &setup, create_asset_args()).expect("create asset");

    let attacker = airdrop_keypair(&mut svm);
    let ix = activate_asset_instruction(attacker.pubkey(), asset);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_activate_asset_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let asset = create_asset(&mut svm, &setup, create_asset_args()).expect("create asset");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);
    let ix = activate_asset_instruction(authority.pubkey(), asset);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}
