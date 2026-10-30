//! End-to-end checks that `settle_metered` requires an immediate cap precompile
//! followed by a claim precompile. The instruction still does not advance
//! `settled`.

#![allow(clippy::result_large_err)]

use litesvm::LiteSVM;
use payment_channels::instructions::settle_metered::DISCRIMINATOR;
use payment_channels::{
    CAP_VOUCHER_PAYLOAD_SIZE, CLAIM_VOUCHER_PAYLOAD_SIZE, PaymentChannelsError,
};
use solana_account::Account;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

use crate::common::{
    ChannelBuilder, INSTRUCTIONS_SYSVAR, PROGRAM_ID, ProgramLoader, expect_custom_err,
    read_channel, voucher::build_ed25519_ix_for_message,
};
use payment_channels::state::ChannelStatus;

const SIGNATURE_IX_INDEX_AT: usize = 4;
const PUBKEY_IX_INDEX_AT: usize = 8;
const MESSAGE_IX_INDEX_AT: usize = 14;

struct Env {
    svm: LiteSVM,
    fee_payer: Keypair,
    channel: Pubkey,
    settled: u64,
    cap_ix: Instruction,
    claim_ix: Instruction,
}

fn setup() -> Env {
    let mut svm = LiteSVM::load_program();
    let fee_payer = Keypair::new();
    svm.airdrop(&fee_payer.pubkey(), 10_000_000_000).unwrap();

    let client = Keypair::new();
    let operator = Keypair::new();
    let channel = Pubkey::new_unique();
    let settled = 250_000u64;
    let data = ChannelBuilder::new()
        .status(ChannelStatus::Open)
        .deposit(1_000_000)
        .settled(settled)
        .authorized_signer(client.pubkey())
        .build();
    svm.set_account(
        channel,
        Account {
            lamports: 10_000_000,
            data,
            owner: PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("set_account");

    let mut cap_message = [0u8; CAP_VOUCHER_PAYLOAD_SIZE];
    cap_message[0] = 0x56;
    cap_message[1] = 0x03;
    let mut claim_message = [0u8; CLAIM_VOUCHER_PAYLOAD_SIZE];
    claim_message[0] = 0x56;
    claim_message[1] = 0x04;

    Env {
        svm,
        fee_payer,
        channel,
        settled,
        cap_ix: signed_ed25519(&client, &cap_message),
        claim_ix: signed_ed25519(&operator, &claim_message),
    }
}

fn signed_ed25519(signer: &Keypair, message: &[u8]) -> Instruction {
    let signature: [u8; 64] = signer.sign_message(message).into();
    build_ed25519_ix_for_message(&signer.pubkey().to_bytes(), &signature, message)
}

fn settle_metered_ix(channel: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        PROGRAM_ID,
        &[DISCRIMINATOR],
        vec![
            AccountMeta::new(*channel, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
        ],
    )
}

fn compute_budget() -> Instruction {
    ComputeBudgetInstruction::set_compute_unit_limit(200_000)
}

fn send(
    env: &mut Env,
    ixs: &[Instruction],
) -> Result<litesvm::types::TransactionMetadata, litesvm::types::FailedTransactionMetadata> {
    let tx = Transaction::new_signed_with_payer(
        ixs,
        Some(&env.fee_payer.pubkey()),
        &[&env.fee_payer],
        env.svm.latest_blockhash(),
    );
    env.svm.send_transaction(tx)
}

fn reject(env: &mut Env, ixs: &[Instruction], expected: PaymentChannelsError) {
    expect_custom_err(send(env, ixs), expected);
    assert_settled_unchanged(env);
}

fn assert_settled_unchanged(env: &Env) {
    let settled = read_channel(&env.svm, &env.channel, |ch| ch.settled());
    assert_eq!(settled, env.settled);
}

fn redirect_index(ix: &Instruction, at: usize, target: u16) -> Instruction {
    let mut redirected = ix.clone();
    redirected.data[at..at + 2].copy_from_slice(&target.to_le_bytes());
    redirected
}

fn assert_precompile_failure_at(env: &mut Env, ixs: &[Instruction], index: u8) {
    let failed = send(env, ixs).expect_err("tx should fail");
    match failed.err {
        TransactionError::InstructionError(ix_index, _) => assert_eq!(ix_index, index),
        other => panic!("expected precompile failure at ix {index}, got {other:?}"),
    }
    assert_settled_unchanged(env);
}

fn prepared(env: &Env) -> (Instruction, Instruction, Instruction) {
    (
        env.cap_ix.clone(),
        env.claim_ix.clone(),
        settle_metered_ix(&env.channel),
    )
}

#[test]
fn pair_immediately_before_settle_metered_is_accepted() {
    let mut env = setup();
    let (cap, claim, settle) = prepared(&env);
    send(&mut env, &[cap, claim, settle]).expect("tx ok");
    assert_settled_unchanged(&env);
}

#[test]
fn compute_budget_before_the_pair_is_accepted() {
    let mut env = setup();
    let (cap, claim, settle) = prepared(&env);
    send(&mut env, &[compute_budget(), cap, claim, settle]).expect("tx ok");
    assert_settled_unchanged(&env);
}

#[test]
fn missing_both_precompiles_rejects() {
    let mut env = setup();
    let settle = settle_metered_ix(&env.channel);
    reject(
        &mut env,
        &[settle],
        PaymentChannelsError::MissingSecondEd25519Verification,
    );
}

#[test]
fn only_cap_precompile_rejects() {
    let mut env = setup();
    let (cap, _, settle) = prepared(&env);
    reject(
        &mut env,
        &[cap, settle],
        PaymentChannelsError::MissingSecondEd25519Verification,
    );
}

#[test]
fn only_claim_precompile_rejects() {
    let mut env = setup();
    let (_, claim, settle) = prepared(&env);
    reject(
        &mut env,
        &[claim, settle],
        PaymentChannelsError::MissingSecondEd25519Verification,
    );
}

#[test]
fn swapped_precompile_order_rejects() {
    let mut env = setup();
    let (cap, claim, settle) = prepared(&env);
    reject(
        &mut env,
        &[claim, cap, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn non_ed25519_cap_slot_rejects() {
    let mut env = setup();
    let (_, claim, settle) = prepared(&env);
    reject(
        &mut env,
        &[compute_budget(), claim, settle],
        PaymentChannelsError::MissingSecondEd25519Verification,
    );
}

#[test]
fn non_ed25519_claim_slot_rejects() {
    let mut env = setup();
    let (cap, _, settle) = prepared(&env);
    reject(
        &mut env,
        &[cap, compute_budget(), settle],
        PaymentChannelsError::MissingEd25519Verification,
    );
}

#[test]
fn instruction_between_the_pair_rejects() {
    let mut env = setup();
    let (cap, claim, settle) = prepared(&env);
    reject(
        &mut env,
        &[cap, compute_budget(), claim, settle],
        PaymentChannelsError::MissingSecondEd25519Verification,
    );
}

#[test]
fn earlier_pair_does_not_satisfy_the_immediate_slots() {
    let mut env = setup();
    let (cap, claim, settle) = prepared(&env);
    let extra_claim = claim.clone();
    reject(
        &mut env,
        &[cap, claim, extra_claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn wrong_cap_message_size_rejects() {
    let mut env = setup();
    let (_, claim, settle) = prepared(&env);
    let short_cap = signed_ed25519(&Keypair::new(), &[0x11; CAP_VOUCHER_PAYLOAD_SIZE - 1]);
    reject(
        &mut env,
        &[short_cap, claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn wrong_claim_message_size_rejects() {
    let mut env = setup();
    let (cap, _, settle) = prepared(&env);
    let long_claim = signed_ed25519(&Keypair::new(), &[0x22; CLAIM_VOUCHER_PAYLOAD_SIZE + 1]);
    reject(
        &mut env,
        &[cap, long_claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn cap_rider_byte_rejects() {
    let mut env = setup();
    let (mut cap, claim, settle) = prepared(&env);
    cap.data.push(0);
    reject(
        &mut env,
        &[cap, claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn claim_rider_byte_rejects() {
    let mut env = setup();
    let (cap, mut claim, settle) = prepared(&env);
    claim.data.push(0);
    reject(
        &mut env,
        &[cap, claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn cap_nonzero_padding_rejects() {
    let mut env = setup();
    let (mut cap, claim, settle) = prepared(&env);
    cap.data[1] = 1;
    reject(
        &mut env,
        &[cap, claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn claim_nonzero_padding_rejects() {
    let mut env = setup();
    let (cap, mut claim, settle) = prepared(&env);
    claim.data[1] = 1;
    reject(
        &mut env,
        &[cap, claim, settle],
        PaymentChannelsError::MalformedEd25519Instruction,
    );
}

#[test]
fn cap_cross_instruction_indexes_reject() {
    for at in [
        SIGNATURE_IX_INDEX_AT,
        PUBKEY_IX_INDEX_AT,
        MESSAGE_IX_INDEX_AT,
    ] {
        let mut env = setup();
        let (cap, claim, settle) = prepared(&env);
        let redirected = redirect_index(&cap, at, 0);
        reject(
            &mut env,
            &[cap, redirected, claim, settle],
            PaymentChannelsError::MalformedEd25519Instruction,
        );
    }
}

#[test]
fn claim_cross_instruction_indexes_reject() {
    for at in [
        SIGNATURE_IX_INDEX_AT,
        PUBKEY_IX_INDEX_AT,
        MESSAGE_IX_INDEX_AT,
    ] {
        let mut env = setup();
        let (cap, claim, settle) = prepared(&env);
        let redirected = redirect_index(&claim, at, 0);
        reject(
            &mut env,
            &[claim, cap, redirected, settle],
            PaymentChannelsError::MalformedEd25519Instruction,
        );
    }
}

#[test]
fn invalid_cap_signature_fails_in_the_precompile() {
    let mut env = setup();
    let (mut cap, claim, settle) = prepared(&env);
    let signature_at = payment_channels::ed25519::SIGNATURE_OFFSET;
    cap.data[signature_at..signature_at + 64].fill(0);
    assert_precompile_failure_at(&mut env, &[cap, claim, settle], 0);
}

#[test]
fn invalid_claim_signature_fails_in_the_precompile() {
    let mut env = setup();
    let (cap, mut claim, settle) = prepared(&env);
    let signature_at = payment_channels::ed25519::SIGNATURE_OFFSET;
    claim.data[signature_at..signature_at + 64].fill(0);
    assert_precompile_failure_at(&mut env, &[cap, claim, settle], 1);
}
