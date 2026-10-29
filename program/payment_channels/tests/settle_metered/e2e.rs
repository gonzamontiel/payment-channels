//! End-to-end validation of the `settle_metered` no-op scaffolding.

#![allow(clippy::result_large_err)]

use litesvm::LiteSVM;
use payment_channels::instructions::settle_metered::DISCRIMINATOR;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::common::{
    ChannelBuilder, INSTRUCTIONS_SYSVAR, PROGRAM_ID, ProgramLoader, read_channel,
};
use payment_channels::state::ChannelStatus;

fn seed_channel(
    svm: &mut LiteSVM,
    channel: &Pubkey,
    deposit: u64,
    settled: u64,
    authorized_signer: &Pubkey,
) {
    let data = ChannelBuilder::new()
        .status(ChannelStatus::Open)
        .deposit(deposit)
        .settled(settled)
        .authorized_signer(*authorized_signer)
        .build();
    svm.set_account(
        *channel,
        Account {
            lamports: 10_000_000,
            data,
            owner: PROGRAM_ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("set_account");
}

fn build_settle_metered_ix(channel: &Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        PROGRAM_ID,
        &[DISCRIMINATOR],
        vec![
            AccountMeta::new(*channel, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
        ],
    )
}

fn read_settled(svm: &LiteSVM, channel: &Pubkey) -> u64 {
    read_channel(svm, channel, |ch| ch.settled())
}

#[test]
fn settle_metered_noop_leaves_settled_unchanged() {
    let mut svm = LiteSVM::load_program();
    let fee_payer = Keypair::new();
    svm.airdrop(&fee_payer.pubkey(), 10_000_000_000).unwrap();

    let signer = Keypair::new();
    let channel = Pubkey::new_unique();
    let settled_before = 250_000u64;
    seed_channel(&mut svm, &channel, 1_000_000, settled_before, &signer.pubkey());

    let settle_metered_ix = build_settle_metered_ix(&channel);
    let tx = Transaction::new_signed_with_payer(
        &[settle_metered_ix],
        Some(&fee_payer.pubkey()),
        &[&fee_payer],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).expect("tx ok");

    assert_eq!(read_settled(&svm, &channel), settled_before);
}
