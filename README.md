# On-curve deposits for a PDA wallet

**Goal:** Give an off-curve smart-wallet PDA `P` one on-curve deposit address
`D` for SOL, legacy SPL tokens, and Token-2022 tokens.

**How:** Grind `D = create_with_seed(P, seed, TOKEN_PROGRAM_ID)` until on-curve,
where `TOKEN_PROGRAM_ID` is legacy Tokenkeg, then initialize `D` as a legacy
SPL multisig whose signer is `P`. Legacy and Token-2022 assets use separate
ATAs derived with their respective token programs, but both are owned by `D`;
Token-2022 accepts a legacy Token-owned multisig authority. Withdraw tokens
through the mint's token program and SOL through legacy Tokenkeg's
`WithdrawExcessLamports`, signing for `P` via `invoke_signed`.

**Rent on mainnet-beta (18 September 2026):** `D` (355 bytes) requires
2,453,640 lamports (**0.00245364 SOL**). This is a rent-exempt reserve, not a
transaction fee.

The LiteSVM test currently exercises the legacy-token path and uses
`with_sigverify(false)` to stand in for the real wallet program's
`invoke_signed` PDA signature.

**Optional sweeper:** Make `D` a 1-of-2 multisig with members `[P, S]`, where
`S` is a sweeper PDA. Its program invokes the relevant token program with `S`
signed only after verifying that the destination is the canonical smart-wallet
account or ATA. This lets a keeper trigger sweeps without giving the keeper's
key custody of the funds.

```sh
cargo test --test oncurve_deposit -- --nocapture
```
# pda-deposit-oncurve
