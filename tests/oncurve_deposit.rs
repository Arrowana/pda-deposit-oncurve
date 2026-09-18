use {
    litesvm::LiteSVM,
    litesvm_token::{
        get_spl_account,
        spl_token::state::{Account as TokenAccount, Multisig},
        CreateAssociatedTokenAccount, CreateMint, MintTo, Transfer, TOKEN_ID,
    },
    solana_address::Address,
    solana_keypair::Keypair,
    solana_message::Message,
    solana_native_token::LAMPORTS_PER_SOL,
    solana_program_pack::Pack,
    solana_signature::Signature,
    solana_signer::Signer,
    solana_system_interface::instruction::{
        create_account_with_seed, transfer as transfer_lamports,
    },
    solana_transaction::Transaction,
    spl_token_interface::instruction::{
        initialize_multisig2, transfer as transfer_tokens, withdraw_excess_lamports,
    },
};

const SMART_WALLET_SEED: &[u8] = b"smart-wallet";
const SWEEPER_SEED: &[u8] = b"sweeper";
const TOKEN_AMOUNT: u64 = 25_000_000;

/// Find a `create_with_seed` address that is on the Ed25519 curve.
///
/// This is deliberately client-side grinding. The seed remains at most 32
/// bytes, as required by the System Program.
fn grind_on_curve_deposit_address(base: &Address) -> (String, Address) {
    for nonce in 0_u64.. {
        let seed = format!("deposit-{nonce}");
        let candidate = Address::create_with_seed(base, &seed, &TOKEN_ID).unwrap();
        if candidate.is_on_curve() {
            return (seed, candidate);
        }
    }
    unreachable!("half of uniformly distributed addresses are expected to be on-curve")
}

/// Submit a transaction in which `phantom_signer` is marked as a signer but
/// has a default (invalid) signature.
///
/// The test VM is configured with `with_sigverify(false)`, so this models the
/// signer privilege a real sweeper program would grant its PDA through
/// `invoke_signed`. It does *not* claim that a PDA can sign a top-level
/// transaction on a real cluster.
fn send_with_phantom_signer(
    svm: &mut LiteSVM,
    payer: &Keypair,
    instructions: &[solana_message::Instruction],
    phantom_signer: &Address,
) {
    let payer_address = payer.pubkey();
    let blockhash = svm.latest_blockhash();
    let message = Message::new_with_blockhash(instructions, Some(&payer_address), &blockhash);

    let phantom_index = message
        .account_keys
        .iter()
        .position(|address| address == phantom_signer)
        .expect("phantom signer must be present in the message");
    assert!(message.is_signer(phantom_index));

    let mut transaction = Transaction {
        signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
        message,
    };
    transaction.partial_sign(&[payer], blockhash);
    svm.send_transaction(transaction).unwrap();
}

#[test]
fn on_curve_deposits_auto_sweep_to_the_smart_wallet() {
    // Signature verification is disabled only so an off-curve PDA can stand in
    // for the signer privilege that a production sweeper program would
    // obtain with invoke_signed. The System and Token programs still execute.
    let mut svm = LiteSVM::new().with_sigverify(false);
    let payer = Keypair::new();
    let payer_address = payer.pubkey();
    svm.airdrop(&payer_address, 10 * LAMPORTS_PER_SOL).unwrap();

    // P: the off-curve smart-wallet destination. It never signs this flow.
    let smart_wallet_program = Address::new_from_array([42; 32]);
    let (smart_wallet, _bump) =
        Address::find_program_address(&[SMART_WALLET_SEED], &smart_wallet_program);
    assert!(!smart_wallet.is_on_curve());

    // S: the sweeper-program PDA, deterministically scoped to P.
    let sweeper_program = Address::new_from_array([43; 32]);
    let (sweeper, _bump) =
        Address::find_program_address(&[SWEEPER_SEED, smart_wallet.as_ref()], &sweeper_program);
    assert!(!sweeper.is_on_curve());

    // D: an on-curve, Token-Program-owned address based on S.
    let (deposit_seed, deposit_address) = grind_on_curve_deposit_address(&sweeper);
    assert!(deposit_address.is_on_curve());
    assert_eq!(
        deposit_address,
        Address::create_with_seed(&sweeper, &deposit_seed, &TOKEN_ID).unwrap()
    );

    // Create D without a D signature. Required signers are only the payer and
    // base S. Then initialize D as a 1-of-1 SPL Token multisig whose only
    // member is S; P has no authority over D.
    let multisig_rent = svm.minimum_balance_for_rent_exemption(Multisig::LEN);
    let create_deposit_address = create_account_with_seed(
        &payer_address,
        &deposit_address,
        &sweeper,
        &deposit_seed,
        multisig_rent,
        Multisig::LEN as u64,
        &TOKEN_ID,
    );
    assert!(
        !create_deposit_address
            .accounts
            .iter()
            .find(|meta| meta.pubkey == deposit_address)
            .unwrap()
            .is_signer
    );

    let initialize_deposit_multisig =
        initialize_multisig2(&TOKEN_ID, &deposit_address, &[&sweeper], 1).unwrap();
    send_with_phantom_signer(
        &mut svm,
        &payer,
        &[create_deposit_address, initialize_deposit_multisig],
        &sweeper,
    );

    let multisig: Multisig = get_spl_account(&svm, &deposit_address).unwrap();
    assert!(multisig.is_initialized);
    assert_eq!(multisig.m, 1);
    assert_eq!(multisig.n, 1);
    assert_eq!(multisig.signers[0], sweeper);
    assert_eq!(svm.get_account(&deposit_address).unwrap().owner, TOKEN_ID);

    // Create a mock six-decimal stablecoin plus token accounts for the sender,
    // the on-curve deposit address, and P's canonical destination ATA.
    let mint = CreateMint::new(&mut svm, &payer)
        .decimals(6)
        .send()
        .unwrap();
    let sender_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
        .send()
        .unwrap();
    let deposit_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
        .owner(&deposit_address)
        .send()
        .unwrap();
    let receiver_ata = CreateAssociatedTokenAccount::new(&mut svm, &payer, &mint)
        .owner(&smart_wallet)
        .send()
        .unwrap();

    MintTo::new(&mut svm, &payer, &mint, &sender_ata, TOKEN_AMOUNT)
        .send()
        .unwrap();

    // Deposit tokens exactly as an exchange would: D is just the owner of its
    // ATA and does not sign the incoming transfer.
    Transfer::new(&mut svm, &payer, &mint, &deposit_ata, TOKEN_AMOUNT)
        .source(&sender_ata)
        .send()
        .unwrap();

    let deposited: TokenAccount = get_spl_account(&svm, &deposit_ata).unwrap();
    assert_eq!(deposited.owner, deposit_address);
    assert_eq!(deposited.amount, TOKEN_AMOUNT);

    // Auto-sweep: D is the token authority/multisig account and S is its only
    // signer. The sweeper program must enforce receiver_ata == ATA(P, mint)
    // before signing for S. P does not interact or sign.
    let withdraw_tokens = transfer_tokens(
        &TOKEN_ID,
        &deposit_ata,
        &receiver_ata,
        &deposit_address,
        &[&sweeper],
        TOKEN_AMOUNT,
    )
    .unwrap();
    send_with_phantom_signer(&mut svm, &payer, &[withdraw_tokens], &sweeper);

    let deposited: TokenAccount = get_spl_account(&svm, &deposit_ata).unwrap();
    let received: TokenAccount = get_spl_account(&svm, &receiver_ata).unwrap();
    assert_eq!(deposited.amount, 0);
    assert_eq!(received.amount, TOKEN_AMOUNT);

    // The same D can receive native SOL even though it is Token-Program-owned.
    let sol_deposit = LAMPORTS_PER_SOL;
    let tx = Transaction::new_signed_with_payer(
        &[transfer_lamports(
            &payer_address,
            &deposit_address,
            sol_deposit,
        )],
        Some(&payer_address),
        &[&payer],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();
    assert_eq!(
        svm.get_balance(&deposit_address).unwrap(),
        multisig_rent + sol_deposit
    );

    // WithdrawExcessLamports recognizes that the source itself is a multisig,
    // validates D with member S, sends to P, and leaves D's rent reserve.
    let receiver_before = svm.get_balance(&smart_wallet).unwrap_or(0);
    let withdraw_sol = withdraw_excess_lamports(
        &TOKEN_ID,
        &deposit_address,
        &smart_wallet,
        &deposit_address,
        &[&sweeper],
    )
    .unwrap();
    send_with_phantom_signer(&mut svm, &payer, &[withdraw_sol], &sweeper);

    assert_eq!(svm.get_balance(&deposit_address).unwrap(), multisig_rent);
    assert_eq!(
        svm.get_balance(&smart_wallet).unwrap(),
        receiver_before + sol_deposit
    );
}
